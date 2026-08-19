use serde::Serialize;

use crate::{expression::Expr, span::Span, r#type::Type};

#[derive(Debug, Serialize, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub ty_span: Span,
    pub value: Option<Box<Expr>>,
    pub public: bool,
}
