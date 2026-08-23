//! Asc lexer — turns `.asc` source into a flat token stream with spans.
//!
//! No recovery, no heuristics: a lexical error is reported with file:line:col
//! and compilation stops. Correctness rules never downgrade to warnings.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokKind {
    Ident(String),
    Int(u64),
    Char(u8),
    Str(String),

    // keywords
    Fn,
    Let,
    Const,
    Return,
    If,
    Else,
    While,
    True,
    False,
    TypeKw,
    Distinct,
    Struct,
    Asm,
    Volatile,
    Packed,
    As,
    Never,
    Bool,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,

    // punctuation / operators
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Arrow,
    Dot,
    Hash,
    Assign,
    Eq,
    Neq,
    Lt,
    Gt,
    Le,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    AmpAmp,
    PipePipe,
    Amp,
    Pipe,
    Caret,
    Shl,
    Shr,
}

impl std::fmt::Display for TokKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            TokKind::Ident(n) => return write!(f, "identifier `{}`", n),
            TokKind::Int(v) => return write!(f, "integer `{}`", v),
            TokKind::Char(c) => return write!(f, "char literal '{}'", *c as char),
            TokKind::Str(_) => "string literal".to_string(),
            TokKind::Fn => "`fn`".to_string(),
            TokKind::Let => "`let`".to_string(),
            TokKind::Const => "`const`".to_string(),
            TokKind::Return => "`return`".to_string(),
            TokKind::If => "`if`".to_string(),
            TokKind::Else => "`else`".to_string(),
            TokKind::While => "`while`".to_string(),
            TokKind::True | TokKind::False => "boolean literal".to_string(),
            TokKind::TypeKw => "`type`".to_string(),
            TokKind::Distinct => "`distinct`".to_string(),
            TokKind::Struct => "`struct`".to_string(),
            TokKind::Asm => "`asm`".to_string(),
            TokKind::Volatile => "`volatile`".to_string(),
            TokKind::Packed => "`packed`".to_string(),
            TokKind::As => "`as`".to_string(),
            TokKind::Never => "`never`".to_string(),
            TokKind::Bool => "`bool`".to_string(),
            TokKind::U8 => "`u8`".to_string(),
            TokKind::U16 => "`u16`".to_string(),
            TokKind::U32 => "`u32`".to_string(),
            TokKind::U64 => "`u64`".to_string(),
            TokKind::I8 => "`i8`".to_string(),
            TokKind::I16 => "`i16`".to_string(),
            TokKind::I32 => "`i32`".to_string(),
            TokKind::I64 => "`i64`".to_string(),
            TokKind::LParen => "`(`".to_string(),
            TokKind::RParen => "`)`".to_string(),
            TokKind::LBrace => "`{`".to_string(),
            TokKind::RBrace => "`}`".to_string(),
            TokKind::LBracket => "`[`".to_string(),
            TokKind::RBracket => "`]`".to_string(),
            TokKind::Comma => "`,`".to_string(),
            TokKind::Semi => "`;`".to_string(),
            TokKind::Colon => "`:`".to_string(),
            TokKind::Arrow => "`->`".to_string(),
            TokKind::Dot => "`.`".to_string(),
            TokKind::Hash => "`#`".to_string(),
            TokKind::Assign => "`=`".to_string(),
            TokKind::Eq => "`==`".to_string(),
            TokKind::Neq => "`!=`".to_string(),
            TokKind::Lt => "`<`".to_string(),
            TokKind::Gt => "`>`".to_string(),
            TokKind::Le => "`<=`".to_string(),
            TokKind::Ge => "`>=`".to_string(),
            TokKind::Plus => "`+`".to_string(),
            TokKind::Minus => "`-`".to_string(),
            TokKind::Star => "`*`".to_string(),
            TokKind::Slash => "`/`".to_string(),
            TokKind::Percent => "`%`".to_string(),
            TokKind::Bang => "`!`".to_string(),
            TokKind::AmpAmp => "`&&`".to_string(),
            TokKind::PipePipe => "`||`".to_string(),
            TokKind::Amp => "`&`".to_string(),
            TokKind::Pipe => "`|`".to_string(),
            TokKind::Caret => "`^`".to_string(),
            TokKind::Shl => "`<<`".to_string(),
            TokKind::Shr => "`>>`".to_string(),
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokKind,
    pub span: Span,
}

