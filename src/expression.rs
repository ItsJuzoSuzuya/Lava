use std::fmt::{Display};

use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue};
use serde::{Serialize};

use crate::token::Token;
use crate::traits::codegen::Codegen;
use crate::{codegen_ctx::CodegenContext};

#[derive(Debug, Serialize, Clone)]
pub enum Expr {
    Print(Box<Expr>),
    Int32(i32),
    BinaryExpr {
        lhs: Box<Expr>,
        op: Token,
        rhs: Box<Expr>
    },
    FunctionCall {
        name: String,
        params: Vec<Expr>
    },
    FieldAccess {
        name: String,
        field: Box<Expr>
    },
    Identifier(String),
}

impl Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expr::Print(expr) => write!(f, "Print({})", expr),
            Expr::Int32(value) => write!(f, "Int32({})", value),
            Expr::BinaryExpr { lhs: _, op, rhs: _} => write!(f, "BinaryExpr({})", op),
            Expr::FunctionCall{ name, params: _ } => write!(f, "FunctionCall({})", name), 
            Expr::FieldAccess { name, field: _ } => write!(f, "{}", name), 
            Expr::Identifier(name) => write!(f, "{}", name), 
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
            Expr::BinaryExpr { lhs, op, rhs } => {
                match op {
                    Token::Plus => {
                        let lhs_value = lhs.codegen(ctx).unwrap().into_int_value();
                        let rhs_value = rhs.codegen(ctx).unwrap().into_int_value();
                        let sum = ctx.builder.build_int_add(lhs_value, rhs_value, "tmpadd").unwrap();
                        return Some(Box::new(sum.into()));
                    }
                    _ => panic!("unsupported binary operator: {:?}", op),
                }
            }
            Expr::FunctionCall{ name, params } => {
                let defaults: Option<Vec<Option<Expr>>> = ctx
                    .load_func_params(name)
                    .map(|ps| ps.iter().map(|p| p.value.as_deref().cloned()).collect());

                let mut llvm_params: Vec<BasicMetadataValueEnum> = Vec::new();

                match defaults {
                    Some(defaults) => {
                        for (i, default) in defaults.iter().enumerate() {
                            let value = match params.get(i) {
                                Some(passed) => *passed.codegen(ctx).unwrap(),
                                None => {
                                    let default_expr = default.as_ref().unwrap_or_else(|| {
                                        panic!("missing argument {} for function '{}' and it has no default", i, name)
                                    });
                                    *default_expr.codegen(ctx).unwrap()
                                }
                            };
                            llvm_params.push(value.into());
                        }
                    }
                    None => {
                        for param in params {
                            llvm_params.push((*param.codegen(ctx).unwrap()).into());
                        }
                    }
                }

                println!("Getting Function: {}", name);
                let function = ctx.module.get_function(name)
                    .unwrap_or_else(|| panic!("Requested function'{}' not found!", name));
                let return_val = ctx.builder.build_call(function, &llvm_params, "fn")
                    .unwrap_or_else(|_| panic!("Requested function'{}' could not be called!", name));
                return Some(Box::new(return_val.try_as_basic_value().basic()?));

            }
            Expr::FieldAccess { name, field } => {
                let (field_index, field_param) = ctx.classes[name].iter()
                    .enumerate()
                    .find(|(_, _)| *name == *field.to_string())
                    .unwrap();

                let ptr = ctx.scope[name].ptr;

                let field_ptr = ctx.builder.build_struct_gep(
                    *field_param, ptr, field_index as u32, name).unwrap();

                let field_val = ctx.builder.build_load(*field_param, field_ptr, "tmpfield").unwrap();

                return Some(Box::new(field_val));
            },
            Expr::Identifier(name) => {
                let value = ctx.load_var(name)
                    .unwrap_or_else(|| panic!("Requested variable '{}' not found in this scope!", name));
                return Some(Box::new(value));
            }
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

