//! Asc parser: recursive descent with a Pratt loop for binary expressions.
//!
//! Produces the AST in `ast.rs`. Fails fast on the first syntax error with a
//! file:line:col diagnostic; there is no error recovery in v0.

use crate::ast::*;
use crate::lexer::{Span, TokKind, Token};

pub struct ParseError {
    pub msg: String,
    pub span: Span,
}

type PResult<T> = Result<T, ParseError>;

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

/// Binary operator precedence: higher binds tighter. Mirrors spec §5 + C-style
/// bitwise placement.
fn binop_precedence(kind: &TokKind) -> Option<u8> {
    Some(match kind {
        TokKind::PipePipe => 1,
        TokKind::AmpAmp => 2,
        TokKind::Pipe => 3,
        TokKind::Caret => 4,
        TokKind::Amp => 5,
        TokKind::Eq | TokKind::Neq => 6,
        TokKind::Lt | TokKind::Gt | TokKind::Le | TokKind::Ge => 7,
        TokKind::Shl | TokKind::Shr => 8,
        TokKind::Plus | TokKind::Minus => 9,
        TokKind::Star | TokKind::Slash | TokKind::Percent => 10,
        _ => return None,
    })
}

fn binop_kind(kind: &TokKind) -> BinOp {
    match kind {
        TokKind::PipePipe => BinOp::Or,
        TokKind::AmpAmp => BinOp::And,
        TokKind::Pipe => BinOp::BitOr,
        TokKind::Caret => BinOp::BitXor,
        TokKind::Amp => BinOp::BitAnd,
        TokKind::Eq => BinOp::EqCmp,
        TokKind::Neq => BinOp::NeCmp,
        TokKind::Lt => BinOp::LtCmp,
        TokKind::Gt => BinOp::GtCmp,
        TokKind::Le => BinOp::LeCmp,
        TokKind::Ge => BinOp::GeCmp,
        TokKind::Shl => BinOp::Shl,
        TokKind::Shr => BinOp::Shr,
        TokKind::Plus => BinOp::Add,
        TokKind::Minus => BinOp::Sub,
        TokKind::Star => BinOp::Mul,
        TokKind::Slash => BinOp::Div,
        TokKind::Percent => BinOp::Mod,
        other => unreachable!("non-binary token reached binop_kind: {:?}", other),
    }
}

pub fn parse(tokens: Vec<Token>) -> PResult<Module> {
    let mut p = Parser { tokens, pos: 0 };
    let mut decls = Vec::new();
    while !p.at_end() {
        decls.push(p.parse_decl()?);
    }
    Ok(Module { decls })
}

impl Parser {
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek2(&self) -> Option<&Token> {
        self.tokens.get(self.pos + 1)
    }

    fn err<T>(&self, msg: impl Into<String>) -> PResult<T> {
        let span = self
            .peek()
            .map(|t| t.span)
            .unwrap_or(Span { line: u32::MAX, col: 0 });
        Err(ParseError { msg: msg.into(), span })
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        self.pos += 1;
        t
    }

    fn expect(&mut self, kind: TokKind, what: &str) -> PResult<Token> {
        match self.peek() {
            Some(t) if t.kind == kind => Ok(self.advance()),
            Some(t) => self.err(format!("expected {} but found {}", what, t.kind)),
            None => Err(ParseError {
                msg: format!("unexpected end of file, expected {}", what),
                span: Span { line: u32::MAX, col: 0 },
            }),
        }
    }

    fn eat(&mut self, kind: TokKind) -> bool {
        if self.peek().map(|t| t.kind == kind).unwrap_or(false) {
            self.advance();
            true
        } else {
            false
        }
    }

    // ---- declarations -------------------------------------------------

    fn parse_decl(&mut self) -> PResult<Decl> {
        // #[packed] struct ...: the only attribute in v0
        let mut packed = false;
        if self.peek().map(|t| t.kind == TokKind::Hash).unwrap_or(false) {
            self.advance();
            self.expect(TokKind::LBracket, "`[` after `#`")?;
            match self.advance() {
                // `packed` is a keyword token, not an identifier: the
                // Ident arm here could never fire and #[packed] was dead
                Token { kind: TokKind::Packed, span } => {
                    packed = true;
                    let _ = span;
                }
                t => return self.err(format!("unknown attribute `{}` (only #[packed] exists in v0)", t.kind)),
            }
            self.expect(TokKind::RBracket, "`]` to close attribute")?;
            if !matches!(self.peek().map(|t| t.kind.clone()), Some(TokKind::Struct)) {
                return self.err("`#[packed]` applies only to structs");
            }
        }

        match self.peek().map(|t| t.kind.clone()) {
            Some(TokKind::TypeKw) => self.parse_type_alias(),
            Some(TokKind::Struct) => self.parse_struct(packed),
            Some(TokKind::Const) => self.parse_const(),
            Some(TokKind::Let) => self.parse_static(),
            Some(TokKind::Fn) => self.parse_function(),
            Some(other) => self.err(format!("expected declaration (`fn`, `const`, `let`, `type`, `struct`), found {}", other)),
            None => self.err("expected declaration"),
        }
    }

