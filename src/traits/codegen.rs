use inkwell::values::BasicValueEnum;

use crate::codegen_ctx::CodegenContext;

pub trait Codegen {
    fn codegen<'ctx>(&self, ctx: &CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>>;
}
