use std::collections::HashMap;

use crate::{declaration::{Declaration, DeclarationKind, Visibility}, expression::{Expr, ExprKind}, field::Field, param::Param, span::Span, statement::{MatchArm, Pattern, Stmt, StmtKind}, r#type::Type};

enum SemanticError {
    AssignOnUnmutable { var: String, span: Span },
    CannotNegate { ty: Type, span: Span },
    InvalidReceiver { recv_ty: Type, span: Span, context: &'static str },
    LiteralOutOfRange { value: i64, ty: Type, span: Span },
    MissingArgument { function: String, index: usize, span: Span },
    PrivateFieldAccess { field_name: String, span: Span },
    PrivateFunctionCall { func_name: String, parent: String, span: Span },
    TooManyArguments { func_name: String, parent: String, span: Span },
    TypeMismatch { lhs: Type, rhs: Type, span: Span, context: &'static str },
    UnknownEnum { name: String, span: Span },
    UnknownField { class: String, field: String, span: Span },
    UnknownFunction { fn_name: String, span: Span },
    UnknownIdentifier { identifier: String, span: Span },
    UnknownType { name: String, span: Span },
    UnknownVariant { enum_name: String, variant: String, span: Span },
}

impl SemanticError {
    fn throw(self, source: &str) -> ! {
        let (line, col, msg) = match self {
            SemanticError::AssignOnUnmutable{ var, span } =>
                (span.line, span.col, format!("Tried reassigning a value to the unmutable Variable '{}'", var)),
            SemanticError::CannotNegate { ty, span } =>
                (span.line, span.col, format!("cannot negate a value of type '{}'", ty)),
            SemanticError::InvalidReceiver { recv_ty, span, context } =>
                (span.line, span.col, format!("'{}' {}", recv_ty, context)),
            SemanticError::LiteralOutOfRange { value, ty, span } =>
                (span.line, span.col, format!("literal '{}' doesn't fit into '{}'", value, ty)),
            SemanticError::MissingArgument { function, index, span } =>
                (span.line, span.col, format!("missing argument {} for '{}'", index, function)),
            SemanticError::PrivateFieldAccess { field_name, span } =>
                (span.line, span.col, format!("Tried accessing a private field '{}'", field_name)),
            SemanticError::PrivateFunctionCall { func_name, parent,  span } =>
                (span.line, span.col, format!("Tried calling a private function '{}' in '{}'", func_name, parent)),
            SemanticError::TooManyArguments {  func_name, parent,  span } =>
                (span.line, span.col, format!("Too many arguments provided for function '{}' in '{}'", func_name, parent)),
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
            SemanticError::UnknownType { name, span } =>
                (span.line, span.col, format!("unknown type '{}'", name)),
            SemanticError::UnknownVariant { enum_name, variant, span } =>
                (span.line, span.col, format!("enum '{}' has no variant '{}'", enum_name, variant)),
        };
        let line_txt = source.lines().nth(line).unwrap_or("");
        let pointer = " ".repeat(col) + "^";
        panic!("\nerror: {}\n  --> {}:{}\n   |\n   | {}\n   | {}\n",
            msg, line + 1, col + 1, line_txt, pointer)
    }
}

pub struct Sema {
    scope: HashMap<String, (Type, bool)>,
    /// Params, return type, visibility and the name of the module the function is declared in.
    functions: HashMap<String, (Vec<Param>, Type, Visibility, String)>,
    classes: HashMap<String, Vec<Field>>,
    enums: HashMap<String, Vec<(String, u32, Type)>>,
    current_class: Option<String>,
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
        let mut sema = Self { scope: HashMap::new(), functions: HashMap::new(), classes: HashMap::new(), enums: HashMap::new(), current_class: None, source: String::new(), module: String::new(), current_return_type: None, oversized_literals: HashMap::new() };
        sema.push_func("sqrt".to_string(), vec![Param::builtin("x", Type::Float { bits: 32 })], Type::Float { bits: 32 }, Visibility::Public);
        sema
    }

    /// Switches to checking the module `module` with the source code `source`.
    ///
    /// Declarations of previously checked modules stay known, local variables are dropped.
    pub fn enter_module(&mut self, module: String, source: String) {
        self.module = module;
        self.source = source;
        self.scope.clear();
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
    /// [`Sema::declare_decl`] fills in their fields and variants later.
    fn declare_type_names(&mut self, ast: &[Stmt]) {
        for stmt in ast {
            if let StmtKind::Declaration(decl) = &stmt.kind {
                match &decl.kind {
                    DeclarationKind::Class { name, .. } => { self.classes.entry(name.clone()).or_default(); }
                    DeclarationKind::Enum { name, .. } => { self.enums.entry(name.clone()).or_default(); }
                    DeclarationKind::Function { .. } => {}
                }
            }
        }
    }

    fn canonicalize_type(&self, ty: &Type, span: &Span) -> Type {
        match ty {
            // The parser can't tell classes and enums apart, both come in as `Class`
            Type::Object(name) | Type::Class(name) => {
                if self.enums.contains_key(name) {
                    Type::Enum(name.clone())
                } else if self.classes.contains_key(name) {
                    Type::Class(name.clone())
                } else {
                    SemanticError::UnknownType {
                        name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source)
                }
            }
            Type::Ref(inner) => Type::Ref(Box::new(self.canonicalize_type(inner, span))),
            other => other.clone(),
        }
    }

    fn canonicalize_decl(&self, decl: &mut Declaration) {
        match &mut decl.kind {
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
                for p in params.iter_mut() {
                    let span = p.ty_span.clone();
                    p.ty = self.canonicalize_type(&p.ty, &span);
                }
                if let Some(rt) = return_type.as_mut() {
                    let span = decl.span.clone();
                    *rt = self.canonicalize_type(rt, &span);
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
            Type::Ref(inner) => self.check_type(inner, span),
            _ => {}
        }
    }

    fn validate_decl_types(&self, decl: &Declaration, span: &Span) {
        match &decl.kind {
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
        }
    }

    fn declare_decl(&mut self, decl: &mut Declaration) {
        match &mut decl.kind {
            DeclarationKind::Class { name, fields, methods } => {
                self.push_class(name.clone(), fields.clone());
                let prev_class = self.current_class.take();
                self.current_class = Some(name.clone());
                for method in methods.iter_mut() {
                    self.declare_decl(method);
                }
                self.current_class = prev_class;
            }
            DeclarationKind::Enum { name, fields } => {
                let stripped = fields.iter()
                    .map(|(n, i, t, _)| (n.clone(), *i, t.clone()))
                    .collect();
                self.push_enum(name.clone(), stripped);
            },
            DeclarationKind::Function { name, params, return_type, mangled, visibility, .. } => {
                let is_ctor = matches!(&self.current_class, Some(cls) if cls == name);
                if is_ctor {
                    *return_type = Some(Type::Class(name.clone()));
                }
                let ret = return_type.clone().unwrap_or(Type::Void);
                let m = match &self.current_class {
                    Some(cls) if cls != name => format!("{}_{}", cls, name),
                    _ => name.clone(),
                };
                self.push_func(m.clone(), params.clone(), ret, visibility.clone());
                *mangled = Some(m);
            }
        }
    }

    fn check_stmt(&mut self, stmt: &mut Stmt) {
        let stmt_span = stmt.span.clone();
        let ty = match &mut stmt.kind {
            StmtKind::Return(expr) => {
                let cur = self.current_return_type.clone().unwrap();
                self.check_expr(expr);
                self.coerce(expr, &cur);
                let ret = expr.ty.clone();
                if ret != cur {
                    SemanticError::TypeMismatch { lhs: ret, rhs: cur , span: expr.span.clone(), context: "function return type" }.throw(&self.source);
                }
                ret
            },
            StmtKind::Declaration(decl) => self.check_decl(decl),
            StmtKind::Instantiation { name, typedef, value, mutable } => {
                let value_ty = self.check_expr(value);
                let ty = match typedef {
                    Some(td) => {
                        let ty = self.canonicalize_type(td, &stmt_span);
                        self.check_type(&ty, &stmt_span);
                        self.coerce(value, &ty);
                        if (ty.is_number() || value.ty.is_number()) && value.ty != ty {
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
        };

        stmt.ty = ty.clone();
    }

    fn check_match_arms(&mut self, scrutinee_ty: &Type, arms: &mut Vec<MatchArm>, span: &Span) {
        for arm in arms.iter_mut() {
            let prev_scope = self.scope.clone();
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
            self.scope = prev_scope;
        }
        let _ = span;
    }

    fn check_expr(&mut self, expr: &mut Expr) -> Type {
        let span = expr.span.clone();

        let ty = match &mut expr.kind {
            ExprKind::Boolean(_) => Type::Bool,
            ExprKind::IntLiteral(value) => {
                // Reported at the end of the module unless the context gives it a bigger type
                if !int_fits(*value, 32, true) {
                    self.oversized_literals.insert((span.line, span.col), (*value, span.clone()));
                }
                Type::INT32
            }
            ExprKind::FloatLiteral(_) => Type::FLOAT64,
            ExprKind::StringLiteral(_) => Type::String,
            ExprKind::Identifier(name) => self.scope.get(name).cloned()
                .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                    identifier: name.clone(),
                    span: span.clone(),
                }.throw(&self.source)).0,
            ExprKind::ModuleImport { .. } => Type::Void,
            ExprKind::FieldAccess { name, field } => {
                let recv_ty = self.scope.get(name).cloned()
                    .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                        identifier: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source));

                let peeled = match &recv_ty.0 {
                    Type::Ref(inner) => (**inner).clone(),
                    other => other.clone(),
                };

                let field_name = match &field.kind {
                    ExprKind::Identifier(n) => n.clone(),
                    other => panic!("field access target must be an identifier, got '{}'", other),
                };

                if let Type::Enum(enum_name) = &peeled {
                    if field_name == "tag" {
                        field.ty = Type::INT32;
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

                field.ty = field_ty.clone();
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

                if let ExprKind::FieldAccess {name, field:_} = &lhs.kind {
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

                let lhs_ty = self.check_expr(lhs);
                self.check_expr(rhs);
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

                rhs_ty
            },
            ExprKind::Convert(inner) => {
                self.check_expr(inner);
                expr.ty.clone()
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
                let inner_ty = self.check_expr(inner);
                Type::Ref(Box::new(inner_ty))
            },
            ExprKind::BinaryExpr { lhs, rhs, op: _} => {
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

                lhs_ty
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
            ExprKind::FunctionCall { name, params, parent_name, mangled } => {
                let (m, recv_class) = self.resolve_call(name, parent_name, &span);

                let (defs, ret, visibility, module) = self.functions.get(&m).cloned()
                    .unwrap_or_else(|| SemanticError::UnknownFunction {
                        fn_name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source));

                if params.len() > defs.len(){
                    SemanticError::TooManyArguments{ func_name: name.clone(), parent: module, span: span.clone() }
                        .throw(&self.source);
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
                            self.check_expr(arg);
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
                self.check_expr(inner);
                Type::Void
            }
            ExprKind::EnumInit { .. } => self.check_struct_init(expr),
            ExprKind::StructInit { .. } => self.check_struct_init(expr)
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
        *expr = Expr { kind: ExprKind::Convert(Box::new(inner)), span, ty: target.clone() };
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

    /// Resolves which function a call refers to.
    ///
    /// Returns the mangled name and, for method calls like `obj.method()`, the
    /// class of the receiver.
    ///
    /// # Panics
    ///
    /// Panics if the receiver is unknown or isn't a class instance.
    fn resolve_call(&self, name: &str, parent_name: &Option<String>, span: &Span) -> (String, Option<String>) {
        let Some(recv) = parent_name else {
            // Without a receiver, a call inside a class refers to one of its methods.
            let mangled = match &self.current_class {
                Some(cls) if cls != name => format!("{}_{}", cls, name),
                _ => name.to_string(),
            };
            return (mangled, None);
        };

        let (recv_ty, _) = self.scope.get(recv).cloned()
            .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                identifier: recv.clone(),
                span: span.clone(),
            }.throw(&self.source));

        match recv_ty {
            Type::Class(cls) => (format!("{}_{}", cls, name), Some(cls)),
            other => {
                let context = match other {
                    Type::Enum(_) => "has no methods (enum)",
                    _             => "is not an object",
                };
                SemanticError::InvalidReceiver { recv_ty: other, span: span.clone(), context }.throw(&self.source)
            }
        }
    }

    fn check_struct_init(&mut self, expr: &mut Expr) -> Type {
        let span = expr.span.clone();
        let is_enum = matches!(expr.kind, ExprKind::EnumInit { .. });

        // For enum initializers, first canonicalize the payload value:
        // if it's a struct-init, rewrite its class name to match the variant's
        // declared payload type, then check the value expression so that any
        // StructInit inside gets lowered to a constructor FunctionCall.
        if let ExprKind::EnumInit { name, tag, value } = &mut expr.kind {
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

            self.check_expr(value);
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
            ExprKind::EnumInit { name, tag, value } => {
                let constructor = format!("{}__{}", name, tag);
                (name.to_string(), vec![*value.clone()], constructor)
            }
            ExprKind::StructInit { class, fields } => {
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
                    let mut arg = match slot {
                        Some(e) => e,
                        None => match arg_defs[i].value.as_ref() {
                            Some(default) => (**default).clone(),
                            None => SemanticError::MissingArgument {
                                function: class.to_string(),
                                index: i,
                                span: span.clone(),
                            }.throw(&self.source),
                        },
                    };
                    let def_ty = &arg_defs[i].ty;
                    self.check_expr(&mut arg);
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
            params: params,
            parent_name: None,
            mangled: Some(constructor),
        };
        expr.ty = ty.clone();
        ty
    }

    fn check_decl(&mut self, decl: &mut Declaration) -> Type {
        let ty = match &mut decl.kind {
            DeclarationKind::Class { name, fields, methods } => {
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
            DeclarationKind::Function { params, body, return_type, .. } => {
                for p in params.iter_mut() {
                    if let Some(default) = &mut p.value {
                        self.check_default(default, &p.ty);
                    }
                }

                let ret = return_type.clone().unwrap_or(Type::Void);
                self.current_return_type = Some(ret.clone());

                let prev_scope = self.scope.clone();
                if let Some(cls) = self.current_class.clone() {
                    self.scope.insert("self".to_string(), (Type::Class(cls), false));
                }
                for p in params.iter() {
                    self.scope.insert(p.name.clone(), (p.ty.clone(), false));
                }
                if let Some(stmts) = body {
                    self.check_stmts(stmts);
                }
                self.scope = prev_scope;
                self.current_return_type = None;

                ret
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
        self.check_expr(default);
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
