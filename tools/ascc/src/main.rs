//! ascc: the Asc compiler driver.
//!
//! Usage:
//!   ascc input.asc -o output.o [--kernel] [--emit ir]
//!
//! `--kernel` selects the kernel-mode compilation profile (freestanding,
//! no red zone, no FPU/SSE, size-first codegen level, kernel code model).

use std::path::PathBuf;
use std::process::ExitCode;

mod ast;
mod codegen;
mod lexer;
mod parser;
mod typeck;

use lexer::{lex, LexError};
use parser::{parse, ParseError};
use typeck::{check, TypeError};

struct Args {
    input: PathBuf,
    output: Option<PathBuf>,
    kernel: bool,
    emit_ir: bool,
}

fn usage() -> &'static str {
    "usage: ascc <input.asc> -o <output.o> [--kernel] [--emit ir]"
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut input = None;
    let mut output = None;
    let mut kernel = false;
    let mut emit_ir = false;

    while let Some(a) = args.next() {
        match a.as_str() {
            "--kernel" => kernel = true,
            "-o" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("missing path after `-o`")?,
                ))
            }
            "--emit" => match args.next().as_deref() {
                Some("ir") => emit_ir = true,
                Some(other) => return Err(format!("unknown --emit target `{other}`")),
                None => return Err("missing value after --emit".into()),
            },
            other if other.starts_with('-') => return Err(format!("unknown option `{other}`")),
            other => {
                if input.is_some() {
                    return Err("multiple input files given".into());
                }
                input = Some(PathBuf::from(other));
            }
        }
    }

    Ok(Args {
        input: input.ok_or_else(|| usage().to_string())?,
        output,
        kernel,
        emit_ir,
    })
}

fn report(stage: &str, file: &str, span: lexer::Span, msg: &str) {
    eprintln!("{file}:{}:{}: error: {}", span.line, span.col, msg);
    let _ = stage;
}

fn run() -> Result<(), String> {
    let args = parse_args()?;

    let src = std::fs::read_to_string(&args.input)
        .map_err(|e| format!("cannot read {}: {}", args.input.display(), e))?;
    let fname = args.input.display().to_string();

    // lex → parse → typecheck → codegen; any failure is a hard stop with
    // file:line:col. Nothing downgrades to a warning.
    let tokens = lex(&src, &fname).map_err(|LexError { msg, span }| {
        report("lex", &fname, span, &msg);
        String::new()
    })?;

    let module = parse(tokens).map_err(|ParseError { msg, span }| {
        report("parse", &fname, span, &msg);
        String::new()
    })?;

    let checked = check(&module).map_err(|TypeError { msg, span }| {
        report("typeck", &fname, span, &msg);
        String::new()
    })?;

    if args.emit_ir {
        let ir = codegen::emit_ir(&checked, args.kernel).map_err(|e| e.msg)?;
        match &args.output {
            Some(p) => std::fs::write(p, ir)
                .map_err(|e| format!("cannot write {}: {}", p.display(), e))?,
            None => print!("{ir}"),
        }
        return Ok(());
    }

    let out_path = args.output.clone().unwrap_or_else(|| {
        let mut p = args.input.clone();
        p.set_extension("o");
        p
    });

    codegen::emit_object(&checked, args.kernel, &out_path).map_err(|e| {
        eprintln!("codegen error: {}", e.msg);
        String::new()
    })?;

    eprintln!("{} -> {} ({})", fname, out_path.display(), if args.kernel { "kernel" } else { "host-freestanding" });
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("ascc: {msg}");
            }
            ExitCode::FAILURE
        }
    }
}
