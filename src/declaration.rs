use std::{fmt::{Display, Formatter, Result}, ops::Not};

use inkwell::{AddressSpace, types::BasicMetadataTypeEnum, values::BasicValueEnum};
use serde::Serialize;
use crate::{codegen_ctx::CodegenContext, field::Field, param::Param, span::Span, statement::Stmt, traits::codegen::{Codegen, Declare}, r#type::Type};

/// Scope of declarations and fields in the global context. 
#[derive(Debug, Clone, Serialize)]
pub enum Visibility {
    Public, Private, Protected
}

/// Type of declarations
pub enum DeclarationKind {
    Class {
        name: String,
        fields: Vec<Field>,
        methods: Vec<Declaration>,
    },
    Enum {
        name: String,
        fields: Vec<(String, u32, Type, Span)>
    },
    Function {
        name: String,
        params: Vec<Param>,
        body: Option<Vec<Stmt>>,
        return_type: Option<Type>,
        parent: Option<String>,
        mangled: Option<String>,
        visibility: Visibility 
    },
}

/// A declaration built by [`Declaration::new`], with the [`Span`] where it occurred.
pub struct Declaration {
    pub kind: DeclarationKind,
    pub span: Span,
    pub visibility: Visibility,
}

impl Declaration {
    pub fn new(kind: DeclarationKind, span: Span, visibility: Visibility) -> Self {
        Self { kind, span, visibility }
    }
}


impl Declare for Declaration {
    fn declare<'ctx>(&self, ctx: &mut CodegenContext<'ctx>) {
        match &self.kind {
            DeclarationKind::Class { name, fields, methods } => {
                let named_fields: Vec<(String, _)> = fields.iter()
                    .map(|p| (p.name.clone(), p.ty.to_llvm(ctx)))
                    .collect();
                let field_types: Vec<_> = named_fields.iter().map(|(_, t)| *t).collect();
                ctx.context.opaque_struct_type(name).set_body(&field_types, false);
                ctx.push_class(name.to_string(), named_fields);

                ctx.current_class = Some(name.clone());
                for method in methods {
                    method.declare(ctx);
                }
                ctx.current_class = None;
            }
            DeclarationKind::Enum { name, fields } => {
                let named_fields: Vec<(String, _)> = fields.iter()
                    .map(|p| (p.0.clone(), p.2.to_llvm(ctx)))
                    .collect();

                let target_data = ctx.machine.get_target_data();
                let max_payload = named_fields.iter()
                    .map(|(_, t)| target_data.get_store_size(t))
                    .max()
                    .unwrap_or(0);

                let i32_ty = ctx.context.i32_type();
                let payload_ty = ctx.context.i8_type().array_type(max_payload as u32);
                ctx.context.opaque_struct_type(name)
                    .set_body(&[i32_ty.into(), payload_ty.into()], false);

                for (variant, _, payload_ty, ty_span) in fields.iter() {
                    let mangled = format!("{}__{}", name, variant);
                    let payload_llvm = payload_ty.to_llvm(ctx);
                    let fn_type = Type::Enum(name.to_string())
                        .to_fn_type(&[payload_llvm.into()], ctx);
                    ctx.module.add_function(&mangled, fn_type, None);
                    ctx.push_func(mangled, vec![Param {
                        name: "payload".to_string(),
                        ty: payload_ty.clone(),
                        ty_span: ty_span.clone(),
                        value: None,
                        mutable: false,
                    }]);
                }

                ctx.push_enum(name.to_string(), named_fields);
            }
            DeclarationKind::Function { params, return_type, mangled, .. } => {
                let mangled = mangled.as_ref()
                    .expect("function mangled name missing — sema must run first");

                let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::new();
                param_types.extend(params.iter().map(|p| -> BasicMetadataTypeEnum {
                    if p.name == "self" {
                        ctx.context.ptr_type(AddressSpace::default()).into()
                    } else {
                        p.ty.to_llvm(ctx).into()
                    }
                }));

                let fn_type = match return_type {
                    Some(ty) => ty.to_fn_type(&param_types, ctx),
                    None     => ctx.context.void_type().fn_type(&param_types, false),
                };
                ctx.module.add_function(mangled, fn_type, None);
                ctx.push_func(mangled.clone(), params.to_vec());
            }
        }
    }
}

impl Display for DeclarationKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            DeclarationKind::Class { name, .. }    => write!(f, "Class Declaration: {}", name),
            DeclarationKind::Enum { name, .. }     => write!(f, "Emum Declaration: {}", name),
            DeclarationKind::Function { name, .. } => write!(f, "Function Declaration: func {}", name),
        }
    }
}

