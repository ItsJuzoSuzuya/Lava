use std::fmt::{Display};

use inkwell::{module::Linkage, types::{BasicMetadataTypeEnum, BasicTypeEnum}};

use crate::{statement::{Param, Stmt}, traits::codegen::Codegen, r#type::Type};

pub enum Declaration {
    Class {
        name: String,
        fields: Vec<Param>,
        methods: Vec<Stmt>
    },
    Function {
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
        return_type: Option<Type>,
    },
}

impl Display for Declaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Declaration::Class { name, fields: _, methods: _ } => write!(f, "Class Declaration: {} ", name),
            Declaration::Function { name, params: _, body: _, return_type: _ } => write!(f, "Function Declaration: func {} ", name),
        }
    }
}

impl Codegen for Declaration {
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut crate::codegen_ctx::CodegenContext<'ctx>) -> Option<Box<inkwell::values::BasicValueEnum<'ctx>>> {
        match self {
            Declaration::Class { name, fields, methods } => { 
                let field_types: Vec<BasicTypeEnum> = fields.iter().map(|stmt| stmt.ty.to_llvm(ctx)).collect();
                for method in methods { method.codegen(ctx); }

                ctx.context.opaque_struct_type(name).set_body(&field_types, false);
                ctx.push_class(name.to_string(), field_types);
                None 
            },
            Declaration::Function { name, params, body, return_type } => {
                let prev_block = ctx.builder.get_insert_block()?;
                let param_types: Vec<BasicMetadataTypeEnum> = params.iter().map(|p| p.ty.to_llvm(ctx).into()).collect();
                let fn_type = match return_type {
                    Some(ty) => ty.to_fn_type(&param_types, ctx),
                    None     => ctx.context.void_type().fn_type(&param_types, false),
                };
                let function = ctx.module.add_function(name, fn_type, Some(Linkage::Common));
                ctx.push_func(name.to_string(), params.to_vec());
                let bb = ctx.context.append_basic_block(function, "tmp_fn");
                ctx.builder.position_at_end(bb);

                for (i, param) in params.iter().enumerate() {
                    let ty = param.ty.to_llvm(ctx);
                    let value = function.get_nth_param(i as u32)?;
                    let ptr = ctx.builder.build_alloca(ty, &param.name).unwrap();
                    ctx.builder.build_store(ptr, value).unwrap();
                    ctx.push_var(param.name.to_string(), ty, ptr);
                }

                for stmt in body {
                    stmt.codegen(ctx);
                }

                ctx.builder.position_at_end(prev_block);
                println!("Function {}() just build!", name);

                return None;
            }
        }
    }
}


