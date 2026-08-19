use std::collections::HashMap;

use crate::{declaration::{Declaration, DeclarationKind}, expression::{Expr, ExprKind}, field::Field, param::Param, span::Span, statement::{MatchArm, Pattern, Stmt, StmtKind}, r#type::Type};

enum SemanticError {
    AssignOnUnmutable { var: String, span: Span },
    InvalidReceiver { recv_ty: Type, span: Span, context: &'static str },
    MissingArgument { function: String, index: usize, span: Span },
    PrivateFieldAccess { field_name: String, span: Span },
    PrivateFunctionCall { func_name: String, parent: String, span: Span },
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
            SemanticError::InvalidReceiver { recv_ty, span, context } =>
                (span.line, span.col, format!("'{}' {}", recv_ty, context)),
            SemanticError::MissingArgument { function, index, span } =>
                (span.line, span.col, format!("missing argument {} for '{}'", index, function)),
            SemanticError::PrivateFieldAccess { field_name, span } =>
                (span.line, span.col, format!("Tried accessing a private field '{}'", field_name)),
            SemanticError::PrivateFunctionCall { func_name, parent,  span } =>
                (span.line, span.col, format!("Tried calling a private function '{}' in '{}'", func_name, parent)),
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
    functions: HashMap<String, (Vec<Param>, Type, bool)>,
    classes: HashMap<String, Vec<Field>>,
    enums: HashMap<String, Vec<(String, u32, Type)>>,
    current_class: Option<String>,
    current_return_type: Option<Type>,
    source: String
}

impl Sema {
    pub fn new(source: String) -> Self {
        Self { scope: HashMap::new(), functions: HashMap::new(), classes: HashMap::new(), enums: HashMap::new(), current_class: None, source, current_return_type: None }
    }

    pub fn push_class(&mut self, name: String, fields: Vec<Field>) {
        self.classes.insert(name, fields);
    }

    pub fn push_enum(&mut self, name: String, fields: Vec<(String, u32, Type)>) {
        self.enums.insert(name, fields);
    }

    pub fn push_func(&mut self, name: String, params: Vec<Param>, return_type: Type, public: bool) {
        self.functions.insert(name, (params, return_type, public));
    }


    pub fn check(&mut self, ast: &mut Vec<Stmt>) {
        for stmt in ast.iter_mut() {
            if let StmtKind::Declaration(decl) = &mut stmt.kind {
                self.declare_decl(decl);
            }
        }
        for stmt in ast.iter_mut() {
            if let StmtKind::Declaration(decl) = &mut stmt.kind {
                self.canonicalize_decl(decl);
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

    fn canonicalize_type(&self, ty: &Type, span: &Span) -> Type {
        match ty {
            Type::Object(name) => {
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
            DeclarationKind::Function { name, params, return_type, mangled, public, .. } => {
                let is_ctor = matches!(&self.current_class, Some(cls) if cls == name);
                if is_ctor {
                    *return_type = Some(Type::Class(name.clone()));
                }
                let ret = return_type.clone().unwrap_or(Type::Void);
                let m = match &self.current_class {
                    Some(cls) if cls != name => format!("{}_{}", cls, name),
                    _ => name.clone(),
                };
                self.push_func(m.clone(), params.clone(), ret, *public);
                *mangled = Some(m);
            }
        }
    }

    fn check_stmt(&mut self, stmt: &mut Stmt) {
        let stmt_span = stmt.span.clone();
        let ty = match &mut stmt.kind {
            StmtKind::Return(expr) => {
                let cur = self.current_return_type.clone().unwrap();
                let ret = self.check_expr(expr);
                if ret != cur {
                    SemanticError::TypeMismatch { lhs: ret, rhs: cur , span: expr.span.clone(), context: "function return type" }.throw(&self.source);
                }
                ret
            },
            StmtKind::Declaration(decl) => self.check_decl(decl),
            StmtKind::Instantiation { name, typedef, value, mutable } => {
                let value_ty = self.check_expr(value);
                let ty = match typedef {
                    Some(td) => self.canonicalize_type(td, &stmt_span),
                    None => value_ty,
                };
                self.check_type(&ty, &stmt_span);
                self.scope.insert(name.clone(), (ty.clone(), *mutable));
                *typedef = Some(ty.clone());
                ty
            }
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
            match &arm.pattern {
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
                    self.scope.insert(arm.pattern.to_string(), (scrutinee_ty.clone(), false));
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
            ExprKind::Boolean(val) => Type::Bool(*val),
            ExprKind::Int32(_) => Type::Int32,
            ExprKind::StringLiteral(_) => Type::String,
            ExprKind::Identifier(name) => self.scope.get(name).cloned()
                .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                    identifier: name.clone(),
                    span: span.clone(),
                }.throw(&self.source)).0,
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
                        field.ty = Type::Int32;
                        expr.ty = Type::Int32;
                        return Type::Int32;
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

                let is_public = self.classes.get(&class_name).unwrap()
                    .iter().find(|field| *field.name == field_name).unwrap().public;
                let inside_self = match self.current_class.clone() {
                    Some(name) => {
                        class_name == name 
                    }
                    None => false,
                };
                
                if !is_public && !inside_self {
                    SemanticError::PrivateFieldAccess{ field_name, span  }.throw(&self.source);
                }

                let field_ty = self.classes.get(&class_name)
                    .and_then(|fields| fields.iter().find(|p| p.name == field_name))
                    .map(|p| p.ty.clone())
                    .unwrap_or_else(|| SemanticError::UnknownField {
                        class: class_name.clone(),
                        field: field_name.clone(),
                        span: span.clone(),
                    }.throw(&self.source));

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
                let rhs_ty = self.check_expr(rhs);
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
            ExprKind::Ref(inner) => {
                if !matches!(inner.kind, ExprKind::Identifier(_) | ExprKind::FieldAccess { .. }) {
                    panic!("'&' operand must be a place expression, got '{}'", inner.kind);
                }
                let inner_ty = self.check_expr(inner);
                Type::Ref(Box::new(inner_ty))
            },
            ExprKind::BinaryExpr { lhs, rhs, op: _} => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
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
                let mut is_self = false;
                let m = match parent_name {
                    Some(recv) => {
                        let (recv_ty, _is_mut) = self.scope.get(recv).cloned()
                            .unwrap_or_else(|| SemanticError::UnknownIdentifier {
                                identifier: recv.clone(),
                                span: span.clone(),
                            }.throw(&self.source));
                        if let Some(class) = self.current_class.clone() {
                            if recv_ty == Type::Class(class) {
                                is_self = true;
                            }
                        }
                        match recv_ty {
                            Type::Class(cls) => format!("{}_{}", cls, name),
                            other @ Type::Enum(_) => SemanticError::InvalidReceiver {
                                recv_ty: other,
                                span: span.clone(),
                                context: "has no methods (enum)",
                            }.throw(&self.source),
                            other => SemanticError::InvalidReceiver {
                                recv_ty: other,
                                span: span.clone(),
                                context: "is not an object",
                            }.throw(&self.source),
                        }
                    }
                    None => match &self.current_class {
                        Some(cls) if cls != name => format!("{}_{}", cls, name),
                        _ => name.clone(),
                    }
                };

                let (defs, ret, public) = self.functions.get(&m).cloned()
                    .unwrap_or_else(|| SemanticError::UnknownFunction {
                        fn_name: name.clone(),
                        span: span.clone(),
                    }.throw(&self.source));

                if !public && !is_self {
                    let class_name = self.scope.get(&parent_name.clone().unwrap()).unwrap().0.clone();
                    SemanticError::PrivateFunctionCall { func_name: name.to_string(), parent: class_name.to_string(), span: span }.throw(&self.source);
                }

                *mangled = Some(m.clone());

                let arg_defs: Vec<_> = defs.iter().filter(|d| d.name != "self").collect();
                for (i, def) in arg_defs.iter().enumerate() {
                    match params.get_mut(i) {
                        Some(arg) => {
                            let arg_ty = self.check_expr(arg);
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

            let value_ty = self.check_expr(value);
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
                let (defs, _ret, _public) = self.functions.get(class).cloned()
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
                    let arg_ty = self.check_expr(&mut arg);
                    let def_ty = &arg_defs[i].ty;
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
            DeclarationKind::Class { name, methods, .. } => {
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
                    self.check(stmts);
                }
                self.scope = prev_scope;
                self.current_return_type = None;

                ret
            }
        };

        decl.ty = ty.clone();
        ty
    }
}
