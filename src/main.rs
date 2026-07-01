use inkwell::context::Context;
use crate::program::Program;

mod traits;
mod r#type;
mod token;
mod declaration;
mod codegen_ctx;
mod lexer;
mod parser;
mod expression;
mod statement;
mod program;

fn main() {
    let context = Context::create();
    let mut program = Program::new(&context, "func foo() { print(22); } foo();".to_string());
    program.run();
}
