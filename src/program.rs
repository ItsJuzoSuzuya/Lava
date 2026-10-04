use std::process::Command;

use inkwell::context::Context;

use crate::{codegen_ctx::CodegenContext, module::ModuleGraph, parser::Parser, 
    sema::Sema, statement::{Stmt, StmtKind}, traits::codegen::{Codegen, Declare}};

/// Parses, checks semantics and runs the code.
///
/// Built by [`Program::new`] from a [`ModuleGraph`].
pub struct Program<'ctx> {
    pub context: CodegenContext<'ctx>,
    module_graph: &'ctx mut ModuleGraph,
    ast: Vec<Stmt>,
}

impl<'ctx> Program<'ctx> {
    pub fn new(context: &'ctx Context, module_graph: &'ctx mut ModuleGraph) -> Self {
        Self {
            context: CodegenContext::new(context),
            module_graph: module_graph,
            ast: Vec::new(),
        }
    }

    // Generates and runs the code from each module by parsing, semantic checking and then
    // generating the llvm code.
    pub fn run(&mut self) {
        self.context.build_entry();

        // Shared across modules, so declarations of imported modules stay known.
        let mut sema = Sema::new();

        for module in &self.module_graph.modules {
            let mut parser = Parser::new(module.source.clone());
            sema.enter_module(module.name.clone(), module.source.clone());

            self.context.source = module.source.clone();

            // Only check and generate this module's statements, earlier modules are already done.
            let mut stmts = parser.parse_file();
            sema.check(&mut stmts);

            for stmt in &stmts {
                if let StmtKind::Declaration(decl) = &stmt.kind {
                    decl.declare(&mut self.context);
                }
            }

            for stmt in &stmts {
                stmt.codegen(&mut self.context);
            }

            self.ast.extend(stmts);
        }

        self.context.compile();

        Command::new("cc").arg("main.o").status().unwrap();
        Command::new("./a.out").status().unwrap();
    }
}
