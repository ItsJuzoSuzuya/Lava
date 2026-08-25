use std::process::Command;

use inkwell::context::Context;

use crate::{codegen_ctx::CodegenContext, expression::{Expr, ExprKind}, parser::Parser, sema::Sema, statement::{Stmt, StmtKind}, traits::codegen::{Codegen, Declare}};

pub struct Program<'ctx> {
    pub context: CodegenContext<'ctx>,
    parser: Parser,
    sema: Sema,
    ast: Vec<Stmt>,
}

impl<'ctx> Program<'ctx> {
    pub fn new(context: &'ctx Context, root: &'static str) -> Self {
        Self {
            context: CodegenContext::new(context, root.to_string().clone()),
            parser: Parser::new(&root),
            sema: Sema::new(root.to_string()),
            ast: Vec::new(),
        }
    }

    pub fn run(&mut self) {
        self.context.build_entry();

        let stmts = self.parser.parse_file();
        for stmt in stmts {
            if let StmtKind::Import(expr) = stmt.kind {
                if let ExprKind::ModuleImport { module, objects } = expr.kind {
                    let module_path = get_path_from_module(module);
                    Parser::new(module_path).parse_file();
                }
            }
        }

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

fn get_path_from_module(module: Vec<Expr>) -> String {
    let mut path = String::new();
    path += "./";
    for expr in module {
        path += &expr.kind.to_string();
    }
    path
}
