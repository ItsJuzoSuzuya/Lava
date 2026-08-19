use std::fmt::Display;

use inkwell::AddressSpace;
use inkwell::basic_block::BasicBlock;
use inkwell::module::Linkage;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use serde::Serialize;
use serde::de::value;

use crate::codegen_ctx::{CodegenError, CodegenContext};
use crate::span::Span;
use crate::token::Token;
use crate::traits::codegen::Codegen;
use crate::r#type::Type;

#[derive(Debug, Serialize, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub ty: Type,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span, ty: Type::Unknown }
    }
}

#[derive(Debug, Serialize, Clone)]
pub enum ExprKind {
    Boolean(bool),
    Identifier(String),
    Int32(i32),
    Print(Box<Expr>),
    Ref(Box<Expr>),
    StringLiteral(String),

    Assignment {
        lhs: Box<Expr>,
        rhs: Box<Expr>
    },
    BinaryExpr {
        lhs: Box<Expr>,
        op: Token,
        rhs: Box<Expr>,
    },
    ColonAccess {
        name: String,
        tag: String,
    },
    FieldAccess {
        name: String,
        field: Box<Expr>,
    },
    FunctionCall {
        name: String,
        params: Vec<Expr>,
        parent_name: Option<String>,
        mangled: Option<String>,
    },

    EnumInit { 
        name: String, 
        tag: String,
        value: Box<Expr>
    },             
    StructInit {
        class: String,
        fields: Vec<(String, Expr)>,
    },
}

impl Display for ExprKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExprKind::Boolean(val)                     => write!(f, "{}", val),
            ExprKind::Identifier(name)                 => write!(f, "{}", name),
            ExprKind::Int32(value)                     => write!(f, "{}", value),
            ExprKind::Print(expr)                      => write!(f, "Print({})", expr.kind),
            ExprKind::Ref(inner)                       => write!(f, "&{}", inner.kind),
            ExprKind::StringLiteral(lit)               => write!(f, "\"{}\"", lit),

            ExprKind::Assignment { lhs, rhs }          => write!(f, "Assignment({} = {})", lhs.kind, rhs.kind),
            ExprKind::BinaryExpr { lhs, op, rhs }      => write!(f, "{} {} {}", lhs.kind, op, rhs.kind),
            ExprKind::ColonAccess { name, tag }        => write!(f, "{}::{}", name, tag),
            ExprKind::FieldAccess { name, field }      => write!(f, "{}.{}", name, field.kind),
            ExprKind::FunctionCall { name, .. }        => write!(f, "{}()", name),
            ExprKind::EnumInit { name, .. }             => write!(f, "{}(..)", name),
            ExprKind::StructInit { class, .. }         => write!(f, "{} {{ .. }}", class),
        }
    }
}

