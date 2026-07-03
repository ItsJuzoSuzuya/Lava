use std::fmt::Display;

use inkwell::types::BasicTypeEnum;
use serde::Serialize;

use crate::{codegen_ctx::CodegenContext, expression::Expr::{self}};

#[derive(Serialize, Debug, Clone)]
pub enum Type {
    Int32,
    String,
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Int32 => write!(f, "Int32"),
            Type::String => write!(f, "String"),
        }
    }
}

impl<'ctx> Type {
    pub fn to_llvm(&self, ctx: &CodegenContext<'ctx>) -> BasicTypeEnum<'ctx> {
        match self {
            Type::Int32 => ctx.context.i32_type().into(),
            _ => panic!() 
        }

    }
}

impl From<Expr> for Type {
    fn from(value: Expr) -> Self {
        match value {
            Expr::Int32(_) => Type::Int32,
            Expr::Identifier(ref name) => match name.as_str() {
                "int32" => Type::Int32,
                "string" => Type::String,
                _ => panic!("Expr {}, isnt type applicable", value)

            }
            _ => panic!("Expr {}, isnt type applicable", value)
        }
    }
}
