use std::fmt::Display;

use inkwell::types::BasicTypeEnum;

use crate::{codegen_ctx::CodegenContext};

pub enum Type {
    Int32,
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Int32 => write!(f, "Int32"),
        }
    }
}

impl<'ctx> Type {
    pub fn to_llvm(&self, ctx: &CodegenContext<'ctx>) -> BasicTypeEnum<'ctx> {
        match self {
            Type::Int32 => ctx.context.i32_type().into()
        }

    }
}
