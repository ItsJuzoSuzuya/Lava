use std::process::Command;

use inkwell::context::Context;

use crate::{codegen_ctx::CodegenContext, parser::Parser, sema::Sema, statement::{Stmt, StmtKind}, traits::codegen::{Codegen, Declare}};

pub struct Program<'ctx> {
    pub context: CodegenContext<'ctx>,
    parser: Parser,
    sema: Sema,
    ast: Vec<Stmt>,
}

impl<'ctx> Program<'ctx> {
    pub fn new(context: &'ctx Context, source: String) -> Self {
        Self {
            context: CodegenContext::new(context, source.clone()),
            parser: Parser::new(&source),
            sema: Sema::new(source),
            ast: Vec::new(),
        }
    }

    pub fn parse_all(&mut self) {
        while let Some(stmt) = self.parser.parse_statement(){
            self.ast.push(stmt);
        }
    }

    pub fn run(&mut self) {
        self.context.build_entry();
        self.parse_all();
        self.sema.check(&mut self.ast);

        for stmt in &self.ast {
            if let StmtKind::Declaration(decl) = &stmt.kind {
                decl.declare(&mut self.context);
            }
        }

        for stmt in &self.ast {
            stmt.codegen(&mut self.context);
        }

        self.context.compile();

        Command::new("cc").arg("main.o").status().unwrap();
        Command::new("./a.out").status().unwrap();
    }
}