    fn parse_type_alias(&mut self) -> PResult<Decl> {
        self.advance(); // `type`
        let name = match self.advance() {
            Token { kind: TokKind::Ident(n), span } => (n, span),
            t => return self.err(format!("expected type name after `type`, found {}", t.kind)),
        };
        self.expect(TokKind::Assign, "`=` in type alias")?;
        self.expect(TokKind::Distinct, "`distinct` (v0 aliases must be distinct)")?;
        let base = self.parse_type()?;
        self.expect(TokKind::Semi, "`;` after type alias")?;
        Ok(Decl::TypeAlias {
            name: name.0,
            span: name.1,
            base,
        })
    }

    fn parse_struct(&mut self, packed: bool) -> PResult<Decl> {
        self.advance(); // `struct`
        let name = match self.advance() {
            Token { kind: TokKind::Ident(n), span } => (n, span),
            t => return self.err(format!("expected struct name after `struct`, found {}", t.kind)),
        };
        self.expect(TokKind::LBrace, "`{` to open struct body")?;
        let mut fields = Vec::new();
        while !self.eat(TokKind::RBrace) {
            let fname = match self.advance() {
                Token { kind: TokKind::Ident(n), .. } => n,
                t => return self.err(format!("expected field name in struct, found {}", t.kind)),
            };
            self.expect(TokKind::Colon, "`:` after field name")?;
            let fty = self.parse_type()?;
            fields.push(StructField { name: fname, ty: fty });
            // trailing comma optional: a missing comma must mean `}` next
            if !self.eat(TokKind::Comma) {
                self.expect(TokKind::RBrace, "`}` or `,` after field")?;
                break;
            }
        }
        Ok(Decl::Struct {
            name: name.0,
            span: name.1,
            packed,
            fields,
        })
    }

    fn parse_const(&mut self) -> PResult<Decl> {
        self.advance(); // `const`
        let name = match self.advance() {
            Token { kind: TokKind::Ident(n), span } => (n, span),
            t => return self.err(format!("expected const name after `const`, found {}", t.kind)),
        };
        self.expect(TokKind::Colon, "`:` after const name")?;
        let ty = self.parse_type()?;
        self.expect(TokKind::Assign, "`=` in const initializer")?;
        let init = self.parse_expr(0)?;
        self.expect(TokKind::Semi, "`;` after const declaration")?;
        Ok(Decl::Const {
            name: name.0,
            span: name.1,
            ty,
            init,
        })
    }

    /// Module-level binding: `let name: T;` — annotation required (there is
    /// no initializer to infer from), initializer forbidden (zero-initialized).
    fn parse_static(&mut self) -> PResult<Decl> {
        self.advance(); // `let`
        let name = match self.advance() {
            Token { kind: TokKind::Ident(n), span: s } => (n, s),
            t => return self.err(format!("expected binding name after `let`, found {}", t.kind)),
        };
        self.expect(TokKind::Colon, "`:` after module-level binding name (annotation required)")?;
        let ty = self.parse_type()?;
        if self.eat(TokKind::Assign) {
            return self.err("module-level bindings are zero-initialized; initializers are not allowed");
        }
        self.expect(TokKind::Semi, "`;` after module-level binding")?;
        Ok(Decl::Static {
            name: name.0,
            span: name.1,
            ty,
        })
    }

    fn parse_function(&mut self) -> PResult<Decl> {
        self.advance(); // `fn`
        let name = match self.advance() {
            Token { kind: TokKind::Ident(n), span } => (n, span),
            t => return self.err(format!("expected function name after `fn`, found {}", t.kind)),
        };
        self.expect(TokKind::LParen, "`(` after function name")?;
        let mut params = Vec::new();
        if !self.eat(TokKind::RParen) {
            loop {
                let pname = match self.advance() {
                    Token { kind: TokKind::Ident(n), .. } => n,
                    t => return self.err(format!("expected parameter name, found {}", t.kind)),
                };
                self.expect(TokKind::Colon, "`:` after parameter name")?;
                let pty = self.parse_type()?;
                params.push(Param { name: pname, ty: pty });
                if self.eat(TokKind::Comma) {
                    continue;
                }
                self.expect(TokKind::RParen, "`)` or `,` in parameter list")?;
                break;
            }
        }
        // `-> T` optional: absence means `void`
        let ret = if self.eat(TokKind::Arrow) {
            self.parse_type()?
        } else {
            TypeExpr::Named {
                name: "void".to_string(),
                span: name.1,
            }
        };
        // `{ ... }` = definition; `;` = bodyless extern declaration
        if self.eat(TokKind::Semi) {
            return Ok(Decl::FnProto {
                name: name.0,
                span: name.1,
                ret,
                params,
            });
        }
        let body = self.parse_block()?;
        Ok(Decl::Function {
            name: name.0,
            span: name.1,
            ret,
            params,
            body,
        })
    }

