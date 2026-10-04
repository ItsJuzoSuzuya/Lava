use std::{fmt::Display};

use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;

use crate::{codegen_ctx::{CodegenContext, CodegenError}, declaration::Declaration, expression::{Expr, ExprKind}, span::Span, traits::codegen::Codegen, r#type::Type};

pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
    pub ty: Type,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span, ty: Type) -> Self {
        Self { kind, span, ty }
    }
}

pub enum Pattern {
    EnumVariant {
        enum_name: String,
        variant: String,
        binding: Option<String>,
    },
    Literal(Expr),
    Wildcard,
}

impl Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pattern::Literal(expr) => write!(f, "{}", expr.kind),
            _ => panic!()
        }
    }
}

pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Vec<Stmt>,
    pub span: Span,
}

pub enum StmtKind {
    Declaration(Declaration),
    Expression(Expr),
    Return(Expr),
    Import(Expr),

    Instantiation {
        name: String,
        typedef: Option<Type>,
        value: Expr,
        mutable: bool,
    },
    Match {
        scrutinee: Expr,
        arms: Vec<MatchArm>,
    },
}

impl Display for StmtKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StmtKind::Declaration(decl)                 => write!(f, "DeclarationStmt({})", decl.kind),
            StmtKind::Expression(expr)                  => write!(f, "ExpressionStmt({})", expr.kind),
            StmtKind::Return(expr)                      => write!(f, "Return({})", expr.kind),
            StmtKind::Import(expr)                      => write!(f, "{}", expr.kind),

            StmtKind::Instantiation { name, typedef, value, ..}    =>
                write!(f, "Instantiation({}, {}, {})", name, typedef.clone().unwrap_or_else(|| Type::from(value.kind.clone())) , value.kind),
            StmtKind::Match { scrutinee, .. }  => write!(f, "Match({})", scrutinee.kind),
        }
    }
}

impl Codegen for Stmt {
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>> {
        match &self.kind {
            StmtKind::Declaration(decl) => decl.codegen(ctx),
            StmtKind::Expression(expr) => expr.codegen(ctx),
            StmtKind::Return(expr) => {
                let expr_val = expr.codegen(ctx).unwrap();
                ctx.builder.build_return(Some(&*expr_val)).unwrap();
                None
            }
            StmtKind::Import(_) => None,

            StmtKind::Instantiation { name, typedef, value, .. } => {
                let ty = typedef.clone().unwrap_or_else(|| Type::from(value.kind.clone()));
                if ty == Type::Unknown {
                    CodegenError::UndefinedTypeInit(name.to_string()).throw(&ctx.source, &self.span);
                }

                let llvm_ty = ty.to_llvm(ctx);
                let ptr = ctx.builder.build_alloca(llvm_ty, name).unwrap();

                let nominal_name = match &ty {
                    Type::Class(n) | Type::Enum(n) => Some(n.clone()),
                    _ => None,
                };

                if nominal_name.is_some() {
                    if let ExprKind::FunctionCall { params, parent_name: None, mangled, .. } = &value.kind {
                        match mangled {
                            Some(fn_name) => {
                                let function = match ctx.module.get_function(fn_name) {
                                    Some(f) => f,
                                    None => panic!("constructor '{}' not defined", fn_name),
                                };
                                let mut llvm_params: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
                                for param in params {
                                    llvm_params.push((*param.codegen(ctx).unwrap()).into());
                                }
                                let ret_val = ctx.builder.build_call(function, &llvm_params, "ctor")
                                    .unwrap()
                                    .try_as_basic_value()
                                    .basic()
                                    .expect("constructor must return a value");
                                ctx.builder.build_store(ptr, ret_val).unwrap();
                                ctx.push_var(name.to_string(), llvm_ty, ptr, nominal_name);
                            }
                            None => {
                                let llvm_val = *value.codegen(ctx).unwrap();
                                ctx.builder.build_store(ptr, llvm_val).unwrap();
                                ctx.push_var(name.to_string(), llvm_ty, ptr, nominal_name);
                            }
                        }
                    } else {
                        let llvm_val = *value.codegen(ctx).unwrap();
                        ctx.builder.build_store(ptr, llvm_val).unwrap();
                        ctx.push_var(name.to_string(), llvm_ty, ptr, nominal_name);
                    }
                } else {
                    let llvm_val = *value.codegen(ctx).unwrap();
                    ctx.builder.build_store(ptr, llvm_val).unwrap();
                    ctx.push_var(name.to_string(), llvm_ty, ptr, None);
                }

                return None;
            }
            StmtKind::Match { scrutinee, arms } => {
                codegen_match(ctx, scrutinee, arms, &self.span);
                None
            }
        }
    }

}

