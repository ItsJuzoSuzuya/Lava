use inkwell::context::Context;
use crate::program::Program;

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

fn main() {
    let context = Context::create();
    let mut program = Program::new(&context, "
    class Foo {
        x: int32 = a;
        name: String = b;

        Constructor(a: int32, b: String) {
            printIT();
        }

        func printIT() {
            print(self.name);
        }
    }


    let foo: Foo = Foo(22, 23);"
    .to_string());
    program.run();
}