    // ---- types ---------------------------------------------------------

    fn parse_type(&mut self) -> PResult<TypeExpr> {
        let span = self.peek().map(|t| t.span).unwrap_or(Span { line: 0, col: 0 });
        if self.eat(TokKind::LBracket) {
            // [N]T: N is a const-evaluable integer expression; parse_expr
            // stops naturally at `]` (not an operator).
            let len = self.parse_expr(0)?;
            self.expect(TokKind::RBracket, "`]` to close array length")?;
            let elem = Box::new(self.parse_type()?);
            return Ok(TypeExpr::Array { len: Box::new(len), elem, span });
        }
        if self.eat(TokKind::Star) {
            // *const T / *volatile T / both: `const` is documentation-only in
            // v0 (no enforcement of writes through it yet)
            let _read_only = self.eat(TokKind::Const);
            let volatile = self.eat(TokKind::Volatile);
            let pointee = Box::new(self.parse_type()?);
            return Ok(TypeExpr::Pointer {
                pointee,
                volatile,
                span,
            });
        }
        let name = match self.peek() {
            Some(t) => match t.kind {
                TokKind::U8
                | TokKind::U16
                | TokKind::U32
                | TokKind::U64
                | TokKind::I8
                | TokKind::I16
                | TokKind::I32
                | TokKind::I64
                | TokKind::Bool
                | TokKind::Never
                | TokKind::Ident(_) => {
                    let tok = self.advance();
                    match tok.kind {
                        TokKind::U8 => "u8",
                        TokKind::U16 => "u16",
                        TokKind::U32 => "u32",
                        TokKind::U64 => "u64",
                        TokKind::I8 => "i8",
                        TokKind::I16 => "i16",
                        TokKind::I32 => "i32",
                        TokKind::I64 => "i64",
                        TokKind::Bool => "bool",
                        TokKind::Never => "never",
                        TokKind::Ident(ref n) => n.as_str(),
                        _ => unreachable!(),
                    }
                    .to_string()
                }
                ref other => return self.err(format!("expected type, found {}", other)),
            },
            None => return self.err("expected type, found end of file"),
        };
        Ok(TypeExpr::Named { name, span })
    }

    // ---- statements ------------------------------------------------------

