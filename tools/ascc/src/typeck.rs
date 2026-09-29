//! Asc type checker: enforces the correctness rules before codegen:
//!
//! - nominal `distinct` types with NO implicit conversions (hard errors)
//! - explicit-only conversions (`Vaddr(x)` wrap, `u64(p)` unwrap, `as` casts
//!   restricted to non-distinct operands)
//! - literal typing: unsuffixed integers take their type from context
//! - same-type discipline for arithmetic/comparisons
//! - `never`/`void` placement rules
//!
//! The move-checking scaffold lives here too: every v0 type is Copy, so the
//! pass is currently vacuous, but the walk exists so Phase 1 ownership types
//! plug in without redesign (see `FnChecker::check_moves`).

use std::collections::HashMap;

use crate::ast::*;
use crate::lexer::Span;

#[derive(Debug, Clone)]
pub struct TypeError {
    pub msg: String,
    pub span: Span,
}

type TResult<T> = Result<T, TypeError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    Bool,
    Void,
    Never,
    Pointer {
        pointee: Box<Ty>,
        volatile: bool,
    },
    /// `[N]T` fixed-size value array (v1: no nesting).
    Array {
        elem: Box<Ty>,
        len: u64,
    },
    Distinct {
        name: String,
        base: Box<Ty>,
    },
    Struct(usize), // index into CheckedModule::structs
}

