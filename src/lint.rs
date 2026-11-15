//! Warnings about code that compiles but is likely a mistake or hard to read.
//!
//! [`lint`] runs on a checked program with its [`Index`]. Rules about names,
//! like unused variables, use the index. Rules about the shape of code, like
//! unreachable statements, walk the statements.
//!
//! A name starting with `_` is never reported as unused.

use crate::{analysis::{show_type, DefKind, Index}, expression::{Block, Expr, ExprKind}, module, program::CheckedModule, span::Span,
    statement::{Stmt, StmtKind}, declaration::DeclarationKind, token::Token, r#type::Type, sema::written_type};

/// A lint warning.
#[derive(Debug, Clone)]
pub struct Warning {
    /// Index into [`Index::files`].
    pub file: usize,
    pub span: Span,
    pub message: String,
    /// Name of the rule, like `unused_variable`.
    pub code: &'static str,
}

impl Warning {
    /// The warning as the linter prints it, with the offending line.
    pub fn render(&self, index: &Index) -> String {
        let file = &index.files[self.file];
        let line_txt: String = file.line(self.span.line).iter().collect();
        let pointer = " ".repeat(self.span.col) + &"^".repeat(self.span.len.max(1));
        format!("warning: {} [{}]\n  --> {}:{}:{}\n   |\n   | {}\n   | {}\n",
            self.message, self.code, file.path.file_name().unwrap_or_default().to_string_lossy(),
            self.span.line + 1, self.span.col + 1, line_txt, pointer)
    }
}

/// Lints every fully checked module of `modules`, sorted by file and position.
///
/// Modules that failed to check are skipped, their error comes first.
pub fn lint(index: &Index, modules: &[CheckedModule]) -> Vec<Warning> {
    // The prelude and standard library are part of the compiler, not of the program
    let user_modules = || modules.iter().filter(|m| m.checked && m.path != module::prelude_path() && !m.name.starts_with("std::"));
    let checked: Vec<usize> = user_modules().filter_map(|m| index.file_index(&m.path)).collect();

    let mut warnings = Vec::new();
    lint_names(index, &checked, &mut warnings);
    lint_imports(index, &checked, &mut warnings);
    for module in user_modules() {
        let Some(file) = index.file_index(&module.path) else { continue };
        let mut walker = CodeLints { file, warnings: &mut warnings, return_type: None };
        walker.block(&module.stmts[..module.own_stmts]);
    }

    warnings.sort_by_key(|w| (w.file, w.span.line, w.span.col));
    warnings
}

/// Unused, needlessly mutable and badly named declarations.
fn lint_names(index: &Index, files: &[usize], warnings: &mut Vec<Warning>) {
    for def in &index.defs {
        let Some(file) = def.file.filter(|f| files.contains(f)) else { continue };
        // As written, without the module's name, like `help` for `helper::help`
        let name = def.name.rsplit("::").next().unwrap_or(&def.name);
        // Names the compiler makes up, like `for$1` of a `for` loop, can't be written
        if name.contains('$') {
            continue;
        }
        let mut warn = |code, message: String| warnings.push(Warning { file, span: def.span.clone(), message, code });
        let ignored = name.starts_with('_');
        let used = def.reads > 0;

        match def.kind {
            DefKind::Variable if !ignored && !used => {
                let message = if def.writes > 0 {
                    format!("variable '{}' is assigned but never read", name)
                } else {
                    format!("variable '{}' is never used", name)
                };
                warn("unused_variable", message);
            }
            DefKind::Param if !ignored && !used && name != "self" => {
                // Trait methods have to take what the trait says, even if they don't need it
                let function = def.function.map(|f| &index.defs[f]);
                if function.is_some_and(|f| f.has_body && !f.in_trait) {
                    warn("unused_parameter", format!("parameter '{}' is never used", name));
                }
            }
            // `main` is called by the program itself
            DefKind::Function if !ignored && !used && !def.public && name != "main" => {
                warn("unused_function", format!("function '{}' is never used", name));
            }
            DefKind::Method if !ignored && !used && !def.public && !def.in_trait => {
                warn("unused_method", format!("method '{}' is never used", name));
            }
            DefKind::Field if !ignored && !used && !def.public => {
                warn("unused_field", format!("field '{}' is never read", name));
            }
            _ => {}
        }

        let can_be_mut = matches!(def.kind, DefKind::Variable | DefKind::Param);
        if can_be_mut && def.mutable && !def.unchecked && def.writes == 0 && def.mutations == 0 && !def.borrowed {
            let function = def.function.map(|f| &index.defs[f]);
            if name == "self" {
                // A trait can require `mut &self` even where an impl doesn't change anything
                if function.is_some_and(|f| !f.in_trait) {
                    warn("unused_mut", format!("'{}' doesn't change 'self', it can take '&self'",
                        function.map(|f| f.name.as_str()).unwrap_or("method")));
                }
            } else {
                warn("unused_mut", format!("'{}' is never changed, it can be declared with 'let'", name));
            }
        }

        let (expected, ok) = match def.kind {
            // An `extern func` has the C function's name
            DefKind::Function if !def.has_body => ("", true),
            DefKind::Function | DefKind::Method | DefKind::Field | DefKind::Param | DefKind::Variable =>
                ("snake_case", is_snake_case(&name)),
            DefKind::Class | DefKind::Enum | DefKind::Trait | DefKind::Variant =>
                ("PascalCase", is_pascal_case(&name)),
            _ => ("", true),
        };
        if !ok {
            let suggestion = if expected == "snake_case" { to_snake_case(&name) } else { to_pascal_case(&name) };
            warn("naming", format!("'{}' should be {}, like '{}'", name, expected, suggestion));
        }
    }
}

/// Imports of modules nothing is used from.
fn lint_imports(index: &Index, files: &[usize], warnings: &mut Vec<Warning>) {
    for &file in files {
        for (name, span) in &index.files[file].imports {
            let Some(imported) = index.files.iter().position(|f| &f.name == name) else { continue };
            let used = index.refs.iter().any(|r| {
                let def = &index.defs[r.def];
                r.file == file && def.file == Some(imported) && def.kind != DefKind::Module
            });
            if !used {
                warnings.push(Warning {
                    file,
                    span: Span { len: name.chars().count(), ..span.clone() },
                    message: format!("module '{}' is imported but nothing from it is used", name),
                    code: "unused_import",
                });
            }
        }
    }
}

/// Whether `name` is like `my_name`. Leading underscores are fine.
fn is_snake_case(name: &str) -> bool {
    name.trim_start_matches('_').chars().all(|c| c.is_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Whether `name` is like `MyName`.
fn is_pascal_case(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase) && !name.contains('_')
}

fn to_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            // A word starts after a lowercase letter, or at the last capital of an acronym like `HTTPCode`
            let prev = i.checked_sub(1).map(|p| chars[p]);
            let next = chars.get(i + 1);
            let starts_word = prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
                || prev.is_some_and(char::is_uppercase) && next.is_some_and(|n| n.is_lowercase());
            if starts_word && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn to_pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        })
        .collect()
}

