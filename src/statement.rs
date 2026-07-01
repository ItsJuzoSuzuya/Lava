use std::fmt::Display;

use inkwell::{module::Linkage, types::VoidType, values::BasicValueEnum};

use crate::{codegen_ctx::CodegenContext, declaration::Declaration, expression::Expr, traits::codegen::Codegen, r#type::Type};


pub enum Stmt {
    Expression(Expr),
    Declaration(Declaration),
    Instantiation {
        name: String,
        typedef: Type,
        value: Expr
    }
}

impl Display for Stmt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stmt::Expression(expr) => write!(f, "ExpressionStmt({})", expr),
            Stmt::Declaration(decl) => write!(f, "DeclarationStmt({})", decl),
            Stmt::Instantiation { name, typedef, value } => write!(f, "Instantiation(name: {}, typename: {}, value: {})", name.to_string(), typedef, value)
        }
    }
}

impl Codegen for Stmt {
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>> {
        match self {
            Stmt::Expression(expr) => {
                match expr.codegen(ctx) {
                    Some(expr_value) => return Some(expr_value),
                    None => return None
                };
            },
            Stmt::Declaration(decl) => {
                match decl {
                    Declaration::Function { name, params: _, body, return_type: _} => {
                        let prev_block = ctx.builder.get_insert_block()?;
                        let void_ty = ctx.context.void_type();
                        let llvm_ty = void_ty.fn_type(&[], false);
                        let function = ctx.module.add_function(name, llvm_ty, Some(Linkage::Common));
                        ctx.push_func(name.to_string());
                        let bb = ctx.context.append_basic_block(function, "tmp_fn");
                        ctx.builder.position_at_end(bb);

                        for stmt in body {
                            stmt.codegen(ctx);
                        }

                        ctx.builder.build_return(None).unwrap();
                        ctx.builder.position_at_end(prev_block);

                        return None;
                    }
                }
            }
            Stmt::Instantiation { name, typedef, value } => {
                let ty = typedef.to_llvm(ctx);
                let ptr = ctx.builder.build_alloca(ty, &name).unwrap();
                let llvm_val = *value.codegen(ctx).unwrap();
                ctx.builder.build_store(ptr, llvm_val).unwrap();
                ctx.push_var(name.to_string(), ty, ptr);
                
                return None;
            }
        }
    }
}
