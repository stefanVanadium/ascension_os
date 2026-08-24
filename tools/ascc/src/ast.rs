//! Asc AST: the shape the parser produces and every later stage consumes.

use crate::lexer::Span;

#[derive(Debug, Clone)]
pub struct Module {
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone)]
pub enum Decl {
    /// `type Name = distinct BaseType;`
    TypeAlias {
        name: String,
        span: Span,
        base: TypeExpr,
    },
    Struct {
        name: String,
        span: Span,
        packed: bool,
        fields: Vec<StructField>,
    },
    Const {
        name: String,
        span: Span,
        ty: TypeExpr,
        init: Expr,
    },
    Function {
        name: String,
        span: Span,
        ret: TypeExpr,
        params: Vec<Param>,
        body: Block,
    },
    /// Bodyless function declaration (`fn f(...) -> T;`): an extern symbol.
    /// No definition is emitted; the linker resolves it against another unit.
    FnProto {
        name: String,
        span: Span,
        ret: TypeExpr,
        params: Vec<Param>,
    },
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone)]
pub enum TypeExpr {
    /// Primitive keyword (`u64`, `bool`, ...) or a named user type.
    Named { name: String, span: Span },
    Pointer {
        pointee: Box<TypeExpr>,
        volatile: bool,
        span: Span,
    },
    /// `[N]T`: fixed-size value array. `len` is a compile-time integer
    /// expression (literal or const reference). No nesting in v1.
    Array {
        len: Box<Expr>,
        elem: Box<TypeExpr>,
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        span: Span,
        ty: Option<TypeExpr>,
        init: Option<Expr>,
    },
    Assign {
        target: Expr,
        value: Expr,
        span: Span,
    },
    Expr(Expr),
    If {
        cond: Expr,
        then_body: Block,
        else_body: Option<Block>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Asm(AsmBlock),
}

#[derive(Debug, Clone)]
pub struct AsmBlock {
    pub template: String,
    pub outputs: Vec<AsmOperand>, // constraint + pre-declared local binding
    pub inputs: Vec<(String, Expr)>, // constraint + expression
    pub clobbers: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct AsmOperand {
    pub constraint: String,
    pub binding: String,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(u64, Span),
    Char(u8, Span),
    Str(String, Span),
    Bool(bool, Span),
    Ident(String, Span),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Call {
        callee: String,
        args: Vec<Expr>,
        span: Span,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Field {
        base: Box<Expr>,
        field: String,
        span: Span,
    },
    Cast {
        expr: Box<Expr>,
        ty: TypeExpr,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    /// `&expr`: address-of an lvalue; result is `*mut T`.
    AddrOf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    EqCmp,
    NeCmp,
    LtCmp,
    GtCmp,
    LeCmp,
    GeCmp,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}
