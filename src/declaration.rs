use std::fmt::{Display};

use crate::{statement::{Param, Stmt}, r#type::Type};

pub enum Declaration {
    Function {
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
        return_type: Option<Type>,
    }
}

impl Display for Declaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Declaration::Function { name, params: _, body: _, return_type: _ } => write!(f, "Function Declaration: func {} ", name),
        }
    }
}

