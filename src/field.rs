use serde::Serialize;

use crate::{declaration::Visibility, expression::Expr, span::Span, r#type::Type};

/// A field declared in a class.
///
/// Fields have no mutability of their own: a field is mutable exactly when
/// the binding that owns the instance is.
#[derive(Debug, Serialize, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    /// Span of the type annotation, used for type error messages.
    pub ty_span: Span,
    /// Default value. If `None`, every initializer must set this field.
    pub value: Option<Box<Expr>>,
    pub visibility: Visibility,
}