impl Codegen for Declaration {
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>> {
        match &self.kind {
            DeclarationKind::Class { name, fields, methods } => {
                ctx.current_class = Some(name.clone());
                ctx.current_class_fields = Some(fields.clone());
                for method in methods {
                    method.codegen(ctx);
                }
                ctx.current_class = None;
                ctx.current_class_fields = None;
                None
            }
            DeclarationKind::Enum { name, fields } => {
                let prev_block = ctx.builder.get_insert_block();
                let struct_ty = ctx.module.get_struct_type(name)
                    .expect("enum struct not defined — declare pass must run first");
                let i32_ty = ctx.context.i32_type();

                for (i, (variant, _, _, _)) in fields.iter().enumerate() {
                    let mangled = format!("{}__{}", name, variant);
                    let function = ctx.module.get_function(&mangled)
                        .expect("enum constructor missing — declare pass must run first");

                    let bb = ctx.context.append_basic_block(function, "entry");
                    ctx.builder.position_at_end(bb);

                    let payload = function.get_nth_param(0)?;
                    let local = ctx.builder.build_alloca(struct_ty, "enum_val").unwrap();

                    let tag_ptr = ctx.builder
                        .build_struct_gep(struct_ty, local, 0, "tag_ptr")
                        .unwrap();
                    let tag_val = i32_ty.const_int(i as u64, false);
                    ctx.builder.build_store(tag_ptr, tag_val).unwrap();

                    let payload_ptr = ctx.builder
                        .build_struct_gep(struct_ty, local, 1, "payload_ptr")
                        .unwrap();
                    ctx.builder.build_store(payload_ptr, payload).unwrap();

                    let loaded = ctx.builder.build_load(struct_ty, local, "ret_val").unwrap();
                    ctx.builder.build_return(Some(&loaded)).unwrap();
                }

                if let Some(prev) = prev_block {
                    ctx.builder.position_at_end(prev);
                }
                None
            }
            DeclarationKind::Function { params, body, return_type, mangled, .. } => {
                let mangled = mangled.as_ref()
                    .expect("function mangled name missing — sema must run first");
                let function = ctx.module.get_function(mangled)
                    .expect("function signature missing — declare pass must run first");

                let prev_block = ctx.builder.get_insert_block()?;
                // Params and locals of this function must not leak into the outer scope
                let prev_scope = ctx.scope.clone();
                let bb = ctx.context.append_basic_block(function, "entry");
                ctx.builder.position_at_end(bb);

                let self_class_name = ctx.current_class.clone();
                let is_ctor = self_class_name.as_ref().map_or(false, |cn| cn == mangled);


                let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
                let self_ptr = ctx.builder.build_alloca(ptr_ty, "self").unwrap();

                if is_ctor {
                    let class_name = self_class_name.clone().unwrap();
                    let struct_ty = ctx.module.get_struct_type(&class_name).unwrap();
                    let self_local = ctx.builder.build_alloca(struct_ty, "self_local").unwrap();
                    ctx.builder.build_store(self_ptr, self_local).unwrap();
                    ctx.push_var("self".to_string(), ptr_ty.into(), self_ptr, Some(class_name));
                } 

                for (i, param) in params.iter().enumerate() {
                    if ctx.func_has_self(mangled) && i == 0 {
                        let self_val = function.get_nth_param(0)?;
                        ctx.builder.build_store(self_ptr, self_val).unwrap();
                        ctx.push_var("self".to_string(), ptr_ty.into(), self_ptr, self_class_name.clone());
                        continue;
                    }

                    let ty = param.ty.to_llvm(ctx);
                    let value = function.get_nth_param(i as u32)?;
                    let ptr = ctx.builder.build_alloca(ty, &param.name).unwrap();
                    ctx.builder.build_store(ptr, value).unwrap();
                    ctx.push_var(param.name.to_string(), ty, ptr, param.ty.class_name());
                }

                if is_ctor {
                    let class_name = self_class_name.clone().unwrap();
                    let struct_ty = ctx.module.get_struct_type(&class_name).unwrap();
                    let self_ptr = ctx.load_var("self").unwrap().into_pointer_value();
                    let field_defs = ctx.current_class_fields.clone().unwrap();
                    for (i, field) in field_defs.iter().enumerate() {
                        if let Some(default_expr) = &field.value {
                            let val = *default_expr.codegen(ctx).unwrap();
                            let field_ptr = ctx.builder.
                                build_struct_gep(struct_ty, self_ptr, i as u32, "tmp_field").
                                unwrap();
                            ctx.builder.build_store(field_ptr, val).unwrap();
                        }
                    }
                }

                match body {
                    Some(stmts) => for stmt in stmts {
                        stmt.codegen(ctx);
                    }
                    None => ()
                }

                if ctx.builder.get_insert_block()
                    .and_then(|b| b.get_terminator())
                    .is_none()
                {
                    if is_ctor {
                        let class_name = self_class_name.clone().unwrap();
                        let struct_ty = ctx.module.get_struct_type(&class_name).unwrap();
                        let self_ptr = ctx.load_var("self").unwrap().into_pointer_value();
                        let loaded = ctx.builder.build_load(struct_ty, self_ptr, "ret_val").unwrap();
                        ctx.builder.build_return(Some(&loaded)).unwrap();
                    } else if return_type.is_none() {
                        ctx.builder.build_return(None).unwrap();
                    }
                }

                ctx.builder.position_at_end(prev_block);
                ctx.scope = prev_scope;
                None
            },
        }

    }
}
