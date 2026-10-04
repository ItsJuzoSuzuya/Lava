use serde::Serialize;

use crate::{expression::Expr, span::Span, r#type::Type};

#[derive(Debug, Serialize, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub ty_span: Span,
    pub value: Option<Box<Expr>>,
    pub mutable: bool,
}

impl Param {
    /// Creates a parameter of a built-in function, which has no position in any source file.
    pub fn builtin(name: &str, ty: Type) -> Self {
        Self { name: name.to_string(), ty, ty_span: Span { line: 0, col: 0, len: 0 }, value: None, mutable: false }
    }
}
