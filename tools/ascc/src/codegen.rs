//! Asc codegen — lowers the type-checked AST to LLVM IR via inkwell.
//!
//! All Asc-level guarantees (distinct types, literal narrowing, capability and
//! channel rules once they exist) are ERASED here: LLVM sees plain data and
//! control flow, exactly like Rust erases borrow-check information.
//!
//! Kernel-mode target configuration (CLAUDE.md "STACK" guarantees):
//! - freestanding triple `x86_64-unknown-none-elf`, no runtime, no implicit heap
//! - no red zone on every function (enum attribute)
//! - SSE/MMX/AVX target features disabled — no FPU instructions emitted
//!   (Asc has no float literals at all; this is defense in depth)
//! - kernel code model (top-2GiB addressing for higher-half linking)
//! - static relocation
//!
//! Optimization: inkwell exposes only codegen levels 0..=3 (no SizeLevel), so
//! `--kernel` uses OptLevel 1 ("Less") as the closest available size-first
//! profile. A custom pass pipeline with SizeLevel=1 is a Phase 1 item.

use std::collections::HashMap;
use std::ffi::CString;

use inkwell::attributes::AttributeLoc;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple,
};
use inkwell::types::{
    AsTypeRef, BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType,
};
use inkwell::values::{
    AsValueRef, BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue,
};
use inkwell::{AddressSpace, IntPredicate, OptimizationLevel};

use llvm_sys::core::{
    LLVMGetEnumAttributeKindForName, LLVMGetInlineAsm, LLVMBuildCall2, LLVMBuildGEP2,
};
use llvm_sys::transforms::pass_builder::{
    LLVMCreatePassBuilderOptions, LLVMDisposePassBuilderOptions, LLVMRunPasses,
    LLVMPassBuilderOptionsSetLoopInterleaving, LLVMPassBuilderOptionsSetLoopVectorization,
    LLVMPassBuilderOptionsSetMergeFunctions, LLVMPassBuilderOptionsSetSLPVectorization,
};
use llvm_sys::error::{LLVMConsumeError, LLVMGetErrorMessage};
use llvm_sys::LLVMInlineAsmDialect;

use crate::ast::*;
use crate::typeck::{CheckedModule, ConstVal, Ty};

#[derive(Debug)]
pub struct CodegenError {
    pub msg: String,
}

type CResult<T> = Result<T, CodegenError>;

impl From<inkwell::builder::BuilderError> for CodegenError {
    fn from(e: inkwell::builder::BuilderError) -> Self {
        cerr(e.to_string())
    }
}

impl From<inkwell::values::InstructionValueError> for CodegenError {
    fn from(e: inkwell::values::InstructionValueError) -> Self {
        cerr(format!("{e:?}"))
    }
}

/// Value slot: `None` = void (no value).
type V<'ctx> = Option<BasicValueEnum<'ctx>>;

fn cerr(msg: impl Into<String>) -> CodegenError {
    CodegenError { msg: msg.into() }
}

const KERNEL_TRIPLE: &str = "x86_64-unknown-none-elf";
/// Features disabled for kernel-mode units: every vector/FPU extension we can
/// name, so a miscompile that reaches vector instructions fails loudly instead
/// of silently requiring an enabled FPU.
const KERNEL_FEATURES: &str =
    "-mmx,-sse,-sse2,-sse3,-ssse3,-sse4.1,-sse4.2,-avx,-avx2,-avx512f,-x87";

pub fn emit_object(
    checked: &CheckedModule,
    kernel_mode: bool,
    obj_path: &std::path::Path,
) -> CResult<()> {
    let ctx = Context::create();
    let module = generate(&ctx, checked, kernel_mode)?;
    if kernel_mode && std::env::var("ASCC_SKIP_OPT").is_err() {
        optimize_kernel(&module)?;
    }
    finish_and_write(&module, kernel_mode, obj_path)
}

pub fn emit_ir(checked: &CheckedModule, kernel_mode: bool) -> CResult<String> {
    let ctx = Context::create();
    let module = generate(&ctx, checked, kernel_mode)?;
    if kernel_mode {
        optimize_kernel(&module)?;
    }
    Ok(module
        .print_to_string()
        .to_str()
        .map(|s| s.to_owned())
        .map_err(|_| cerr("invalid UTF-8 in IR"))?)
}

/// Size-first optimization for kernel-mode units (true `-Os` semantics).
///
/// LLVM 20's C API gained LLVMPassBuilderOptionsSetOptLevel/SetSizeLevel, but
/// Debian's libLLVM-19 does not export them, so the size level cannot be set
/// through options here. Equivalent effect, measured in QA:
///   - `optsize` attribute on every function (inliner/unroller honor it),
///   - `default<O2>` pipeline with loop/SLP vectorization OFF and
///     MergeFunctions ON.
fn optimize_kernel(module: &Module) -> CResult<()> {
    // optsize on every defined function before the pipeline runs
    let kind = unsafe {
        let cname = b"optsize\0";
        LLVMGetEnumAttributeKindForName(cname.as_ptr().cast(), (cname.len() - 1) as usize)
    };
    if kind == 0 {
        return Err(cerr("optsize attribute kind not found in this LLVM build"));
    }
    let attr = module.get_context().create_enum_attribute(kind as u32, 0);
    for f in module.get_functions() {
        f.add_attribute(AttributeLoc::Function, attr);
    }

    let tm = kernel_target_machine()?;
    unsafe {
        let opts = LLVMCreatePassBuilderOptions();
        LLVMPassBuilderOptionsSetLoopInterleaving(opts, 0);
        LLVMPassBuilderOptionsSetLoopVectorization(opts, 0);
        LLVMPassBuilderOptionsSetSLPVectorization(opts, 0);
        LLVMPassBuilderOptionsSetMergeFunctions(opts, 1);
        let passes = c"default<O2>";
        let err = LLVMRunPasses(module.as_mut_ptr(), passes.as_ptr(), tm.as_mut_ptr(), opts);
        LLVMDisposePassBuilderOptions(opts);
        if !err.is_null() {
            let msg = LLVMGetErrorMessage(err);
            let text = std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned();
            LLVMConsumeError(err);
            return Err(cerr(format!("kernel optimization pipeline failed: {text}")));
        }
    }
    Ok(())
}

