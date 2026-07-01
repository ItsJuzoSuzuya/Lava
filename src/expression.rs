use std::fmt::{Display};

use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::values::{BasicValueEnum, FunctionValue};
use serde::{Serialize};

use crate::traits::codegen::Codegen;
use crate::{codegen_ctx::CodegenContext};

#[derive(Debug, Serialize)]
pub enum Expr {
    Print(Box<Expr>),
    Int32(i32),
    Param {
        name: String,
        typename: String,
        default_value: Option<Box<Expr>>
    },
    FunctionCall(String),
    Identifier(String),

}

impl Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expr::Print(expr) => write!(f, "Print({})", expr),
            Expr::Int32(value) => write!(f, "Int32({})", value),
            Expr::Identifier(name) => write!(f, "Identifier({})", name), 
            Expr::FunctionCall(name) => write!(f, "FunctionCall({})", name), 
            Expr::Param { name, typename, default_value: _}  => write!(f, "Param(name: {}, type: {})", name, typename)
        }
    }
}

impl Codegen for Expr {
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>> {
        match self {
            Expr::Print(expr) => {
                let expr_value = expr.codegen(ctx).unwrap();
                let printfn = match ctx.module.get_function("printf") {
                    Some(x) => x,
                    None => create_printfn(ctx)
                };
                let format_str = ctx.builder.build_global_string_ptr("%d\n", "fmt").unwrap();
                let args = &[format_str.as_pointer_value().into(), (*expr_value).into()];

                ctx.builder.build_call(printfn, args, "printFn").unwrap();
                return None;
            },
            Expr::Int32(value) => {
                Some(Box::new(ctx.module.get_context().i32_type().const_int(*value as u64, false).into()))
            }
            Expr::FunctionCall(name) => {
                let function = ctx.module.get_function(name).unwrap();
                ctx.builder.build_call(function, &[], "fn")
                    .unwrap_or_else(|_| panic!("Requested function'{}' not found!", name));
                return None;
                
            }
            Expr::Identifier(name) => {
                let value = ctx.load_var(name)
                    .unwrap_or_else(|| panic!("Requested variable '{}' not found in this scope!", name));
                return Some(Box::new(value));
            }
            _ => None
        }
    }
}

fn create_printfn<'ctx>(ctx: &CodegenContext<'ctx>) -> FunctionValue<'ctx> {
    let context = ctx.module.get_context();
    let i32_type = context.i32_type();
    let str_ptr_type = context.ptr_type(AddressSpace::default());
    // printf has no body here - it's just a prototype so LLVM can typecheck
    // the call; the real definition is resolved against libc at link time.
    let printf_type = i32_type.fn_type(&[str_ptr_type.into()], true);

    ctx.module.add_function("printf", printf_type, Some(Linkage::External))
}