impl Codegen for Expr {
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>> {
        let span = &self.span;
        match &self.kind {
            ExprKind::Boolean(val) => {
                let int = match val {
                    true => 1,
                    false => 0
                };
                Some(Box::new(ctx.context.bool_type().const_int(int, false).into()))
            }
            ExprKind::Identifier(name) => {
                match ctx.load_var(name) {
                    Some(v) => Some(Box::new(v)),
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedVariable(name.clone()).throw(&source, span)
                    }
                }
            }
            ExprKind::Int32(value) => {
                Some(Box::new(ctx.context.i32_type().const_int(*value as u64, false).into()))
            }
            ExprKind::Print(expr) => {
                let printfn = match ctx.module.get_function("printf") {
                    Some(x) => x,
                    None => create_printfn(ctx),
                };

                match &expr.ty {
                    Type::Bool(val) => {

                        let chosen = match val {
                            true => {
                                let t = ctx.builder.build_global_string_ptr("true", "true_str").unwrap();
                                t.as_pointer_value()
                            }
                            false => {
                                let f = ctx.builder.build_global_string_ptr("false", "false_str").unwrap();
                                f.as_pointer_value()
                            }
                        };
                        
                        let fmt = ctx.builder.build_global_string_ptr("%s\n", "fmt").unwrap();
                        ctx.builder.build_call(
                            printfn,
                            &[fmt.as_pointer_value().into(), chosen.into()],
                            "printFn",
                        ).unwrap();
                    }
                    Type::Enum(name) => {
                        let printer_name = format!("__print_{}", name);
                        let printer = match ctx.module.get_function(&printer_name) {
                            Some(f) => f,
                            None => create_enum_printer(ctx, name, printfn),
                        };
                        let ptr = match expr.codegen_place(ctx) {
                            Some(p) => p,
                            None => {
                                let val = *expr.codegen(ctx).unwrap();
                                let ty = expr.ty.to_llvm(ctx);
                                let tmp = ctx.builder.build_alloca(ty, "tmp_enum").unwrap();
                                ctx.builder.build_store(tmp, val).unwrap();
                                tmp
                            }
                        };
                        ctx.builder.build_call(printer, &[ptr.into()], "").unwrap();
                    }
                    _ => {
                        let expr_value = expr.codegen(ctx).unwrap();
                        let formatter = expr.ty.formatter().unwrap_or_else(|| CodegenError::UndefinedFormatter(expr.ty.clone()).throw(&ctx.source, span));
                        let format_str = ctx.builder.build_global_string_ptr((formatter.to_owned() + &"\n").as_str(), "fmt").unwrap();
                        let args = &[format_str.as_pointer_value().into(), (*expr_value).into()];
                        ctx.builder.build_call(printfn, args, "printFn").unwrap();
                    }
                }

                None
            }
            ExprKind::Ref(inner) => {
                let ptr = inner.codegen_place(ctx)
                    .unwrap_or_else(|| panic!("'&' operand is not a place expression"));
                Some(Box::new(ptr.into()))
            },
            ExprKind::StringLiteral(lit) => {
                let string = ctx.builder.build_global_string_ptr(lit, "str").unwrap();
                Some(Box::new(string.as_pointer_value().into()))
            },

            ExprKind::Assignment { lhs, rhs } => {
                let ptr = lhs.codegen_place(ctx)
                    .unwrap_or_else(|| panic!("assignment lhs is not a place expression"));
                let llvm_val = *rhs.codegen(ctx).unwrap();
                ctx.builder.build_store(ptr, llvm_val).unwrap();

                None
            },
            ExprKind::BinaryExpr { lhs, op, rhs } => {
                match op {
                    Token::Plus => {
                        let lhs_value = lhs.codegen(ctx).unwrap().into_int_value();
                        let rhs_value = rhs.codegen(ctx).unwrap().into_int_value();
                        let sum = ctx.builder.build_int_add(lhs_value, rhs_value, "tmpadd").unwrap();
                        Some(Box::new(sum.into()))
                    }
                    Token::Minus => {
                        let lhs_value = lhs.codegen(ctx).unwrap().into_int_value();
                        let rhs_value = rhs.codegen(ctx).unwrap().into_int_value();
                        let dif = ctx.builder.build_int_add(lhs_value, rhs_value.const_neg(), "tmpadd").unwrap();
                        Some(Box::new(dif.into()))

                    }
                    other => {
                        let source = ctx.source.clone();
                        CodegenError::UnsupportedOperator(other.to_string()).throw(&source, span)
                    }
                }
            }
            ExprKind::ColonAccess { name, tag } => {
                let variants = match ctx.enums.get(name) {
                    Some(v) => v,
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedClass(name.clone()).throw(&source, span)
                    }
                };
                let discriminant = variants.iter()
                    .position(|(n, _)| *n == tag.to_string())
                    .unwrap_or_else(|| {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedField {
                            class: name.clone(),
                            field: tag.to_string(),
                        }.throw(&source, span)
                    });
                let value = ctx.context.i32_type().const_int(discriminant as u64, false);
                Some(Box::new(value.into()))
            },
            ExprKind::FieldAccess { name, field } => {
                let (recv_ty, recv_ptr, class_name) = {
                    let sym = match ctx.scope.get(name) {
                        Some(s) => s,
                        None => {
                            let source = ctx.source.clone();
                            CodegenError::UndefinedVariable(name.clone()).throw(&source, span)
                        }
                    };
                    let class_name = match &sym.class_name {
                        Some(c) => c.clone(),
                        None => panic!("field access on non-class variable '{}'", name),
                    };
                    (sym.ty, sym.ptr, class_name)
                };
                let field_name = field.kind.to_string();
                let struct_ty = ctx.module.get_struct_type(&class_name).expect("struct not defined");
                let obj_ptr = if recv_ty.is_pointer_type() {
                    ctx.builder.build_load(recv_ty, recv_ptr, "recv_load")
                        .unwrap()
                        .into_pointer_value()
                } else {
                    recv_ptr
                };

                if ctx.enums.contains_key(&class_name) {
                    if field_name != "tag" {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedField {
                            class: class_name.clone(),
                            field: field_name,
                        }.throw(&source, span)
                    }
                    let tag_ptr = ctx.builder.build_struct_gep(struct_ty, obj_ptr, 0, "tag_ptr").unwrap();
                    let tag_val = ctx.builder.build_load(ctx.context.i32_type(), tag_ptr, "tag").unwrap();
                    return Some(Box::new(tag_val));
                }

                let class_fields = match ctx.classes.get(&class_name) {
                    Some(f) => f.clone(),
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedClass(class_name.clone()).throw(&source, span)
                    }
                };
                let (field_index, field_type) = match class_fields.iter()
                    .enumerate()
                    .find(|(_, (n, _))| n == &field_name)
                {
                    Some((i, (_, t))) => (i, *t),
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedField {
                            class: class_name.clone(),
                            field: field_name,
                        }.throw(&source, span)
                    }
                };
                let field_ptr = ctx.builder.build_struct_gep(struct_ty, obj_ptr, field_index as u32, &field_name).unwrap();
                let field_val = ctx.builder.build_load(field_type, field_ptr, "tmpfield").unwrap();
                Some(Box::new(field_val))
            }
            ExprKind::FunctionCall { name: _, params, parent_name, mangled } => {
                let mangled = mangled.as_ref()
                    .expect("function call mangled name missing — sema must run first");

                // Get Parent — only when the target function actually takes self.
                let target_has_self = ctx.func_has_self(mangled);
                let recv_name = if target_has_self {
                    match parent_name {
                        Some(name) => Some(name.as_str()),
                        None => match &ctx.current_class {
                            Some(_) => Some("self"),
                            None => None,
                        }
                    }
                } else {
                    None
                };
                let parent_sym = recv_name.map(|name| {
                    let (ty, ptr) = match ctx.scope.get(name) {
                        Some(s) => (s.ty, s.ptr),
                        None => {
                            let source = ctx.source.clone();
                            CodegenError::UndefinedVariable(name.to_string()).throw(&source, span)
                        }
                    };
                    if ty.is_pointer_type() {
                        ctx.builder.build_load(ty, ptr, "recv").unwrap().into_pointer_value()
                    } else {
                        ptr
                    }
                });
                // Put all Params into the function (parent is always first parent)
                let mut llvm_params: Vec<BasicMetadataValueEnum> = match parent_sym {
                    Some(sym) => vec![sym.into()],
                    None => Vec::new()
                };

                let decl_info: Option<Vec<(String, Type, Option<Expr>)>> = ctx
                    .load_func_params(mangled)
                    .map(|ps| ps.iter()
                        .map(|p| (p.name.clone(), p.ty.clone(), p.value.as_ref().map(|v| *v.clone())))
                        .collect());

                match decl_info {
                    Some(infos) => {
                        let arg_infos: Vec<_> = infos.iter().filter(|(n, _, _)| n != "self").collect();
                        for (i, (_, def_ty, default)) in arg_infos.iter().enumerate() {
                            let value: BasicValueEnum = match params.get(i) {
                                Some(passed) => {
                                    let needs_ref = matches!(def_ty, Type::Ref(_))
                                        && !matches!(passed.ty, Type::Ref(_));
                                    if needs_ref {
                                        let ptr = passed.codegen_place(ctx).unwrap_or_else(|| {
                                            panic!("cannot take address of non-place expression for arg {}", i)
                                        });
                                        ptr.into()
                                    } else {
                                        *passed.codegen(ctx).unwrap()
                                    }
                                },
                                None => {
                                    let default_expr = default.as_ref().unwrap_or_else(|| {
                                        let source = ctx.source.clone();
                                        CodegenError::MissingArgument { index: i, function: mangled.clone() }
                                            .throw(&source, span)
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

                let function = match ctx.module.get_function(mangled) {
                    Some(f) => f,
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedFunction(mangled.clone()).throw(&source, span)
                    }
                };

                let return_val = ctx.builder.build_call(function, &llvm_params, "fn")
                    .unwrap_or_else(|_| panic!("function '{}' could not be called", mangled));
                Some(Box::new(return_val.try_as_basic_value().basic()?))
            }
            ExprKind::EnumInit { .. } => {
                unreachable!("EnumInit must be lowered to FunctionCall by sema before codegen")
            }
            ExprKind::StructInit { .. } => {
                unreachable!("StructInit must be lowered to FunctionCall by sema before codegen")
            }
        }
    }

    fn codegen_place<'ctx>(&self, ctx: &mut CodegenContext<'ctx>)
              -> Option<PointerValue<'ctx>> {
        let span = &self.span;
        match &self.kind {
            ExprKind::Identifier(name) => {
                match ctx.load_var_ptr(name) {
                    Some(v) => Some(v),
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedVariable(name.clone()).throw(&source, span)
                    }
                }
            }
            ExprKind::FieldAccess { name, field } => {
                let (recv_ty, recv_ptr, class_name) = {
                    let sym = match ctx.scope.get(name) {
                        Some(s) => s,
                        None => {
                            let source = ctx.source.clone();
                            CodegenError::UndefinedVariable(name.clone()).throw(&source, span)
                        }
                    };
                    let class_name = match &sym.class_name {
                        Some(c) => c.clone(),
                        None => panic!("field access on non-class variable '{}'", name),
                    };
                    (sym.ty, sym.ptr, class_name)
                };
                let class_fields = match ctx.classes.get(&class_name) {
                    Some(f) => f.clone(),
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedClass(class_name.clone()).throw(&source, span)
                    }
                };
                let field_name = field.kind.to_string();
                let field_index = match class_fields.iter()
                    .enumerate()
                    .find(|(_, (n, _))| n == &field_name)
                {
                    Some((i, _)) => i,
                    None => {
                        let source = ctx.source.clone();
                        CodegenError::UndefinedField {
                            class: class_name.clone(),
                            field: field_name,
                        }.throw(&source, span)
                    }
                };
                let struct_ty = ctx.module.get_struct_type(&class_name).expect("struct not defined");
                let obj_ptr = if recv_ty.is_pointer_type() {
                    ctx.builder.build_load(recv_ty, recv_ptr, "recv_load")
                        .unwrap()
                        .into_pointer_value()
                } else {
                    recv_ptr
                };
                let field_ptr = ctx.builder.build_struct_gep(struct_ty, obj_ptr, field_index as u32, &field_name).unwrap();
                Some(field_ptr)
            }
            _ => None,
        }
    }
}

