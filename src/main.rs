use std::{fs::{self}};

use inkwell::context::Context;
use crate::{program::Program};

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
    let mut program = Program::new(&context, load_file("main.lv"));
    program.run();
}