fn codegen_match<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    scrutinee: &Expr,
    arms: &[MatchArm],
    span: &Span,
) {
    let function = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
        Some(f) => f,
        None => panic!("match statement outside of a function"),
    };

    let scrutinee_ty = scrutinee.ty.clone();
    let is_enum = matches!(scrutinee_ty, Type::Enum(_));

    let merge_bb = ctx.context.append_basic_block(function, "match_end");
    let default_bb = ctx.context.append_basic_block(function, "match_default");

    if is_enum {
        let enum_name = match &scrutinee_ty {
            Type::Enum(n) => n.clone(),
            _ => unreachable!(),
        };
        let enum_ptr = match scrutinee.codegen_place(ctx) {
            Some(p) => p,
            None => {
                let val = *scrutinee.codegen(ctx).unwrap();
                let ty = scrutinee_ty.to_llvm(ctx);
                let tmp = ctx.builder.build_alloca(ty, "match_scrut").unwrap();
                ctx.builder.build_store(tmp, val).unwrap();
                tmp
            }
        };
        let struct_ty = ctx.module.get_struct_type(&enum_name).expect("enum struct not defined");
        let i32_ty = ctx.context.i32_type();
        let tag_ptr = ctx.builder.build_struct_gep(struct_ty, enum_ptr, 0, "tag_ptr").unwrap();
        let tag_val = ctx.builder.build_load(i32_ty, tag_ptr, "tag").unwrap().into_int_value();

        let variants = ctx.enums.get(&enum_name)
            .cloned()
            .unwrap_or_else(|| CodegenError::UndefinedClass(enum_name.clone()).throw(&ctx.source, span));

        let mut cases: Vec<(inkwell::values::IntValue<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> = Vec::new();
        let mut arm_blocks: Vec<inkwell::basic_block::BasicBlock<'ctx>> = Vec::with_capacity(arms.len());
        let mut wildcard_bb: Option<inkwell::basic_block::BasicBlock<'ctx>> = None;

        for arm in arms {
            let bb = ctx.context.append_basic_block(function, "arm");
            arm_blocks.push(bb);
            match &arm.pattern {
                Pattern::EnumVariant { variant, .. } => {
                    let idx = variants.iter().position(|(n, _)| n == variant)
                        .unwrap_or_else(|| CodegenError::UndefinedField {
                            class: enum_name.clone(),
                            field: variant.clone(),
                        }.throw(&ctx.source, &arm.span));
                    cases.push((i32_ty.const_int(idx as u64, false), bb));
                }
                Pattern::Wildcard => {
                    wildcard_bb = Some(bb);
                }
                Pattern::Literal(expr) if matches!(expr.kind, ExprKind::Identifier(_)) => {
                    wildcard_bb = Some(bb);
                }
                Pattern::Literal(_) => {
                    panic!("literal pattern on enum scrutinee");
                }
            }
        }

        let switch_default = wildcard_bb.unwrap_or(default_bb);
        ctx.builder.build_switch(tag_val, switch_default, &cases).unwrap();

        for (arm, bb) in arms.iter().zip(arm_blocks.iter()) {
            ctx.builder.position_at_end(*bb);

            let prev_scope = ctx.scope.clone();
            if let Pattern::Literal(expr) = &arm.pattern {
                if let ExprKind::Identifier(name) = &expr.kind {
                    ctx.push_var(name.clone(), struct_ty.as_basic_type_enum(), enum_ptr, Some(enum_name.clone()));
                }
            }
            if let Pattern::EnumVariant { binding: Some(name), variant, .. } = &arm.pattern {
                let payload_ty = variants.iter().find(|(n, _)| n == variant)
                    .map(|(_, t)| *t)
                    .expect("variant payload missing");
                let payload_ptr = ctx.builder
                    .build_struct_gep(struct_ty, enum_ptr, 1, "payload_ptr")
                    .unwrap();
                let class_name = match &arm.pattern {
                    Pattern::EnumVariant { variant, .. } => variants.iter()
                        .find(|(n, _)| n == variant)
                        .and_then(|(_, t)| if t.is_struct_type() {
                            Some(t.into_struct_type().get_name()?.to_str().ok()?.to_string())
                        } else { None }),
                    _ => None,
                };
                ctx.push_var(name.clone(), payload_ty, payload_ptr, class_name);
            }

            for stmt in &arm.body {
                stmt.codegen(ctx);
            }
            if ctx.builder.get_insert_block().and_then(|b| b.get_terminator()).is_none() {
                ctx.builder.build_unconditional_branch(merge_bb).unwrap();
            }
            ctx.scope = prev_scope;
        }

        ctx.builder.position_at_end(default_bb);
        ctx.builder.build_unconditional_branch(merge_bb).unwrap();
    } else {
        let scrutinee_val = *scrutinee.codegen(ctx).unwrap();
        if !scrutinee_val.is_int_value() {
            panic!("match on non-integer, non-enum scrutinee is not yet supported");
        }
        let scrutinee_int = scrutinee_val.into_int_value();

        let mut cases: Vec<(inkwell::values::IntValue<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> = Vec::new();
        let mut arm_blocks: Vec<inkwell::basic_block::BasicBlock<'ctx>> = Vec::with_capacity(arms.len());
        let mut wildcard_bb: Option<inkwell::basic_block::BasicBlock<'ctx>> = None;

        for arm in arms {
            let bb = ctx.context.append_basic_block(function, "arm");
            arm_blocks.push(bb);
            match &arm.pattern {
                Pattern::Literal(expr) => {
                    let val = *expr.codegen(ctx).unwrap();
                    if !val.is_int_value() {
                        panic!("literal pattern must be an integer or boolean for now");
                    }
                    cases.push((val.into_int_value(), bb));
                }
                Pattern::Wildcard => {
                    wildcard_bb = Some(bb);
                }
                Pattern::EnumVariant { .. } => {
                    panic!("enum variant pattern requires an enum scrutinee");
                }
            }
        }

        let switch_default = wildcard_bb.unwrap_or(default_bb);
        ctx.builder.build_switch(scrutinee_int, switch_default, &cases).unwrap();

        for (arm, bb) in arms.iter().zip(arm_blocks.iter()) {
            ctx.builder.position_at_end(*bb);
            for stmt in &arm.body {
                stmt.codegen(ctx);
            }
            if ctx.builder.get_insert_block().and_then(|b| b.get_terminator()).is_none() {
                ctx.builder.build_unconditional_branch(merge_bb).unwrap();
            }
        }

        ctx.builder.position_at_end(default_bb);
        ctx.builder.build_unconditional_branch(merge_bb).unwrap();
    }

    ctx.builder.position_at_end(merge_bb);
}