/// Kernel-mode target machine: freestanding triple, every vector/FPU feature
/// disabled, static relocation, kernel code model. Shared by the optimizer
/// and object emission.
fn kernel_target_machine() -> CResult<inkwell::targets::TargetMachine> {
    Target::initialize_x86(&InitializationConfig::default());
    let triple = TargetTriple::create(KERNEL_TRIPLE);
    let target = Target::from_triple(&triple).map_err(|e| cerr(e.to_string()))?;
    target
        .create_target_machine(
            &triple,
            "generic",
            KERNEL_FEATURES,
            OptimizationLevel::Default,
            RelocMode::Static,
            CodeModel::Kernel,
        )
        .ok_or_else(|| cerr("failed to create x86_64 kernel target machine"))
}

fn finish_and_write(module: &Module, kernel_mode: bool, obj_path: &std::path::Path) -> CResult<()> {
    let tm = if kernel_mode {
        // IR is already optimized; codegen level stays Default (-Os parity)
        kernel_target_machine()?
    } else {
        Target::initialize_x86(&InitializationConfig::default());
        let triple = TargetTriple::create("x86_64-unknown-none-elf");
        let target = Target::from_triple(&triple).map_err(|e| cerr(e.to_string()))?;
        target
            .create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::None,
                RelocMode::Static,
                CodeModel::Kernel,
            )
            .ok_or_else(|| cerr("failed to create x86_64 target machine"))?
    };

    module.verify().map_err(|e| cerr(e.to_string()))?;
    tm.write_to_file(module, FileType::Object, obj_path)
        .map_err(|e| cerr(e.to_string()))?;
    Ok(())
}

struct Codegen<'a, 'ctx> {
    checked: &'a CheckedModule<'a>,
    ctx: &'ctx Context,
    module: Module<'ctx>,
    builder: inkwell::builder::Builder<'ctx>,
    /// const name → global holding string bytes
    str_globals: HashMap<String, PointerValue<'ctx>>,
    /// dedup for anonymous string literals: raw bytes → global pointer
    anon_globals: HashMap<Vec<u8>, PointerValue<'ctx>>,
    /// current function locals: name → alloca
    locals: HashMap<String, PointerValue<'ctx>>,
    /// current function locals: name → Asc type
    local_tys: HashMap<String, Ty>,
    /// return type of the function being generated
    cur_ret: Ty,
}

pub fn generate<'a>(
    ctx: &'a Context,
    checked: &'a CheckedModule<'a>,
    _kernel_mode: bool,
) -> CResult<Module<'a>> {
    let module = ctx.create_module("asc");
    let builder = ctx.create_builder();

    let mut cg = Codegen {
        checked,
        ctx,
        module,
        builder,
        str_globals: HashMap::new(),
        anon_globals: HashMap::new(),
        locals: HashMap::new(),
        local_tys: HashMap::new(),
        cur_ret: Ty::Void,
    };

    // materialize globals for all string consts up front
    let str_consts: Vec<(String, Vec<u8>)> = cg
        .checked
        .consts
        .iter()
        .filter_map(|(k, v)| match &v.value {
            ConstVal::Str(bytes) => Some((k.clone(), bytes.clone())),
            ConstVal::Int(_) => None,
        })
        .collect();
    for (name, bytes) in str_consts {
        let g = cg.string_global(&bytes);
        cg.str_globals.insert(name, g);
    }

    for decl in &cg.checked.module.decls {
        if let Decl::Function {
            name,
            params,
            body,
            ..
        } = decl
        {
            cg.gen_function(name, params, body)?;
        }
    }

    Ok(cg.module)
}

impl<'a, 'ctx> Codegen<'a, 'ctx> {
    // ---- types ---------------------------------------------------------

