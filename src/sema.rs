use std::collections::{HashMap, HashSet};

use crate::{declaration::{Declaration, DeclarationKind, TypeParam, Visibility}, expression::{Expr, ExprKind}, field::Field, param::Param, span::Span, statement::{MatchArm, Pattern, Stmt, StmtKind}, token::Token, r#type::Type};

enum SemanticError {
    AmbiguousMethod { method: String, class: String, traits: Vec<String>, span: Span },
    AssignOnUnmutable { var: String, span: Span },
    AssignToSelf { span: Span },
    CannotInferTypeParam { param: String, function: String, span: Span },
    CannotNegate { ty: Type, span: Span },
    CannotPrintUnique { span: Span },
    DuplicateImpl { trait_name: String, ty: String, span: Span },
    GenericMethod { method: String, span: Span },
    InvalidConversion { from: Type, to: Type, span: Span },
    InvalidImplTarget { ty: Type, span: Span },
    InvalidOperands { op: Token, ty: Type, span: Span },
    InvalidReceiver { recv_ty: Type, span: Span, context: &'static str },
    LiteralOutOfRange { value: i64, ty: Type, span: Span },
    MissingArgument { function: String, index: usize, span: Span },
    MissingTraitMethod { method: String, trait_name: String, ty: String, span: Span },
    MissingTypeArgs { name: String, span: Span },
    MoveInLoop { var: String, span: Span },
    MutatingCallOnImmutable { method: String, var: String, span: Span },
    NotATraitMethod { method: String, trait_name: String, span: Span },
    NotGeneric { function: String, span: Span },
    PrivateFieldAccess { field_name: String, span: Span },
    PrivateFunctionCall { func_name: String, parent: String, span: Span },
    TooManyArguments { func_name: String, parent: String, span: Span },
    TraitMethodMismatch { method: String, trait_name: String, span: Span },
    TraitNotImplemented { ty: Type, trait_name: String, span: Span },
    TypeMismatch { lhs: Type, rhs: Type, span: Span, context: &'static str },
    UnknownEnum { name: String, span: Span },
    UnknownField { class: String, field: String, span: Span },
    UnknownFunction { fn_name: String, span: Span },
    UnknownIdentifier { identifier: String, span: Span },
    UnknownTrait { name: String, span: Span },
    UnknownType { name: String, span: Span },
    UnknownVariant { enum_name: String, variant: String, span: Span },
    UniqueNotAllowed { context: &'static str, span: Span },
    UseAfterMove { var: String, span: Span },
    WrongArgCount { function: String, expected: usize, got: usize, span: Span },
    WrongTypeArgCount { function: String, expected: usize, got: usize, span: Span },
}

impl SemanticError {
    fn throw(self, source: &str) -> ! {
        let (line, col, msg) = match self {
            SemanticError::AmbiguousMethod { method, class, traits, span } =>
                (span.line, span.col, format!("method '{}' of '{}' is ambiguous, it comes from the traits '{}'", method, class, traits.join("', '"))),
            SemanticError::AssignOnUnmutable{ var, span } =>
                (span.line, span.col, format!("Tried reassigning a value to the unmutable Variable '{}'", var)),
            SemanticError::AssignToSelf { span } =>
                (span.line, span.col, "can't assign to 'self' of a class or enum, assign its fields instead".to_string()),
            SemanticError::CannotInferTypeParam { param, function, span } =>
                (span.line, span.col, format!("can't infer type parameter '{}' of '{}', give it explicitly like {}<...>(...)", param, function, function)),
            SemanticError::CannotPrintUnique { span } =>
                (span.line, span.col, "a Unique or Buffer can't be printed, print the values inside instead".to_string()),
            SemanticError::CannotNegate { ty, span } =>
                (span.line, span.col, format!("cannot negate a value of type '{}'", ty)),
            SemanticError::DuplicateImpl { trait_name, ty, span } =>
                (span.line, span.col, format!("trait '{}' is already implemented for '{}'", trait_name, ty)),
            SemanticError::GenericMethod { method, span } =>
                (span.line, span.col, format!("method '{}' can't be generic, trait methods and their impls can't have type parameters", method)),
            SemanticError::InvalidConversion { from, to, span } =>
                (span.line, span.col, format!("cannot convert '{}' to '{}'", from, to)),
            SemanticError::InvalidImplTarget { ty, span } =>
                (span.line, span.col, format!("traits can't be implemented for '{}'", ty)),
            SemanticError::InvalidOperands { op, ty, span } =>
                (span.line, span.col, format!("'{}' can't be used on values of type '{}'", op, ty)),
            SemanticError::InvalidReceiver { recv_ty, span, context } =>
                (span.line, span.col, format!("'{}' {}", recv_ty, context)),
            SemanticError::LiteralOutOfRange { value, ty, span } =>
                (span.line, span.col, format!("literal '{}' doesn't fit into '{}'", value, ty)),
            SemanticError::MissingArgument { function, index, span } =>
                (span.line, span.col, format!("missing argument {} for '{}'", index, function)),
            SemanticError::MissingTraitMethod { method, trait_name, ty, span } =>
                (span.line, span.col, format!("'{}' is missing method '{}' of trait '{}'", ty, method, trait_name)),
            SemanticError::MissingTypeArgs { name, span } =>
                (span.line, span.col, format!("generic type '{}' needs type arguments, like {}<...>", name, name)),
            SemanticError::MoveInLoop { var, span } =>
                (span.line, span.col, format!("'{}' is moved inside the loop, it would be gone in the next round", var)),
            SemanticError::MutatingCallOnImmutable { method, var, span } =>
                (span.line, span.col, format!("method '{}' changes 'self', so '{}' has to be mutable", method, var)),
            SemanticError::NotATraitMethod { method, trait_name, span } =>
                (span.line, span.col, format!("method '{}' is not a member of trait '{}'", method, trait_name)),
            SemanticError::NotGeneric { function, span } =>
                (span.line, span.col, format!("'{}' isn't generic, it takes no type arguments", function)),
            SemanticError::PrivateFieldAccess { field_name, span } =>
                (span.line, span.col, format!("Tried accessing a private field '{}'", field_name)),
            SemanticError::PrivateFunctionCall { func_name, parent,  span } =>
                (span.line, span.col, format!("Tried calling a private function '{}' in '{}'", func_name, parent)),
            SemanticError::TooManyArguments {  func_name, parent,  span } =>
                (span.line, span.col, format!("Too many arguments provided for function '{}' in '{}'", func_name, parent)),
            SemanticError::TraitMethodMismatch { method, trait_name, span } =>
                (span.line, span.col, format!("method '{}' doesn't match its signature in trait '{}'", method, trait_name)),
            SemanticError::TraitNotImplemented { ty, trait_name, span } =>
                (span.line, span.col, format!("'{}' doesn't implement trait '{}'", ty, trait_name)),
            SemanticError::TypeMismatch { lhs, rhs, span, context } =>
                (span.line, span.col, format!("type '{}' does not match '{}' in {}", lhs, rhs, context)),
            SemanticError::UnknownEnum { name, span } =>
                (span.line, span.col, format!("unknown enum '{}'", name)),
            SemanticError::UnknownField { class, field, span } =>
                (span.line, span.col, format!("class '{}' has no field '{}'", class, field)),
            SemanticError::UnknownFunction { fn_name, span } =>
                (span.line, span.col, format!("unknown function '{}'", fn_name)),
            SemanticError::UnknownIdentifier { identifier, span } =>
                (span.line, span.col, format!("unknown identifier '{}'", identifier)),
            SemanticError::UnknownTrait { name, span } =>
                (span.line, span.col, format!("unknown trait '{}'", name)),
            SemanticError::UnknownType { name, span } =>
                (span.line, span.col, format!("unknown type '{}'", name)),
            SemanticError::UniqueNotAllowed { context, span } =>
                (span.line, span.col, format!("a Unique, Buffer or class owning one can't be {} for now", context)),
            SemanticError::UseAfterMove { var, span } =>
                (span.line, span.col, format!("use of moved value '{}', its Unique was moved somewhere else", var)),
            SemanticError::WrongArgCount { function, expected, got, span } =>
                (span.line, span.col, format!("'{}' takes {} arguments, got {}", function, expected, got)),
            SemanticError::WrongTypeArgCount { function, expected, got, span } =>
                (span.line, span.col, format!("'{}' takes {} type arguments, got {}", function, expected, got)),
            SemanticError::UnknownVariant { enum_name, variant, span } =>
                (span.line, span.col, format!("enum '{}' has no variant '{}'", enum_name, variant)),
        };
        let line_txt = source.lines().nth(line).unwrap_or("");
        let pointer = " ".repeat(col) + "^";
        panic!("\nerror: {}\n  --> {}:{}\n   |\n   | {}\n   | {}\n",
            msg, line + 1, col + 1, line_txt, pointer)
    }
}

/// A generic function, method, class or enum together with where it's declared,
/// since its instances are checked in that context.
#[derive(Clone)]
struct GenericDecl {
    decl: Declaration,
    module: String,
    source: String,
    /// The class a generic method belongs to, `None` for everything else.
    class: Option<String>,
    /// Type arguments already fixed by the surroundings, like `T = int32` for a
    /// generic method of `Box<int32>`.
    outer_subst: HashMap<String, Type>,
}

/// An instance of a generic class or enum whose method bodies still have to
/// be checked. That happens at the end of the module, since instances can be
/// created while the module's own declarations are still being processed.
struct PendingInstance {
    decl: Declaration,
    subst: HashMap<String, Type>,
    module: String,
    source: String,
}

pub struct Sema {
    scope: HashMap<String, (Type, bool)>,
    /// Params, return type, visibility and the name of the module the function is declared in.
    functions: HashMap<String, (Vec<Param>, Type, Visibility, String)>,
    classes: HashMap<String, Vec<Field>>,
    enums: HashMap<String, Vec<(String, u32, Type)>>,
    /// Methods of every trait, by trait name.
    traits: HashMap<String, Vec<Declaration>>,
    /// Names of the traits each type implements, by type name.
    impls: HashMap<String, Vec<String>>,
    current_class: Option<String>,
    /// Trait of the impl block currently being declared.
    current_trait: Option<String>,
    /// Generic functions by name. They are templates, only their instances get checked.
    generic_functions: HashMap<String, GenericDecl>,
    /// Generic classes and enums by name, templates like the generic functions.
    generic_types: HashMap<String, GenericDecl>,
    /// Template name and type arguments of every instance of a generic class or
    /// enum, by instance name like `Box<int32>`.
    type_instances: HashMap<String, (String, Vec<Type>)>,
    /// Instances of generic classes and enums whose bodies aren't checked yet.
    pending_instances: Vec<PendingInstance>,
    /// Instances of generic functions, classes and enums created while checking
    /// the current module, waiting to be handed to codegen with [`Sema::take_instances`].
    instances: Vec<Declaration>,
    /// Type arguments of the generic instance currently being checked, by type parameter name.
    type_subst: HashMap<String, Type>,
    /// Type the next checked expression is expected to have, see [`Sema::check_expr_expecting`].
    expected_type: Option<Type>,
    /// Variables in scope whose `Unique` got moved out, so they can't be used anymore.
    moved: HashSet<String>,
    current_return_type: Option<Type>,
    /// Int literals too big for their default type `int32` that haven't gotten
    /// another type from their context yet, by their position.
    oversized_literals: HashMap<(usize, usize), (i64, Span)>,
    /// Name of the module currently being checked.
    module: String,
    source: String
}

impl Sema {
    /// Creates a sema with the built-in functions already declared.
    ///
    /// Codegen defines the same functions in `CodegenContext::define_builtins`.
    pub fn new() -> Self {
        let mut sema = Self {
            scope: HashMap::new(), functions: HashMap::new(), classes: HashMap::new(), enums: HashMap::new(),
            traits: HashMap::new(), impls: HashMap::new(), current_class: None, current_trait: None,
            generic_functions: HashMap::new(), generic_types: HashMap::new(), type_instances: HashMap::new(),
            pending_instances: Vec::new(), instances: Vec::new(), type_subst: HashMap::new(), expected_type: None,
            moved: HashSet::new(),
            source: String::new(), module: String::new(), current_return_type: None, oversized_literals: HashMap::new(),
        };
        sema.push_func("sqrt".to_string(), vec![Param::builtin("x", Type::Float { bits: 64 })], Type::Float { bits: 64 }, Visibility::Public);
        sema
    }

    /// Switches to checking the module `module` with the source code `source`.
    ///
    /// Declarations of previously checked modules stay known, local variables are dropped.
    pub fn enter_module(&mut self, module: String, source: String) {
        self.module = module;
        self.source = source;
        self.scope.clear();
        self.moved.clear();
    }

    /// Type of the variable `name`, for using it in an expression.
    ///
    /// # Panics
    ///
    /// Panics if there's no such variable, or its `Unique` got moved out already.
    fn use_variable(&self, name: &str, span: &Span) -> Type {
        let (ty, _) = self.scope.get(name).cloned()
            .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                identifier: name.to_string(),
                span: span.clone(),
            }.throw(&self.source));
        if self.moved.contains(name) {
            SemanticError::UseAfterMove { var: name.to_string(), span: span.clone() }.throw(&self.source);
        }
        ty
    }

    /// Whether `expr` creates a built-in `Buffer` like `Buffer<int32>(8)`. A user
    /// type or function with that name takes precedence.
    fn is_buffer_call(&self, expr: &Expr) -> bool {
        matches!(&expr.kind, ExprKind::FunctionCall { name, receiver: None, .. }
            if name == "Buffer"
                && !self.functions.contains_key("Buffer")
                && !self.generic_functions.contains_key("Buffer")
                && !self.generic_types.contains_key("Buffer"))
    }

    /// Checks `Buffer<T>(capacity)` and turns it into an [`ExprKind::NewBuffer`].
    ///
    /// `T` comes from the explicit type argument, else from an expected `Buffer<T>`.
    ///
    /// # Panics
    ///
    /// Panics if `T` is unknown or owns memory itself, or the capacity isn't
    /// exactly one `uint64` argument.
    fn check_buffer_call(&mut self, expr: &mut Expr, expected: Option<&Type>) -> Type {
        let span = expr.span.clone();
        let ExprKind::FunctionCall { type_args, params, .. } = &mut expr.kind else {
            unreachable!("only `Buffer` calls get here");
        };
        let element = match (type_args.as_slice(), expected) {
            ([ty], _) => self.canonicalize_type(ty, &span),
            ([], Some(Type::Buffer(element))) => (**element).clone(),
            ([], _) => SemanticError::CannotInferTypeParam {
                param: "T".to_string(),
                function: "Buffer".to_string(),
                span,
            }.throw(&self.source),
            (args, _) => SemanticError::WrongTypeArgCount {
                function: "Buffer".to_string(),
                expected: 1,
                got: args.len(),
                span,
            }.throw(&self.source),
        };
        self.check_type(&element, &span);

        if params.len() != 1 {
            SemanticError::WrongArgCount { function: "Buffer".to_string(), expected: 1, got: params.len(), span }
                .throw(&self.source);
        }
        let mut capacity = params.remove(0);
        self.check_argument(&mut capacity, &Type::UINT64, "Buffer capacity");

        let ty = Type::Buffer(Box::new(element));
        expr.kind = ExprKind::NewBuffer(Box::new(capacity));
        ty
    }

    /// Checks a call of the built-in `Buffer` method `method` on `receiver`, a
    /// `Buffer<T>`, and returns its result type:
    ///
    /// ```text
    /// get(index: uint64) -> T
    /// set(mut &self, index: uint64, value: T)
    /// capacity() -> uint64
    /// resize(mut &self, capacity: uint64)
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if there's no such method, the arguments don't match, or a
    /// changing method is called on an immutable variable.
    fn check_buffer_method(&mut self, method: &str, receiver: &Expr, args: &mut Vec<Expr>, span: &Span) -> Type {
        let element = match &receiver.ty {
            Type::Buffer(element) => (**element).clone(),
            Type::Ref(inner) | Type::Unique(inner) => match inner.as_ref() {
                Type::Buffer(element) => (**element).clone(),
                _ => unreachable!("only Buffer receivers get here"),
            },
            _ => unreachable!("only Buffer receivers get here"),
        };
        let (params, result, changes_self) = match method {
            "get" => (vec![Type::UINT64], element, false),
            "set" => (vec![Type::UINT64, element], Type::Void, true),
            "capacity" => (vec![], Type::UINT64, false),
            "resize" => (vec![Type::UINT64], Type::Void, true),
            _ => SemanticError::UnknownFunction { fn_name: method.to_string(), span: span.clone() }.throw(&self.source),
        };

        if args.len() != params.len() {
            SemanticError::WrongArgCount { function: method.to_string(), expected: params.len(), got: args.len(), span: span.clone() }
                .throw(&self.source);
        }
        for (arg, ty) in args.iter_mut().zip(&params) {
            self.check_argument(arg, ty, "function argument");
        }

        if changes_self {
            if let Some(variable) = root_variable(receiver) {
                if self.scope.get(variable).is_some_and(|(_, mutable)| !mutable) {
                    SemanticError::MutatingCallOnImmutable {
                        method: method.to_string(),
                        var: variable.to_string(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
            }
        }
        result
    }

    /// Checks `arg` for a parameter of type `ty`, adapting literals and widening
    /// numbers like for every other argument.
    ///
    /// # Panics
    ///
    /// Panics if `arg` doesn't have the type `ty` afterwards.
    fn check_argument(&mut self, arg: &mut Expr, ty: &Type, context: &'static str) {
        self.check_expr_expecting(arg, ty);
        self.coerce(arg, ty);
        if arg.ty != *ty {
            SemanticError::TypeMismatch { lhs: arg.ty.clone(), rhs: ty.clone(), span: arg.span.clone(), context }
                .throw(&self.source);
        }
    }

    /// Whether values of `ty` own heap memory: a `Unique`, a `Buffer`, or a
    /// class with a field that does.
    fn owns_memory(&self, ty: &Type) -> bool {
        match ty {
            Type::Unique(_) | Type::Buffer(_) => true,
            Type::Class(name) => self.classes.get(name)
                .is_some_and(|fields| fields.iter().any(|f| self.owns_memory(&f.ty))),
            _ => false,
        }
    }

    /// Whether `expr` calls the built-in `unique`. A user function with that name takes precedence.
    fn is_unique_call(&self, expr: &Expr) -> bool {
        matches!(&expr.kind, ExprKind::FunctionCall { name, receiver: None, .. }
            if name == "unique"
                && !self.functions.contains_key("unique")
                && !self.generic_functions.contains_key("unique"))
    }

    /// Checks a call of the built-in `unique(value)` and turns it into an
    /// [`ExprKind::Unique`], which moves the value onto the heap.
    ///
    /// The value's type comes from an explicit `unique<T>(...)`, else from an
    /// expected `Unique<T>`, else from the value itself.
    ///
    /// # Panics
    ///
    /// Panics if there isn't exactly one argument or it doesn't have the type `T`.
    fn check_unique_call(&mut self, expr: &mut Expr, expected: Option<&Type>) -> Type {
        let span = expr.span.clone();
        let ExprKind::FunctionCall { type_args, params, .. } = &mut expr.kind else {
            unreachable!("only calls of `unique` get here");
        };
        if params.is_empty() {
            SemanticError::MissingArgument { function: "unique".to_string(), index: 0, span }.throw(&self.source);
        }
        if params.len() > 1 {
            SemanticError::TooManyArguments { func_name: "unique".to_string(), parent: "builtin".to_string(), span }
                .throw(&self.source);
        }
        let target = match (type_args.as_slice(), expected) {
            ([], Some(Type::Unique(inner))) => Some((**inner).clone()),
            ([], _) => None,
            ([ty], _) => Some(self.canonicalize_type(ty, &span)),
            (args, _) => SemanticError::WrongTypeArgCount {
                function: "unique".to_string(),
                expected: 1,
                got: args.len(),
                span,
            }.throw(&self.source),
        };

        let mut value = params.remove(0);
        match &target {
            Some(ty) => {
                self.check_expr_expecting(&mut value, ty);
                self.coerce(&mut value, ty);
                if value.ty != *ty {
                    SemanticError::TypeMismatch {
                        lhs: value.ty.clone(),
                        rhs: ty.clone(),
                        span: value.span.clone(),
                        context: "unique value",
                    }.throw(&self.source);
                }
            }
            None => {
                self.check_expr(&mut value);
            }
        }

        let ty = Type::Unique(Box::new(value.ty.clone()));
        expr.kind = ExprKind::Unique(Box::new(value));
        ty
    }

    /// Returns the instances of generic functions, classes and enums created since the last call.
    ///
    /// They are ordinary declarations and have to be declared and generated
    /// like the module's own ones.
    pub fn take_instances(&mut self) -> Vec<Declaration> {
        std::mem::take(&mut self.instances)
    }

    pub fn push_class(&mut self, name: String, fields: Vec<Field>) {
        self.classes.insert(name, fields);
    }

    pub fn push_enum(&mut self, name: String, fields: Vec<(String, u32, Type)>) {
        self.enums.insert(name, fields);
    }

    pub fn push_func(&mut self, name: String, params: Vec<Param>, return_type: Type, visibility: Visibility) {
        self.functions.insert(name, (params, return_type, visibility, self.module.clone()));
    }


    /// Checks the statements of a whole module.
    ///
    /// # Panics
    ///
    /// Panics on the first semantic error.
    pub fn check(&mut self, ast: &mut Vec<Stmt>) {
        self.check_stmts(ast);

        // Checking one instance's methods can create more instances
        while let Some(pending) = self.pending_instances.pop() {
            self.check_type_instance(pending);
        }

        // Literals that never got a type from their context stay int32
        let first = self.oversized_literals.drain()
            .map(|(_, literal)| literal)
            .min_by_key(|(_, span)| (span.line, span.col));
        if let Some((value, span)) = first {
            SemanticError::LiteralOutOfRange { value, ty: Type::INT32, span }.throw(&self.source);
        }
    }

    fn check_stmts(&mut self, ast: &mut Vec<Stmt>) {
        // Types are resolved before declaring, so the declared signatures use the final types
        self.declare_type_names(ast);
        for stmt in ast.iter_mut() {
            if let StmtKind::Declaration(decl) = &mut stmt.kind {
                self.canonicalize_decl(decl);
            }
        }
        // All traits are known before any impl is checked against them
        for stmt in ast.iter() {
            if let StmtKind::Declaration(Declaration { kind: DeclarationKind::Trait { name, methods }, .. }) = &stmt.kind {
                self.traits.insert(name.clone(), methods.clone());
            }
        }
        for stmt in ast.iter_mut() {
            if let StmtKind::Declaration(decl) = &mut stmt.kind {
                if matches!(decl.kind, DeclarationKind::Impl { .. }) {
                    self.complete_impl(decl);
                }
            }
        }
        for stmt in ast.iter_mut() {
            if let StmtKind::Declaration(decl) = &mut stmt.kind {
                self.declare_decl(decl);
            }
        }
        for stmt in ast.iter_mut() {
            if let StmtKind::Declaration(decl) = &stmt.kind {
                self.validate_decl_types(decl, &stmt.span);
            }
        }
        for stmt in ast.iter_mut() {
            self.check_stmt(stmt);
        }
    }

    /// Registers the names of all classes and enums declared in `ast`, so types
    /// referring to them can be resolved before the declarations are processed.
    ///
    /// [`Sema::declare_decl`] fills in their fields and variants later. Generic
    /// classes and enums are registered as templates for their instances.
    fn declare_type_names(&mut self, ast: &[Stmt]) {
        for stmt in ast {
            if let StmtKind::Declaration(decl) = &stmt.kind {
                match &decl.kind {
                    DeclarationKind::Class { name, type_params, .. } | DeclarationKind::Enum { name, type_params, .. }
                        if !type_params.is_empty() =>
                    {
                        self.generic_types.insert(name.clone(), GenericDecl {
                            decl: decl.clone(),
                            module: self.module.clone(),
                            source: self.source.clone(),
                            class: None,
                            outer_subst: HashMap::new(),
                        });
                    }
                    DeclarationKind::Class { name, .. } => { self.classes.entry(name.clone()).or_default(); }
                    DeclarationKind::Enum { name, .. } => { self.enums.entry(name.clone()).or_default(); }
                    DeclarationKind::Function { .. } | DeclarationKind::Trait { .. } | DeclarationKind::Impl { .. } => {}
                }
            }
        }
    }

    /// Checks an impl block against its trait and adds the trait's default
    /// methods it doesn't override.
    ///
    /// # Panics
    ///
    /// Panics if the trait is unknown, the target can't have methods, the trait is
    /// already implemented for the target, or the methods don't match the
    /// trait's: a method the trait doesn't have, a different signature, or a
    /// missing method without default body.
    fn complete_impl(&mut self, decl: &mut Declaration) {
        let span = decl.span.clone();
        let DeclarationKind::Impl { trait_name, target, methods } = &mut decl.kind else { return };

        let trait_methods = self.traits.get(trait_name).cloned()
            .unwrap_or_else(|| SemanticError::UnknownTrait { name: trait_name.clone(), span: span.clone() }.throw(&self.source));
        let type_name = target.owner_name()
            .unwrap_or_else(|| SemanticError::InvalidImplTarget { ty: target.clone(), span: span.clone() }.throw(&self.source));
        if self.impls.get(&type_name).is_some_and(|traits| traits.contains(trait_name)) {
            SemanticError::DuplicateImpl { trait_name: trait_name.clone(), ty: type_name, span }.throw(&self.source);
        }

        // The parser can't type `self` for built-in targets, so every method gets the target here
        for method in methods.iter_mut() {
            if let DeclarationKind::Function { params, .. } = &mut method.kind {
                for p in params.iter_mut().filter(|p| p.name == "self") {
                    p.ty = target.clone();
                }
            }
        }

        // Every method of the impl has to be declared by the trait, with the same signature
        for method in methods.iter() {
            let method_name = function_name(method);
            let trait_method = trait_methods.iter().find(|m| function_name(m) == method_name)
                .unwrap_or_else(|| SemanticError::NotATraitMethod {
                    method: method_name.to_string(),
                    trait_name: trait_name.clone(),
                    span: method.span.clone(),
                }.throw(&self.source));
            if !same_signature(&trait_method.kind, &method.kind) {
                SemanticError::TraitMethodMismatch {
                    method: method_name.to_string(),
                    trait_name: trait_name.clone(),
                    span: method.span.clone(),
                }.throw(&self.source);
            }
        }

        // Methods the impl doesn't have come from the trait's default bodies
        for trait_method in &trait_methods {
            let method_name = function_name(trait_method);
            if methods.iter().any(|m| function_name(m) == method_name) {
                continue;
            }
            let DeclarationKind::Function { body: Some(_), .. } = &trait_method.kind else {
                SemanticError::MissingTraitMethod {
                    method: method_name.to_string(),
                    trait_name: trait_name.clone(),
                    ty: type_name.clone(),
                    span: span.clone(),
                }.throw(&self.source)
            };

            let mut copy = trait_method.clone();
            copy.visibility = Visibility::Public;
            if let DeclarationKind::Function { params, parent, visibility, .. } = &mut copy.kind {
                // In the trait `self` stands for any implementing type, here it's the target
                for p in params.iter_mut().filter(|p| p.name == "self") {
                    p.ty = target.clone();
                }
                *parent = Some(type_name.clone());
                *visibility = Visibility::Public;
            }
            methods.push(copy);
        }

        self.impls.entry(type_name).or_default().push(trait_name.clone());
    }

    fn canonicalize_type(&mut self, ty: &Type, span: &Span) -> Type {
        match ty {
            // The parser can't tell classes and enums apart, both come in as `Class`
            Type::Object(name) | Type::Class(name) => {
                if self.enums.contains_key(name) {
                    Type::Enum(name.clone())
                } else if self.classes.contains_key(name) {
                    Type::Class(name.clone())
                } else if self.generic_types.contains_key(name) {
                    SemanticError::MissingTypeArgs { name: name.clone(), span: span.clone() }.throw(&self.source)
                } else {
                    SemanticError::UnknownType {
                        name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source)
                }
            }
            Type::Ref(inner) => Type::Ref(Box::new(self.canonicalize_type(inner, span))),
            // Inside a generic instance, `T` stands for its type argument
            Type::Param(name) => self.type_subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
            Type::Generic { name, args } => {
                let args: Vec<Type> = args.iter().map(|arg| self.canonicalize_type(arg, span)).collect();
                // The built-in `Unique<T>` and `Buffer<T>`, unless a user type has the same name
                if (name == "Unique" || name == "Buffer") && !self.generic_types.contains_key(name) {
                    if args.len() != 1 {
                        SemanticError::WrongTypeArgCount {
                            function: name.clone(),
                            expected: 1,
                            got: args.len(),
                            span: span.clone(),
                        }.throw(&self.source);
                    }
                    let arg = Box::new(args[0].clone());
                    return if name == "Unique" { Type::Unique(arg) } else { Type::Buffer(arg) };
                }
                // Inside a generic template the arguments can still be type parameters
                if args.iter().any(contains_type_param) {
                    return Type::Generic { name: name.clone(), args };
                }
                self.instantiate_type(name, args, span)
            }
            Type::Unique(inner) => Type::Unique(Box::new(self.canonicalize_type(inner, span))),
            Type::Buffer(inner) => Type::Buffer(Box::new(self.canonicalize_type(inner, span))),
            other => other.clone(),
        }
    }

    fn canonicalize_decl(&mut self, decl: &mut Declaration) {
        match &mut decl.kind {
            // Generic templates are canonicalized per instance, see `instantiate_type`
            DeclarationKind::Class { type_params, .. } | DeclarationKind::Enum { type_params, .. }
                if !type_params.is_empty() => {}
            DeclarationKind::Class { fields, methods, .. } => {
                for f in fields.iter_mut() {
                    let span = f.ty_span.clone();
                    f.ty = self.canonicalize_type(&f.ty, &span);
                }
                for m in methods.iter_mut() {
                    self.canonicalize_decl(m);
                }
            }
            DeclarationKind::Enum { fields, .. } => {
                for (_, _, ty, ty_span) in fields.iter_mut() {
                    *ty = self.canonicalize_type(ty, ty_span);
                }
            }
            DeclarationKind::Function { params, return_type, .. } => {
                // `self` is typed by the parser already, in traits it isn't a real type
                for p in params.iter_mut().filter(|p| p.name != "self") {
                    let span = p.ty_span.clone();
                    p.ty = self.canonicalize_type(&p.ty, &span);
                }
                if let Some(rt) = return_type.as_mut() {
                    let span = decl.span.clone();
                    *rt = self.canonicalize_type(rt, &span);
                }
            }
            DeclarationKind::Trait { methods, .. } => {
                for m in methods.iter_mut() {
                    self.canonicalize_decl(m);
                }
            }
            DeclarationKind::Impl { target, methods, .. } => {
                *target = self.canonicalize_type(target, &decl.span);
                for m in methods.iter_mut() {
                    self.canonicalize_decl(m);
                }
            }
        }
    }

    fn check_type(&self, ty: &Type, span: &Span) {
        match ty {
            Type::Class(name) => {
                if !self.classes.contains_key(name) {
                    SemanticError::UnknownType {
                        name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
            }
            Type::Enum(name) => {
                if !self.enums.contains_key(name) {
                    SemanticError::UnknownEnum {
                        name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
            }
            Type::Object(name) => {
                SemanticError::UnknownType {
                    name: name.clone(),
                    span: span.clone(),
                }.throw(&self.source);
            }
            Type::Ref(inner) | Type::Unique(inner) | Type::Buffer(inner) => self.check_type(inner, span),
            _ => {}
        }
    }

    fn validate_decl_types(&self, decl: &Declaration, span: &Span) {
        match &decl.kind {
            DeclarationKind::Class { type_params, .. } | DeclarationKind::Enum { type_params, .. }
                if !type_params.is_empty() => {}
            DeclarationKind::Class { fields, methods, .. } => {
                for f in fields {
                    self.check_type(&f.ty, &f.ty_span);
                }
                for m in methods {
                    self.validate_decl_types(m, span);
                }
            }
            DeclarationKind::Enum { fields, .. } => {
                for (_, _, ty, ty_span) in fields {
                    self.check_type(ty, ty_span);
                    // Enums don't free their payload yet
                    if self.owns_memory(ty) {
                        SemanticError::UniqueNotAllowed { context: "an enum payload", span: ty_span.clone() }
                            .throw(&self.source);
                    }
                }
            }
            DeclarationKind::Function { params, return_type, .. } => {
                for p in params {
                    self.check_type(&p.ty, &p.ty_span);
                }
                if let Some(rt) = return_type {
                    self.check_type(rt, span);
                }
            }
            // Trait methods are checked through the impls they get copied into
            DeclarationKind::Trait { .. } => {}
            DeclarationKind::Impl { methods, .. } => {
                for m in methods {
                    self.validate_decl_types(m, span);
                }
            }
        }
    }

    fn declare_decl(&mut self, decl: &mut Declaration) {
        if matches!(&decl.kind, DeclarationKind::Function { type_params, .. } if !type_params.is_empty()) {
            self.declare_generic(decl);
            return;
        }

        match &mut decl.kind {
            // Registered as templates already in `declare_type_names`
            DeclarationKind::Class { type_params, .. } | DeclarationKind::Enum { type_params, .. }
                if !type_params.is_empty() => {}
            DeclarationKind::Class { name, fields, methods, .. } => {
                self.push_class(name.clone(), fields.clone());
                let prev_class = self.current_class.take();
                self.current_class = Some(name.clone());
                for method in methods.iter_mut() {
                    self.declare_decl(method);
                }
                self.current_class = prev_class;
            }
            DeclarationKind::Enum { name, fields, .. } => {
                let stripped = fields.iter()
                    .map(|(n, i, t, _)| (n.clone(), *i, t.clone()))
                    .collect();
                self.push_enum(name.clone(), stripped);
            },
            DeclarationKind::Function { name, params, return_type, mangled, visibility, .. } => {
                let is_ctor = self.current_trait.is_none()
                    && matches!(&self.current_class, Some(cls) if cls == name);
                if is_ctor {
                    *return_type = Some(Type::Class(name.clone()));
                }
                let ret = return_type.clone().unwrap_or(Type::Void);
                // Trait methods include the trait, so two traits can have methods with the same name
                let m = match (&self.current_class, &self.current_trait) {
                    (Some(cls), Some(tr)) => format!("{}_{}_{}", cls, tr, name),
                    (Some(cls), None) if cls != name => format!("{}_{}", cls, name),
                    _ => name.clone(),
                };
                self.push_func(m.clone(), params.clone(), ret, visibility.clone());
                *mangled = Some(m);
            }
            DeclarationKind::Trait { .. } => {}
            DeclarationKind::Impl { trait_name, target, methods } => {
                let prev_class = self.current_class.take();
                self.current_class = target.owner_name();
                self.current_trait = Some(trait_name.clone());
                for method in methods.iter_mut() {
                    self.declare_decl(method);
                }
                self.current_trait = None;
                self.current_class = prev_class;
            }
        }
    }

    /// Registers the generic function or method `decl` as a template for its instances.
    ///
    /// Methods are registered under their mangled name like `Box<int32>_convert`,
    /// together with the class and its type arguments.
    ///
    /// # Panics
    ///
    /// Panics if it's a method of an impl block, which can't be generic, or a
    /// bound isn't a known trait.
    fn declare_generic(&mut self, decl: &Declaration) {
        let DeclarationKind::Function { name, type_params, .. } = &decl.kind else { return };
        // A trait method can't be generic, an impl couldn't match all its instances
        if self.current_trait.is_some() {
            SemanticError::GenericMethod { method: name.clone(), span: decl.span.clone() }.throw(&self.source);
        }
        for bound in type_params.iter().flat_map(|p| &p.bounds) {
            if !self.traits.contains_key(bound) {
                SemanticError::UnknownTrait { name: bound.clone(), span: decl.span.clone() }.throw(&self.source);
            }
        }

        // Generic methods are registered under their mangled name, like `Box<int32>_convert`
        let key = match &self.current_class {
            Some(class) => format!("{}_{}", class, name),
            None => name.clone(),
        };
        self.generic_functions.insert(key, GenericDecl {
            decl: decl.clone(),
            module: self.module.clone(),
            source: self.source.clone(),
            class: self.current_class.clone(),
            // Inside a generic class instance, its type arguments are already fixed
            outer_subst: self.type_subst.clone(),
        });
    }

    /// Returns the instance of the generic function `name` for a call with the
    /// arguments `args`, creating and checking it the first time.
    ///
    /// The type arguments are `type_args` if given explicitly. Otherwise they are
    /// inferred from the types of typed arguments first, then from the type
    /// `expected` for the call's result, and last from literal arguments, so
    /// `max(x, 7)` with `x: int64` gives `int64` instead of the literal's `int32`.
    ///
    /// The instance's body is checked in the module that declares the generic
    /// function, with every type parameter replaced by its type argument.
    ///
    /// # Panics
    ///
    /// Panics if the explicit type arguments have the wrong count, a type
    /// parameter can't be inferred, a type argument doesn't implement a bound,
    /// or the instance's body contains an error.
    fn instantiate(&mut self, name: &str, type_args: &[Type], args: &mut [Expr], expected: Option<&Type>, span: &Span) -> String {
        let generic = self.generic_functions.get(name).cloned().unwrap();
        let DeclarationKind::Function { type_params, params, return_type, .. } = &generic.decl.kind else {
            unreachable!("generic functions are function declarations");
        };

        for arg in args.iter_mut() {
            self.check_expr(arg);
        }
        let bindings: Vec<(Type, &Expr)> = params.iter()
            .filter(|p| p.name != "self")
            .map(|p| p.ty.clone())
            .zip(args.iter())
            .collect();
        let result = return_type.as_ref().zip(expected);
        let subst = self.infer_type_args(name, type_params, type_args, &bindings, result, span);

        let instance = instance_name(name, type_params, &subst);
        if self.functions.contains_key(&instance) {
            return instance;
        }

        let mut decl = generic.decl.clone();
        let module = generic.module.clone();
        self.in_template_context(subst, &generic, |sema| {
            let DeclarationKind::Function { type_params, params, return_type, mangled, visibility, .. } = &mut decl.kind else {
                unreachable!("generic functions are function declarations");
            };
            type_params.clear();
            // Resolves `T`, and instantiates types like `Box<T>` in the signature
            for p in params.iter_mut() {
                let span = p.ty_span.clone();
                p.ty = sema.canonicalize_type(&p.ty, &span);
            }
            if let Some(ty) = return_type.as_mut() {
                *ty = sema.canonicalize_type(ty, span);
            }
            *mangled = Some(instance.clone());

            // Registered before checking the body, so the instance can call itself
            let ret = return_type.clone().unwrap_or(Type::Void);
            sema.functions.insert(instance.clone(), (params.clone(), ret, visibility.clone(), module));

            sema.check_decl(&mut decl);
        });

        self.instances.push(decl);
        instance
    }

    /// Infers the type arguments of the generic `owner` with the type parameters
    /// `type_params`, and checks their bounds.
    ///
    /// The type arguments are `explicit` if given. Otherwise they are inferred
    /// from `bindings`, pairs of a declared type like `T` or `Box<T>` and an
    /// already checked expression for it: typed expressions first, then the
    /// declared result type against the expected one in `result`, and last
    /// literals, so `max(x, 7)` with `x: int64` gives `int64` instead of the
    /// literal's `int32`.
    ///
    /// # Panics
    ///
    /// Panics if the explicit type arguments have the wrong count, a type
    /// parameter can't be inferred or a type argument doesn't implement a bound.
    fn infer_type_args(
        &mut self,
        owner: &str,
        type_params: &[TypeParam],
        explicit: &[Type],
        bindings: &[(Type, &Expr)],
        result: Option<(&Type, &Type)>,
        span: &Span,
    ) -> HashMap<String, Type> {
        let mut subst: HashMap<String, Type> = HashMap::new();
        if !explicit.is_empty() {
            // `impl Trait` parameters can't be given explicitly
            let named: Vec<&TypeParam> = type_params.iter().filter(|p| !p.is_hidden()).collect();
            if named.len() != explicit.len() {
                SemanticError::WrongTypeArgCount {
                    function: owner.to_string(),
                    expected: named.len(),
                    got: explicit.len(),
                    span: span.clone(),
                }.throw(&self.source);
            }
            for (param, arg) in named.iter().zip(explicit) {
                let ty = self.canonicalize_type(arg, span);
                self.check_type(&ty, span);
                subst.insert(param.name.clone(), ty);
            }
        }

        for (declared, arg) in bindings {
            if !is_number_literal(arg) {
                self.bind_type_params(declared, &arg.ty, &mut subst);
            }
        }
        if let Some((declared, expected)) = result {
            self.bind_type_params(declared, expected, &mut subst);
        }
        for (declared, arg) in bindings {
            if is_number_literal(arg) {
                self.bind_type_params(declared, &arg.ty, &mut subst);
            }
        }

        for param in type_params {
            let Some(ty) = subst.get(&param.name) else {
                SemanticError::CannotInferTypeParam {
                    param: param.name.clone(),
                    function: owner.to_string(),
                    span: span.clone(),
                }.throw(&self.source)
            };
            for bound in &param.bounds {
                if !self.implements(ty, bound) {
                    SemanticError::TraitNotImplemented {
                        ty: ty.clone(),
                        trait_name: bound.clone(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
            }
        }
        subst
    }

    /// Binds the type parameters in the declared type `pattern` to the matching
    /// part of the actual type `actual`. Earlier bindings are kept.
    fn bind_type_params(&self, pattern: &Type, actual: &Type, subst: &mut HashMap<String, Type>) {
        match (pattern, actual) {
            (_, Type::Unknown | Type::Void) => {}
            (Type::Param(name), _) => {
                subst.entry(name.clone()).or_insert_with(|| actual.clone());
            }
            (Type::Ref(pattern), Type::Ref(actual)) => self.bind_type_params(pattern, actual, subst),
            (Type::Unique(pattern), Type::Unique(actual)) | (Type::Buffer(pattern), Type::Buffer(actual)) => {
                self.bind_type_params(pattern, actual, subst);
            }
            // `Unique<T>` or `Buffer<T>` in a template, which stays generic until `T` is known
            (Type::Generic { name, args }, Type::Unique(actual)) if name == "Unique" && args.len() == 1 => {
                self.bind_type_params(&args[0], actual, subst);
            }
            (Type::Generic { name, args }, Type::Buffer(actual)) if name == "Buffer" && args.len() == 1 => {
                self.bind_type_params(&args[0], actual, subst);
            }
            // Arguments for `&T` parameters are referenced automatically
            (Type::Ref(pattern), _) => self.bind_type_params(pattern, actual, subst),
            // `Box<T>` against the instance `Box<int32>` binds `T = int32`
            (Type::Generic { name, args }, Type::Class(instance) | Type::Enum(instance)) => {
                if let Some((template, instance_args)) = self.type_instances.get(instance) {
                    if template == name {
                        for (pattern, actual) in args.iter().zip(instance_args) {
                            self.bind_type_params(pattern, actual, subst);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Runs `f` like inside the declaration of the generic `template`: with
    /// fresh local variables, the type parameters replaced by `subst` and the
    /// ones fixed by its surroundings, inside its class for a generic method,
    /// and with the module and source of the template, so its private helpers
    /// are visible and errors point at its code. Everything is restored afterwards.
    fn in_template_context<R>(&mut self, subst: HashMap<String, Type>, template: &GenericDecl, f: impl FnOnce(&mut Self) -> R) -> R {
        let mut full_subst = template.outer_subst.clone();
        full_subst.extend(subst);

        let scope = std::mem::take(&mut self.scope);
        let moved = std::mem::take(&mut self.moved);
        let outer_subst = std::mem::replace(&mut self.type_subst, full_subst);
        let class = std::mem::replace(&mut self.current_class, template.class.clone());
        let trait_name = self.current_trait.take();
        let return_ty = self.current_return_type.take();
        let module = std::mem::replace(&mut self.module, template.module.clone());
        let source = std::mem::replace(&mut self.source, template.source.clone());

        let result = f(self);

        self.scope = scope;
        self.moved = moved;
        self.type_subst = outer_subst;
        self.current_class = class;
        self.current_trait = trait_name;
        self.current_return_type = return_ty;
        self.module = module;
        self.source = source;
        result
    }

    /// Returns the instance of the generic class or enum `name` for the type
    /// arguments `args`, like `Box<int32>` for `Box` and `int32`.
    ///
    /// The first time, the instance is created as an ordinary class or enum
    /// named `Box<int32>`, with the type parameters replaced, and declared right
    /// away. Its method bodies are checked at the end of the module.
    ///
    /// # Panics
    ///
    /// Panics if `name` isn't a generic class or enum, the number of type
    /// arguments is wrong, or one doesn't implement a bound.
    fn instantiate_type(&mut self, name: &str, args: Vec<Type>, span: &Span) -> Type {
        let generic = self.generic_types.get(name).cloned()
            .unwrap_or_else(|| SemanticError::UnknownType { name: name.to_string(), span: span.clone() }.throw(&self.source));
        let (is_enum, type_params) = match &generic.decl.kind {
            DeclarationKind::Class { type_params, .. } => (false, type_params.clone()),
            DeclarationKind::Enum { type_params, .. } => (true, type_params.clone()),
            _ => unreachable!("generic types are classes or enums"),
        };
        if args.len() != type_params.len() {
            SemanticError::WrongTypeArgCount {
                function: name.to_string(),
                expected: type_params.len(),
                got: args.len(),
                span: span.clone(),
            }.throw(&self.source);
        }
        let subst: HashMap<String, Type> = type_params.iter().map(|p| p.name.clone()).zip(args.iter().cloned()).collect();
        for param in &type_params {
            for bound in &param.bounds {
                if !self.implements(&subst[&param.name], bound) {
                    SemanticError::TraitNotImplemented {
                        ty: subst[&param.name].clone(),
                        trait_name: bound.clone(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
            }
        }

        let instance = instance_name(name, &type_params, &subst);
        let ty = if is_enum { Type::Enum(instance.clone()) } else { Type::Class(instance.clone()) };
        if self.type_instances.contains_key(&instance) {
            return ty;
        }

        let mut decl = generic.decl.clone();
        match &mut decl.kind {
            DeclarationKind::Class { name: class_name, type_params, methods, .. } => {
                *class_name = instance.clone();
                type_params.clear();
                for method in methods.iter_mut() {
                    if let DeclarationKind::Function { name: method_name, params, parent, .. } = &mut method.kind {
                        // The constructor is named after the class
                        if method_name == name {
                            *method_name = instance.clone();
                        }
                        if parent.is_some() {
                            *parent = Some(instance.clone());
                        }
                        for p in params.iter_mut().filter(|p| p.name == "self") {
                            p.ty = ty.clone();
                        }
                    }
                }
                self.classes.entry(instance.clone()).or_default();
            }
            DeclarationKind::Enum { name: enum_name, type_params, .. } => {
                *enum_name = instance.clone();
                type_params.clear();
                self.enums.entry(instance.clone()).or_default();
            }
            _ => unreachable!("generic types are classes or enums"),
        }
        // Registered before resolving its types, so the instance can refer to itself
        self.type_instances.insert(instance.clone(), (name.to_string(), args));

        self.in_template_context(subst.clone(), &generic, |sema| {
            sema.canonicalize_decl(&mut decl);
            sema.declare_decl(&mut decl);
            sema.validate_decl_types(&decl, span);
        });
        self.pending_instances.push(PendingInstance {
            decl,
            subst,
            module: generic.module.clone(),
            source: generic.source.clone(),
        });
        ty
    }

    /// Checks the method bodies of an instance of a generic class or enum and
    /// hands it to codegen.
    ///
    /// # Panics
    ///
    /// Panics if a method body contains an error.
    fn check_type_instance(&mut self, pending: PendingInstance) {
        let PendingInstance { mut decl, subst, module, source } = pending;
        let template = GenericDecl { decl: decl.clone(), module, source, class: None, outer_subst: HashMap::new() };
        self.in_template_context(subst, &template, |sema| {
            sema.check_decl(&mut decl);
        });
        self.instances.push(decl);
    }

    /// The generic result type of a generic class or enum's constructors, like
    /// `Box<T>` for `class Box<T>`, to infer type arguments from an expected type.
    fn generic_self_type(name: &str, type_params: &[TypeParam]) -> Type {
        Type::Generic {
            name: name.to_string(),
            args: type_params.iter().map(|p| Type::Param(p.name.clone())).collect(),
        }
    }

    /// Returns the instance of the generic class `name` constructed with `args`,
    /// like `Box<int32>` for `Box(5)` or `Box<int32>(5)`. Its constructor has
    /// the instance's name.
    ///
    /// The type arguments are inferred like for generic functions, from the
    /// constructor arguments and the `expected` type.
    ///
    /// # Panics
    ///
    /// Panics if the type arguments can't be inferred or are invalid.
    fn instantiate_constructor(&mut self, name: &str, type_args: &[Type], args: &mut [Expr], expected: Option<&Type>, span: &Span) -> String {
        let generic = self.generic_types.get(name).cloned().unwrap();
        let DeclarationKind::Class { type_params, methods, .. } = &generic.decl.kind else {
            unreachable!("only generic classes have constructors");
        };
        let constructor_params = constructor_params(name, methods);

        for arg in args.iter_mut() {
            self.check_expr(arg);
        }
        let bindings: Vec<(Type, &Expr)> = constructor_params.iter().map(|p| p.ty.clone()).zip(args.iter()).collect();
        let self_type = Self::generic_self_type(name, type_params);
        let result = expected.map(|expected| (&self_type, expected));
        let subst = self.infer_type_args(name, type_params, type_args, &bindings, result, span);

        let type_args = type_params.iter().map(|p| subst[&p.name].clone()).collect();
        self.instantiate_type(name, type_args, span).owner_name().unwrap()
    }

    /// Returns the instance of the generic class `class` for a brace initializer
    /// with `fields`, like `Box<int32>` for `Box { value: 5 }`.
    ///
    /// The type arguments are inferred like for constructor calls. The field
    /// values get checked for that, the caller mustn't check them again.
    ///
    /// # Panics
    ///
    /// Panics if the type arguments can't be inferred or are invalid.
    fn instantiate_struct_init(&mut self, class: &str, type_args: &[Type], fields: &mut [(String, Expr)], expected: Option<&Type>, span: &Span) -> String {
        let generic = self.generic_types.get(class).cloned().unwrap();
        let DeclarationKind::Class { type_params, methods, .. } = &generic.decl.kind else {
            SemanticError::UnknownFunction { fn_name: class.to_string(), span: span.clone() }.throw(&self.source)
        };
        let constructor_params = constructor_params(class, methods);

        for (_, value) in fields.iter_mut() {
            self.check_expr(value);
        }
        // Unknown field names are reported later, when the fields are matched to the constructor
        let bindings: Vec<(Type, &Expr)> = fields.iter()
            .filter_map(|(field, value)| {
                constructor_params.iter().find(|p| p.name == *field).map(|p| (p.ty.clone(), value))
            })
            .collect();
        let self_type = Self::generic_self_type(class, type_params);
        let result = expected.map(|expected| (&self_type, expected));
        let subst = self.infer_type_args(class, type_params, type_args, &bindings, result, span);

        let type_args = type_params.iter().map(|p| subst[&p.name].clone()).collect();
        self.instantiate_type(class, type_args, span).owner_name().unwrap()
    }

    /// Returns the instance of the generic enum `name` for constructing its
    /// variant `tag` with `value`, like `Option<int32>` for `Option::Some(5)`.
    ///
    /// The type arguments are inferred from the payload, unless `check_value`
    /// is false, and the `expected` type. If the payload got checked, the
    /// caller mustn't check it again.
    ///
    /// # Panics
    ///
    /// Panics if the variant doesn't exist or the type arguments can't be inferred.
    fn instantiate_enum_init(&mut self, name: &str, type_args: &[Type], tag: &str, value: &mut Expr, check_value: bool, expected: Option<&Type>, span: &Span) -> String {
        let generic = self.generic_types.get(name).cloned().unwrap();
        let DeclarationKind::Enum { type_params, fields, .. } = &generic.decl.kind else {
            SemanticError::UnknownEnum { name: name.to_string(), span: span.clone() }.throw(&self.source)
        };
        let payload = fields.iter().find(|(variant, _, _, _)| variant == tag)
            .map(|(_, _, ty, _)| ty.clone())
            .unwrap_or_else(|| SemanticError::UnknownVariant {
                enum_name: name.to_string(),
                variant: tag.to_string(),
                span: span.clone(),
            }.throw(&self.source));

        let mut bindings: Vec<(Type, &Expr)> = Vec::new();
        if check_value {
            self.check_expr(value);
            bindings.push((payload, value));
        }
        let self_type = Self::generic_self_type(name, type_params);
        let result = expected.map(|expected| (&self_type, expected));
        let subst = self.infer_type_args(name, type_params, type_args, &bindings, result, span);

        let type_args = type_params.iter().map(|p| subst[&p.name].clone()).collect();
        self.instantiate_type(name, type_args, span).owner_name().unwrap()
    }

    /// Whether `ty` implements the trait `trait_name`.
    fn implements(&self, ty: &Type, trait_name: &str) -> bool {
        ty.owner_name()
            .and_then(|owner| self.impls.get(&owner))
            .is_some_and(|traits| traits.iter().any(|t| t == trait_name))
    }

    fn check_stmt(&mut self, stmt: &mut Stmt) {
        let stmt_span = stmt.span.clone();
        let ty = match &mut stmt.kind {
            StmtKind::Return(expr) => {
                let cur = self.current_return_type.clone().unwrap();
                self.check_expr_expecting(expr, &cur);
                self.coerce(expr, &cur);
                let ret = expr.ty.clone();
                if ret != cur {
                    SemanticError::TypeMismatch { lhs: ret, rhs: cur , span: expr.span.clone(), context: "function return type" }.throw(&self.source);
                }
                ret
            },
            StmtKind::Declaration(decl) => self.check_decl(decl),
            StmtKind::Instantiation { name, typedef, value, mutable } => {
                let declared = typedef.as_ref().map(|td| {
                    let ty = self.canonicalize_type(td, &stmt_span);
                    self.check_type(&ty, &stmt_span);
                    ty
                });
                let value_ty = match &declared {
                    Some(ty) => self.check_expr_expecting(value, ty),
                    None => self.check_expr(value),
                };
                let ty = match declared {
                    Some(ty) => {
                        self.coerce(value, &ty);
                        if value.ty != ty {
                            SemanticError::TypeMismatch {
                                lhs: value.ty.clone(),
                                rhs: ty,
                                span: value.span.clone(),
                                context: "variable declaration",
                            }.throw(&self.source);
                        }
                        ty
                    }
                    None => value_ty,
                };
                self.check_type(&ty, &stmt_span);
                self.scope.insert(name.clone(), (ty.clone(), *mutable));
                // A new variable, even if an old one with the same name got moved
                self.moved.remove(name.as_str());
                *typedef = Some(ty.clone());
                ty
            }
            StmtKind::Import(expr) => self.check_expr(expr),
            StmtKind::Expression(expr) => self.check_expr(expr),
            StmtKind::Match { scrutinee, arms } => {
                let scrutinee_ty = self.check_expr(scrutinee);
                self.check_match_arms(&scrutinee_ty, arms, &stmt_span);
                Type::Void
            }
            StmtKind::If { condition, then_body, else_body } => {
                self.check_condition(condition, "if condition");

                // Like match arms: each branch starts from the same moves, afterwards
                // a variable counts as moved if a branch moved it
                let moved_before = self.moved.clone();
                self.check_block(then_body);
                let mut moved_after = std::mem::replace(&mut self.moved, moved_before);
                if let Some(else_body) = else_body {
                    self.check_block(else_body);
                }
                moved_after.extend(self.moved.drain());
                self.moved = moved_after;
                Type::Void
            }
            StmtKind::While { condition, body } => {
                self.check_condition(condition, "while condition");
                let moved_before = self.moved.clone();
                self.check_block(body);
                // The body can run several times, a value moved in one round would be gone in the next
                if let Some(var) = self.moved.iter().find(|var| !moved_before.contains(*var)) {
                    SemanticError::MoveInLoop { var: var.clone(), span: stmt_span.clone() }.throw(&self.source);
                }
                Type::Void
            }
        };

        stmt.ty = ty.clone();
    }

    /// Checks the condition of an `if` or `while`, which has to be a `bool`.
    ///
    /// # Panics
    ///
    /// Panics if the condition has another type.
    fn check_condition(&mut self, condition: &mut Expr, context: &'static str) {
        let ty = self.check_expr_expecting(condition, &Type::Bool);
        if ty != Type::Bool {
            SemanticError::TypeMismatch {
                lhs: ty,
                rhs: Type::Bool,
                span: condition.span.clone(),
                context,
            }.throw(&self.source);
        }
    }

    /// Checks the statements of a block like the body of an `if` or `while`.
    /// Variables declared in it are gone afterwards.
    fn check_block(&mut self, body: &mut Vec<Stmt>) {
        let prev_scope = self.scope.clone();
        for stmt in body.iter_mut() {
            self.check_stmt(stmt);
        }
        self.moved.retain(|var| prev_scope.contains_key(var));
        self.scope = prev_scope;
    }

    fn check_match_arms(&mut self, scrutinee_ty: &Type, arms: &mut Vec<MatchArm>, span: &Span) {
        // Every arm starts with what was moved before the match. Afterwards a
        // variable counts as moved if any arm moved it, since that arm might have run.
        let moved_before = self.moved.clone();
        let mut moved_after = moved_before.clone();

        for arm in arms.iter_mut() {
            let prev_scope = self.scope.clone();
            self.moved = moved_before.clone();
            match &mut arm.pattern {
                Pattern::EnumVariant { enum_name, variant, binding } => {
                    let scrut_enum = match scrutinee_ty {
                        Type::Enum(n) => n.clone(),
                        other => SemanticError::InvalidReceiver {
                            recv_ty: other.clone(),
                            span: arm.span.clone(),
                            context: "cannot be matched against an enum variant pattern",
                        }.throw(&self.source),
                    };
                    // `Option::Some(x)` matches any instance of the generic enum `Option`,
                    // codegen needs the exact one like `Option<int32>`
                    let is_instance_of_pattern = self.type_instances.get(&scrut_enum)
                        .is_some_and(|(template, _)| template == enum_name);
                    if is_instance_of_pattern {
                        *enum_name = scrut_enum.clone();
                    }
                    if enum_name != &scrut_enum {
                        SemanticError::TypeMismatch {
                            lhs: Type::Enum(enum_name.clone()),
                            rhs: Type::Enum(scrut_enum.clone()),
                            span: arm.span.clone(),
                            context: "match arm pattern",
                        }.throw(&self.source);
                    }
                    let variants = self.enums.get(&scrut_enum).cloned()
                        .unwrap_or_else(|| SemanticError::UnknownEnum {
                            name: scrut_enum.clone(),
                            span: arm.span.clone(),
                        }.throw(&self.source));
                    let payload_ty = variants.iter().find(|(v, _, _)| v == variant)
                        .map(|(_, _, t)| t.clone())
                        .unwrap_or_else(|| SemanticError::UnknownVariant {
                            enum_name: scrut_enum.clone(),
                            variant: variant.clone(),
                            span: arm.span.clone(),
                        }.throw(&self.source));
                    if let Some(name) = binding {
                        self.scope.insert(name.clone(), (payload_ty, false));
                    }
                }
                Pattern::Literal(expr) if matches!(expr.kind, ExprKind::Identifier(_)) => {
                    self.scope.insert(expr.kind.to_string(), (scrutinee_ty.clone(), false));
                    let mut expr = expr.clone();
                    let lit_ty = self.check_expr(&mut expr);
                    if &lit_ty != scrutinee_ty {
                        SemanticError::TypeMismatch {
                            lhs: lit_ty,
                            rhs: scrutinee_ty.clone(),
                            span: arm.span.clone(),
                            context: "match arm pattern",
                        }.throw(&self.source);
                    }
                }
                Pattern::Literal(expr) => {
                    self.check_expr(expr);
                    self.coerce_literal(expr, scrutinee_ty);
                    let lit_ty = expr.ty.clone();
                    if &lit_ty != scrutinee_ty {
                        SemanticError::TypeMismatch {
                            lhs: lit_ty,
                            rhs: scrutinee_ty.clone(),
                            span: arm.span.clone(),
                            context: "match arm pattern",
                        }.throw(&self.source);
                    }
                }
                Pattern::Wildcard => {}
            }
            for stmt in arm.body.iter_mut() {
                self.check_stmt(stmt);
            }
            // Variables declared in the arm are gone afterwards
            moved_after.extend(self.moved.iter().filter(|name| prev_scope.contains_key(*name)).cloned());
            self.scope = prev_scope;
        }
        self.moved = moved_after;
        let _ = span;
    }

    /// Checks `expr` knowing it should have the type `expected`. A generic call
    /// uses that to infer its type arguments, like `T = int64` in
    /// `let x: int64 = max(3, 7);`.
    fn check_expr_expecting(&mut self, expr: &mut Expr, expected: &Type) -> Type {
        self.expected_type = Some(expected.clone());
        self.check_expr(expr)
    }

    fn check_expr(&mut self, expr: &mut Expr) -> Type {
        let span = expr.span.clone();
        // Only meant for this expression, not for the ones nested in it
        let expected = self.expected_type.take();

        if self.is_unique_call(expr) {
            let ty = self.check_unique_call(expr, expected.as_ref());
            expr.ty = ty.clone();
            return ty;
        }
        if self.is_buffer_call(expr) {
            let ty = self.check_buffer_call(expr, expected.as_ref());
            expr.ty = ty.clone();
            return ty;
        }

        let ty = match &mut expr.kind {
            ExprKind::Boolean(_) => Type::Bool,
            ExprKind::Unique(inner) => {
                self.check_expr(inner);
                Type::Unique(Box::new(inner.ty.clone()))
            }
            ExprKind::NewBuffer(capacity) => {
                self.check_expr(capacity);
                expr.ty.clone()
            }
            ExprKind::IntLiteral(value) => {
                // Reported at the end of the module unless the context gives it a bigger type
                if !int_fits(*value, 32, true) {
                    self.oversized_literals.insert((span.line, span.col), (*value, span.clone()));
                }
                Type::INT32
            }
            ExprKind::FloatLiteral(_) => Type::FLOAT64,
            ExprKind::StringLiteral(_) => Type::String,
            ExprKind::Identifier(name) => {
                let ty = self.use_variable(name, &span);
                // Using a `Unique` or `Buffer` as a value moves it out of the variable
                if ty.is_move_only() {
                    self.moved.insert(name.clone());
                }
                ty
            }
            ExprKind::ModuleImport { .. } => Type::Void,
            ExprKind::FieldAccess { object, field } => {
                let recv_ty = self.check_receiver(object);

                // Fields of a referenced value or one in a `Unique` are accessed directly
                let peeled = match &recv_ty {
                    Type::Ref(inner) | Type::Unique(inner) => (**inner).clone(),
                    other => other.clone(),
                };
                let field_name = field.clone();

                if let Type::Enum(enum_name) = &peeled {
                    if field_name == "tag" {
                        expr.ty = Type::INT32;
                        return Type::INT32;
                    }

                    SemanticError::UnknownField {
                        class: enum_name.clone(),
                        field: field_name,
                        span: span.clone(),
                    }.throw(&self.source);
                }

                let class_name = match peeled {
                    Type::Class(c) => c,
                    other => SemanticError::InvalidReceiver {
                        recv_ty: other,
                        span: span.clone(),
                        context: "is not an object",
                    }.throw(&self.source),
                };

                let (field_ty, visibility) = self.classes.get(&class_name)
                    .and_then(|fields| fields.iter().find(|p| p.name == field_name))
                    .map(|p| (p.ty.clone(), p.visibility.clone()))
                    .unwrap_or_else(|| SemanticError::UnknownField {
                        class: class_name.clone(),
                        field: field_name.clone(),
                        span: span.clone(),
                    }.throw(&self.source));

                let inside_self = match self.current_class.clone() {
                    Some(name) => {
                        class_name == name
                    }
                    None => false,
                };

                if matches!(visibility, Visibility::Private) && !inside_self {
                    SemanticError::PrivateFieldAccess{ field_name, span  }.throw(&self.source);
                }

                field_ty
            },
            ExprKind::Assignment { lhs, rhs } => {
                if let ExprKind::Identifier(name) = &lhs.kind {
                    let (_, mutable) = self.scope.get(name).cloned()
                        .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                            identifier: name.clone(),
                            span: span.clone(),
                        }.throw(&self.source));
                    if !mutable {
                        SemanticError::AssignOnUnmutable {
                            var: name.clone(),
                            span: span.clone(),
                        }.throw(&self.source)
                    }
                }

                // `a.b.c = value` changes the variable `a`
                if let (ExprKind::FieldAccess { .. }, Some(name)) = (&lhs.kind, root_variable(lhs)) {
                    let (_, mutable) = self.scope.get(name).cloned()
                        .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                            identifier: name.to_string(),
                            span: span.clone(),
                        }.throw(&self.source));
                    if !mutable {
                        SemanticError::AssignOnUnmutable {
                            var: name.to_string(),
                            span: span.clone(),
                        }.throw(&self.source)
                    }
                }

                // Assigning to a variable doesn't use its old value, a moved `Unique` can get a new one
                let lhs_ty = match &lhs.kind {
                    ExprKind::Identifier(name) => {
                        let ty = self.scope.get(name).map(|(ty, _)| ty.clone()).unwrap();
                        // In class methods `self` only points to the object, replacing it
                        // wouldn't change the object. Built-in types' `self` is the value itself
                        if name == "self" && matches!(ty, Type::Class(_) | Type::Enum(_)) {
                            SemanticError::AssignToSelf { span: span.clone() }.throw(&self.source);
                        }
                        lhs.ty = ty.clone();
                        ty
                    }
                    _ => self.check_expr(lhs),
                };
                self.check_expr_expecting(rhs, &lhs_ty);
                self.coerce(rhs, &lhs_ty);
                let rhs_ty = rhs.ty.clone();
                if lhs_ty != rhs_ty {
                    SemanticError::TypeMismatch {
                        lhs: lhs_ty,
                        rhs: rhs_ty,
                        span: span.clone(),
                        context: "assignment",
                    }.throw(&self.source);
                }
                if let ExprKind::Identifier(name) = &lhs.kind {
                    self.moved.remove(name);
                }

                rhs_ty
            },
            ExprKind::Convert { value, target } => {
                // Resolves type parameters, so `x as T` works in generic functions
                *target = self.canonicalize_type(target, &span);
                self.check_expr(value);
                // A literal takes the target type directly, so `3000000000 as int64` works
                if target.is_number() {
                    self.coerce_literal(value, target);
                }

                // Numbers convert to any number type, bools to numbers as 0 or 1
                let allowed = target.is_number() && (value.ty.is_number() || value.ty == Type::Bool);
                if !allowed {
                    SemanticError::InvalidConversion {
                        from: value.ty.clone(),
                        to: target.clone(),
                        span: span.clone(),
                    }.throw(&self.source);
                }
                target.clone()
            }
            ExprKind::Negation(inner) => {
                let inner_ty = self.check_expr(inner);
                if !matches!(inner_ty, Type::Int { signed: true, .. } | Type::Float { .. }) {
                    SemanticError::CannotNegate { ty: inner_ty, span: span.clone() }.throw(&self.source);
                }
                inner_ty
            }
            ExprKind::Ref(inner) => {
                if !matches!(inner.kind, ExprKind::Identifier(_) | ExprKind::FieldAccess { .. }) {
                    panic!("'&' operand must be a place expression, got '{}'", inner.kind);
                }
                // Referencing a variable borrows it, a `Unique` doesn't get moved by that
                let inner_ty = match &inner.kind {
                    ExprKind::Identifier(name) => {
                        let ty = self.use_variable(name, &span);
                        inner.ty = ty.clone();
                        ty
                    }
                    _ => self.check_expr(inner),
                };
                Type::Ref(Box::new(inner_ty))
            },
            ExprKind::BinaryExpr { lhs, rhs, op } => {
                self.check_expr(lhs);
                self.check_expr(rhs);

                // A literal takes the type of the other side, like `1` in `x + 1` with `x: uint8`
                if is_number_literal(lhs) && !is_number_literal(rhs) {
                    self.coerce_literal(lhs, &rhs.ty);
                } else if is_number_literal(rhs) && !is_number_literal(lhs) {
                    self.coerce_literal(rhs, &lhs.ty);
                }

                // The narrower side widens to the other, like `int32 + int64` to `int64`
                if lhs.ty.widens_to(&rhs.ty) {
                    let target = rhs.ty.clone();
                    self.coerce(lhs, &target);
                } else if rhs.ty.widens_to(&lhs.ty) {
                    let target = lhs.ty.clone();
                    self.coerce(rhs, &target);
                }

                let (lhs_ty, rhs_ty) = (lhs.ty.clone(), rhs.ty.clone());
                if lhs_ty != rhs_ty {
                    SemanticError::TypeMismatch {
                        lhs: lhs_ty,
                        rhs: rhs_ty,
                        span: span.clone(),
                        context: "binary expression",
                    }.throw(&self.source);
                }

                // `==` and `!=` work on numbers and bools, ordering only on numbers
                let valid = match op {
                    Token::EqEq | Token::NotEq => lhs_ty.is_number() || lhs_ty == Type::Bool,
                    Token::Lt | Token::LtEq | Token::Gt | Token::GtEq => lhs_ty.is_number(),
                    _ => true,
                };
                if !valid {
                    SemanticError::InvalidOperands { op: op.clone(), ty: lhs_ty, span: span.clone() }.throw(&self.source);
                }

                if op.is_comparison() { Type::Bool } else { lhs_ty }
            }
            ExprKind::ColonAccess { name, tag } => {
                let variants = self.enums.get(name).cloned()
                    .unwrap_or_else(|| SemanticError::UnknownEnum {
                        name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source));

                match variants.iter().find(|(v, _, _)| v == tag) {
                    Some((_, _, ty)) => ty.clone(),
                    None => SemanticError::UnknownVariant {
                        enum_name: name.clone(),
                        variant: tag.clone(),
                        span: span.clone(),
                    }.throw(&self.source),
                }
            }
            ExprKind::FunctionCall { name, type_args, params, receiver, mangled } => {
                let (mut m, recv_class) = self.resolve_call(name, receiver, &span);
                if m.starts_with("Buffer.") {
                    let receiver = receiver.as_deref().expect("Buffer methods have a receiver");
                    let ty = self.check_buffer_method(name, receiver, params, &span);
                    *mangled = Some(m);
                    expr.ty = ty.clone();
                    return ty;
                }
                // Inferring type arguments checks the arguments already. Checking them
                // again would fail, sema rewrites some of them, like struct initializers
                let mut args_checked = false;
                if !self.functions.contains_key(&m) {
                    if self.generic_functions.contains_key(&m) {
                        // A generic method like `Box<int32>_convert`, or a generic function
                        m = self.instantiate(&m.clone(), type_args, params, expected.as_ref(), &span);
                        args_checked = true;
                    } else if recv_class.is_none() && self.generic_functions.contains_key(name.as_str()) {
                        // A generic function called inside a class, where `name` got looked up as a method
                        m = self.instantiate(&name.clone(), type_args, params, expected.as_ref(), &span);
                        args_checked = true;
                    } else if recv_class.is_none() && self.generic_types.contains_key(name.as_str()) {
                        // Constructor call of a generic class, like `Box(5)`
                        m = self.instantiate_constructor(&name.clone(), type_args, params, expected.as_ref(), &span);
                        args_checked = true;
                    } else if recv_class.is_none() && self.functions.contains_key(name.as_str()) {
                        // A free function called inside a class
                        m = name.clone();
                    }
                }
                // Unknown names fall through to the unknown function error below
                if !args_checked && !type_args.is_empty() && self.functions.contains_key(&m) {
                    SemanticError::NotGeneric { function: name.clone(), span: span.clone() }.throw(&self.source);
                }

                let (defs, ret, visibility, module) = self.functions.get(&m).cloned()
                    .unwrap_or_else(|| SemanticError::UnknownFunction {
                        fn_name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source));

                if params.len() > defs.len(){
                    SemanticError::TooManyArguments{ func_name: name.clone(), parent: module, span: span.clone() }
                        .throw(&self.source);
                }

                // A method changing `self` needs a receiver that may be changed. Without
                // receiver the call goes to the calling method's own `self`
                if defs.first().is_some_and(|p| p.name == "self" && p.mutable) {
                    // `a.b.change()` changes the variable `a`. A temporary receiver like
                    // the result of a call has no variable, changing it is fine
                    let variable = match receiver {
                        Some(receiver) => root_variable(receiver),
                        None => Some("self"),
                    };
                    if let Some(variable) = variable {
                        if self.scope.get(variable).is_some_and(|(_, mutable)| !mutable) {
                            SemanticError::MutatingCallOnImmutable {
                                method: name.clone(),
                                var: variable.to_string(),
                                span: span.clone(),
                            }.throw(&self.source);
                        }
                    }
                }

                // Free functions from other modules need to be public.
                if recv_class.is_none() && matches!(visibility, Visibility::Private) && module != self.module {
                    SemanticError::PrivateFunctionCall { func_name: name.clone(), parent: module, span: span.clone() }
                        .throw(&self.source);
                }

                // Private methods can only be called from inside their own class.
                if let Some(cls) = recv_class {
                    if matches!(visibility, Visibility::Private) && self.current_class.as_ref() != Some(&cls) {
                        SemanticError::PrivateFunctionCall { func_name: name.clone(), parent: cls, span: span.clone() }
                            .throw(&self.source);
                    }
                }

                *mangled = Some(m.clone());

                let arg_defs: Vec<_> = defs.iter().filter(|d| d.name != "self").collect();
                for (i, def) in arg_defs.iter().enumerate() {
                    match params.get_mut(i) {
                        Some(arg) => {
                            if !args_checked {
                                self.check_expr_expecting(arg, &def.ty);
                            }
                            self.coerce(arg, &def.ty);
                            let arg_ty = arg.ty.clone();
                            let matches_ty = arg_ty == def.ty
                                || matches!(&def.ty, Type::Ref(inner) if **inner == arg_ty);
                            if !matches_ty {
                                SemanticError::TypeMismatch {
                                    lhs: arg_ty,
                                    rhs: def.ty.clone(),
                                    span: arg.span.clone(),
                                    context: "function argument",
                                }.throw(&self.source);
                            }
                        }
                        None if def.value.is_none() => {
                            SemanticError::MissingArgument {
                                function: m.clone(),
                                index: i,
                                span: span.clone(),
                            }.throw(&self.source);
                        }
                        None => {}
                    }
                }

                ret
            }
            ExprKind::Print(inner) => {
                let inner_ty = self.check_expr(inner);
                if inner_ty.is_move_only() {
                    SemanticError::CannotPrintUnique { span: span.clone() }.throw(&self.source);
                }
                Type::Void
            }
            ExprKind::EnumInit { .. } => self.check_struct_init(expr, expected.as_ref()),
            ExprKind::StructInit { .. } => self.check_struct_init(expr, expected.as_ref())
        };

        expr.ty = ty.clone();
        ty
    }

    /// Makes `expr` have the type `target` where that's possible without losing
    /// information.
    ///
    /// Number literals take the type directly (see [`Sema::coerce_literal`]),
    /// other values get wrapped in an [`ExprKind::Convert`] if their type
    /// [widens](Type::widens_to) to `target`. Anything else is left unchanged,
    /// so the caller's type check reports the mismatch.
    ///
    /// # Panics
    ///
    /// Panics if an integer literal doesn't fit into `target`.
    fn coerce(&mut self, expr: &mut Expr, target: &Type) {
        self.coerce_literal(expr, target);
        if !expr.ty.widens_to(target) {
            return;
        }

        let span = expr.span.clone();
        let inner = std::mem::replace(expr, Expr::new(ExprKind::IntLiteral(0), span.clone()));
        *expr = Expr { kind: ExprKind::Convert { value: Box::new(inner), target: target.clone() }, span, ty: target.clone() };
    }

    /// Gives a number literal the type `target`, if it can take it.
    ///
    /// Without context, integer literals are `int32` and float literals `float64`.
    /// Where the context expects another number type, like in `let x: uint8 = 5;`,
    /// the literal takes that type instead. This also works for expressions made
    /// only of literals, like `2 + 3`. Anything else is left unchanged, so the
    /// caller's type check reports the mismatch.
    ///
    /// # Panics
    ///
    /// Panics if an integer literal doesn't fit into `target`.
    fn coerce_literal(&mut self, expr: &mut Expr, target: &Type) {
        // Also runs when the type already matches, so the range still gets checked
        if !is_number_literal(expr) {
            return;
        }

        match (&mut expr.kind, target) {
            (ExprKind::IntLiteral(value), Type::Int { bits, signed }) => {
                if !int_fits(*value, *bits, *signed) {
                    SemanticError::LiteralOutOfRange {
                        value: *value,
                        ty: target.clone(),
                        span: expr.span.clone(),
                    }.throw(&self.source);
                }
                self.oversized_literals.remove(&(expr.span.line, expr.span.col));
            }
            (ExprKind::FloatLiteral(_), Type::Float { .. }) => {}
            (ExprKind::BinaryExpr { lhs, rhs, .. }, _) => {
                self.coerce_literal(lhs, target);
                self.coerce_literal(rhs, target);
                if lhs.ty != *target || rhs.ty != *target {
                    return;
                }
            }
            _ => return,
        }

        expr.ty = target.clone();
    }

    /// Checks the object of a field access or method call, like `a.b` in
    /// `a.b.get()`, and returns its type.
    ///
    /// Unlike using a variable as a value, this doesn't move a `Unique` out
    /// of it, the object stays where it is.
    ///
    /// # Panics
    ///
    /// Panics if the object contains an error or a variable in it got moved.
    fn check_receiver(&mut self, object: &mut Expr) -> Type {
        let ty = match &object.kind {
            ExprKind::Identifier(name) => self.use_variable(name, &object.span),
            _ => return self.check_expr(object),
        };
        object.ty = ty.clone();
        ty
    }

    /// Resolves which function a call refers to.
    ///
    /// Returns the mangled name and, for method calls like `obj.method()`, the
    /// [owner name](Type::owner_name) of the receiver's type.
    ///
    /// # Panics
    ///
    /// Panics if the receiver is unknown or its type can't have methods.
    fn resolve_call(&mut self, name: &str, receiver: &mut Option<Box<Expr>>, span: &Span) -> (String, Option<String>) {
        let Some(receiver) = receiver else {
            // Without a receiver, a call inside a class refers to one of its methods.
            let mangled = match &self.current_class {
                Some(cls) if cls != name => self.find_method(cls, name, span),
                _ => name.to_string(),
            };
            return (mangled, None);
        };

        let recv_ty = match self.check_receiver(receiver) {
            // Methods of a referenced value or one in a `Unique` are called directly on it
            Type::Ref(inner) | Type::Unique(inner) => *inner,
            other => other,
        };

        // `Buffer` methods are built in, marked like `Buffer.get` for `check_buffer_method`
        if matches!(recv_ty, Type::Buffer(_)) {
            return (format!("Buffer.{}", name), Some("Buffer".to_string()));
        }

        match recv_ty.owner_name() {
            Some(owner) => (self.find_method(&owner, name, span), Some(owner)),
            None => SemanticError::InvalidReceiver {
                recv_ty,
                span: span.clone(),
                context: "has no methods",
            }.throw(&self.source),
        }
    }

    /// Mangled name of the method `name` called on a value whose type has the
    /// [owner name](Type::owner_name) `class`.
    ///
    /// The class's own methods come first, then the methods of the traits the
    /// type implements. If there's none, the name of a missing own method is
    /// returned, so the caller reports it as unknown.
    ///
    /// # Panics
    ///
    /// Panics if more than one implemented trait has a method called `name`.
    fn find_method(&self, class: &str, name: &str, span: &Span) -> String {
        let own = format!("{}_{}", class, name);
        if self.functions.contains_key(&own) {
            return own;
        }

        let traits: Vec<&String> = self.impls.get(class).into_iter().flatten()
            .filter(|tr| self.functions.contains_key(&format!("{}_{}_{}", class, tr, name)))
            .collect();
        match traits.as_slice() {
            [] => own,
            [tr] => format!("{}_{}_{}", class, tr, name),
            _ => SemanticError::AmbiguousMethod {
                method: name.to_string(),
                class: class.to_string(),
                traits: traits.iter().map(|tr| tr.to_string()).collect(),
                span: span.clone(),
            }.throw(&self.source),
        }
    }

    /// Checks a brace initializer or enum variant construction and lowers it to
    /// a constructor call. `expected` helps inferring the type arguments of a
    /// generic class or enum.
    ///
    /// # Panics
    ///
    /// Panics if the class, enum, variant or a field is unknown, or a value has the wrong type.
    fn check_struct_init(&mut self, expr: &mut Expr, expected: Option<&Type>) -> Type {
        let span = expr.span.clone();
        let is_enum = matches!(expr.kind, ExprKind::EnumInit { .. });

        // Generic classes and enums are first replaced by their instance, like `Box<int32>`.
        // Inferring the type arguments checks the values, they mustn't be checked again.
        let mut values_checked = false;
        match &mut expr.kind {
            ExprKind::StructInit { class, type_args, fields } if self.generic_types.contains_key(class.as_str()) => {
                *class = self.instantiate_struct_init(&class.clone(), type_args, fields, expected, &span);
                values_checked = true;
            }
            ExprKind::EnumInit { name, type_args, tag, value } if self.generic_types.contains_key(name.as_str()) => {
                // A brace initializer payload only knows its class once the instance is known
                let check_value = !matches!(value.kind, ExprKind::StructInit { .. });
                *name = self.instantiate_enum_init(&name.clone(), type_args, tag, value, check_value, expected, &span);
                values_checked = check_value;
            }
            ExprKind::StructInit { class: name, type_args, .. } | ExprKind::EnumInit { name, type_args, .. }
                if !type_args.is_empty() =>
            {
                if !self.classes.contains_key(name.as_str()) && !self.enums.contains_key(name.as_str()) {
                    SemanticError::UnknownType { name: name.clone(), span: span.clone() }.throw(&self.source);
                }
                SemanticError::NotGeneric { function: name.clone(), span: span.clone() }.throw(&self.source);
            }
            _ => {}
        }

        // For enum initializers, first canonicalize the payload value:
        // if it's a struct-init, rewrite its class name to match the variant's
        // declared payload type, then check the value expression so that any
        // StructInit inside gets lowered to a constructor FunctionCall.
        if let ExprKind::EnumInit { name, tag, value, .. } = &mut expr.kind {
            let variants = self.enums.get(name).cloned()
                .unwrap_or_else(|| SemanticError::UnknownEnum {
                    name: name.clone(),
                    span: span.clone(),
                }.throw(&self.source));
            let payload_ty = variants.iter().find(|(v, _, _)| v == tag)
                .map(|(_, _, t)| t.clone())
                .unwrap_or_else(|| SemanticError::UnknownVariant {
                    enum_name: name.clone(),
                    variant: tag.clone(),
                    span: span.clone(),
                }.throw(&self.source));

            if let ExprKind::StructInit { class, .. } = &mut value.kind {
                if let Type::Class(payload_class) = &payload_ty {
                    *class = payload_class.clone();
                }
            }

            if !values_checked {
                self.check_expr_expecting(value, &payload_ty);
            }
            self.coerce(value, &payload_ty);
            let value_ty = value.ty.clone();
            if value_ty != payload_ty {
                SemanticError::TypeMismatch {
                    lhs: value_ty,
                    rhs: payload_ty,
                    span: value.span.clone(),
                    context: "enum variant payload",
                }.throw(&self.source);
            }
        }

        let (struct_name, params, constructor) = match &expr.kind {
            ExprKind::EnumInit { name, tag, value, .. } => {
                let constructor = format!("{}__{}", name, tag);
                (name.to_string(), vec![*value.clone()], constructor)
            }
            ExprKind::StructInit { class, fields, .. } => {
                let (defs, _ret, _visibility, _module) = self.functions.get(class).cloned()
                    .unwrap_or_else(|| SemanticError::UnknownFunction {
                        fn_name: class.to_string(),
                        span: span.clone(),
                    }.throw(&self.source));

                let arg_defs: Vec<Param> = defs.into_iter().filter(|d| d.name != "self").collect();

                let mut slots: Vec<Option<Expr>> = (0..arg_defs.len()).map(|_| None).collect();
                for (fname, fexpr) in fields {
                    let idx = arg_defs.iter().position(|d| d.name == *fname)
                        .unwrap_or_else(|| SemanticError::UnknownField {
                            class: class.to_string(),
                            field: fname.clone(),
                            span: fexpr.span.clone(),
                        }.throw(&self.source));
                    slots[idx] = Some(fexpr.clone());
                }

                let mut ordered: Vec<Expr> = Vec::with_capacity(arg_defs.len());
                for (i, slot) in slots.into_iter().enumerate() {
                    let (mut arg, given) = match slot {
                        Some(e) => (e, true),
                        None => match arg_defs[i].value.as_ref() {
                            Some(default) => ((**default).clone(), false),
                            None => SemanticError::MissingArgument {
                                function: class.to_string(),
                                index: i,
                                span: span.clone(),
                            }.throw(&self.source),
                        },
                    };
                    let def_ty = &arg_defs[i].ty;
                    if !(values_checked && given) {
                        self.check_expr_expecting(&mut arg, def_ty);
                    }
                    self.coerce(&mut arg, def_ty);
                    let arg_ty = arg.ty.clone();
                    let matches_ty = &arg_ty == def_ty
                        || matches!(def_ty, Type::Ref(inner) if **inner == arg_ty);
                    if !matches_ty {
                        SemanticError::TypeMismatch {
                            lhs: arg_ty,
                            rhs: def_ty.clone(),
                            span: arg.span.clone(),
                            context: "struct initializer field",
                        }.throw(&self.source);
                    }
                    ordered.push(arg);
                }
                (class.to_string(), ordered, class.to_string())
            }
            _ => unreachable!(),
        };

        let ty = if is_enum {
            Type::Enum(struct_name.clone())
        } else {
            Type::Class(struct_name.clone())
        };
        expr.kind = ExprKind::FunctionCall {
            name: struct_name.clone(),
            type_args: vec![],
            params: params,
            receiver: None,
            mangled: Some(constructor),
        };
        expr.ty = ty.clone();
        ty
    }

    fn check_decl(&mut self, decl: &mut Declaration) -> Type {
        let ty = match &mut decl.kind {
            // Generic templates are checked per instance
            DeclarationKind::Class { type_params, .. } | DeclarationKind::Enum { type_params, .. }
                if !type_params.is_empty() => Type::Void,
            DeclarationKind::Class { name, fields, methods, .. } => {
                // Field defaults are evaluated in the constructor, so they can use its parameters
                let prev_scope = self.scope.clone();
                for method in methods.iter() {
                    if let DeclarationKind::Function { name: fn_name, params, .. } = &method.kind {
                        if fn_name == name {
                            for p in params {
                                self.scope.insert(p.name.clone(), (p.ty.clone(), false));
                            }
                        }
                    }
                }
                for field in fields.iter_mut() {
                    if let Some(default) = &mut field.value {
                        self.check_default(default, &field.ty);
                    }
                }
                self.scope = prev_scope;

                let prev_class = self.current_class.take();
                self.current_class = Some(name.clone());
                for method in methods.iter_mut() {
                    self.check_decl(method);
                }
                self.current_class = prev_class;
                Type::Class(name.clone())
            }
            DeclarationKind::Enum { name, .. } => {
                Type::Enum(name.clone())
            }
            // Generic functions are templates, their instances get checked instead
            DeclarationKind::Function { type_params, .. } if !type_params.is_empty() => Type::Void,
            DeclarationKind::Function { name, params, body, return_type, .. } => {
                for p in params.iter_mut() {
                    if let Some(default) = &mut p.value {
                        self.check_default(default, &p.ty);
                    }
                }

                let ret = return_type.clone().unwrap_or(Type::Void);
                self.current_return_type = Some(ret.clone());

                let prev_scope = self.scope.clone();
                // The function's parameters start out owning their values
                let prev_moved = std::mem::take(&mut self.moved);
                if let Some(cls) = self.current_class.clone() {
                    // A constructor builds its object, so it may change it. Methods
                    // declare `self` as a parameter, which replaces this one
                    let is_ctor = *name == cls;
                    self.scope.insert("self".to_string(), (Type::Class(cls), is_ctor));
                }
                for p in params.iter() {
                    self.scope.insert(p.name.clone(), (p.ty.clone(), p.mutable));
                }
                if let Some(stmts) = body {
                    self.check_stmts(stmts);
                }
                self.scope = prev_scope;
                self.moved = prev_moved;
                self.current_return_type = None;

                ret
            }
            // Trait methods are checked through the impls they get copied into
            DeclarationKind::Trait { .. } => Type::Void,
            DeclarationKind::Impl { target, methods, .. } => {
                let prev_class = self.current_class.take();
                self.current_class = target.owner_name();
                for method in methods.iter_mut() {
                    self.check_decl(method);
                }
                self.current_class = prev_class;
                Type::Void
            }
        };

        ty
    }

    /// Checks the default value of a parameter or field against its type `ty`.
    ///
    /// # Panics
    ///
    /// Panics if the value doesn't have the type `ty`.
    fn check_default(&mut self, default: &mut Expr, ty: &Type) {
        self.check_expr_expecting(default, ty);
        self.coerce(default, ty);
        if default.ty != *ty {
            SemanticError::TypeMismatch {
                lhs: default.ty.clone(),
                rhs: ty.clone(),
                span: default.span.clone(),
                context: "default value",
            }.throw(&self.source);
        }
    }
}

/// Whether `ty` contains a type parameter like `T`, also nested like in `Box<T>`.
fn contains_type_param(ty: &Type) -> bool {
    match ty {
        Type::Param(_) => true,
        Type::Ref(inner) | Type::Unique(inner) | Type::Buffer(inner) => contains_type_param(inner),
        Type::Generic { args, .. } => args.iter().any(contains_type_param),
        _ => false,
    }
}

/// Name of the instance of the generic `name` with the type arguments in
/// `subst`, like `max<int32>` or `Box<Point,String>`.
fn instance_name(name: &str, type_params: &[TypeParam], subst: &HashMap<String, Type>) -> String {
    let args: Vec<String> = type_params.iter()
        .map(|p| {
            let ty = &subst[&p.name];
            ty.class_name().filter(|_| !matches!(ty, Type::Ref(_))).unwrap_or_else(|| ty.to_string())
        })
        .collect();
    format!("{}<{}>", name, args.join(","))
}

/// Parameters of the constructor of the class `class` among its `methods`,
/// empty if it has none.
fn constructor_params(class: &str, methods: &[Declaration]) -> Vec<Param> {
    methods.iter()
        .find_map(|m| match &m.kind {
            DeclarationKind::Function { name, params, .. } if name == class => Some(params.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The variable a place expression like `a.b.c` starts at, `a` here. `None`
/// if it starts at a temporary value like the result of a call.
fn root_variable(expr: &Expr) -> Option<&str> {
    match &expr.kind {
        ExprKind::Identifier(name) => Some(name),
        ExprKind::FieldAccess { object, .. } => root_variable(object),
        _ => None,
    }
}

/// Name of the function `decl` declares, empty for other declarations.
fn function_name(decl: &Declaration) -> &str {
    match &decl.kind {
        DeclarationKind::Function { name, .. } => name,
        _ => "",
    }
}

/// Whether the functions `a` and `b` have the same signature: both or neither
/// take `self`, both or neither change it (`mut &self`), the other parameters
/// have the same types and the return types match.
fn same_signature(a: &DeclarationKind, b: &DeclarationKind) -> bool {
    let (
        DeclarationKind::Function { params: params_a, return_type: return_a, .. },
        DeclarationKind::Function { params: params_b, return_type: return_b, .. },
    ) = (a, b) else {
        return false;
    };

    // `None` without `self`, otherwise whether `self` is mutable
    let self_kind = |params: &[Param]| params.first().filter(|p| p.name == "self").map(|p| p.mutable);
    let arg_types = |params: &[Param]| params.iter()
        .filter(|p| p.name != "self")
        .map(|p| p.ty.clone())
        .collect::<Vec<_>>();

    self_kind(params_a) == self_kind(params_b)
        && arg_types(params_a) == arg_types(params_b)
        && return_a == return_b
}

/// Whether `expr` is a number literal or an expression made only of number literals.
fn is_number_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::IntLiteral(_) | ExprKind::FloatLiteral(_) => true,
        ExprKind::BinaryExpr { lhs, rhs, .. } => is_number_literal(lhs) && is_number_literal(rhs),
        _ => false,
    }
}

/// Whether `value` fits into an integer type with `bits` bits.
fn int_fits(value: i64, bits: u32, signed: bool) -> bool {
    let value = value as i128;
    if signed {
        let max = (1i128 << (bits - 1)) - 1;
        (-max - 1..=max).contains(&value)
    } else {
        (0..=(1i128 << bits) - 1).contains(&value)
    }
}