pub struct LexError {
    pub msg: String,
    pub span: Span,
}

pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;

    while i < n {
        let c = chars[i];
        let start_span = Span { line, col };

        // whitespace
        if c == ' ' || c == '\t' || c == '\r' {
            i += 1;
            col += 1;
            continue;
        }
        if c == '\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }

        // comment
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            while i < n && chars[i] != '\n' {
                i += 1;
                col += 1;
            }
            continue;
        }

        // identifier or keyword
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
                col += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let kind = match word.as_str() {
                "fn" => TokKind::Fn,
                "let" => TokKind::Let,
                "const" => TokKind::Const,
                "return" => TokKind::Return,
                "if" => TokKind::If,
                "else" => TokKind::Else,
                "while" => TokKind::While,
                "true" => TokKind::True,
                "false" => TokKind::False,
                "type" => TokKind::TypeKw,
                "distinct" => TokKind::Distinct,
                "struct" => TokKind::Struct,
                "asm" => TokKind::Asm,
                "volatile" => TokKind::Volatile,
                "packed" => TokKind::Packed,
                "as" => TokKind::As,
                "never" => TokKind::Never,
                "bool" => TokKind::Bool,
                "u8" => TokKind::U8,
                "u16" => TokKind::U16,
                "u32" => TokKind::U32,
                "u64" => TokKind::U64,
                "i8" => TokKind::I8,
                "i16" => TokKind::I16,
                "i32" => TokKind::I32,
                "i64" => TokKind::I64,
                _ => TokKind::Ident(word),
            };
            tokens.push(Token { kind, span: start_span });
            continue;
        }

        // integer literals: decimal or 0x hex
        if c.is_ascii_digit() {
            let start = i;
            if c == '0' && i + 1 < n && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
                i += 2;
                col += 2;
                let hstart = i;
                while i < n && chars[i].is_ascii_hexdigit() {
                    i += 1;
                    col += 1;
                }
                if hstart == i {
                    return Err(LexError {
                        msg: "hex literal has no digits after `0x`".to_string(),
                        span: start_span,
                    });
                }
                let text: String = chars[hstart..i].iter().collect();
                let value = u64::from_str_radix(&text, 16).map_err(|_| LexError {
                    msg: format!("hex literal `0x{}` out of range for u64", text),
                    span: start_span,
                })?;
                tokens.push(Token { kind: TokKind::Int(value), span: start_span });
            } else {
                while i < n && chars[i].is_ascii_digit() {
                    i += 1;
                    col += 1;
                }
                let text: String = chars[start..i].iter().collect();
                let value: u64 = text.parse().map_err(|_| LexError {
                    msg: format!("integer literal `{}` out of range for u64", text),
                    span: start_span,
                })?;
                tokens.push(Token { kind: TokKind::Int(value), span: start_span });
            }
            continue;
        }

        // char literal
        if c == '\'' {
            i += 1;
            col += 1;
            let ch = if i < n && chars[i] == '\\' {
                i += 1;
                col += 1;
                if i >= n {
                    return Err(LexError { msg: "unterminated char literal".into(), span: start_span });
                }
                let esc = chars[i];
                i += 1;
                col += 1;
                match esc {
                    'n' => b'\n',
                    't' => b'\t',
                    'r' => b'\r',
                    '0' => 0,
                    '\\' => b'\\',
                    '\'' => b'\'',
                    other => {
                        return Err(LexError {
                            msg: format!("unknown escape sequence `\\{}` in char literal", other),
                            span: start_span,
                        })
                    }
                }
            } else {
                if i >= n || chars[i] == '\'' || chars[i] == '\n' {
                    return Err(LexError { msg: "empty or unterminated char literal".into(), span: start_span });
                }
                let raw = chars[i] as u32;
                if raw > 255 {
                    return Err(LexError { msg: "char literal out of u8 range".into(), span: start_span });
                }
                i += 1;
                col += 1;
                raw as u8
            };
            if i >= n || chars[i] != '\'' {
                return Err(LexError { msg: "char literal not closed with `'`".into(), span: start_span });
            }
            i += 1;
            col += 1;
            tokens.push(Token { kind: TokKind::Char(ch), span: start_span });
            continue;
        }

        // string literal
        if c == '"' {
            i += 1;
            col += 1;
            let mut s: Vec<u8> = Vec::new();
            loop {
                if i >= n || chars[i] == '\n' {
                    return Err(LexError { msg: "unterminated string literal".into(), span: start_span });
                }
                let sc = chars[i];
                if sc == '"' {
                    break;
                }
                if sc == '\\' {
                    i += 1;
                    col += 1;
                    if i >= n {
                        return Err(LexError { msg: "unterminated string literal".into(), span: start_span });
                    }
                    let esc = chars[i];
                    let byte = match esc {
                        'n' => b'\n',
                        't' => b'\t',
                        'r' => b'\r',
                        '0' => 0,
                        '\\' => b'\\',
                        '"' => b'"',
                        '\'' => b'\'',
                        other => {
                            return Err(LexError {
                                msg: format!("unknown escape sequence `\\{}` in string literal", other),
                                span: start_span,
                            })
                        }
                    };
                    s.push(byte);
                    i += 1;
                    col += 1;
                } else {
                    let raw = sc as u32;
                    if raw > 255 {
                        return Err(LexError {
                            msg: "string literal contains non-ASCII character".into(),
                            span: start_span,
                        });
                    }
                    s.push(raw as u8);
                    i += 1;
                    col += 1;
                }
            }
            i += 1;
            col += 1;
            tokens.push(Token { kind: TokKind::Str(s.iter().map(|&b| b as char).collect()), span: start_span });
            continue;
        }

        // punctuation & operators
        i += 1;
        col += 1;
        let two = if i < n { Some(chars[i]) } else { None };
        let kind = match c {
            '(' => TokKind::LParen,
            ')' => TokKind::RParen,
            '{' => TokKind::LBrace,
            '}' => TokKind::RBrace,
            '[' => TokKind::LBracket,
            ']' => TokKind::RBracket,
            ',' => TokKind::Comma,
            ';' => TokKind::Semi,
            ':' => TokKind::Colon,
            '.' => TokKind::Dot,
            '#' => TokKind::Hash,
            '+' => TokKind::Plus,
            '*' => TokKind::Star,
            '/' => TokKind::Slash,
            '%' => TokKind::Percent,
            '^' => TokKind::Caret,
            '-' => {
                if two == Some('>') {
                    i += 1;
                    col += 1;
                    TokKind::Arrow
                } else {
                    TokKind::Minus
                }
            }
            '=' => {
                if two == Some('=') {
                    i += 1;
                    col += 1;
                    TokKind::Eq
                } else {
                    TokKind::Assign
                }
            }
            '!' => {
                if two == Some('=') {
                    i += 1;
                    col += 1;
                    TokKind::Neq
                } else {
                    TokKind::Bang
                }
            }
            '<' => match two {
                Some('<') => {
                    i += 1;
                    col += 1;
                    TokKind::Shl
                }
                Some('=') => {
                    i += 1;
                    col += 1;
                    TokKind::Le
                }
                _ => TokKind::Lt,
            },
            '>' => match two {
                Some('>') => {
                    i += 1;
                    col += 1;
                    TokKind::Shr
                }
                Some('=') => {
                    i += 1;
                    col += 1;
                    TokKind::Ge
                }
                _ => TokKind::Gt,
            },
            '&' => {
                if two == Some('&') {
                    i += 1;
                    col += 1;
                    TokKind::AmpAmp
                } else {
                    TokKind::Amp
                }
            }
            '|' => {
                if two == Some('|') {
                    i += 1;
                    col += 1;
                    TokKind::PipePipe
                } else {
                    TokKind::Pipe
                }
            }
            other => {
                return Err(LexError {
                    msg: format!("unexpected character `{}`", other),
                    span: start_span,
                })
            }
        };
        tokens.push(Token { kind, span: start_span });
    }

    Ok(tokens)
}
