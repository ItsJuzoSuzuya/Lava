use std::path::{Path, PathBuf};

use inkwell::context::Context;

use crate::{codegen_ctx::CodegenContext, declaration::{Declaration, DeclarationKind, ExternItem}, diagnostic::{self, CompileError},
    expression::{Expr, ExprKind}, module::ModuleGraph, parser::{self, Parser}, sema::Sema, statement::{Stmt, StmtKind}, traits::codegen::{Codegen, Declare},
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
/// Returns the modules with every error found in them. Checking goes on
/// after a semantic error, but stops at a syntax error, the module with it
/// isn't included then.
pub fn check_modules(graph: &ModuleGraph) -> (Vec<CheckedModule>, Vec<CompileError>) {
    // Shared across modules, so declarations of imported modules stay known.
    let mut sema = Sema::new();
    let mut done: Vec<CheckedModule> = Vec::new();
    // Imports that couldn't be loaded come first, the rest is checked without them
    let mut errors: Vec<CompileError> = graph.errors.clone();

    for module in &graph.modules {
        let parsed = diagnostic::catch(|| Parser::with_renames(module.source.clone(), module.renames()).parse_file());
        let mut stmts = match parsed {
            Ok(stmts) => stmts,
            Err(error) => {
                errors.push(locate(error, &module.path, graph));
                return (done, errors);
            }
        };
        parser::desugar_inherent_impls(&mut stmts, &module.name);

        sema.enter_module(module.name.clone(), module.source.clone(), &module.imports);
        let own_stmts = stmts.len();
        // The program's main file is last, its `main` runs after its top-level code
        let is_root = graph.modules.last().is_some_and(|root| root.path == module.path);
        let result = diagnostic::catch(|| {
            if is_root {
                add_main_call(&mut stmts, &module.source);
            }
            sema.check(&mut stmts)
        });

        let mut module_errors = sema.take_errors();
        if let Err(error) = &result {
            module_errors.push(error.clone());
        }
        // Instances of generic functions, classes and enums used by this module are
        // generated with it. With errors there's no code to generate, they can be half done
        let instances = sema.take_instances();
        if module_errors.is_empty() {
            for decl in instances {
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
        for error in module_errors {
            let error = locate(error, &module.path, graph);
            // The same instance can be checked from several places
            let duplicate = errors.iter().any(|e| e.message == error.message && e.file == error.file
                && e.span.as_ref().map(|s| (s.line, s.col)) == error.span.as_ref().map(|s| (s.line, s.col)));
            if !duplicate {
                errors.push(error);
            }
        }
    }

    (done, errors)
}

/// Adds the call of the program's `func main` after its top-level code:
/// `main();`, or `return main();` if it returns an `int32` exit code, which
/// a `return` at the top level ends the program with. Like in Rust, `main`
/// takes no parameters, the command line arguments come from `env::args()`.
/// Does nothing if there's no `main`.
///
/// # Panics
///
/// Panics if `main` has another signature.
fn add_main_call(stmts: &mut Vec<Stmt>, source: &str) {
    let main = stmts.iter().find_map(|stmt| match &stmt.kind {
        StmtKind::Declaration(decl) => match &decl.kind {
            DeclarationKind::Function { name, type_params, params, return_type, .. } if name == "main" =>
                Some((decl.span.clone(), type_params.is_empty(), params.clone(), return_type.clone())),
            _ => None,
        },
        _ => None,
    });
    let Some((span, not_generic, params, return_type)) = main else { return };

    let wrong = |message: &str| diagnostic::throw(message, Some(span.clone()), source);
    if !not_generic {
        wrong("`main` can't be generic");
    }
    if !params.is_empty() {
        wrong("`main` takes no parameters, get the command line arguments with `env::args()` of `std::env`");
    }
    let returns_code = match &return_type {
        None | Some(Type::Void) => false,
        Some(ty) if *ty == Type::INT32 => true,
        Some(_) => wrong("`main` returns nothing or an `int32` exit code"),
    };

    let main_call = Expr::new(
        ExprKind::FunctionCall { name: "main".to_string(), type_args: vec![], params: vec![], receiver: None, mangled: None },
        span.clone(),
    );
    let kind = if returns_code { StmtKind::Return(main_call) } else { StmtKind::Expression(main_call) };
    stmts.push(Stmt::new(kind, span.clone(), Type::Unknown));
}

/// The class and enum declarations `types` ordered so each comes after the
/// ones it holds directly, like a class after the enum of one of its fields.
///
/// A `Unique`, `Buffer` or reference is a pointer, so holding one doesn't
/// need the pointed to type's layout. That's how types can contain themselves.
fn layout_order<'a>(types: &[&'a Declaration]) -> Vec<&'a Declaration> {
    fn direct_types(ty: &Type, into: &mut Vec<String>) {
        match ty {
            Type::Class(name) | Type::Enum(name) => into.push(name.clone()),
            // An array holds its elements directly
            Type::Array(element, _) => direct_types(element, into),
            _ => {}
        }
    }
    let name_of = |decl: &Declaration| match &decl.kind {
        DeclarationKind::Class { name, .. } | DeclarationKind::Enum { name, .. } => name.clone(),
        _ => String::new(),
    };
    let held = |decl: &Declaration| {
        let mut names = Vec::new();
        match &decl.kind {
            DeclarationKind::Class { fields, .. } => fields.iter().for_each(|f| direct_types(&f.ty, &mut names)),
            DeclarationKind::Enum { fields, .. } => fields.iter().for_each(|f| direct_types(&f.2, &mut names)),
            _ => {}
        }
        names
    };

    fn visit<'a>(i: usize, types: &[&'a Declaration], deps: &[Vec<usize>], state: &mut [u8], order: &mut Vec<&'a Declaration>) {
        // 1 while visiting, 2 when done. A type holding itself directly can't
        // have a layout, it's simply ordered somewhere in the cycle.
        if state[i] != 0 {
            return;
        }
        state[i] = 1;
        for &dep in &deps[i] {
            visit(dep, types, deps, state, order);
        }
        state[i] = 2;
        order.push(types[i]);
    }

    let names: Vec<String> = types.iter().map(|d| name_of(d)).collect();
    let deps: Vec<Vec<usize>> = types.iter()
        .map(|d| held(d).iter().filter_map(|n| names.iter().position(|m| m == n)).collect())
        .collect();
    let mut state = vec![0u8; types.len()];
    let mut order = Vec::with_capacity(types.len());
    for i in 0..types.len() {
        visit(i, types, &deps, &mut state, &mut order);
    }
    order
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

