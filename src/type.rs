use std::fmt::Display;

use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType};
use serde::Serialize;

use crate::{codegen_ctx::CodegenContext, expression::Expr::{self}};

#[derive(Serialize, Debug, Clone)]
pub enum Type {
    Int32,
    String,
    Object(String)
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Int32         => write!(f, "Int32"),
            Type::String        => write!(f, "String"),
            Type::Object(name)  => write!(f, "Object({})", name),
        }
    }
}

impl<'ctx> Type {
    pub fn to_llvm(&self, ctx: &CodegenContext<'ctx>) -> BasicTypeEnum<'ctx> {
        match self {
            Type::Int32 => ctx.context.i32_type().into(),
            Type::String => ctx.context.i8_type().into(),
            Type::Object(name) => ctx.module.get_struct_type(name).expect("struct not defined").into(),
            _ => panic!() 
        }
    }

    pub fn to_fn_type(&self, param_types: &[BasicMetadataTypeEnum<'ctx>], ctx: &CodegenContext<'ctx>) -> FunctionType<'ctx> {
        match self {
            Type::Int32 => ctx.context.i32_type().fn_type(param_types, false),
            Type::String => ctx.context.i32_type().fn_type(param_types, false),
            Type::Object(name) => ctx.module.get_struct_type(name).expect("struct not defined").fn_type(param_types, false),
        }
    }
}

impl From<Expr> for Type {
    fn from(value: Expr) -> Self {
        match value {
            Expr::Int32(_) => Type::Int32,
            Expr::Identifier(ref name) => match name.as_str() {
                "int32" => Type::Int32,
                "String" => Type::String,
                name => Type::Object(name.to_string())
            }
            _ => panic!("Expr {}, isnt type applicable", value)
        }
    }
}
