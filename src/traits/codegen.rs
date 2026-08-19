use inkwell::values::{BasicValueEnum, PointerValue};

use crate::codegen_ctx::CodegenContext;

pub trait Codegen {
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>>;

    fn codegen_place<'ctx>(&self, _ctx: &mut CodegenContext<'ctx>)
          -> Option<PointerValue<'ctx>> { None }
}

pub trait Declare {
    fn declare<'ctx>(&self, ctx: &mut CodegenContext<'ctx>);
}
