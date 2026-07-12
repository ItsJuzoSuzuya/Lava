use std::fmt::Display;

use inkwell::{values::{BasicValueEnum}};
use serde::Serialize;

use crate::{codegen_ctx::CodegenContext, declaration::Declaration, expression::Expr, traits::codegen::Codegen, r#type::Type};

#[derive(Serialize, Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub value: Option<Box<Expr>>
}

pub enum Stmt {
    Expression(Expr),
    Declaration(Declaration),
    Instantiation {
        name: String,
        typedef: Type,
        value: Expr
    },
    Return(Expr),
}

impl Display for Stmt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stmt::Expression(expr)      => write!(f, "ExpressionStmt({})", expr),
            Stmt::Declaration(decl)     => write!(f, "DeclarationStmt({})", decl),
            Stmt::Instantiation { 
                name, typedef, value 
            }                           => write!(f, "Instantiation(name: {}, typename: {}, value: {})", name.to_string(), typedef, value),
            Stmt::Return(expr)          => write!(f, "Return({})", expr)
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
                decl.codegen(ctx)
            }
            Stmt::Instantiation { name, typedef, value } => {
                let ty = typedef.to_llvm(ctx);
                let ptr = ctx.builder.build_alloca(ty, &name).unwrap();
                let llvm_val = *value.codegen(ctx).unwrap();
                ctx.builder.build_store(ptr, llvm_val).unwrap();
                ctx.push_var(name.to_string(), ty, ptr);
                
                return None;
            }
            Stmt::Return(expr) => {
                let expr_val = expr.codegen(ctx).unwrap();
                ctx.builder.build_return(Some(&*expr_val)).unwrap();
                None
            }
        }
    }
}
