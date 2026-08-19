use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Span {
    pub line: usize,
    pub col: usize,
    pub len: usize
}