    fn llvm_type(&self, ty: &Ty) -> CResult<BasicTypeEnum<'ctx>> {
        Ok(match ty {
            Ty::U8 | Ty::I8 => self.ctx.i8_type().into(),
            Ty::U16 | Ty::I16 => self.ctx.i16_type().into(),
            Ty::U32 | Ty::I32 => self.ctx.i32_type().into(),
            Ty::U64 | Ty::I64 => self.ctx.i64_type().into(),
            Ty::Bool => self.ctx.bool_type().into(),
            Ty::Pointer { .. } => self.ctx.ptr_type(AddressSpace::default()).into(),
            // Distinct types erase to their base — zero-cost by construction.
            Ty::Distinct { base, .. } => self.llvm_type(base)?,
            Ty::Struct(idx) => self.struct_llt(*idx)?,
            Ty::Array { elem, len } => match self.llvm_type(elem)? {
                BasicTypeEnum::IntType(t) => t.array_type(*len as u32).into(),
                BasicTypeEnum::FloatType(_) => {
                    return Err(cerr("floats cannot appear in Asc code"))
                }
                BasicTypeEnum::PointerType(t) => t.array_type(*len as u32).into(),
                BasicTypeEnum::ArrayType(t) => t.array_type(*len as u32).into(),
                BasicTypeEnum::StructType(t) => t.array_type(*len as u32).into(),
                BasicTypeEnum::VectorType(t) => t.array_type(*len as u32).into(),
                BasicTypeEnum::ScalableVectorType(_) => {
                    return Err(cerr("scalable vectors cannot appear in Asc code"))
                }
            },
            Ty::Void | Ty::Never => {
                return Err(cerr("void/never have no value representation"))
            }
        })
    }

    fn struct_llt(&self, idx: usize) -> CResult<BasicTypeEnum<'ctx>> {
        let def = &self.checked.structs[idx];
        let mut field_tys = Vec::with_capacity(def.fields.len());
        for f in &def.fields {
            field_tys.push(self.llvm_type(&f.ty)?);
        }
        Ok(self.ctx.struct_type(&field_tys, def.packed).into())
    }

    fn int_type_of(&self, ty: &Ty) -> CResult<inkwell::types::IntType<'ctx>> {
        match self.llvm_type(ty)? {
            BasicTypeEnum::IntType(t) => Ok(t),
            other => Err(cerr(format!(
                "expected integer LLVM type, got {:?}",
                other
            ))),
        }
    }

    fn resolve_te(&self, te: &TypeExpr) -> CResult<Ty> {
        match te {
            TypeExpr::Named { name, span: _ } => {
                let prim = match name.as_str() {
                    "u8" => Ty::U8,
                    "u16" => Ty::U16,
                    "u32" => Ty::U32,
                    "u64" => Ty::U64,
                    "i8" => Ty::I8,
                    "i16" => Ty::I16,
                    "i32" => Ty::I32,
                    "i64" => Ty::I64,
                    "bool" => Ty::Bool,
                    "void" => Ty::Void,
                    "never" => Ty::Never,
                    _ => self
                        .checked
                        .named_types
                        .get(name)
                        .cloned()
                        .ok_or_else(|| cerr(format!("unknown type `{}`", name)))?,
                };
                Ok(prim)
            }
            TypeExpr::Pointer {
                pointee,
                volatile,
                ..
            } => Ok(Ty::Pointer {
                pointee: Box::new(self.resolve_te(pointee)?),
                volatile: *volatile,
            }),
            TypeExpr::Array { len, elem, .. } => Ok(Ty::Array {
                elem: Box::new(self.resolve_te(elem)?),
                len: self.eval_const_len(len)?,
            }),
        }
    }

    /// Mirrors typeck's array-length evaluator: literals, char literals,
    /// integer const references, and `+ - * /` of those.
    fn eval_const_len(&self, e: &Expr) -> CResult<u64> {
        match e {
            Expr::Int(v, _) => Ok(*v),
            Expr::Char(c, _) => Ok(*c as u64),
            Expr::Ident(name, _) => match self.checked.consts.get(name) {
                Some(info) => match info.value {
                    ConstVal::Int(v) if v >= 0 => Ok(v as u64),
                    _ => Err(cerr(format!("const `{}` is not a non-negative integer", name))),
                },
                None => Err(cerr(format!("unknown const `{}` in array length", name))),
            },
            Expr::Binary { op, lhs, rhs, .. } => {
                let a = self.eval_const_len(lhs)?;
                let b = self.eval_const_len(rhs)?;
                match op {
                    BinOp::Add => a.checked_add(b),
                    BinOp::Sub => a.checked_sub(b),
                    BinOp::Mul => a.checked_mul(b),
                    BinOp::Div if b != 0 => Some(a / b),
                    _ => None,
                }
                .ok_or_else(|| cerr("array length arithmetic overflow/underflow"))
            }
            _ => Err(cerr("array length must be a compile-time constant")),
        }
    }

    // ---- globals ---------------------------------------------------------

    fn string_global(&mut self, bytes: &[u8]) -> PointerValue<'ctx> {
        if let Some(g) = self.anon_globals.get(bytes) {
            return *g;
        }
        let mut data = bytes.to_vec();
        data.push(0); // NUL terminator per spec §2
        let arr = self.ctx.const_string(&data, false);
        let ty = arr.get_type();
        let g = self
            .module
            .add_global(ty, Some(AddressSpace::default()), "asc.str");
        g.set_initializer(&arr);
        g.set_constant(true);
        g.set_unnamed_addr(true);
        // private: string literals are module-local; external linkage would
        // make same-named globals collide across compilation units
        g.set_linkage(Linkage::Private);
        g.set_alignment(1);
        let ptr = g.as_pointer_value();
        self.anon_globals.insert(bytes.to_vec(), ptr);
        ptr
    }

    // ---- functions ---------------------------------------------------------

    fn apply_kernel_attrs(&self, f: FunctionValue<'ctx>) {
        // LangRef spelling is `noredzone` — the hyphenated form does not resolve
        let kind = unsafe {
            let cname = b"noredzone\0";
            LLVMGetEnumAttributeKindForName(cname.as_ptr().cast(), (cname.len() - 1) as usize)
        };
        if kind == 0 {
            eprintln!("ascc: warning: noredzone attribute kind not found in this LLVM build");
            return;
        }
        let attr = self.ctx.create_enum_attribute(kind as u32, 0);
        f.add_attribute(AttributeLoc::Function, attr);
    }

    /// Resolve a callee to a FunctionValue, emitting a declaration (external
    /// linkage, no body) for bodyless prototypes and forward references.
    fn get_or_declare_function(
        &mut self,
        name: &str,
        sig: &crate::typeck::FnSig,
    ) -> CResult<FunctionValue<'ctx>> {
        if let Some(f) = self.module.get_function(name) {
            return Ok(f);
        }
        let mut param_types = Vec::new();
        for p in &sig.params {
            param_types.push(self.llvm_type(p)?);
        }
        let param_meta: Vec<BasicMetadataTypeEnum> = param_types.iter().map(|t| (*t).into()).collect();
        let llvm_ret: Option<BasicTypeEnum> = match &sig.ret {
            Ty::Void | Ty::Never => None,
            t => Some(self.llvm_type(t)?),
        };
        let fnty: FunctionType<'ctx> = match llvm_ret {
            Some(r) => r.fn_type(&param_meta, false),
            None => self.ctx.void_type().fn_type(&param_meta, false),
        };
        let f = self.module.add_function(name, fnty, Some(Linkage::External));
        self.apply_kernel_attrs(f);
        Ok(f)
    }

    fn gen_function(&mut self, name: &str, params: &[Param], body: &Block) -> CResult<()> {
        let sig = self
            .checked
            .fns
            .get(name)
            .cloned()
            .ok_or_else(|| cerr(format!("internal: no signature for `{}`", name)))?;
        self.cur_ret = sig.ret.clone();

        let mut param_types = Vec::new();
        for p in &sig.params {
            param_types.push(self.llvm_type(p)?);
        }
        let param_meta: Vec<BasicMetadataTypeEnum> = param_types.iter().map(|t| (*t).into()).collect();

        let llvm_ret: Option<BasicTypeEnum> = match &sig.ret {
            Ty::Void | Ty::Never => None,
            t => Some(self.llvm_type(t)?),
        };

        let fnty: FunctionType<'ctx> = match llvm_ret {
            Some(r) => r.fn_type(&param_meta, false),
            None => self.ctx.void_type().fn_type(&param_meta, false),
        };

        let f = self.module.add_function(name, fnty, Some(Linkage::External));
        self.apply_kernel_attrs(f);

        let entry = self.ctx.append_basic_block(f, "entry");
        self.builder.position_at_end(entry);

        // bind parameters through allocas (uniform addressing; the backend's
        // allocator handles spilling/promotion)
        self.locals.clear();
        self.local_tys.clear();
        for (i, p) in params.iter().enumerate() {
            let pty = sig.params[i].clone();
            let llty = self.llvm_type(&pty)?;
            let alloca = self.builder.build_alloca(llty, &p.name)?;
            let val = f.get_nth_param(i as u32).unwrap();
            self.builder.build_store(alloca, val)?;
            self.locals.insert(p.name.clone(), alloca);
            self.local_tys.insert(p.name.clone(), pty);
        }

        let terminated = self.gen_block(body)?;

        if !terminated {
            match sig.ret {
                Ty::Void => {
                    self.builder.build_return(None)?;
                }
                Ty::Never => {
                    self.builder.build_unreachable()?;
                }
                ref other => {
                    return Err(cerr(format!(
                        "function `{}` can fall off its end without returning `{}`",
                        name,
                        other.display()
                    )))
                }
            }
        }

        Ok(())
    }

    /// Generates statements; returns whether control flow is already terminated.
    fn gen_block(&mut self, block: &Block) -> CResult<bool> {
        for stmt in &block.stmts {
            if self.gen_stmt(stmt)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn gen_stmt(&mut self, stmt: &Stmt) -> CResult<bool> {
        match stmt {
            Stmt::Let { name, init, ty, .. } => {
                let declared = match ty {
                    Some(te) => Some(self.resolve_te(te)?),
                    None => None,
                };
                let llty = match (&declared, init) {
                    (Some(d), _) => self.llvm_type(d)?,
                    (None, Some(e)) => self.llvm_type(&self.ty_of_expr(e)?)?,
                    (None, None) => {
                        return Err(cerr(
                            "untyped uninitialized binding reached codegen",
                        ))
                    }
                };
                let alloca = self.builder.build_alloca(llty, name)?;
                if let Some(e) = init {
                    // annotated lets decay arrays to pointers here too
                    let v = match &declared {
                        Some(d) => self.gen_value_for(e, d)?,
                        None => self.gen_expr(e)?,
                    };
                    if let Some(v) = v {
                        self.builder.build_store(alloca, v)?;
                    }
                }
                let stored_ty = declared.clone().unwrap_or_else(|| {
                    let e_ref = init
                        .as_ref()
                        .expect("internal: let without annotation or init");
                    self.ty_of_expr(e_ref).expect("typed initializer")
                });
                self.locals.insert(name.clone(), alloca);
                self.local_tys.insert(name.clone(), stored_ty);
                Ok(false)
            }
            Stmt::Assign { target, value, .. } => {
                let want = self.ty_of_expr(target)?;
                let v = self
                    .gen_value_for(value, &want)?
                    .expect("value assignment with void rhs");
                match target {
                    Expr::Ident(n, _) => {
                        let slot = *self
                            .locals
                            .get(n)
                            .ok_or_else(|| cerr(format!("assignment to unknown local `{}`", n)))?;
                        self.builder.build_store(slot, v)?;
                    }
                    Expr::Index { base, index, .. } => {
                        let (ptr, pointee_ty) = self.gen_index_ptr(base, index)?;
                        self.store_maybe_volatile(ptr, v, &pointee_ty)?;
                    }
                    Expr::Field { base, field, .. } => {
                        let (ptr, fty) = self.gen_field_ptr(base, field)?;
                        self.store_maybe_volatile(ptr, v, &fty)?;
                    }
                    _ => return Err(cerr("invalid assignment target reached codegen")),
                }
                Ok(false)
            }
            Stmt::Expr(e) => {
                self.gen_expr(e)?;
                Ok(false)
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                let c = self.gen_bool(cond)?;
                let f = self.current_fn()?;
                let then_bb = self.ctx.append_basic_block(f, "if.then");
                let end_bb = self.ctx.append_basic_block(f, "if.end");
                let else_bb = else_body
                    .as_ref()
                    .map(|_| self.ctx.append_basic_block(f, "if.else"))
                    .unwrap_or(end_bb);
                self.builder.build_conditional_branch(c, then_bb, else_bb)?;

                self.builder.position_at_end(then_bb);
                if !self.gen_block(then_body)? {
                    self.builder.build_unconditional_branch(end_bb)?;
                }

                if let Some(els) = else_body {
                    self.builder.position_at_end(else_bb);
                    if !self.gen_block(els)? {
                        self.builder.build_unconditional_branch(end_bb)?;
                    }
                }

                self.builder.position_at_end(end_bb);
                Ok(false)
            }
            Stmt::While { cond, body, .. } => {
                let f = self.current_fn()?;
                let cond_bb = self.ctx.append_basic_block(f, "loop.cond");
                let body_bb = self.ctx.append_basic_block(f, "loop.body");
                let end_bb = self.ctx.append_basic_block(f, "loop.end");

                self.builder.build_unconditional_branch(cond_bb)?;
                self.builder.position_at_end(cond_bb);
                let c = self.gen_bool(cond)?;
                self.builder.build_conditional_branch(c, body_bb, end_bb)?;

                self.builder.position_at_end(body_bb);
                self.gen_block(body)?;
                self.builder.build_unconditional_branch(cond_bb)?;

                self.builder.position_at_end(end_bb);
                Ok(false)
            }
            Stmt::Return { value, .. } => match value {
                Some(e) => {
                    let want = self.cur_ret.clone();
                    let v = self
                        .gen_value_for(e, &want)?
                        .expect("returning void expression");
                    let want_llt = self.llvm_type(&want)?;
                    let cv = self.coerce_value(v, want_llt, &want)?;
                    match cv {
                        BasicValueEnum::IntValue(iv) => {
                            self.builder.build_return(Some(&iv))?;
                        }
                        BasicValueEnum::PointerValue(p) => {
                            self.builder.build_return(Some(&p))?;
                        }
                        other => {
                            self.builder.build_return(Some(&other))?;
                        }
                    }
                    Ok(true)
                }
                None => {
                    self.builder.build_return(None)?;
                    Ok(true)
                }
            },
            Stmt::Asm(block) => {
                self.gen_asm(block)?;
                Ok(false)
            }
        }
    }

    fn store_maybe_volatile(
        &mut self,
        ptr: PointerValue<'ctx>,
        v: BasicValueEnum<'ctx>,
        ty: &Ty,
    ) -> CResult<()> {
        let inst = self.builder.build_store(ptr, v)?;
        if matches!(ty, Ty::Pointer { volatile: true, .. }) {
            inst.set_volatile(true)?;
        }
        Ok(())
    }

    // ---- expressions -------------------------------------------------------

    fn ty_of_expr(&self, e: &Expr) -> CResult<Ty> {
        self.checked
            .expr_tys
            .get(&(e as *const Expr as usize))
            .cloned()
            .ok_or_else(|| cerr("internal: expression type missing from typeck annotations"))
    }

    /// Narrow/adjust a produced value to the wanted Asc type's LLVM form.
    /// Only integer width changes occur; typeck already proved them valid.
    fn coerce_value(
        &mut self,
        v: BasicValueEnum<'ctx>,
        want_llt: BasicTypeEnum<'ctx>,
        want_ty: &Ty,
    ) -> CResult<BasicValueEnum<'ctx>> {
        match (v, want_llt) {
            (BasicValueEnum::IntValue(iv), BasicTypeEnum::IntType(dt)) => {
                if iv.get_type() == dt {
                    Ok(iv.into())
                } else {
                    let signed = want_ty.is_signed();
                    Ok(self
                        .builder
                        .build_int_cast_sign_flag(iv, dt, signed, "coerce")?
                        .into())
                }
            }
            (o, _) => Ok(o),
        }
    }

    /// Value of `e` coerced for a context expecting `want`: when the context
    /// wants a pointer and `e` is an array, emit its address (decay) instead
    /// of loading the whole aggregate.
    fn gen_value_for(&mut self, e: &Expr, want: &Ty) -> CResult<V<'ctx>> {
        if let Ty::Pointer { .. } = want {
            if matches!(self.ty_of_expr(e)?, Ty::Array { .. }) {
                let p = self.gen_lvalue_addr(e)?;
                return Ok(Some(p.into()));
            }
        }
        self.gen_expr(e)
    }

    fn gen_bool(&mut self, e: &Expr) -> CResult<IntValue<'ctx>> {
        match self.gen_expr(e)? {
            Some(BasicValueEnum::IntValue(iv))
                if iv.get_type().get_bit_width() == 1 =>
            {
                Ok(iv)
            }
            _ => Err(cerr("expected bool value")),
        }
    }

    fn gen_pointer(&mut self, e: &Expr) -> CResult<PointerValue<'ctx>> {
        match self.gen_expr(e)? {
            Some(BasicValueEnum::PointerValue(p)) => Ok(p),
            _ => Err(cerr("expected pointer value")),
        }
    }

    /// Address of an lvalue: a local, an array/pointer element, or a struct
    /// field (through a struct local or via pointer auto-deref). Powers `&`,
    /// array decay, and field/index access on non-pointer bases.
    fn gen_lvalue_addr(&mut self, e: &Expr) -> CResult<PointerValue<'ctx>> {
        match e {
            Expr::Ident(n, _) => Ok(self
                .locals
                .get(n)
                .copied()
                .ok_or_else(|| cerr(format!("internal: address of unknown local `{}`", n)))?),
            Expr::Index { base, index, .. } => Ok(self.gen_index_ptr(base, index)?.0),
            Expr::Field { base, field, .. } => Ok(self.gen_field_ptr(base, field)?.0),
            other => Err(cerr(format!(
                "internal: no lvalue address for {}",
                expr_kind(other)
            ))),
        }
    }

    fn gen_index_ptr(
        &mut self,
        base: &Expr,
        index: &Expr,
    ) -> CResult<(PointerValue<'ctx>, Ty)> {
        let base_ty = self.ty_of_expr(base)?;
        // Array base: GEP into the local's alloca (address of the array, not
        // a loaded value). Pointer base: load the pointer value, then GEP.
        let bp = match &base_ty {
            Ty::Array { .. } => self.gen_lvalue_addr(base)?,
            Ty::Pointer { .. } | Ty::Distinct { .. } => self.gen_pointer(base)?,
            ref o => return Err(cerr(format!("cannot index into `{}`", o.display()))),
        };
        let iv = match self.gen_expr(index)? {
            Some(BasicValueEnum::IntValue(iv)) => iv,
            _ => return Err(cerr("index must be an integer")),
        };
        let idx = if iv.get_type().get_bit_width() == 64 {
            iv
        } else {
            self.builder
                .build_int_cast_sign_flag(iv, self.ctx.i64_type(), false, "idx")?
        };
        let pointee_ty = base_ty.pointee_for_store();
        let elem_llt = self.llvm_type(&pointee_ty)?;
        let mut indices = [idx.as_value_ref()];
        let res = unsafe {
            LLVMBuildGEP2(
                self.builder.as_mut_ptr(),
                elem_llt.as_type_ref(),
                bp.as_value_ref(),
                indices.as_mut_ptr(),
                indices.len() as u32,
                c"elem".as_ptr(),
            )
        };
        Ok((unsafe { PointerValue::new(res) }, base_ty.pointee_for_store()))
    }

    fn gen_field_ptr(
        &mut self,
        base: &Expr,
        field: &str,
    ) -> CResult<(PointerValue<'ctx>, Ty)> {
        // `.field` on a struct LOCAL takes its address directly; through a
        // pointer-to-struct (Go-style auto-deref) it loads the pointer first.
        let sty_idx = match self.ty_of_expr(base)? {
            Ty::Struct(i) => {
                let bp = self.gen_lvalue_addr(base)?;
                return self.field_gep(bp, i, field);
            }
            Ty::Pointer { pointee, .. } => match *pointee {
                Ty::Struct(i) => i,
                ref o => {
                    return Err(cerr(format!(
                        "field access through pointer to non-struct `{}`",
                        o.display()
                    )))
                }
            },
            Ty::Distinct { base: b, name } => match *b {
                Ty::Pointer { pointee, .. } => match *pointee {
                    Ty::Struct(i) => i,
                    ref o => {
                        return Err(cerr(format!(
                            "field access through `{}` (pointer to non-struct `{}`)",
                            name,
                            o.display()
                        )))
                    }
                },
                ref o => return Err(cerr(format!("not a pointer: {}", o.display()))),
            },
            ref o => return Err(cerr(format!("not a struct: {}", o.display()))),
        };
        let bp = self.gen_pointer(base)?;
        self.field_gep(bp, sty_idx, field)
    }

    /// GEP `[0, field_index]` into a struct at address `bp`.
    fn field_gep(
        &mut self,
        bp: PointerValue<'ctx>,
        sty_idx: usize,
        field: &str,
    ) -> CResult<(PointerValue<'ctx>, Ty)> {
        let def = &self.checked.structs[sty_idx];
        let (fty, idx) = def
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field)
            .map(|(i, f)| (f.ty.clone(), i as u32))
            .ok_or_else(|| cerr(format!("no field `{}`", field)))?;

        let sllt = self.struct_llt(sty_idx)?;
        let i32t = self.ctx.i32_type();
        let z = i32t.const_int(0, false);
        let fi = i32t.const_int(idx as u64, false);
        let mut indices = [z.as_value_ref(), fi.as_value_ref()];
        let res = unsafe {
            LLVMBuildGEP2(
                self.builder.as_mut_ptr(),
                sllt.as_type_ref(),
                bp.as_value_ref(),
                indices.as_mut_ptr(),
                indices.len() as u32,
                c"fld".as_ptr(),
            )
        };
        Ok((unsafe { PointerValue::new(res) }, fty))
    }

    fn gen_expr(&mut self, e: &Expr) -> CResult<V<'ctx>> {
        match e {
            Expr::Int(v, _) => {
                // natural width u64; call/assign sites narrow explicitly
                Ok(Some(self.ctx.i64_type().const_int(*v as u64, false).into()))
            }
            Expr::Char(c, _) => Ok(Some(self.ctx.i8_type().const_int(*c as u64, false).into())),
            Expr::Bool(b, _) => Ok(Some(
                self.ctx.bool_type().const_int(*b as u64, false).into(),
            )),
            Expr::Str(s, _) => {
                let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
                Ok(Some(self.string_global(&bytes).into()))
            }
            Expr::Ident(name, _) => {
                if let Some(slot) = self.locals.get(name).copied() {
                    let ty = self
                        .local_tys
                        .get(name)
                        .cloned()
                        .ok_or_else(|| cerr(format!("internal: untyped local `{}`", name)))?;
                    let llt = self.llvm_type(&ty)?;
                    let v = self.builder.build_load(llt, slot, name)?;
                    return Ok(Some(v));
                }
                let info = self
                    .checked
                    .consts
                    .get(name)
                    .cloned()
                    .ok_or_else(|| cerr(format!("unknown identifier `{}`", name)))?;
                match info.value {
                    ConstVal::Int(v) => {
                        let t = self.int_type_of(&info.ty)?;
                        Ok(Some(t.const_int(v as u64, info.ty.is_signed()).into()))
                    }
                    ConstVal::Str(_) => {
                        let g = *self.str_globals.get(name).ok_or_else(|| {
                            cerr(format!("internal: missing global for const `{}`", name))
                        })?;
                        Ok(Some(g.into()))
                    }
                }
            }
            Expr::Unary { op, expr, .. } => {
                // address-of never loads — it produces the lvalue's slot/GEP
                if *op == UnaryOp::AddrOf {
                    return Ok(Some(self.gen_lvalue_addr(expr)?.into()));
                }
                let v = self
                    .gen_expr(expr)?
                    .expect("unary on void")
                    .into_int_value();
                match op {
                    UnaryOp::Neg => Ok(Some(self.builder.build_int_neg(v, "neg")?.into())),
                    UnaryOp::Not => {
                        let one = v.get_type().const_int(1, false);
                        Ok(Some(self.builder.build_xor(v, one, "not")?.into()))
                    }
                    UnaryOp::AddrOf => unreachable!("address-of handled above"),
                }
            }
            Expr::Binary { op, lhs, rhs, .. } => self.gen_binary(op, lhs, rhs),
            Expr::Cast { expr, ty, .. } => {
                let src_ty = self.ty_of_expr(expr)?;
                let dst_ty = self.resolve_te(ty)?;
                // array source: take the element-0 address, then behave
                // exactly like a pointer cast from there on
                let v = if matches!(src_ty, Ty::Array { .. }) {
                    BasicValueEnum::PointerValue(self.gen_lvalue_addr(expr)?)
                } else {
                    self.gen_expr(expr)?.expect("cast of void")
                };
                let dst_llt = self.llvm_type(&dst_ty)?;
                Ok(Some(match (&src_ty, &dst_ty) {
                    // array → pointer: v is already the element-0 address
                    (Ty::Array { .. }, _) => self
                        .builder
                        .build_pointer_cast(
                            v.into_pointer_value(),
                            dst_llt.into_pointer_type(),
                            "cast.a2p",
                        )?
                        .into(),
                    (Ty::Pointer { .. }, Ty::Pointer { .. }) => self
                        .builder
                        .build_pointer_cast(
                            v.into_pointer_value(),
                            dst_llt.into_pointer_type(),
                            "cast.p2p",
                        )?
                        .into(),
                    (Ty::Pointer { .. }, _) => {
                        // ptr → int
                        let wide = self.builder.build_ptr_to_int(
                            v.into_pointer_value(),
                            self.ctx.i64_type(),
                            "cast.p2i",
                        )?;
                        self.builder
                            .build_int_cast_sign_flag(wide, dst_llt.into_int_type(), false, "cast.p2i.n")?
                            .into()
                    }
                    (_, Ty::Pointer { .. }) => {
                        // int → ptr
                        let wide = self.builder.build_int_cast_sign_flag(
                            v.into_int_value(),
                            self.ctx.i64_type(),
                            false,
                            "cast.i2p.w",
                        )?;
                        self.builder
                            .build_int_to_ptr(wide, dst_llt.into_pointer_type(), "cast.i2p")?
                            .into()
                    }
                    (_, _) => {
                        let signed = src_ty.is_signed();
                        self.builder
                            .build_int_cast_sign_flag(
                                v.into_int_value(),
                                dst_llt.into_int_type(),
                                signed,
                                "cast.i2i",
                            )?
                            .into()
                    }
                }))
            }
            Expr::Index { base, index, .. } => {
                let (ptr, base_ptr_ty) = self.gen_index_ptr(base, index)?;
                let pointee = base_ptr_ty.pointee_for_store();
                let llt = self.llvm_type(&pointee)?;
                let loaded = self.builder.build_load(llt, ptr, "load.elem")?;
                if self.ty_of_expr(base)?.is_volatile_base() {
                    self.builder
                        .get_insert_block()
                        .unwrap()
                        .get_last_instruction()
                        .unwrap()
                        .set_volatile(true)?;
                }
                Ok(Some(loaded))
            }
            Expr::Field { base, field, .. } => {
                let (ptr, fty) = self.gen_field_ptr(base, field)?;
                let llt = self.llvm_type(&fty)?;
                Ok(Some(self.builder.build_load(llt, ptr, "load.fld")?))
            }
            Expr::Call { callee, args, .. } => {
                // conversion constructors erase to identity/narrowing ops
                if let Some(target) = self.conversion_target(callee) {
                    let arg = &args[0];
                    let v = self.gen_expr(arg)?.expect("conversion of void");
                    let got = self.llvm_type(&self.ty_of_expr(arg)?)?;
                    return Ok(Some(self.coerce_value(v, got, &target)?));
                }
                let sig = self
                    .checked
                    .fns
                    .get(callee)
                    .cloned()
                    .ok_or_else(|| cerr(format!("unknown function `{}`", callee)))?;
                let f = self.get_or_declare_function(callee, &sig)?;

                let mut vals: Vec<BasicMetadataValueEnum> = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    let want = sig.params[i].clone();
                    let v = self
                        .gen_value_for(a, &want)?
                        .expect("void argument");
                    let want_llt = self.llvm_type(&want)?;
                    let cv = self.coerce_value(v, want_llt, &want)?;
                    vals.push(cv.into());
                }
                let call = self.builder.build_call(f, &vals, "")?;
                Ok(call.try_as_basic_value().basic())
            }
        }
    }

    fn conversion_target(&self, callee: &str) -> Option<Ty> {
        let prim = match callee {
            "u8" => Ty::U8,
            "u16" => Ty::U16,
            "u32" => Ty::U32,
            "u64" => Ty::U64,
            "i8" => Ty::I8,
            "i16" => Ty::I16,
            "i32" => Ty::I32,
            "i64" => Ty::I64,
            _ => return self.checked.named_types.get(callee).cloned(),
        };
        Some(prim)
    }

    #[allow(clippy::too_many_lines)]
    fn gen_binary(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> CResult<V<'ctx>> {
        use BinOp::*;
        match op {
            And | Or => {
                let l = self.gen_bool(lhs)?;
                let f = self.current_fn()?;
                let rhs_bb = self.ctx.append_basic_block(f, "op.rhs");
                let short = self.ctx.append_basic_block(f, "op.short"); // lhs decided it
                let merge = self.ctx.append_basic_block(f, "op.end");

                if *op == And {
                    self.builder.build_conditional_branch(l, rhs_bb, short)?;
                } else {
                    self.builder.build_conditional_branch(l, short, rhs_bb)?;
                }

                self.builder.position_at_end(rhs_bb);
                let r = self.gen_bool(rhs)?;
                let r_end = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(merge)?;

                self.builder.position_at_end(short);
                let short_val = self
                    .ctx
                    .bool_type()
                    .const_int(if *op == And { 0 } else { 1 }, false);
                self.builder.build_unconditional_branch(merge)?;

                self.builder.position_at_end(merge);
                let phi = self.builder.build_phi(self.ctx.bool_type(), "op.val")?;
                phi.add_incoming(&[
                    (&r as &dyn inkwell::values::BasicValue, r_end),
                    (&short_val as &dyn inkwell::values::BasicValue, short),
                ]);
                Ok(Some(phi.as_basic_value()))
            }
            _ => {
                let lt = self.ty_of_expr(lhs)?;
                let lv = self.gen_expr(lhs)?.expect("void lhs").into_int_value();
                let rv_raw = self
                    .gen_expr(rhs)?
                    .expect("void rhs")
                    .into_int_value();
                // literal narrowing: equalize widths (typeck validated legality)
                let rv = if lv.get_type() != rv_raw.get_type() {
                    self.builder.build_int_cast_sign_flag(
                        rv_raw,
                        lv.get_type(),
                        lt.is_signed(),
                        "op.w",
                    )?
                } else {
                    rv_raw
                };
                let signed = lt.is_signed();
                let v: IntValue<'ctx> = match op {
                    Add => self.builder.build_int_add(lv, rv, "bin")?,
                    Sub => self.builder.build_int_sub(lv, rv, "bin")?,
                    Mul => self.builder.build_int_mul(lv, rv, "bin")?,
                    Div => {
                        if signed {
                            self.builder.build_int_signed_div(lv, rv, "bin")?
                        } else {
                            self.builder.build_int_unsigned_div(lv, rv, "bin")?
                        }
                    }
                    Mod => {
                        if signed {
                            self.builder.build_int_signed_rem(lv, rv, "bin")?
                        } else {
                            self.builder.build_int_unsigned_rem(lv, rv, "bin")?
                        }
                    }
                    BitAnd => self.builder.build_and(lv, rv, "bin")?,
                    BitOr => self.builder.build_or(lv, rv, "bin")?,
                    BitXor => self.builder.build_xor(lv, rv, "bin")?,
                    Shl => self.builder.build_left_shift(lv, rv, "bin")?,
                    Shr => self.builder.build_right_shift(lv, rv, signed, "bin")?,
                    EqCmp => {
                        return Ok(Some(
                            self.builder
                                .build_int_compare(IntPredicate::EQ, lv, rv, "cmp")?
                                .into(),
                        ))
                    }
                    NeCmp => {
                        return Ok(Some(
                            self.builder
                                .build_int_compare(IntPredicate::NE, lv, rv, "cmp")?
                                .into(),
                        ))
                    }
                    LtCmp => {
                        let pred = if signed { IntPredicate::SLT } else { IntPredicate::ULT };
                        return Ok(Some(
                            self.builder.build_int_compare(pred, lv, rv, "cmp")?.into(),
                        ));
                    }
                    GtCmp => {
                        let pred = if signed { IntPredicate::SGT } else { IntPredicate::UGT };
                        return Ok(Some(
                            self.builder.build_int_compare(pred, lv, rv, "cmp")?.into(),
                        ));
                    }
                    LeCmp => {
                        let pred = if signed { IntPredicate::SLE } else { IntPredicate::ULE };
                        return Ok(Some(
                            self.builder.build_int_compare(pred, lv, rv, "cmp")?.into(),
                        ));
                    }
                    GeCmp => {
                        let pred = if signed { IntPredicate::SGE } else { IntPredicate::UGE };
                        return Ok(Some(
                            self.builder.build_int_compare(pred, lv, rv, "cmp")?.into(),
                        ));
                    }
                    And | Or => unreachable!(),
                };
                Ok(Some(v.into()))
            }
        }
    }

    fn current_fn(&self) -> CResult<FunctionValue<'ctx>> {
        self.builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or_else(|| cerr("internal: no current function"))
    }

    // ---- asm blocks ---------------------------------------------------------

    fn gen_asm(&mut self, block: &AsmBlock) -> CResult<()> {
        if block.outputs.len() > 1 {
            return Err(cerr(
                "at most ONE asm output is supported in v0 (multiple outputs need struct returns)",
            ));
        }

        // GCC template (%0) → LLVM template ($0); %reg names pass through.
        let template = convert_template(&block.template);

        // constraint string: outputs, then inputs, then ~clobbers
        // Generic x86 register-class constraints (a/b/c/d/S/D) are expanded to
        // explicit registers ({ax}, {bl}, ...) sized by the operand's width —
        // this is what clang emits anyway, and some LLVM builds mis-handle the
        // class form ("couldn't allocate input reg").
        let mut constraints: Vec<String> = Vec::new();
        for o in &block.outputs {
            let ty = self.local_tys[&o.binding].clone();
            let bits = ty.bits().max(8);
            constraints.push(expand_constraint(&o.constraint, bits));
        }
        for (c, e) in &block.inputs {
            let ity = self.ty_of_expr(e)?;
            let bits = if ity.is_integer() { ity.bits() } else { 64 };
            constraints.push(expand_constraint(c, bits));
        }
        for c in &block.clobbers {
            if c.starts_with('~') {
                constraints.push(c.clone());
            } else if c == "memory" || c == "cc" {
                // LLVM constraint syntax braces every clobber: ~{memory}, ~{cc}
                constraints.push(format!("~{{{c}}}"));
            } else {
                constraints.push(format!("~{{{c}}}"));
            }
        }
        let constraint_str = constraints.join(",");

        let mut operands: Vec<BasicMetadataValueEnum> = Vec::new();
        for (_, e) in &block.inputs {
            let v = self.gen_expr(e)?.expect("void asm input");
            let want = self.ty_of_expr(e)?;
            let want_llt = self.llvm_type(&want)?;
            let cv = self.coerce_value(v, want_llt, &want)?;
            operands.push(cv.into());
        }

        // result type: first output's local type, else void; formal params =
        // the INPUT operand types (outputs arrive via the return value)
        let ret_llt: Option<BasicTypeEnum> = match block.outputs.first() {
            Some(out) => {
                let ty = self
                    .local_tys
                    .get(&out.binding)
                    .cloned()
                    .ok_or_else(|| {
                        cerr(format!("asm output binds unknown local `{}`", out.binding))
                    })?;
                Some(self.llvm_type(&ty)?)
            }
            None => None,
        };

        let mut input_llts: Vec<BasicMetadataTypeEnum> = Vec::new();
        for (_, e) in &block.inputs {
            let ity = self.ty_of_expr(e)?;
            input_llts.push(self.llvm_type(&ity)?.into());
        }

        let fnty: FunctionType = match ret_llt {
            Some(r) => r.fn_type(&input_llts, false),
            None => self.ctx.void_type().fn_type(&input_llts, false),
        };

        let asm_c =
            CString::new(template.as_str()).map_err(|_| cerr("asm template contains NUL"))?;
        let cons_c = CString::new(constraint_str.as_str())
            .map_err(|_| cerr("asm constraints contain NUL"))?;

        let mut operand_refs: Vec<llvm_sys::prelude::LLVMValueRef> = operands
            .iter()
            .map(|v| v.as_value_ref())
            .collect();

        let asm_val = unsafe {
            LLVMGetInlineAsm(
                fnty.as_type_ref(),
                asm_c.as_ptr(),
                template.len(),
                cons_c.as_ptr(),
                constraint_str.len(),
                1, // has side effects — always volatile per spec §6
                0, // no special stack alignment
                LLVMInlineAsmDialect::LLVMInlineAsmDialectATT,
                0, // cannot throw
            )
        };

        let call_res = unsafe {
            LLVMBuildCall2(
                self.builder.as_mut_ptr(),
                fnty.as_type_ref(),
                asm_val,
                operand_refs.as_mut_ptr(),
                operand_refs.len() as u32,
                c"".as_ptr(),
            )
        };

        // store returned value into the output binding — its Asc type decides
        // which value wrapper to materialize
        if let Some(out) = block.outputs.first() {
            let slot = *self.locals.get(&out.binding).unwrap();
            let ty = self.local_tys[&out.binding].clone();
            match self.llvm_type(&ty)? {
                BasicTypeEnum::IntType(_) => {
                    let iv = unsafe { IntValue::new(call_res) };
                    self.builder.build_store(slot, iv)?;
                }
                BasicTypeEnum::PointerType(_) => {
                    let pv = unsafe { PointerValue::new(call_res) };
                    self.builder.build_store(slot, pv)?;
                }
                _ => return Err(cerr("unsupported asm output type")),
            }
        }

        Ok(())
    }
}

