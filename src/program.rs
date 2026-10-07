use std::process::Command;

use inkwell::context::Context;

use crate::{codegen_ctx::CodegenContext, declaration::{Declaration, DeclarationKind}, module::ModuleGraph, parser::Parser,
    sema::Sema, statement::{Stmt, StmtKind}, traits::codegen::{Codegen, Declare}, r#type::Type};

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

            // Instances of generic functions, classes and enums used by this module are generated with it
            for decl in sema.take_instances() {
                let span = decl.span.clone();
                stmts.push(Stmt::new(StmtKind::Declaration(decl), span, Type::Unknown));
            }

            // Instances like `Box<int32>` come after the functions using them, so every
            // struct type is created first. Classes are declared before enums, since
            // an enum's size depends on the classes in its payloads.
            let decls: Vec<&Declaration> = stmts.iter()
                .filter_map(|stmt| match &stmt.kind {
                    StmtKind::Declaration(decl) => Some(decl),
                    _ => None,
                })
                .collect();
            for decl in &decls {
                if let DeclarationKind::Class { name, .. } | DeclarationKind::Enum { name, .. } = &decl.kind {
                    self.context.struct_type(name);
                }
            }
            let is_class = |decl: &&&Declaration| matches!(decl.kind, DeclarationKind::Class { .. });
            let is_enum = |decl: &&&Declaration| matches!(decl.kind, DeclarationKind::Enum { .. });
            for decl in decls.iter().filter(is_class) {
                decl.declare(&mut self.context);
            }
            for decl in decls.iter().filter(is_enum) {
                decl.declare(&mut self.context);
            }
            for decl in decls.iter().filter(|d| !is_class(d) && !is_enum(d)) {
                decl.declare(&mut self.context);
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
