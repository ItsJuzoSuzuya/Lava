use std::{panic, path::PathBuf, process::Command};

use inkwell::context::Context;

use crate::{codegen_ctx::CodegenContext, declaration::{Declaration, DeclarationKind}, diagnostic::{self, CompileError},
    module::ModuleGraph, parser::Parser, sema::Sema, statement::{Stmt, StmtKind}, traits::codegen::{Codegen, Declare},
    r#type::Type};

/// A module after parsing and checking its semantics.
pub struct CheckedModule {
    pub name: String,
    pub path: PathBuf,
    pub source: String,
    /// The module's statements, followed by the instances of generic
    /// functions, classes and enums it uses.
    pub stmts: Vec<Stmt>,
    /// How many of [`CheckedModule::stmts`] are written in the module, the
    /// rest are instances created by sema.
    pub own_stmts: usize,
    /// Whether checking finished. If not, the statements are only checked up
    /// to the error.
    pub checked: bool,
}

/// Parses and checks every module of `graph`, imports first.
///
/// Stops at the first error and returns it together with the modules done so
/// far. A module whose check failed is included, checked up to the error.
pub fn check_modules(graph: &ModuleGraph) -> (Vec<CheckedModule>, Option<CompileError>) {
    // Shared across modules, so declarations of imported modules stay known.
    let mut sema = Sema::new();
    let mut done: Vec<CheckedModule> = Vec::new();

    for module in &graph.modules {
        let parsed = diagnostic::catch(|| Parser::new(module.source.clone()).parse_file());
        let mut stmts = match parsed {
            Ok(stmts) => stmts,
            Err(error) => return (done, Some(locate(error, &module.path, graph))),
        };

        sema.enter_module(module.name.clone(), module.source.clone());
        let own_stmts = stmts.len();
        let result = diagnostic::catch(|| sema.check(&mut stmts));

        // Instances of generic functions, classes and enums used by this module are generated with it
        if result.is_ok() {
            for decl in sema.take_instances() {
                let span = decl.span.clone();
                stmts.push(Stmt::new(StmtKind::Declaration(decl), span, Type::Unknown));
            }
        }

        done.push(CheckedModule {
            name: module.name.clone(),
            path: module.path.clone(),
            source: module.source.clone(),
            stmts,
            own_stmts,
            checked: result.is_ok(),
        });
        if let Err(error) = result {
            return (done, Some(locate(error, &module.path, graph)));
        }
    }
    (done, None)
}

/// Gives `error` the file it's in, if it doesn't know it yet.
///
/// Errors in the instance of a generic declared in another module happen
/// while checking `current`, but are in the other module's source.
fn locate(mut error: CompileError, current: &PathBuf, graph: &ModuleGraph) -> CompileError {
    if error.file.is_none() {
        let by_source = graph.modules.iter()
            .find(|m| !error.source.is_empty() && m.source == error.source)
            .map(|m| m.path.clone());
        error.file = Some(by_source.unwrap_or_else(|| current.clone()));
    }
    error
}

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

        let (modules, error) = check_modules(self.module_graph);
        if let Some(error) = error {
            // Already printed by the panic hook when it was thrown
            panic::resume_unwind(Box::new(error));
        }

        for module in modules {
            self.context.source = module.source.clone();
            let stmts = module.stmts;

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
