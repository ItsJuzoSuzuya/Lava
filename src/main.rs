use std::process::ExitCode;

use inkwell::context::Context;
use crate::{analysis::Index, module::ModuleGraph, program::{Program, check_modules}};

mod analysis;
mod traits;
mod span;
mod r#type;
mod token;
mod declaration;
mod codegen_ctx;
mod diagnostic;
mod lexer;
mod lint;
mod lsp;
mod parser;
mod expression;
mod statement;
mod program;
mod sema;
mod sym;
mod param;
mod field;
mod module;

const USAGE: &str = "usage:
  Letter               compile and run main.lv
  Letter lint [file]   check a program and print warnings, main.lv by default
  Letter lsp           run the language server on stdin and stdout";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => {
            diagnostic::install_hook();
            let context = Context::create();
            let mut module_graph = ModuleGraph::new("main.lv");
            let mut program = Program::new(&context, &mut module_graph);
            program.run();
            ExitCode::SUCCESS
        }
        ["lint"] => run_lint("main.lv"),
        ["lint", file] => run_lint(file),
        ["lsp"] => lsp::run(),
        ["help" | "--help" | "-h"] => {
            println!("{}", USAGE);
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{}", USAGE);
            ExitCode::FAILURE
        }
    }
}

/// Checks the program starting at `root` and prints its error and lint warnings.
///
/// Fails if the program doesn't compile.
fn run_lint(root: &str) -> ExitCode {
    diagnostic::silence_panics();
    let graph = match diagnostic::catch(|| ModuleGraph::new(root)) {
        Ok(graph) => graph,
        Err(error) => {
            eprintln!("{}", error.render());
            return ExitCode::FAILURE;
        }
    };

    let (modules, error) = check_modules(&graph);
    let index = Index::build(&graph, &modules);
    let warnings = lint::lint(&index, &modules);
    for warning in &warnings {
        println!("{}", warning.render(&index));
    }
    if let Some(error) = &error {
        eprintln!("{}", error.render());
    }

    let count = |n: usize, what: &str| format!("{} {}{}", n, what, if n == 1 { "" } else { "s" });
    println!("{}, {}", count(error.iter().count(), "error"), count(warnings.len(), "warning"));
    if error.is_some() { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
