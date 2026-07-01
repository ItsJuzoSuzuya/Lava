use inkwell::values::BasicValueEnum;

use crate::codegen_ctx::CodegenContext;

pub trait Codegen {
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>>;
}