fn create_printfn<'ctx>(ctx: &CodegenContext<'ctx>) -> FunctionValue<'ctx> {
    let context = ctx.context;
    let i32_type = context.i32_type();
    let str_ptr_type = context.ptr_type(AddressSpace::default());
    // printf has no body here - it's just a prototype so LLVM can typecheck
    // the call; the real definition is resolved against libc at link time.
    let printf_type = i32_type.fn_type(&[str_ptr_type.into()], true);
    ctx.module.add_function("printf", printf_type, Some(Linkage::External))
}

fn printf_format_for(ty: &BasicTypeEnum<'_>) -> &'static str {
    match ty {
        BasicTypeEnum::IntType(_)     => "%d",
        BasicTypeEnum::PointerType(_) => "%s",
        BasicTypeEnum::FloatType(_)   => "%f",
        _ => "%d",
    }
}

fn create_enum_printer<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    enum_name: &str,
    printfn: FunctionValue<'ctx>,
) -> FunctionValue<'ctx> {
    let prev_block = ctx.builder.get_insert_block();

    let struct_ty = ctx.module.get_struct_type(enum_name)
        .expect("enum struct not defined");
    let variants: Vec<(String, BasicTypeEnum<'ctx>)> = ctx.enums.get(enum_name)
        .cloned()
        .expect("enum variants missing");

    let void_ty = ctx.context.void_type();
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let i32_ty = ctx.context.i32_type();

    let fn_ty = void_ty.fn_type(&[ptr_ty.into()], false);
    let function = ctx.module.add_function(&format!("__print_{}", enum_name), fn_ty, None);

    let entry_bb = ctx.context.append_basic_block(function, "entry");
    let exit_bb = ctx.context.append_basic_block(function, "exit");

    ctx.builder.position_at_end(entry_bb);
    let enum_ptr = function.get_nth_param(0).unwrap().into_pointer_value();
    let tag_ptr = ctx.builder.build_struct_gep(struct_ty, enum_ptr, 0, "tag_ptr").unwrap();
    let tag_val = ctx.builder.build_load(i32_ty, tag_ptr, "tag").unwrap().into_int_value();

    let mut cases: Vec<(IntValue<'ctx>, BasicBlock<'ctx>)> = Vec::with_capacity(variants.len());
    let mut variant_bbs: Vec<BasicBlock<'ctx>> = Vec::with_capacity(variants.len());
    for (i, (variant_name, _)) in variants.iter().enumerate() {
        let bb = ctx.context.append_basic_block(function, &format!("v_{}", variant_name));
        cases.push((i32_ty.const_int(i as u64, false), bb));
        variant_bbs.push(bb);
    }
    ctx.builder.build_switch(tag_val, exit_bb, &cases).unwrap();

    for (bb, (_, payload_ty)) in variant_bbs.iter().zip(variants.iter()) {
        ctx.builder.position_at_end(*bb);
        let payload_ptr = ctx.builder.build_struct_gep(struct_ty, enum_ptr, 1, "payload_ptr").unwrap();
        let payload_val = ctx.builder.build_load(*payload_ty, payload_ptr, "payload").unwrap();

        let fmt = printf_format_for(payload_ty).to_string() + "\n";
        let fmt_str = ctx.builder.build_global_string_ptr(&fmt, "fmt").unwrap();
        ctx.builder.build_call(
            printfn,
            &[fmt_str.as_pointer_value().into(), payload_val.into()],
            "printFn",
        ).unwrap();
        ctx.builder.build_unconditional_branch(exit_bb).unwrap();
    }

    ctx.builder.position_at_end(exit_bb);
    ctx.builder.build_return(None).unwrap();

    if let Some(prev) = prev_block {
        ctx.builder.position_at_end(prev);
    }
    function
}
