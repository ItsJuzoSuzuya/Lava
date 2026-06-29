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
    let mut program = Program::new(&context, "let x: int32 = 20;".to_string());
    program.run();
}