impl Ty {
    /// The type stored THROUGH this pointer (for volatile store decisions).
    fn pointee_for_store(&self) -> Ty {
        match self {
            Ty::Pointer { pointee, .. } => (**pointee).clone(),
            Ty::Array { elem, .. } => (**elem).clone(),
            Ty::Distinct { base, .. } => base.pointee_for_store(),
            other => other.clone(),
        }
    }

    /// Whether accesses through this pointer/array-typed base are volatile.
    fn is_volatile_base(&self) -> bool {
        match self {
            Ty::Pointer { volatile, .. } => *volatile,
            Ty::Distinct { base, .. } => base.is_volatile_base(),
            _ => false,
        }
    }
}

fn expr_kind(e: &Expr) -> &'static str {
    match e {
        Expr::Int(..) => "integer literal",
        Expr::Char(..) => "char literal",
        Expr::Str(..) => "string literal",
        Expr::Bool(..) => "boolean literal",
        Expr::Call { .. } => "call result",
        Expr::Cast { .. } => "cast result",
        Expr::Binary { .. } => "binary expression",
        Expr::Unary { op, .. } => match op {
            UnaryOp::AddrOf => "address-of expression",
            _ => "unary expression",
        },
        Expr::Ident(..) => "identifier",
        Expr::Index { .. } | Expr::Field { .. } => "indexed/field expression",
    }
}