/// Parses, checks semantics and generates the code of a program.
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

    /// Checks every module and writes the program's object file to `object`.
    /// Returns the C libraries to link it with, from `extern link "name";`.
    ///
    /// Returns every error in the program instead if there are any.
    pub fn build(&mut self, object: &Path) -> Result<Vec<String>, Vec<CompileError>> {
        self.context.build_entry();

        let (modules, errors) = check_modules(self.module_graph);
        if !errors.is_empty() {
            return Err(errors);
        }

        let mut libraries: Vec<String> = Vec::new();
        for module in modules {
            self.context.source = module.source.clone();
            let stmts = module.stmts;
            for stmt in &stmts {
                if let StmtKind::Declaration(Declaration { kind: DeclarationKind::Extern(ExternItem::Link(library)), .. }) = &stmt.kind {
                    if !libraries.contains(library) {
                        libraries.push(library.clone());
                    }
                }
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
            // A class or enum holding another one directly needs that one's layout first
            let is_type = |decl: &&&Declaration| matches!(decl.kind, DeclarationKind::Class { .. } | DeclarationKind::Enum { .. });
            let types: Vec<&Declaration> = decls.iter().filter(is_type).copied().collect();
            for decl in layout_order(&types) {
                decl.declare(&mut self.context);
            }
            for decl in decls.iter().filter(|d| !is_type(d)) {
                decl.declare(&mut self.context);
            }

            for stmt in &stmts {
                stmt.codegen(&mut self.context);
            }

            self.ast.extend(stmts);
        }

        self.context.compile(object);
        Ok(libraries)
    }
}
