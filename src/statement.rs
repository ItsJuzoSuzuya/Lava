use std::fmt::Display;

use inkwell::values::BasicValueEnum;

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
    fn codegen<'ctx>(&self, ctx: &CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>> {
        match self {
            Stmt::Expression(expr) => {
                match expr.codegen(ctx) {
                    Some(expr_value) => return Some(expr_value),
                    None => return None
                };
            },
            Stmt::Declaration(_decl) => {
                return None;
            }
            Stmt::Instantiation { name, typedef, value } => {
                let ptr = ctx.builder.build_alloca(typedef.to_llvm(ctx), name).unwrap();
                let llvm_val = *value.codegen(ctx).unwrap();
                ctx.builder.build_store(ptr, llvm_val).unwrap();
                return None;
            }
        }
    }
}