/// Lints about the shape of code, found by walking the statements of one file.
struct CodeLints<'a> {
    file: usize,
    warnings: &'a mut Vec<Warning>,
    /// The return type of the function being walked, as written, like
    /// `Option<LLVMValueRef>`. `None` outside functions and for ones giving nothing.
    return_type: Option<String>,
}

impl CodeLints<'_> {
    fn warn(&mut self, span: &Span, code: &'static str, message: impl Into<String>) {
        self.warnings.push(Warning { file: self.file, span: span.clone(), message: message.into(), code });
    }

    fn block(&mut self, stmts: &[Stmt]) {
        let mut jumped_away: Option<&str> = None;
        for stmt in stmts {
            if let Some(keyword) = jumped_away {
                self.warn(&stmt.span, "unreachable_code", format!("this code never runs, it comes after a '{}'", keyword));
                break;
            }
            self.stmt(stmt);
            jumped_away = match stmt.kind {
                StmtKind::Return(_) => Some("return"),
                StmtKind::Break => Some("break"),
                StmtKind::Continue => Some("continue"),
                _ => None,
            };
        }
    }

    /// The body of a function giving `return_type`.
    fn function(&mut self, body: &[Stmt], return_type: Option<&Type>) {
        let outer = self.return_type.take();
        self.return_type = return_type.filter(|ty| **ty != Type::Void).map(|ty| written_type(&show_type(ty)));
        self.block(body);
        self.return_type = outer;
    }

    /// Warns about the values of the arms of `expr`, a match or if whose value
    /// isn't used, that the function could have returned: like `build_add(..)`
    /// in a function giving an `Option<LLVMValueRef>`, where a `return` is missing.
    fn dropped_values(&mut self, expr: &Expr) {
        let blocks: Vec<&Block> = match &expr.kind {
            ExprKind::Match { arms, .. } => arms.iter().map(|arm| &arm.block).collect(),
            ExprKind::If { then_block, else_block, .. } => std::iter::once(then_block).chain(else_block).collect(),
            _ => return,
        };
        for block in blocks {
            let Some(value) = &block.value else { continue };
            // A match giving such values as the value of an arm is reported once, as a whole
            if matches!(value.kind, ExprKind::Match { .. } | ExprKind::If { .. }) {
                if let Some(ty) = self.returnable_value(value) {
                    let what = if matches!(value.kind, ExprKind::Match { .. }) { "match" } else { "if" };
                    self.warn(&value.span, "dropped_value",
                        format!("this {} gives a '{}' that's thrown away. Did you mean `return {} ...`?", what, ty, what));
                }
                continue;
            }
            if let Some(ty) = self.returnable_value(value) {
                self.warn(&value.span, "dropped_value",
                    format!("this '{}' is thrown away, the match or if around it isn't returned. Did you mean `return`?", ty));
            }
        }
    }

    /// The type of `value`, or of the values of its arms if it's a match or if,
    /// when the function could return it. `None` otherwise.
    fn returnable_value(&self, value: &Expr) -> Option<String> {
        match &value.kind {
            ExprKind::Match { arms, .. } => arms.iter()
                .filter_map(|arm| arm.block.value.as_deref())
                .find_map(|value| self.returnable_value(value)),
            ExprKind::If { then_block, else_block, .. } => std::iter::once(then_block).chain(else_block)
                .filter_map(|block| block.value.as_deref())
                .find_map(|value| self.returnable_value(value)),
            // `lhs = expr` as an arm only stores its value
            ExprKind::Assignment { .. } => None,
            _ if matches!(value.ty, Type::Void | Type::Unknown) => None,
            _ => {
                let returned = self.return_type.as_ref()?;
                let value_ty = written_type(&show_type(&value.ty));
                let fits = *returned == value_ty
                    || *returned == format!("Option<{}>", value_ty)
                    || returned.starts_with(&format!("Result<{}, ", value_ty));
                fits.then_some(value_ty)
            }
        }
    }

    /// A match arm or branch of an `if`: its statements, then its value.
    fn value_block(&mut self, block: &Block) {
        self.block(&block.stmts);
        if let Some(value) = &block.value {
            self.expr(value);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Declaration(decl) => match &decl.kind {
                DeclarationKind::Function { body: Some(body), return_type, .. } => self.function(body, return_type.as_ref()),
                DeclarationKind::Class { methods, .. } | DeclarationKind::Enum { methods, .. }
                | DeclarationKind::Trait { methods, .. } | DeclarationKind::Impl { methods, .. } => {
                    for method in methods {
                        if let DeclarationKind::Function { body: Some(body), return_type, .. } = &method.kind {
                            self.function(body, return_type.as_ref());
                        }
                    }
                }
                _ => {}
            },
            StmtKind::Expression(expr) => {
                if has_no_effect(expr) {
                    self.warn(&expr.span, "no_effect", "this expression does nothing, its value isn't used");
                }
                if matches!(expr.kind, ExprKind::Match { .. } | ExprKind::If { .. }) {
                    self.dropped_values(expr);
                }
                self.expr(expr);
            }
            StmtKind::Return(expr) => self.expr(expr),
            StmtKind::Instantiation { value, .. } | StmtKind::Destructure { value, .. } => self.expr(value),
            StmtKind::Import(_) => {}
            StmtKind::While { condition, body } => {
                if let ExprKind::Boolean(false) = condition.kind {
                    self.warn(&condition.span, "constant_condition", "the condition is always false, the loop never runs");
                }
                self.expr(condition);
                self.block(body);
            }
            StmtKind::Break | StmtKind::Continue => {}
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::BinaryExpr { lhs, op, rhs } => {
                if op.is_comparison() && same_variable(lhs, rhs) {
                    let result = matches!(op, Token::EqEq | Token::LtEq | Token::GtEq);
                    self.warn(&lhs.span, "self_comparison", format!("comparing a value with itself is always {}", result));
                }
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Assignment { lhs, rhs } => {
                if same_variable(lhs, rhs) {
                    self.warn(&lhs.span, "self_assignment", "assigning a value to itself does nothing");
                }
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::FunctionCall { params, receiver, .. } => {
                if let Some(receiver) = receiver {
                    self.expr(receiver);
                }
                for param in params {
                    self.expr(param);
                }
            }
            ExprKind::FieldAccess { object: inner, .. } | ExprKind::Convert { value: inner, .. } | ExprKind::SliceOf(inner)
            | ExprKind::Negation(inner) | ExprKind::Unique(inner) | ExprKind::Shared(inner) | ExprKind::Deref(inner) | ExprKind::Kept(inner) | ExprKind::NewBuffer(inner)
            | ExprKind::Print(inner) | ExprKind::Ref(inner) | ExprKind::EnumInit { value: inner, .. }
            | ExprKind::Not(inner) | ExprKind::Is { value: inner, .. } => self.expr(inner),
            ExprKind::StructInit { fields, .. } => {
                for (_, value) in fields {
                    self.expr(value);
                }
            }
            ExprKind::ArrayLiteral(values) | ExprKind::Tuple(values) | ExprKind::SliceLiteral(values) => {
                for value in values {
                    self.expr(value);
                }
            }
            ExprKind::Index { object, index } => {
                self.expr(object);
                self.expr(index);
            }
            ExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms {
                    self.value_block(&arm.block);
                }
            }
            ExprKind::Ternary { condition, then_value, else_value } => {
                self.expr(condition);
                self.expr(then_value);
                self.expr(else_value);
            }
            ExprKind::If { condition, then_block, else_block } => {
                // A `for` loop is an `if true { ... }` around the loop, see `Parser::parse_for`
                let for_loop = matches!(then_block.stmts.first(),
                    Some(Stmt { kind: StmtKind::Instantiation { name, .. }, .. }) if name.starts_with("for$"));
                if let (ExprKind::Boolean(value), false) = (&condition.kind, for_loop) {
                    let value = *value;
                    let message = match (value, else_block.is_some()) {
                        (true, true) => "the condition is always true, the else branch never runs",
                        (true, false) => "the condition is always true, the 'if' isn't needed",
                        (false, _) => "the condition is always false, the if branch never runs",
                    };
                    self.warn(&condition.span, "constant_condition", message);
                }
                self.expr(condition);
                self.value_block(then_block);
                if let Some(else_block) = else_block {
                    self.value_block(else_block);
                }
            }
            _ => {}
        }
    }
}