/// `%N` digit placeholders → `$N`; register names like `%al` pass through.
fn convert_template(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let chars: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            out.push('$');
            out.push(chars[i + 1]);
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Expand x86 register-class constraints to explicit registers, sized by the
/// operand width. Modifiers (`N`, `=`) and non-register constraints pass
/// through. `Nd` on a u16 → `N{dx}`; `=a` on a u8 → `={al}`.
fn expand_constraint(c: &str, bits: u32) -> String {
    let mut out = String::with_capacity(c.len());
    let mut chars = c.chars().peekable();

    // leading modifiers: '=' (output), digits/letters that are constraint
    // modifiers we don't interpret get copied until we hit a known class letter
    while let Some(&ch) = chars.peek() {
        match ch {
            '=' => {
                out.push('=');
                chars.next();
            }
            'N' | 'M' => {
                // immediate-range modifiers stay attached
                out.push(ch);
                chars.next();
            }
            _ => break,
        }
    }

    if let Some(&ch) = chars.peek() {
        let reg = match ch {
            'a' | 'b' | 'c' | 'd' => {
                let base = ['a', 'b', 'c', 'd'].iter().position(|&r| r == ch).unwrap();
                chars.next();
                Some(match bits {
                    8 => ["al", "bl", "cl", "dl"][base],
                    16 => ["ax", "bx", "cx", "dx"][base],
                    32 => ["eax", "ebx", "ecx", "edx"][base],
                    _ => ["rax", "rbx", "rcx", "rdx"][base],
                })
            }
            'S' => {
                chars.next();
                Some(match bits {
                    8 => "sil",
                    16 => "si",
                    32 => "esi",
                    _ => "rsi",
                })
            }
            'D' => {
                chars.next();
                Some(match bits {
                    8 => "dil",
                    16 => "di",
                    32 => "edi",
                    _ => "rdi",
                })
            }
            _ => None,
        };
        if let Some(r) = reg {
            out.push('{');
            out.push_str(r);
            out.push('}');
            // trailing modifiers after the class letter (e.g. nothing common in v0)
            for rest in chars {
                out.push(rest);
            }
            return out;
        }
    }

    // not a register-class constraint — restore original text
    c.to_string()
}
