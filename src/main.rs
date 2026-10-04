use std::{fs::{self}};

use inkwell::context::Context;
use crate::{module::ModuleGraph, parser::Parser, program::Program};

mod traits;
mod span;
mod r#type;
mod token;
mod declaration;
mod codegen_ctx;
mod lexer;
mod parser;
mod expression;
mod statement;
mod program;
mod sema;
mod sym;
mod param;
mod field;
mod module;

fn main() {
    let context = Context::create();
    let mut module_graph = ModuleGraph::new("main.lv");
    let mut program = Program::new(&context, &mut module_graph);
    program.run();
}