    fn parse_block(&mut self) -> PResult<Block> {
        self.expect(TokKind::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        while !self.eat(TokKind::RBrace) {
            if self.at_end() {
                return self.err("unexpected end of file inside block");
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(Block { stmts })
    }

    fn parse_stmt(&mut self) -> PResult<Stmt> {
        let span = self.peek().map(|t| t.span).unwrap_or(Span { line: 0, col: 0 });

        // #[packed] attribute before nested items is not allowed; attributes only on structs.
        if self.peek().map(|t| t.kind == TokKind::Let).unwrap_or(false) {
            return self.parse_let();
        }
        if self.peek().map(|t| t.kind == TokKind::Return).unwrap_or(false) {
            self.advance();
            if self.eat(TokKind::Semi) {
                return Ok(Stmt::Return { value: None, span });
            }
            let value = self.parse_expr(0)?;
            self.expect(TokKind::Semi, "`;` after return expression")?;
            return Ok(Stmt::Return { value: Some(value), span });
        }
        if self.peek().map(|t| t.kind == TokKind::If).unwrap_or(false) {
            self.advance();
            let cond = self.parse_expr(0)?;
            let then_body = self.parse_block()?;
            let else_body = if self.eat(TokKind::Else) {
                Some(self.parse_block()?)
            } else {
                None
            };
            return Ok(Stmt::If {
                cond,
                then_body,
                else_body,
                span,
            });
        }
        if self.peek().map(|t| t.kind == TokKind::While).unwrap_or(false) {
            self.advance();
            let cond = self.parse_expr(0)?;
            let body = self.parse_block()?;
            return Ok(Stmt::While { cond, body, span });
        }
        if self.peek().map(|t| t.kind == TokKind::Break).unwrap_or(false) {
            self.advance();
            self.expect(TokKind::Semi, "`;` after `break`")?;
            return Ok(Stmt::Break { span });
        }
        if self.peek().map(|t| t.kind == TokKind::Continue).unwrap_or(false) {
            self.advance();
            self.expect(TokKind::Semi, "`;` after `continue`")?;
            return Ok(Stmt::Continue { span });
        }
        if self.peek().map(|t| t.kind == TokKind::Asm).unwrap_or(false) {
            return Ok(Stmt::Asm(self.parse_asm()?));
        }

        // assignment or expression statement: parse an expression; if followed by
        // `=`, it was an lvalue target.
        let expr = self.parse_expr(0)?;
        if self.eat(TokKind::Assign) {
            let value = self.parse_expr(0)?;
            self.expect(TokKind::Semi, "`;` after assignment")?;
            return Ok(Stmt::Assign {
                target: expr,
                value,
                span,
            });
        }
        self.expect(TokKind::Semi, "`;` after expression statement")?;
        Ok(Stmt::Expr(expr))
    }

    fn parse_let(&mut self) -> PResult<Stmt> {
        self.advance(); // `let`
        let name = match self.advance() {
            Token { kind: TokKind::Ident(n), span: s } => (n, s),
            t => return self.err(format!("expected binding name after `let`, found {}", t.kind)),
        };
        let ty = if self.eat(TokKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let init = if self.eat(TokKind::Assign) {
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        self.expect(TokKind::Semi, "`;` after `let`")?;
        Ok(Stmt::Let {
            name: name.0,
            span: name.1,
            ty,
            init,
        })
    }

    /// asm { TEMPLATE (: OUTPUTS)? (: INPUTS)? (: CLOBBERS)? }
    fn parse_asm(&mut self) -> PResult<AsmBlock> {
        self.advance(); // `asm`
        let span = self.peek().map(|t| t.span).unwrap_or(Span { line: 0, col: 0 });
        self.expect(TokKind::LBrace, "`{` after `asm`")?;
        let template = match self.advance() {
            Token { kind: TokKind::Str(s), .. } => s,
            t => return self.err(format!("expected template string in asm block, found {}", t.kind)),
        };

        let mut outputs = Vec::new();
        let mut inputs = Vec::new();
        let mut clobbers = Vec::new();

        let mut section = 0usize; // 0=outputs 1=inputs 2=clobbers
        while self.eat(TokKind::Colon) {
            section += 1;
            if section > 3 {
                return self.err("asm block has too many `:` sections");
            }
            loop {
                match self.peek().map(|t| t.kind.clone()) {
                    Some(TokKind::Str(constraint)) => {
                        self.advance();
                        match section {
                            1 => {
                                // output: constraint(binding)
                                let binding = self.expect_ident_in_parens()?;
                                outputs.push(AsmOperand {
                                    constraint,
                                    binding,
                                });
                            }
                            2 => {
                                let expr = self.expect_expr_in_parens()?;
                                inputs.push((constraint, expr));
                            }
                            _ => {
                                clobbers.push(constraint);
                            }
                        }
                    }
                    _ => break,
                }
                if !self.eat(TokKind::Comma) {
                    break;
                }
            }
        }

        self.expect(TokKind::RBrace, "`}` to close asm block")?;
        Ok(AsmBlock {
            template,
            outputs,
            inputs,
            clobbers,
            span,
        })
    }

    fn expect_ident_in_parens(&mut self) -> PResult<String> {
        self.expect(TokKind::LParen, "`(` around asm operand")?;
        let name = match self.advance() {
            Token { kind: TokKind::Ident(n), .. } => n,
            t => return self.err(format!("expected local variable name in asm output, found {}", t.kind)),
        };
        self.expect(TokKind::RParen, "`)` after asm output binding")?;
        Ok(name)
    }

    fn expect_expr_in_parens(&mut self) -> PResult<Expr> {
        self.expect(TokKind::LParen, "`(` around asm input")?;
        let e = self.parse_expr(0)?;
        self.expect(TokKind::RParen, "`)` after asm input")?;
        Ok(e)
    }

    // ---- expressions -----------------------------------------------------

    fn parse_expr(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            // `as` casts bind tighter than any binary operator (postfix-level).
            if self.peek().map(|t| t.kind == TokKind::As).unwrap_or(false) {
                if min_prec > 11 {
                    break;
                }
                self.advance();
                let ty = self.parse_type()?;
                let span = lhs_span(&lhs);
                lhs = Expr::Cast {
                    expr: Box::new(lhs),
                    ty,
                    span,
                };
                continue;
            }
            let op_prec = self
                .peek()
                .and_then(|t| binop_precedence(&t.kind));
            match op_prec {
                Some(prec) if prec >= min_prec => {
                    let tok = self.advance();
                    let rhs = self.parse_expr(prec + 1)?;
                    let span = lhs_span(&lhs);
                    lhs = Expr::Binary {
                        op: binop_kind(&tok.kind),
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                        span,
                    };
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        let span = self.peek().map(|t| t.span).unwrap_or(Span { line: 0, col: 0 });
        if self.eat(TokKind::Minus) {
            let inner = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(inner),
                span,
            });
        }
        if self.eat(TokKind::Bang) {
            let inner = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(inner),
                span,
            });
        }
        // prefix `&`: binary `&` (BitAnd) is only reachable between operands,
        // so an Amp in unary position is unambiguously address-of
        if self.eat(TokKind::Amp) {
            let inner = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::AddrOf,
                expr: Box::new(inner),
                span,
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.eat(TokKind::Dot) {
                let field = match self.advance() {
                    Token { kind: TokKind::Ident(n), .. } => n,
                    t => return self.err(format!("expected field name after `.`, found {}", t.kind)),
                };
                let span = expr_span(&expr);
                expr = Expr::Field {
                    base: Box::new(expr),
                    field,
                    span,
                };
                continue;
            }
            if self.peek().map(|t| t.kind == TokKind::LBracket).unwrap_or(false) {
                self.advance();
                let index = self.parse_expr(0)?;
                self.expect(TokKind::RBracket, "`]` to close index")?;
                let span = expr_span(&expr);
                expr = Expr::Index {
                    base: Box::new(expr),
                    index: Box::new(index),
                    span,
                };
                continue;
            }
            break;
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let tok = match self.peek() {
            Some(t) => t.clone(),
            None => return self.err("expected expression, found end of file"),
        };
        let span = tok.span;
        match tok.kind {
            TokKind::Int(v) => {
                self.advance();
                Ok(Expr::Int(v, span))
            }
            TokKind::Char(c) => {
                self.advance();
                Ok(Expr::Char(c, span))
            }
            TokKind::Str(s) => {
                self.advance();
                Ok(Expr::Str(s, span))
            }
            TokKind::True => {
                self.advance();
                Ok(Expr::Bool(true, span))
            }
            TokKind::False => {
                self.advance();
                Ok(Expr::Bool(false, span))
            }
            TokKind::LParen => {
                self.advance();
                let inner = self.parse_expr(0)?;
                self.expect(TokKind::RParen, "`)`")?;
                Ok(inner)
            }
            TokKind::U8
            | TokKind::U16
            | TokKind::U32
            | TokKind::U64
            | TokKind::I8
            | TokKind::I16
            | TokKind::I32
            | TokKind::I64 => {
                // conversion-constructor call syntax: u64(expr), i32(expr), ...
                let name = match tok.kind {
                    TokKind::U8 => "u8",
                    TokKind::U16 => "u16",
                    TokKind::U32 => "u32",
                    TokKind::U64 => "u64",
                    TokKind::I8 => "i8",
                    TokKind::I16 => "i16",
                    TokKind::I32 => "i32",
                    TokKind::I64 => "i64",
                    _ => unreachable!(),
                };
                self.advance();
                self.expect(TokKind::LParen, "`(` (primitive names are only valid as conversions)")?;
                let arg = self.parse_expr(0)?;
                self.expect(TokKind::RParen, "`)`")?;
                Ok(Expr::Call {
                    callee: name.to_string(),
                    args: vec![arg],
                    span,
                })
            }
            TokKind::Ident(name) => {
                self.advance();
                if self.peek().map(|t| t.kind == TokKind::LParen).unwrap_or(false) {
                    self.advance();
                    let mut args = Vec::new();
                    if !self.eat(TokKind::RParen) {
                        loop {
                            args.push(self.parse_expr(0)?);
                            if self.eat(TokKind::Comma) {
                                continue;
                            }
                            self.expect(TokKind::RParen, "`)` or `,` in argument list")?;
                            break;
                        }
                    }
                    return Ok(Expr::Call { callee: name, args, span });
                }
                Ok(Expr::Ident(name, span))
            }
            other => self.err(format!("expected expression, found {}", other)),
        }
    }
}

// helper used by parse_let above (kept trivial)
#[allow(dead_code)]
fn return_ok(n: String, s: Span) -> (String, Span) {
    (n, s)
}

fn lhs_span(e: &Expr) -> Span {
    expr_span(e)
}

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