impl Ty {
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 | Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64
        )
    }

    pub fn is_signed(&self) -> bool {
        matches!(self, Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64)
    }

    pub fn bits(&self) -> u32 {
        match self {
            Ty::U8 | Ty::I8 => 8,
            Ty::U16 | Ty::I16 => 16,
            Ty::U32 | Ty::I32 => 32,
            Ty::U64 | Ty::I64 => 64,
            _ => 0,
        }
    }

    pub fn display(&self) -> String {
        match self {
            Ty::U8 => "u8".into(),
            Ty::U16 => "u16".into(),
            Ty::U32 => "u32".into(),
            Ty::U64 => "u64".into(),
            Ty::I8 => "i8".into(),
            Ty::I16 => "i16".into(),
            Ty::I32 => "i32".into(),
            Ty::I64 => "i64".into(),
            Ty::Bool => "bool".into(),
            Ty::Void => "void".into(),
            Ty::Never => "never".into(),
            Ty::Pointer { pointee, volatile } => format!(
                "*{}{}",
                if *volatile { "volatile " } else { "" },
                pointee.display()
            ),
            Ty::Array { elem, len } => format!("[{}]{}", len, elem.display()),
            Ty::Distinct { name, .. } => name.clone(),
            Ty::Struct(i) => format!("<struct #{i}>"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub packed: bool,
    pub fields: Vec<ResolvedField>,
}

#[derive(Debug, Clone)]
pub struct ResolvedField {
    pub name: String,
    pub ty: Ty,
}

/// Compile-time-evaluated constant values.
#[derive(Debug, Clone)]
pub enum ConstVal {
    Int(i128),
    Str(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct ConstInfo {
    pub ty: Ty,
    pub value: ConstVal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnSig {
    pub params: Vec<Ty>,
    pub ret: Ty,
}

/// Everything codegen needs, produced by typeck.
pub struct CheckedModule<'a> {
    pub module: &'a Module,
    pub structs: Vec<StructDef>,
    /// user-declared type names → resolved Ty (distinct aliases and structs)
    pub named_types: HashMap<String, Ty>,
    pub consts: HashMap<String, ConstInfo>,
    pub fns: HashMap<String, FnSig>,
    /// module-level mutable bindings: zero-initialized unit-private storage
    pub statics: HashMap<String, Ty>,
    pub expr_tys: HashMap<usize, Ty>,
}

pub struct ModuleTypes {
    named_types: HashMap<String, Ty>,
    structs: Vec<StructDef>,
}

struct FnChecker<'a> {
    mt: &'a mut ModuleTypes,
    consts: &'a HashMap<String, ConstInfo>,
    fns: &'a HashMap<String, FnSig>,
    statics: &'a HashMap<String, Ty>,
    locals: Vec<HashMap<String, Ty>>,
    fn_ret: Ty,
    in_never_fn: bool,
    /// nesting depth of enclosing `while` loops; `break`/`continue` are only
    /// legal inside one
    loop_depth: u32,
    /// Resolved type per expression node (keyed by node address) so codegen
    /// never re-infers types. The AST is borrowed unchanged downstream.
    expr_tys: HashMap<usize, Ty>,
}

pub fn check(module: &Module) -> TResult<CheckedModule<'_>> {
    let mut mt = ModuleTypes {
        named_types: HashMap::new(),
        structs: Vec::new(),
    };
    let mut consts: HashMap<String, ConstInfo> = HashMap::new();
    let mut fns: HashMap<String, FnSig> = HashMap::new();
    let mut statics: HashMap<String, Ty> = HashMap::new();
    // which entries in `fns` have a body in this unit (vs. are prototypes)
    let mut defined_fns: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Pass 1: register type names (aliases resolved immediately; struct shells
    // inserted so pointers/fields can reference any declared type by name).
    for decl in &module.decls {
        match decl {
            Decl::TypeAlias { name, span, base } => {
                if mt.named_types.contains_key(name) {
                    return Err(terr(*span, format!("duplicate type name `{}`", name)));
                }
                let resolved_base = resolve_type(&mut mt, base, &consts)?;
                if matches!(resolved_base, Ty::Array { .. }) {
                    return Err(terr(
                        *span,
                        "`distinct` over an array is not supported in v1",
                    ));
                }
                validate_value_ty(&resolved_base, *span, "the base of a distinct type")?;
                mt.named_types.insert(
                    name.clone(),
                    Ty::Distinct {
                        name: name.clone(),
                        base: Box::new(resolved_base),
                    },
                );
            }
            Decl::Struct { name, span, .. } => {
                if mt.named_types.contains_key(name) {
                    return Err(terr(*span, format!("duplicate type name `{}`", name)));
                }
                let idx = mt.structs.len();
                mt.structs.push(StructDef {
                    name: String::new(), // filled in pass 3
                    packed: false,
                    fields: Vec::new(),
                });
                mt.named_types.insert(name.clone(), Ty::Struct(idx));
            }
            _ => {}
        }
    }

    // Pass 2: const values (before struct fields, so array lengths may name
    // consts; const initializers depend only on literals and other consts).
    for decl in &module.decls {
        if let Decl::Const { name, span, ty, init } = decl {
            let cty = resolve_type(&mut mt, ty, &consts)?;
            validate_const_ty(&cty, *span)?;
            let mut fc = FnChecker {
                mt: &mut mt,
                consts: &consts,
                fns: &fns,
                statics: &HashMap::new(),
                locals: vec![HashMap::new()],
                fn_ret: Ty::Void,
                in_never_fn: false,
                loop_depth: 0,
                expr_tys: HashMap::new(),
            };
            let (val, vty) = fc.const_eval(init)?;
            coerce(cty.clone(), vty.clone(), init)?;
            consts.insert(name.clone(), ConstInfo { ty: cty, value: val });
        }
    }

    // Pass 3: resolve struct fields.
    for decl in &module.decls {
        if let Decl::Struct { name, packed, fields, span } = decl {
            let idx = match mt.named_types.get(name) {
                Some(Ty::Struct(i)) => *i,
                _ => unreachable!("struct shell missing"),
            };
            let mut resolved = Vec::new();
            for f in fields {
                let fty = resolve_type(&mut mt, &f.ty, &consts)?;
                validate_value_ty(&fty, *span, "a field type")?;
                if resolved.iter().any(|rf: &ResolvedField| rf.name == f.name) {
                    return Err(terr(
                        *span,
                        format!("duplicate field `{}` in struct `{}`", f.name, name),
                    ));
                }
                resolved.push(ResolvedField {
                    name: f.name.clone(),
                    ty: fty,
                });
            }
            mt.structs[idx] = StructDef {
                name: name.clone(),
                packed: *packed,
                fields: resolved,
            };
        }
    }

    // Pass 4: function signatures: definitions AND bodyless declarations.
    // A prototype and a definition of the same name must agree exactly; two
    // definitions or two prototypes are duplicates.
    for decl in &module.decls {
        let (name, span, ret, params, is_def) = match decl {
            Decl::Function { name, span, ret, params, .. } => (name, span, ret, params, true),
            Decl::FnProto { name, span, ret, params } => (name, span, ret, params, false),
            _ => continue,
        };
        let rty = resolve_type(&mut mt, ret, &consts)?;
        if !matches!(rty, Ty::Void | Ty::Never) {
            validate_value_ty(&rty, *span, "a return type")?;
        }
        if matches!(rty, Ty::Array { .. }) {
            return Err(terr(*span, "arrays cannot be returned by value (pass a pointer)"));
        }
        let mut ptys = Vec::new();
        for p in params {
            let pty = resolve_type(&mut mt, &p.ty, &consts)?;
            validate_value_ty(&pty, *span, "a parameter type")?;
            if matches!(pty, Ty::Array { .. }) {
                return Err(terr(*span, "arrays cannot be passed by value (pass a pointer)"));
            }
            ptys.push(pty);
        }
        let sig = FnSig { params: ptys, ret: rty };
        if let Some(existing) = fns.get(name) {
            if existing != &sig {
                return Err(terr(
                    *span,
                    format!(
                        "declaration of `{}` conflicts with earlier signature (params `{}`, returns `{}`)",
                        name,
                        existing.params.iter().map(|t| t.display()).collect::<Vec<_>>().join(", "),
                        existing.ret.display()
                    ),
                ));
            }
            if is_def && defined_fns.contains(name) {
                return Err(terr(*span, format!("duplicate function definition `{}`", name)));
            }
            if !is_def && !defined_fns.contains(name) {
                return Err(terr(*span, format!("duplicate declaration of `{}`", name)));
            }
        }
        // a matching entry is kept as-is; new names are inserted
        fns.entry(name.clone()).or_insert(sig);
        if is_def {
            defined_fns.insert(name.clone());
        }
    }

    // Pass 4.5: module-level mutable bindings. Registered AFTER function
    // signatures so one namespace check can cover consts + fns + statics.
    for decl in &module.decls {
        if let Decl::Static { name, span, ty } = decl {
            let sty = resolve_type(&mut mt, ty, &consts)?;
            validate_value_ty(&sty, *span, "a module-level binding type")?;
            if consts.contains_key(name) || fns.contains_key(name) {
                return Err(terr(
                    *span,
                    format!("`{}` already declared as a {} in this unit", name,
                        if consts.contains_key(name) { "const" } else { "function" }),
                ));
            }
            if statics.insert(name.clone(), sty).is_some() {
                return Err(terr(*span, format!("duplicate module-level binding `{}`", name)));
            }
        }
    }

    // Pass 5: function bodies.
    let mut all_expr_tys: HashMap<usize, Ty> = HashMap::new();
    for decl in &module.decls {
        if let Decl::Function { name, params, body, .. } = decl {
            let sig = fns.get(name).cloned().ok_or_else(|| TypeError {
                msg: format!("internal: signature for `{}` missing", name),
                span: Span { line: 0, col: 0 },
            })?;
            let mut fc = FnChecker {
                mt: &mut mt,
                consts: &consts,
                fns: &fns,
                statics: &statics,
                locals: vec![HashMap::new()],
                fn_ret: sig.ret.clone(),
                in_never_fn: matches!(sig.ret, Ty::Never),
                loop_depth: 0,
                expr_tys: HashMap::new(),
            };
            let top = fc.locals.last_mut().unwrap();
            for (p, pty) in params.iter().zip(sig.params.iter()) {
                if top.contains_key(&p.name) {
                    return Err(terr(
                        Span { line: 0, col: 0 },
                        format!("duplicate parameter name `{}`", p.name),
                    ));
                }
                top.insert(p.name.clone(), pty.clone());
            }
            fc.check_block(body)?;
            fc.check_moves(body);
            all_expr_tys.extend(fc.expr_tys);
        }
    }

    Ok(CheckedModule {
        module,
        structs: mt.structs,
        named_types: mt.named_types.clone(),
        consts,
        fns,
        statics,
        expr_tys: all_expr_tys,
    })
}

fn terr(span: Span, msg: impl Into<String>) -> TypeError {
    TypeError {
        msg: msg.into(),
        span,
    }
}

fn validate_value_ty(ty: &Ty, span: Span, what: &str) -> TResult<()> {
    match ty {
        Ty::Void => Err(terr(span, format!("`void` cannot be used as {}", what))),
        Ty::Never => Err(terr(
            span,
            "`never` can only be used as a function return type",
        )),
        Ty::Pointer { pointee, .. } => match **pointee {
            Ty::Void | Ty::Never => Err(terr(
                span,
                format!("pointer to `{}` is not allowed in v0", pointee.display()),
            )),
            _ => Ok(()),
        },
        Ty::Distinct { base, .. } => validate_value_ty(base, span, what),
        Ty::Array { elem, .. } => validate_value_ty(elem, span, what),
        _ => Ok(()),
    }
}

fn validate_const_ty(ty: &Ty, span: Span) -> TResult<()> {
    match ty {
        t if t.is_integer() || matches!(t, Ty::Bool) => Ok(()),
        Ty::Pointer { .. } => Ok(()),
        Ty::Distinct { base, .. } => validate_const_ty(base, span),
        other => Err(terr(
            span,
            format!("`{}` cannot be a const type in v0", other.display()),
        )),
    }
}

pub fn resolve_type(
    mt: &mut ModuleTypes,
    te: &TypeExpr,
    consts: &HashMap<String, ConstInfo>,
) -> TResult<Ty> {
    match te {
        TypeExpr::Named { name, span } => {
            let prim = primitive_by_name(name);
            if let Some(t) = prim {
                return Ok(t);
            }
            match mt.named_types.get(name) {
                Some(t) => Ok(t.clone()),
                None => Err(terr(*span, format!("unknown type `{}`", name))),
            }
        }
        TypeExpr::Pointer {
            pointee,
            volatile,
            span,
        } => {
            let inner = resolve_type(mt, pointee, consts)?;
            match inner {
                Ty::Void | Ty::Never => Err(terr(
                    *span,
                    format!("pointer to `{}` is not allowed in v0", inner.display()),
                )),
                _ => Ok(Ty::Pointer {
                    pointee: Box::new(inner),
                    volatile: *volatile,
                }),
            }
        }
        TypeExpr::Array { len, elem, span } => {
            let inner = resolve_type(mt, elem, consts)?;
            if matches!(inner, Ty::Array { .. }) {
                return Err(terr(*span, "nested arrays ([N][M]T) are not supported in v1"));
            }
            validate_value_ty(&inner, *span, "an array element type")?;
            let n = eval_array_len(len, consts)?;
            if n == 0 {
                return Err(terr(expr_span(len), "array length must be at least 1"));
            }
            Ok(Ty::Array {
                elem: Box::new(inner),
                len: n,
            })
        }
    }
}

/// Compile-time array length: integer literals, char literals, const
/// references of integer type, and `+ - * /` combinations of those.
fn eval_array_len(expr: &Expr, consts: &HashMap<String, ConstInfo>) -> TResult<u64> {
    match expr {
        Expr::Int(v, _) => Ok(*v),
        Expr::Char(c, _) => Ok(*c as u64),
        Expr::Ident(name, span) => match consts.get(name) {
            Some(ConstInfo { value: ConstVal::Int(v), .. }) if *v >= 0 => Ok(*v as u64),
            Some(_) => Err(terr(
                *span,
                format!("const `{}` is not a non-negative integer (array length)", name),
            )),
            None => Err(terr(
                *span,
                format!("unknown const `{}` in array length", name),
            )),
        },
        Expr::Binary { op, lhs, rhs, span } => {
            let a = eval_array_len(lhs, consts)?;
            let b = eval_array_len(rhs, consts)?;
            let r = match op {
                BinOp::Add => a.checked_add(b),
                BinOp::Sub => a.checked_sub(b),
                BinOp::Mul => a.checked_mul(b),
                BinOp::Div => {
                    if b == 0 {
                        None
                    } else {
                        Some(a / b)
                    }
                }
                other => {
                    return Err(terr(
                        *span,
                        format!("operator not allowed in array length: {:?}", other),
                    ))
                }
            };
            r.ok_or_else(|| terr(*span, "array length arithmetic overflow/underflow"))
        }
        other => Err(terr(
            expr_span(other),
            "array length must be a compile-time integer constant",
        )),
    }
}

fn primitive_by_name(name: &str) -> Option<Ty> {
    Some(match name {
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
        _ => return None,
    })
}

/// Compatibility for assignment/argument/return positions: exact match, except
/// pointer volatility does not affect identity.
fn types_compatible(expected: &Ty, actual: &Ty) -> bool {
    if expected == actual {
        return true;
    }
    if let (
        Ty::Pointer {
            pointee: pe,
            volatile: _,
        },
        Ty::Pointer {
            pointee: pa,
            volatile: _,
        },
    ) = (expected, actual)
    {
        return types_compatible(pe, pa);
    }
    false
}

fn coerce(expected: Ty, actual: Ty, value_expr: &Expr) -> TResult<()> {
    if types_compatible(&expected, &actual) {
        return Ok(());
    }
    // array decay: a `[N]T` value used where `*T`/`*const T` is expected
    // decays to a pointer to its first element (codegen emits the address,
    // not a load)
    if let (Ty::Pointer { pointee, .. }, Ty::Array { elem, .. }) = (&expected, &actual) {
        if types_compatible(pointee, elem) {
            return Ok(());
        }
    }
    if is_literal(value_expr) && expected.is_integer() && actual.is_integer() {
        // literal narrowing validated against expected width
        let v = match value_expr {
            Expr::Int(v, _) => *v as i128,
            Expr::Char(c, _) => *c as i128,
            _ => unreachable!(),
        };
        let (bits, signed) = int_width(&expected).unwrap();
        let fits = if signed {
            v >= -(1i128 << (bits - 1)) && v < (1i128 << (bits - 1))
        } else {
            v >= 0 && v < (1i128 << bits)
        };
        if fits {
            return Ok(());
        }
    }
    Err(terr(
        expr_span(value_expr),
        format!(
            "type mismatch: expected `{}`, found `{}`",
            expected.display(),
            actual.display()
        ),
    ))
}

fn int_width(t: &Ty) -> Option<(u32, bool)> {
    Some(match t {
        Ty::U8 => (8, false),
        Ty::U16 => (16, false),
        Ty::U32 => (32, false),
        Ty::U64 => (64, false),
        Ty::I8 => (8, true),
        Ty::I16 => (16, true),
        Ty::I32 => (32, true),
        Ty::I64 => (64, true),
        _ => return None,
    })
}

impl<'a> FnChecker<'a> {
    // ---- scopes --------------------------------------------------------

    fn declare_local(&mut self, name: &str, ty: Ty, span: Span) -> TResult<()> {
        let top = self.locals.last_mut().unwrap();
        if top.contains_key(name) {
            return Err(terr(span, format!("duplicate binding `{}` in this scope", name)));
        }
        top.insert(name.to_string(), ty);
        Ok(())
    }

    fn lookup_local(&self, name: &str) -> Option<Ty> {
        for scope in self.locals.iter().rev() {
            if let Some(t) = scope.get(name) {
                return Some(t.clone());
            }
        }
        None
    }

    // ---- statements ------------------------------------------------------

    fn check_block(&mut self, block: &Block) -> TResult<()> {
        self.locals.push(HashMap::new());
        for stmt in &block.stmts {
            self.check_stmt(stmt)?;
        }
        self.locals.pop();
        Ok(())
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> TResult<()> {
        match stmt {
            Stmt::Let { name, span, ty, init } => {
                let declared = match ty {
                    Some(te) => {
                        let t = resolve_type(self.mt, te, self.consts)?;
                        validate_value_ty(&t, *span, "a variable type")?;
                        Some(t)
                    }
                    None => None,
                };
                match (init, &declared) {
                    (Some(e), Some(dt)) => {
                        let (it, _) = self.check_expr(e)?;
                        coerce(dt.clone(), it, e)?;
                    }
                    (Some(e), None) => {
                        let (it, _) = self.check_expr(e)?;
                        if matches!(it, Ty::Void | Ty::Never) {
                            return Err(terr(
                                *span,
                                format!(
                                    "cannot bind `{}` to an expression of type `{}`",
                                    name,
                                    it.display()
                                ),
                            ));
                        }
                        self.declare_local(name, it, *span)?;
                        return Ok(());
                    }
                    (None, None) => {
                        return Err(terr(
                            *span,
                            format!(
                                "binding `{}` has neither a type annotation nor an initializer",
                                name
                            ),
                        ))
                    }
                    (None, Some(_)) => {}
                }
                self.declare_local(name, declared.unwrap(), *span)?;
                Ok(())
            }
            Stmt::Assign { target, value, .. } => {
                let (tt, _) = self.check_expr(target)?;
                let (vt, _) = self.check_expr(value)?;
                coerce(tt, vt, value)?;
                ensure_lvalue(target)?;
                Ok(())
            }
            Stmt::Expr(e) => {
                self.check_expr(e)?;
                Ok(())
            }
            Stmt::If { cond, then_body, else_body, .. } => {
                let (ct, _) = self.check_expr(cond)?;
                if ct != Ty::Bool {
                    return Err(terr(
                        expr_span(cond),
                        format!("`if` condition must be `bool`, found `{}`", ct.display()),
                    ));
                }
                self.check_block(then_body)?;
                if let Some(els) = else_body {
                    self.check_block(els)?;
                }
                Ok(())
            }
            Stmt::While { cond, body, .. } => {
                let (ct, _) = self.check_expr(cond)?;
                if ct != Ty::Bool {
                    return Err(terr(
                        expr_span(cond),
                        format!("`while` condition must be `bool`, found `{}`", ct.display()),
                    ));
                }
                self.loop_depth += 1;
                self.check_block(body)?;
                self.loop_depth -= 1;
                Ok(())
            }
            Stmt::Break { span } => {
                if self.loop_depth == 0 {
                    return Err(terr(*span, "`break` outside of a loop"));
                }
                Ok(())
            }
            Stmt::Continue { span } => {
                if self.loop_depth == 0 {
                    return Err(terr(*span, "`continue` outside of a loop"));
                }
                Ok(())
            }
            Stmt::Return { value, span } => {
                if self.in_never_fn {
                    return Err(terr(
                        *span,
                        "`return` inside a `-> never` function is forbidden: it cannot return",
                    ));
                }
                let expected = self.fn_ret.clone();
                match (&expected, value) {
                    (Ty::Void, None) => Ok(()),
                    (_, None) => Err(terr(
                        *span,
                        format!(
                            "missing return value; function returns `{}`",
                            expected.display()
                        ),
                    )),
                    (
                        Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 | Ty::I8 | Ty::I16 | Ty::I32
                        | Ty::I64 | Ty::Bool | Ty::Pointer { .. } | Ty::Distinct { .. }
                        | Ty::Struct(_) | Ty::Array { .. },
                        Some(e),
                    ) => {
                        let (actual, _) = self.check_expr(e)?;
                        coerce(expected, actual, e)?;
                        Ok(())
                    }
                    (Ty::Void, Some(e)) => {
                        self.check_expr(e)?;
                        Err(terr(expr_span(e), "void function cannot return a value"))
                    }
                    (Ty::Never, _) => unreachable!(),
                }
            }
            Stmt::Asm(block) => self.check_asm(block),
        }
    }

    fn check_asm(&mut self, block: &AsmBlock) -> TResult<()> {
        for out in &block.outputs {
            if self.lookup_local(&out.binding).is_none() {
                return Err(terr(
                    block.span,
                    format!("asm output binds unknown local `{}`", out.binding),
                ));
            }
        }
        for (_, e) in &block.inputs {
            self.check_expr(e)?;
        }
        Ok(())
    }

    // ---- expressions -------------------------------------------------------

    /// Returns (type, compile-time folded value when fully constant).
    /// Every expression's resolved type is recorded for codegen.
    fn check_expr(&mut self, expr: &Expr) -> TResult<(Ty, Option<i128>)> {
        let result = self.check_expr_inner(expr);
        if let Ok((ty, _)) = &result {
            self.expr_tys
                .insert(expr as *const Expr as usize, ty.clone());
        }
        result
    }

    fn check_expr_inner(&mut self, expr: &Expr) -> TResult<(Ty, Option<i128>)> {
        match expr {
            Expr::Int(v, span) => {
                if (*v as i128) < (1i128 << 64) {
                    Ok((Ty::U64, Some(*v as i128))) // pseudo-type; narrowed via coerce
                } else {
                    Err(terr(*span, "integer literal does not fit u64"))
                }
            }
            Expr::Char(c, _) => Ok((Ty::U8, Some(*c as i128))),
            Expr::Bool(b, _) => Ok((Ty::Bool, Some(*b as i128))),
            Expr::Str(_, _) => Ok((
                Ty::Pointer {
                    pointee: Box::new(Ty::U8),
                    volatile: false,
                },
                None,
            )),
            Expr::Ident(name, span) => {
                if let Some(t) = self.lookup_local(name) {
                    return Ok((t, None));
                }
                if let Some(ci) = self.consts.get(name) {
                    let folded = match ci.value {
                        ConstVal::Int(v) => Some(v),
                        ConstVal::Str(_) => None,
                    };
                    return Ok((ci.ty.clone(), folded));
                }
                if let Some(t) = self.statics.get(name) {
                    // statics are never compile-time folded: they are runtime
                    // storage, readable and writable from any function here
                    return Ok((t.clone(), None));
                }
                Err(terr(*span, format!("unknown identifier `{}`", name)))
            }
            Expr::Unary { op, expr, span } => {
                let (t, cv) = self.check_expr(expr)?;
                match op {
                    UnaryOp::Not => {
                        if t != Ty::Bool {
                            return Err(terr(
                                *span,
                                format!("`!` requires `bool`, found `{}`", t.display()),
                            ));
                        }
                        Ok((Ty::Bool, None))
                    }
                    UnaryOp::Neg => {
                        if !t.is_integer() {
                            return Err(terr(
                                *span,
                                format!("unary `-` requires an integer, found `{}`", t.display()),
                            ));
                        }
                        Ok((t, cv.map(|v| -v)))
                    }
                    UnaryOp::AddrOf => {
                        ensure_lvalue(expr)?;
                        Ok((
                            Ty::Pointer {
                                pointee: Box::new(t),
                                volatile: false,
                            },
                            None,
                        ))
                    }
                }
            }
            Expr::Binary { op, lhs, rhs, span } => self.check_binary(op, lhs, rhs, *span),
            Expr::Cast { expr, ty, span } => {
                let target = resolve_type(self.mt, ty, self.consts)?;
                let (src, _) = self.check_expr(expr)?;
                check_cast(&src, &target, *span)?;
                Ok((target, None))
            }
            Expr::Index { base, index, span } => {
                let (bt, _) = self.check_expr(base)?;
                let (it, _) = self.check_expr(index)?;
                if !it.is_integer() {
                    return Err(terr(
                        expr_span(index),
                        format!("index must be an integer, found `{}`", it.display()),
                    ));
                }
                match bt {
                    Ty::Pointer { pointee, .. } => Ok((*pointee, None)),
                    Ty::Array { elem, .. } => Ok((*elem, None)),
                    other => Err(terr(
                        *span,
                        format!("cannot index into `{}` (only pointers and arrays)", other.display()),
                    )),
                }
            }
            Expr::Field { base, field, span } => {
                let (bt, _) = self.check_expr(base)?;
                // Go-style: `.field` through a pointer-to-struct auto-derefs
                let struct_ty = match &bt {
                    Ty::Struct(_) => bt.clone(),
                    Ty::Pointer { pointee, volatile } => {
                        if !matches!(**pointee, Ty::Struct(_)) {
                            return Err(terr(
                                *span,
                                format!(
                                    "cannot access field `.{}` through pointer to non-struct `{}`",
                                    field,
                                    pointee.display()
                                ),
                            ));
                        }
                        Ty::Pointer {
                            pointee: pointee.clone(),
                            volatile: *volatile,
                        }
                    }
                    Ty::Distinct { base: db, name } => match **db {
                        Ty::Pointer { ref pointee, .. } => {
                            if !matches!(**pointee, Ty::Struct(_)) {
                                return Err(terr(
                                    *span,
                                    format!(
                                        "cannot access field `.{}` through `{}` (pointer to non-struct `{}`)",
                                        field,
                                        name,
                                        pointee.display()
                                    ),
                                ));
                            }
                            bt.clone()
                        }
                        ref o => {
                            return Err(terr(
                                *span,
                                format!("cannot access field `.{}` on distinct type `{}` wrapping `{}`", field, name, o.display()),
                            ))
                        }
                    },
                    ref o => {
                        return Err(terr(
                            *span,
                            format!(
                                "cannot access field `.{}` on non-struct type `{}`",
                                field,
                                o.display()
                            ),
                        ))
                    }
                };
                let idx = match &struct_ty {
                    Ty::Struct(i) => *i,
                    Ty::Pointer { pointee, .. } | Ty::Distinct { base: pointee, .. } => {
                        match **pointee {
                            Ty::Struct(i) => i,
                            _ => unreachable!("checked above"),
                        }
                    }
                    _ => unreachable!("checked above"),
                };
                let def = self.mt.structs[idx].clone();
                for f in &def.fields {
                    if &f.name == field {
                        return Ok((f.ty.clone(), None));
                    }
                }
                Err(terr(
                    *span,
                    format!("struct `{}` has no field `{}`", def.name, field),
                ))
            }
            Expr::Call { callee, args, span } => self.check_call(callee, args, *span),
        }
    }

    /// Call resolution order:
    ///   1. user-declared function
    ///   2. conversion constructor: distinct wrap (`Vaddr(x)`), unwrap
    ///      (`u64(p)`), or literal narrowing through a primitive name
    fn check_call(&mut self, callee: &str, args: &[Expr], span: Span) -> TResult<(Ty, Option<i128>)> {
        if let Some(sig) = self.fns.get(callee).cloned() {
            if args.len() != sig.params.len() {
                return Err(terr(
                    span,
                    format!(
                        "function `{}` takes {} argument(s), got {}",
                        callee,
                        sig.params.len(),
                        args.len()
                    ),
                ));
            }
            for (p, a) in sig.params.iter().zip(args.iter()) {
                let (at, _) = self.check_expr(a)?;
                coerce(p.clone(), at, a)?;
            }
            return Ok((sig.ret.clone(), None));
        }

        let target = self.lookup_named_type(callee, span)?;

        if args.len() != 1 {
            return Err(terr(
                span,
                format!(
                    "conversion to `{}` takes exactly 1 argument, got {}",
                    target.display(),
                    args.len()
                ),
            ));
        }
        let arg = &args[0];
        let (arg_ty, folded) = self.check_expr(arg)?;

        match &target {
            Ty::Distinct { base, .. } => {
                // wrap: base-typed value (or fitting literal) -> distinct
                if types_compatible(base, &arg_ty)
                    || (arg_ty.is_integer() && base.is_integer() && is_literal(arg))
                {
                    coerce((**base).clone(), arg_ty, arg)?;
                    Ok((target, folded))
                } else {
                    Err(terr(
                        expr_span(arg),
                        format!(
                            "cannot construct `{}` from `{}` (expects its exact base type `{}`)",
                            target.display(),
                            arg_ty.display(),
                            base.display()
                        ),
                    ))
                }
            }
            prim if prim.is_integer() => {
                match &arg_ty {
                    Ty::Distinct { base, name } => {
                        if **base == *prim {
                            Ok((prim.clone(), folded))
                        } else {
                            Err(terr(
                                span,
                                format!(
                                    "cannot unwrap `{}` to `{}` (its base type is `{}`)",
                                    name,
                                    target.display(),
                                    base.display()
                                ),
                            ))
                        }
                    }
                    t if *t == *prim => Ok((prim.clone(), folded)),
                    t if t.is_integer() && is_literal(arg) => {
                        coerce(prim.clone(), t.clone(), arg)?;
                        Ok((prim.clone(), folded))
                    }
                    other => Err(terr(
                        span,
                        format!(
                            "invalid conversion from `{}` to `{}`",
                            other.display(),
                            target.display()
                        ),
                    )),
                }
            }
            other => Err(terr(
                span,
                format!(
                    "type `{}` cannot be used as a conversion constructor in v0",
                    other.display()
                ),
            )),
        }
    }

    fn lookup_named_type(&self, name: &str, span: Span) -> TResult<Ty> {
        if let Some(t) = primitive_by_name(name) {
            if matches!(t, Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 | Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64)
            {
                return Ok(t);
            }
            return Err(terr(span, format!("type `{}` cannot be constructed", name)));
        }
        match self.mt.named_types.get(name) {
            Some(t) => Ok(t.clone()),
            None => Err(terr(span, format!("unknown function or type `{}`", name))),
        }
    }

    fn check_binary(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
    ) -> TResult<(Ty, Option<i128>)> {
        let (lt, lcv) = self.check_expr(lhs)?;
        let (rt, rcv) = self.check_expr(rhs)?;

        match op {
            BinOp::And | BinOp::Or => {
                if lt != Ty::Bool || rt != Ty::Bool {
                    return Err(terr(
                        span,
                        format!(
                            "`&&`/`||` require `bool` operands, found `{}` and `{}`",
                            lt.display(),
                            rt.display()
                        ),
                    ));
                }
                Ok((Ty::Bool, None))
            }
            BinOp::EqCmp | BinOp::NeCmp | BinOp::LtCmp | BinOp::GtCmp | BinOp::LeCmp
            | BinOp::GeCmp => {
                self.common_int_or_error(&lt, &rt, lhs, rhs, span)?;
                Ok((Ty::Bool, None))
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::BitAnd
            | BinOp::BitOr | BinOp::BitXor => {
                let common = self.common_int_or_error(&lt, &rt, lhs, rhs, span)?;
                let folded = match (lcv, rcv) {
                    (Some(a), Some(b)) => apply_arith(op, a, b),
                    _ => None,
                };
                Ok((common, folded))
            }
            BinOp::Shl | BinOp::Shr => {
                if !lt.is_integer() || !rt.is_integer() {
                    return Err(terr(
                        span,
                        format!(
                            "shift operands must be integers, found `{}` and `{}`",
                            lt.display(),
                            rt.display()
                        ),
                    ));
                }
                reject_distinct_operand(&lt, lhs)?;
                reject_distinct_operand(&rt, rhs)?;
                let folded = match (lcv, rcv) {
                    (Some(a), Some(b)) => apply_shift(op, a, b),
                    _ => None,
                };
                Ok((lt, folded))
            }
        }
    }

    /// Arithmetic/comparison discipline: both sides end up the SAME integer
    /// primitive type. Unsuffixed literals adapt to the other side. Distinct
    /// types are rejected BY DESIGN: they must be unwrapped explicitly first
    /// (this is rule #3 of CLAUDE.md enforced at compile time).
    fn common_int_or_error(
        &mut self,
        lt: &Ty,
        rt: &Ty,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
    ) -> TResult<Ty> {
        reject_distinct_operand(lt, lhs)?;
        reject_distinct_operand(rt, rhs)?;

        if lt.is_integer() && rt.is_integer() {
            let lit_lhs = is_literal(lhs);
            let lit_rhs = is_literal(rhs);
            if lit_lhs && !lit_rhs {
                check_literal_fits(lhs, rt)?;
                return Ok(rt.clone());
            }
            if lit_rhs && !lit_lhs {
                check_literal_fits(rhs, lt)?;
                return Ok(lt.clone());
            }
            if lit_lhs && lit_rhs {
                return Ok(Ty::U64);
            }
            if lt == rt {
                return Ok(lt.clone());
            }
            return Err(terr(
                span,
                format!(
                    "operand types differ: `{}` vs `{}`: no implicit widening/narrowing between integer widths",
                    lt.display(),
                    rt.display()
                ),
            ));
        }
        Err(terr(
            span,
            format!(
                "operator requires integer operands, found `{}` and `{}`",
                lt.display(),
                rt.display()
            ),
        ))
    }

    // ---- const folding -----------------------------------------------------

    fn const_eval(&mut self, expr: &Expr) -> TResult<(ConstVal, Ty)> {
        match expr {
            Expr::Str(s, _) => Ok((
                ConstVal::Str(s.bytes().collect()),
                Ty::Pointer {
                    pointee: Box::new(Ty::U8),
                    volatile: false,
                },
            )),
            _ => {
                let (ty, folded) = self.check_expr(expr)?;
                match folded {
                    Some(v) => Ok((ConstVal::Int(v), ty)),
                    None => Err(terr(
                        expr_span(expr),
                        "const initializer is not compile-time evaluable (only literals, const references, and integer ops in v0)",
                    )),
                }
            }
        }
    }

    // ---- move checking (Phase 1 extension point) -----------------------------

    /// v0: every type is Copy, so this pass records nothing. Phase 1 adds owned
    /// buffers/channels/capabilities where use-after-move is a hard error; the
    /// expression walker already visits every binding use, so that checker
    /// plugs into this call site without restructuring anything else.
    #[allow(dead_code)]
    fn check_moves(&mut self, _body: &Block) {}
}

fn reject_distinct_operand(t: &Ty, e: &Expr) -> TResult<()> {
    if let Ty::Distinct { name, .. } = t {
        Err(terr(
            expr_span(e),
            format!(
                "distinct type `{}` does not participate in arithmetic/comparison: unwrap it explicitly first, e.g. `u64({})`",
                name,
                expr_text(e)
            ),
        ))
    } else {
        Ok(())
    }
}

fn apply_arith(op: &BinOp, a: i128, b: i128) -> Option<i128> {
    Some(match op {
        BinOp::Add => a.checked_add(b)?,
        BinOp::Sub => a.checked_sub(b)?,
        BinOp::Mul => a.checked_mul(b)?,
        BinOp::Div => a.checked_div(b)?,
        BinOp::Mod => a.checked_rem(b)?,
        BinOp::BitAnd => a & b,
        BinOp::BitOr => a | b,
        BinOp::BitXor => a ^ b,
        _ => return None,
    })
}

fn apply_shift(op: &BinOp, a: i128, b: i128) -> Option<i128> {
    if b < 0 || b >= 64 {
        return None;
    }
    Some(match op {
        BinOp::Shl => a << b,
        BinOp::Shr => a >> b,
        _ => return None,
    })
}

fn is_literal(e: &Expr) -> bool {
    matches!(e, Expr::Int(..) | Expr::Char(..))
}

fn check_literal_fits(lit: &Expr, target: &Ty) -> TResult<()> {
    let v: i128 = match lit {
        Expr::Int(v, _) => *v as i128,
        Expr::Char(c, _) => *c as i128,
        _ => return Ok(()),
    };
    let (bits, signed) = int_width(target).ok_or_else(|| TypeError {
        msg: "literal cannot be narrowed to a non-integer type".into(),
        span: expr_span(lit),
    })?;
    let fits = if signed {
        v >= -(1i128 << (bits - 1)) && v < (1i128 << (bits - 1))
    } else {
        v >= 0 && v < (1i128 << bits)
    };
    if !fits {
        return Err(terr(
            expr_span(lit),
            format!("literal `{}` does not fit in `{}`", v, target.display()),
        ));
    }
    Ok(())
}

fn ensure_lvalue(target: &Expr) -> TResult<()> {
    match target {
        Expr::Ident(..) | Expr::Index { .. } | Expr::Field { .. } => Ok(()),
        other => Err(terr(
            expr_span(other),
            "invalid assignment target (not a variable, index, or field)",
        )),
    }
}

/// Cast legality matrix (spec §5): pointer↔pointer, integer↔integer,
/// integer↔pointer: always explicit-only. Anything touching a distinct type
/// MUST go through constructor/unwrap calls instead; `x as Paddr` is a hard
/// error so there is exactly one blessed conversion syntax per direction.
fn check_cast(src: &Ty, dst: &Ty, span: Span) -> TResult<()> {
    for t in [src, dst] {
        if let Ty::Distinct { name, .. } = t {
            return Err(terr(
                span,
                format!(
                    "`as` cannot touch distinct type `{name}`: construct/unwrap explicitly ({name}(x) / u64(x))"
                ),
            ));
        }
        if matches!(t, Ty::Void | Ty::Never | Ty::Bool | Ty::Struct(_)) {
            return Err(terr(
                span,
                format!("`as` casts cannot involve `{}`", t.display()),
            ));
        }
    }
    match (src.is_integer(), dst.is_integer()) {
        (true, true) => Ok(()),
        (true, false) => match dst {
            Ty::Pointer { .. } => Ok(()),
            _ => Err(terr(span, "invalid cast")),
        },
        (false, true) => match src {
            Ty::Pointer { .. } | Ty::Array { .. } => Ok(()),
            _ => Err(terr(span, "invalid cast")),
        },
        (false, false) => match (src, dst) {
            (Ty::Pointer { .. }, Ty::Pointer { .. }) => Ok(()),
            // explicit array decay: `buf as *const u8`
            (Ty::Array { elem, .. }, Ty::Pointer { pointee, .. }) => {
                if types_compatible(pointee, elem) {
                    Ok(())
                } else {
                    Err(terr(
                        span,
                        format!(
                            "cannot cast array of `{}` to pointer to `{}`",
                            elem.display(),
                            pointee.display()
                        ),
                    ))
                }
            }
            _ => Err(terr(span, "invalid cast")),
        },
    }
}

// ---- diagnostics helpers -------------------------------------------------

fn expr_span(e: &Expr) -> Span {
    match e {
        Expr::Int(_, s)
        | Expr::Char(_, s)
        | Expr::Str(_, s)
        | Expr::Bool(_, s)
        | Expr::Ident(_, s)
        | Expr::Call { span: s, .. }
        | Expr::Index { span: s, .. }
        | Expr::Field { span: s, .. }
        | Expr::Cast { span: s, .. }
        | Expr::Unary { span: s, .. }
        | Expr::Binary { span: s, .. } => *s,
    }
}

fn init_span(e: &Expr) -> Span {
    expr_span(e)
}

fn expr_text(e: &Expr) -> String {
    match e {
        Expr::Ident(n, _) => n.clone(),
        Expr::Int(v, _) => v.to_string(),
        Expr::Char(c, _) => format!("'{}'", *c as char),
        Expr::Bool(b, _) => b.to_string(),
        Expr::Str(_, _) => "\"...\"".to_string(),
        Expr::Unary { op, expr, .. } => format!(
            "{}{}",
            match op {
                UnaryOp::Neg => "-",
                UnaryOp::Not => "!",
                UnaryOp::AddrOf => "&",
            },
            expr_text(expr)
        ),
        Expr::Binary { op, lhs, rhs, .. } => format!(
            "{} {} {}",
            expr_text(lhs),
            match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Mod => "%",
                BinOp::EqCmp => "==",
                BinOp::NeCmp => "!=",
                BinOp::LtCmp => "<",
                BinOp::GtCmp => ">",
                BinOp::LeCmp => "<=",
                BinOp::GeCmp => ">=",
                BinOp::And => "&&",
                BinOp::Or => "||",
                BinOp::BitAnd => "&",
                BinOp::BitOr => "|",
                BinOp::BitXor => "^",
                BinOp::Shl => "<<",
                BinOp::Shr => ">>",
            },
            expr_text(rhs)
        ),
        Expr::Call { callee, args, .. } => format!(
            "{}({})",
            callee,
            args.iter().map(expr_text).collect::<Vec<_>>().join(", ")
        ),
        Expr::Index { base, index, .. } => format!("{}[{}]", expr_text(base), expr_text(index)),
        Expr::Field { base, field, .. } => format!("{}.{}", expr_text(base), field),
        Expr::Cast { expr, ty, .. } => format!("{} as {}", expr_text(expr), typeexpr_name(ty)),
    }
}

fn typeexpr_name(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Named { name, .. } => name.clone(),
        TypeExpr::Pointer {
            pointee,
            volatile,
            ..
        } => format!(
            "*{}{}",
            if *volatile { "volatile " } else { "" },
            typeexpr_name(pointee)
        ),
        TypeExpr::Array { len, elem, .. } => {
            format!("[{}]{}", expr_text(len), typeexpr_name(elem))
        }
    }
}