/// Whether `lhs` and `rhs` are the same variable or field, like `a.x` and `a.x`.
fn same_variable(lhs: &Expr, rhs: &Expr) -> bool {
    match (&lhs.kind, &rhs.kind) {
        (ExprKind::Identifier(a), ExprKind::Identifier(b)) => a == b,
        (ExprKind::FieldAccess { object: a, field: fa }, ExprKind::FieldAccess { object: b, field: fb }) =>
            fa == fb && same_variable(a, b),
        _ => false,
    }
}

/// Whether evaluating `expr` as a statement can't have any effect.
fn has_no_effect(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Identifier(_) | ExprKind::IntLiteral(_) | ExprKind::FloatLiteral(_) | ExprKind::Boolean(_)
        | ExprKind::StringLiteral(_) | ExprKind::ColonAccess { .. } => true,
        ExprKind::BinaryExpr { lhs, rhs, .. } | ExprKind::Index { object: lhs, index: rhs } => has_no_effect(lhs) && has_no_effect(rhs),
        ExprKind::ArrayLiteral(values) | ExprKind::Tuple(values) => values.iter().all(has_no_effect),
        ExprKind::FieldAccess { object, .. } | ExprKind::Negation(object) | ExprKind::Convert { value: object, .. } =>
            has_no_effect(object),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_checks() {
        assert!(is_snake_case("my_var"));
        assert!(is_snake_case("_unused"));
        assert!(is_snake_case("x2"));
        assert!(!is_snake_case("myVar"));
        assert!(is_pascal_case("Point"));
        assert!(!is_pascal_case("point"));
        assert!(!is_pascal_case("My_Point"));
    }

    #[test]
    fn case_suggestions() {
        assert_eq!(to_snake_case("myVar"), "my_var");
        assert_eq!(to_snake_case("HTTPCode"), "http_code");
        assert_eq!(to_snake_case("Point"), "point");
        assert_eq!(to_pascal_case("my_point"), "MyPoint");
        assert_eq!(to_pascal_case("point"), "Point");
    }
}
