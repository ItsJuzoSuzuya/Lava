use std::fmt::{Display};

use inkwell::llvm_sys::LLVMType;

use crate::{expression::Expr, statement::{Param, Stmt}};

pub enum Declaration {
    Function {
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
        return_type: Option<LLVMType>,
    }
}

impl Display for Declaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Declaration::Function { name, params: _, body: _, return_type: _ } => write!(f, "Function Declaration: func {} ", name),
        }
    }
}

