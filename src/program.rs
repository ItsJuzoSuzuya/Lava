use std::{process::Command};

use inkwell::{context::Context};

use crate::{codegen_ctx::CodegenContext, traits::codegen::Codegen, parser::Parser};

pub struct Program<'ctx> {
    context: CodegenContext<'ctx>,
    parser: Parser
}

impl<'ctx> Program<'ctx> {
    pub fn new(context: &'ctx Context, string: String) -> Self {
        Self {
            context: CodegenContext::new(context),
            parser: Parser::new(string)
        }
    }

    pub fn run(&mut self) {
        self.context.build_entry();

        while let Some(stmt) = self.parser.parse_statement() {
            stmt.codegen(&mut self.context);
        } 

        self.context.compile();
        
        // Link and run
        Command::new("cc").arg("main.o").status().unwrap();
        Command::new("./a.out").status().unwrap();
    }
}


