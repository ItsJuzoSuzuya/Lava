use std::{fmt::Display};

use inkwell::basic_block::BasicBlock;
use inkwell::types::BasicType;
use inkwell::IntPredicate;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

use crate::{codegen_ctx::{CodegenContext, CodegenError, LoopTarget, function_symbol}, sym::OwnedSlot, declaration::Declaration, expression::{Block, Expr, ExprKind, ListRest, MatchArm, Pattern, build_comparison, strings_equal}, token::Token, span::Span, traits::codegen::Codegen, r#type::Type};

#[derive(Clone)]
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

#[derive(Clone)]
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
    /// `let (a, b) = value;`, a variable for each value of a tuple. `_`
    /// doesn't keep its value.
    Destructure {
        names: Vec<String>,
        value: Expr,
        mutable: bool,
    },
    /// `while condition { ... }`
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    /// `break;`, leaves the innermost loop.
    Break,
    /// `continue;`, goes on with the next round of the innermost loop.
    Continue,
}

impl Display for StmtKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StmtKind::Declaration(decl)                 => write!(f, "DeclarationStmt({})", decl.kind),
            StmtKind::Expression(expr)                  => write!(f, "ExpressionStmt({})", expr.kind),
            StmtKind::Return(expr)                      => write!(f, "Return({})", expr.kind),
            StmtKind::Import(expr)                      => write!(f, "{}", expr.kind),

            StmtKind::Destructure { names, value, .. } => write!(f, "Destructure(({}), {})", names.join(", "), value.kind),
            StmtKind::Instantiation { name, typedef, value, ..}    =>
                write!(f, "Instantiation({}, {}, {})", name, typedef.clone().unwrap_or_else(|| Type::from(value.kind.clone())) , value.kind),
            StmtKind::While { condition, .. }  => write!(f, "While({})", condition.kind),
            StmtKind::Break                    => write!(f, "Break"),
            StmtKind::Continue                 => write!(f, "Continue"),
        }
    }
}

impl Codegen for Stmt {
    fn codegen<'ctx, 'a>(&self, ctx: &'a mut CodegenContext<'ctx>) -> Option<Box<BasicValueEnum<'ctx>>> {
        match &self.kind {
            StmtKind::Declaration(decl) => decl.codegen(ctx),
            StmtKind::Expression(expr) => {
                let value = expr.codegen(ctx);
                // Nobody keeps a result like the one of `make();`, so its memory is freed right away
                if let Some(value) = &value {
                    if ctx.owns_memory(&expr.ty) {
                        let mark = ctx.temporaries.len();
                        ctx.push_temporary(**value, &expr.ty);
                        ctx.drop_temporaries(mark);
                    }
                }
                value
            }
            StmtKind::Return(expr) => {
                // The return value is computed first, it may move a `Unique` out of a variable
                let expr_val = expr.codegen(ctx).unwrap();
                // The value is computed, temporaries of the expressions around the `return` are freed too
                ctx.emit_temporary_drops(0);
                ctx.emit_all_drops();
                ctx.builder.build_return(Some(&*expr_val)).unwrap();
                // A `return` at the top level ends the program in the entry point.
                // The code after it is never run, but still needs a block to go into
                let function = ctx.builder.get_insert_block().and_then(|b| b.get_parent()).unwrap();
                if function.get_name().to_bytes() == b"main" {
                    let after = ctx.context.append_basic_block(function, "after_exit");
                    ctx.builder.position_at_end(after);
                }
                None
            }
            StmtKind::Import(_) => None,

            StmtKind::Destructure { names, value, .. } => {
                let tuple = value.codegen(ctx).unwrap().into_struct_value();
                let Type::Tuple(types) = &value.ty else { panic!("sema only destructures tuples") };
                for (i, (name, ty)) in names.iter().zip(types).enumerate() {
                    let item = ctx.builder.build_extract_value(tuple, i as u32, "item").unwrap();
                    // `_` keeps nothing, what the value owns is freed right away
                    if name == "_" {
                        if ctx.owns_memory(ty) {
                            let mark = ctx.temporaries.len();
                            ctx.push_temporary(item, ty);
                            ctx.drop_temporaries(mark);
                        }
                        continue;
                    }
                    let llvm_ty = ty.to_llvm(ctx);
                    let ptr = ctx.build_entry_alloca(llvm_ty, name);
                    ctx.builder.build_store(ptr, item).unwrap();
                    if ctx.owns_memory(ty) {
                        ctx.push_owned_var(name.clone(), ptr, ty.clone());
                    } else {
                        ctx.push_var(name.clone(), llvm_ty, ptr, ty.class_name());
                    }
                }
                None
            }
            StmtKind::Instantiation { name, typedef, value, .. } => {
                let ty = typedef.clone().unwrap_or_else(|| Type::from(value.kind.clone()));
                if ty == Type::Unknown {
                    CodegenError::UndefinedTypeInit(name.to_string()).throw(&ctx.source, &self.span);
                }

                let llvm_ty = ty.to_llvm(ctx);
                let ptr = ctx.build_entry_alloca(llvm_ty, name);

                // The variable owns its `Unique`, `Buffer` or the memory of its class
                // and frees it at the end of the scope
                if ctx.owns_memory(&ty) {
                    let llvm_val = *value.codegen(ctx).unwrap();
                    ctx.builder.build_store(ptr, llvm_val).unwrap();
                    ctx.push_owned_var(name.to_string(), ptr, ty.clone());
                    return None;
                }

                let nominal_name = match &ty {
                    Type::Class(n) | Type::Enum(n) => Some(n.clone()),
                    _ => None,
                };

                if nominal_name.is_some() {
                    if let ExprKind::FunctionCall { params, receiver: None, mangled, .. } = &value.kind {
                        match mangled {
                            Some(fn_name) => {
                                let function = match ctx.module.get_function(&function_symbol(fn_name)) {
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
            StmtKind::While { condition, body } => {
                let function = ctx.builder.get_insert_block().and_then(|b| b.get_parent())
                    .expect("while loop outside of a function");
                let cond_bb = ctx.context.append_basic_block(function, "while_cond");
                let body_bb = ctx.context.append_basic_block(function, "while_body");
                let end_bb = ctx.context.append_basic_block(function, "while_end");

                ctx.builder.build_unconditional_branch(cond_bb).unwrap();
                ctx.builder.position_at_end(cond_bb);
                let condition = condition.codegen(ctx).unwrap().into_int_value();
                ctx.builder.build_conditional_branch(condition, body_bb, end_bb).unwrap();

                // After the body, the condition is checked again
                ctx.builder.position_at_end(body_bb);
                ctx.loops.push(LoopTarget {
                    continue_bb: cond_bb,
                    break_bb: end_bb,
                    drop_depth: ctx.drop_scopes.len(),
                    temporary_depth: ctx.temporaries.len(),
                });
                codegen_block(ctx, body, cond_bb);
                ctx.loops.pop();

                ctx.builder.position_at_end(end_bb);
                None
            }
            StmtKind::Break | StmtKind::Continue => {
                let target = ctx.loops.last().copied().expect("break outside of a loop, sema must reject it");
                // Values declared inside the loop are gone when leaving the round, and so are
                // temporaries of the expression it leaves, like a match's scrutinee
                ctx.emit_temporary_drops(target.temporary_depth);
                for scope in ctx.drop_scopes[target.drop_depth..].iter().rev() {
                    ctx.emit_drops(scope);
                }
                let jump_to = if matches!(self.kind, StmtKind::Break) { target.break_bb } else { target.continue_bb };
                ctx.builder.build_unconditional_branch(jump_to).unwrap();

                // Code after it never runs, but still needs a block to go into
                let function = ctx.builder.get_insert_block().and_then(|b| b.get_parent()).unwrap();
                let dead_bb = ctx.context.append_basic_block(function, "after_jump");
                ctx.builder.position_at_end(dead_bb);
                None
            }
        }
    }

}

/// Generates the statements of a block, like a match arm or the body of an
/// `if` or `while`, and jumps to `next_bb` afterwards.
///
/// The block is its own scope: its variables are gone afterwards, and the
/// `Unique`s declared in it are freed at its end. If it ends with `return`,
/// that freed them already.
fn codegen_block<'ctx>(ctx: &mut CodegenContext<'ctx>, body: &[Stmt], next_bb: BasicBlock<'ctx>) {
    let prev_scope = ctx.scope.clone();
    ctx.drop_scopes.push(Vec::new());
    for stmt in body {
        stmt.codegen(ctx);
    }
    let owned = ctx.drop_scopes.pop().unwrap_or_default();
    if ctx.builder.get_insert_block().and_then(|b| b.get_terminator()).is_none() {
        ctx.emit_drops(&owned);
        ctx.builder.build_unconditional_branch(next_bb).unwrap();
    }
    ctx.scope = prev_scope;
}

/// Generates `block`, a match arm or a branch of an `if`, and jumps to
/// `next_bb` afterwards.
///
/// Like [`codegen_block`] the block is its own scope. Its value is stored in
/// `result` for a match or if used as a value, otherwise it's unused and
/// freed if it owns memory.
fn codegen_value_block<'ctx>(ctx: &mut CodegenContext<'ctx>, block: &Block, next_bb: BasicBlock<'ctx>, result: Option<PointerValue<'ctx>>) {
    let prev_scope = ctx.scope.clone();
    ctx.drop_scopes.push(Vec::new());
    for stmt in &block.stmts {
        stmt.codegen(ctx);
    }
    let open = |ctx: &CodegenContext<'ctx>| ctx.builder.get_insert_block().and_then(|b| b.get_terminator()).is_none();
    if let Some(value) = block.value.as_ref().filter(|_| open(ctx)) {
        match (value.codegen(ctx), result) {
            (Some(computed), Some(slot)) => {
                ctx.builder.build_store(slot, *computed).unwrap();
            }
            (Some(computed), None) if ctx.owns_memory(&value.ty) => {
                let mark = ctx.temporaries.len();
                ctx.push_temporary(*computed, &value.ty);
                ctx.drop_temporaries(mark);
            }
            _ => {}
        }
    }
    let owned = ctx.drop_scopes.pop().unwrap_or_default();
    if open(ctx) {
        ctx.emit_drops(&owned);
        ctx.builder.build_unconditional_branch(next_bb).unwrap();
    }
    ctx.scope = prev_scope;
}

/// Generates an `if`. For an `if` used as a value both branches store their
/// value in `result`, sema made sure there's an else branch.
pub fn codegen_if<'ctx>(ctx: &mut CodegenContext<'ctx>, condition: &Expr, then_block: &Block, else_block: Option<&Block>, result: Option<PointerValue<'ctx>>) {
    let function = ctx.builder.get_insert_block().and_then(|b| b.get_parent())
        .expect("if outside of a function");
    let then_bb = ctx.context.append_basic_block(function, "then");
    let else_bb = ctx.context.append_basic_block(function, "else");
    let merge_bb = ctx.context.append_basic_block(function, "if_end");

    let condition = condition.codegen(ctx).unwrap().into_int_value();
    ctx.builder.build_conditional_branch(condition, then_bb, else_bb).unwrap();

    ctx.builder.position_at_end(then_bb);
    codegen_value_block(ctx, then_block, merge_bb, result);
    ctx.builder.position_at_end(else_bb);
    match else_block {
        Some(else_block) => codegen_value_block(ctx, else_block, merge_bb, result),
        None => {
            ctx.builder.build_unconditional_branch(merge_bb).unwrap();
        }
    }
    ctx.builder.position_at_end(merge_bb);
}

/// Generates a match on `scrutinee`. For a match used as a value every arm
/// stores its value in `result`, sema made sure one of the arms runs.
pub fn codegen_match<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    scrutinee: &Expr,
    arms: &[MatchArm],
    span: &Span,
    result: Option<PointerValue<'ctx>>,
) {
    let function = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
        Some(f) => f,
        None => panic!("match statement outside of a function"),
    };

    // A `&` is matched by the value it refers to
    let scrutinee_ty = match &scrutinee.ty {
        Type::Ref(inner) => (**inner).clone(),
        other => other.clone(),
    };
    let is_enum = matches!(scrutinee_ty, Type::Enum(_));

    // Ints, bools and enums switch on their value or tag, anything else like
    // arrays, vectors and strings tests one arm after the other
    if !is_enum && !matches!(scrutinee_ty, Type::Int { .. } | Type::Bool | Type::Char) {
        codegen_chain_match(ctx, scrutinee, arms, result);
        return;
    }

    let merge_bb = ctx.context.append_basic_block(function, "match_end");
    let default_bb = ctx.context.append_basic_block(function, "match_default");

    if is_enum {
        let enum_name = match &scrutinee_ty {
            Type::Enum(n) => n.clone(),
            _ => unreachable!(),
        };
        // A temporary scrutinee, like a call's result, is freed after the match,
        // or when an arm leaves early with `return` or `break`
        let mut temporary_scope = Vec::new();
        let enum_ptr = scrutinee_place(ctx, scrutinee, &scrutinee_ty, &mut temporary_scope);
        ctx.drop_scopes.push(temporary_scope);
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
                Pattern::Literal(_) | Pattern::List { .. } | Pattern::Tuple(_) => {
                    panic!("this pattern can't match an enum, sema rejects it");
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
                // The class the binding is or points to, through a `Unique` payload too
                let index = variants.iter().position(|(n, _)| n == variant);
                let class_name = index
                    .and_then(|i| ctx.enum_variant_types.get(&enum_name).and_then(|types| types.get(i)))
                    .and_then(|ty| ty.class_name());
                ctx.push_var(name.clone(), payload_ty, payload_ptr, class_name);
            }

            codegen_value_block(ctx, &arm.block, merge_bb, result);
            ctx.scope = prev_scope;
        }
        let temporary_scope = ctx.drop_scopes.pop().unwrap_or_default();

        // Without a matching arm nothing happens. A match giving a value always has one.
        ctx.builder.position_at_end(default_bb);
        if result.is_some() {
            ctx.builder.build_unreachable().unwrap();
        } else {
            ctx.builder.build_unconditional_branch(merge_bb).unwrap();
        }
        ctx.builder.position_at_end(merge_bb);
        ctx.emit_drops(&temporary_scope);
        return;
    } else {
        let scrutinee_val = match &scrutinee.ty {
            Type::Ref(_) => {
                let address = scrutinee.codegen(ctx).unwrap().into_pointer_value();
                ctx.builder.build_load(scrutinee_ty.to_llvm(ctx), address, "matched").unwrap()
            }
            _ => *scrutinee.codegen(ctx).unwrap(),
        };
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
                // A name matches any value and gets bound to it
                Pattern::Literal(expr) if matches!(expr.kind, ExprKind::Identifier(_)) => {
                    wildcard_bb = Some(bb);
                }
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
                Pattern::EnumVariant { .. } | Pattern::List { .. } | Pattern::Tuple(_) => {
                    panic!("this pattern can't match a number or bool, sema rejects it");
                }
            }
        }

        let switch_default = wildcard_bb.unwrap_or(default_bb);
        ctx.builder.build_switch(scrutinee_int, switch_default, &cases).unwrap();

        for (arm, bb) in arms.iter().zip(arm_blocks.iter()) {
            ctx.builder.position_at_end(*bb);
            let prev_scope = ctx.scope.clone();
            if let Pattern::Literal(Expr { kind: ExprKind::Identifier(name), .. }) = &arm.pattern {
                let slot = ctx.build_entry_alloca(scrutinee_int.get_type(), name);
                ctx.builder.build_store(slot, scrutinee_int).unwrap();
                ctx.push_var(name.clone(), scrutinee_int.get_type().into(), slot, None);
            }
            codegen_value_block(ctx, &arm.block, merge_bb, result);
            ctx.scope = prev_scope;
        }
    }

    // Without a matching arm nothing happens. A match giving a value always has one.
    ctx.builder.position_at_end(default_bb);
    if result.is_some() {
        ctx.builder.build_unreachable().unwrap();
    } else {
        ctx.builder.build_unconditional_branch(merge_bb).unwrap();
    }
    ctx.builder.position_at_end(merge_bb);
}

/// Generates a match that tests one arm after the other, for scrutinees that
/// can't be switched on, like arrays, vectors and strings.
fn codegen_chain_match<'ctx>(ctx: &mut CodegenContext<'ctx>, scrutinee: &Expr, arms: &[MatchArm], result: Option<PointerValue<'ctx>>) {
    let function = ctx.builder.get_insert_block().and_then(|b| b.get_parent()).expect("match outside of a function");
    let ty = match &scrutinee.ty {
        Type::Ref(inner) => (**inner).clone(),
        other => other.clone(),
    };
    let merge_bb = ctx.context.append_basic_block(function, "match_end");

    // A temporary scrutinee is freed after the match, like for enums
    let mut temporary_scope = Vec::new();
    let place = scrutinee_place(ctx, scrutinee, &ty, &mut temporary_scope);
    ctx.drop_scopes.push(temporary_scope);

    for arm in arms {
        let arm_bb = ctx.context.append_basic_block(function, "arm");
        let next_bb = ctx.context.append_basic_block(function, "next_arm");
        let matched = pattern_test(ctx, &arm.pattern, place, &ty);
        ctx.builder.build_conditional_branch(matched, arm_bb, next_bb).unwrap();

        ctx.builder.position_at_end(arm_bb);
        let prev_scope = ctx.scope.clone();
        bind_pattern(ctx, &arm.pattern, place, &ty);
        codegen_value_block(ctx, &arm.block, merge_bb, result);
        ctx.scope = prev_scope;
        ctx.builder.position_at_end(next_bb);
    }
    let temporary_scope = ctx.drop_scopes.pop().unwrap_or_default();

    // No arm matched. A match giving a value always has one that does.
    if result.is_some() {
        ctx.builder.build_unreachable().unwrap();
    } else {
        ctx.builder.build_unconditional_branch(merge_bb).unwrap();
    }
    ctx.builder.position_at_end(merge_bb);
    ctx.emit_drops(&temporary_scope);
}

/// The address of the value of type `ty` that the match on `scrutinee` looks
/// at. A `&` gives the value it refers to. A temporary like a call's result is
/// stored in a slot, added to `temporary_scope` so it's freed after the match.
fn scrutinee_place<'ctx>(ctx: &mut CodegenContext<'ctx>, scrutinee: &Expr, ty: &Type, temporary_scope: &mut Vec<OwnedSlot<'ctx>>) -> PointerValue<'ctx> {
    if let Type::Ref(_) = scrutinee.ty {
        return scrutinee.codegen(ctx).unwrap().into_pointer_value();
    }
    if let Some(place) = scrutinee.codegen_place(ctx) {
        return place;
    }
    let value = *scrutinee.codegen(ctx).unwrap();
    let slot = ctx.build_entry_alloca(ty.to_llvm(ctx), "match_scrut");
    ctx.builder.build_store(slot, value).unwrap();
    if ctx.owns_memory(ty) {
        let alive = ctx.build_entry_alloca(ctx.context.bool_type(), "match_scrut_alive");
        ctx.builder.build_store(alive, ctx.context.bool_type().const_int(1, false)).unwrap();
        temporary_scope.push(OwnedSlot { slot, alive, ty: ty.clone() });
    }
    slot
}

/// Whether the value of type `ty` at `place` matches `pattern`, as an `i1`.
fn pattern_test<'ctx>(ctx: &mut CodegenContext<'ctx>, pattern: &Pattern, place: PointerValue<'ctx>, ty: &Type) -> IntValue<'ctx> {
    let always = ctx.context.bool_type().const_int(1, false);
    match pattern {
        Pattern::Wildcard => always,
        Pattern::Literal(Expr { kind: ExprKind::Identifier(_), .. }) => always,
        Pattern::Literal(expected) if ty.is_string() => {
            // The literal is a new string, compared and freed again
            let expected = *expected.codegen(ctx).unwrap();
            ctx.push_temporary(expected, ty);
            let expected_slot = ctx.temporaries.last().unwrap().0;
            let equal = strings_equal(ctx, place, expected_slot);
            let mark = ctx.temporaries.len() - 1;
            ctx.drop_temporaries(mark);
            equal
        }
        Pattern::Literal(expected) => {
            let actual = ctx.builder.build_load(ty.to_llvm(ctx), place, "item").unwrap();
            let expected = *expected.codegen(ctx).unwrap();
            build_comparison(ctx, &Token::EqEq, actual, expected, ty)
        }
        Pattern::EnumVariant { enum_name, variant, .. } => {
            let struct_ty = ctx.module.get_struct_type(enum_name).expect("enum struct not defined");
            let index = ctx.enums.get(enum_name).and_then(|variants| variants.iter().position(|(n, _)| n == variant))
                .expect("sema checked the variant");
            let tag_ptr = ctx.builder.build_struct_gep(struct_ty, place, 0, "tag_ptr").unwrap();
            let tag = ctx.builder.build_load(ctx.context.i32_type(), tag_ptr, "tag").unwrap().into_int_value();
            let wanted = ctx.context.i32_type().const_int(index as u64, false);
            ctx.builder.build_int_compare(IntPredicate::EQ, tag, wanted, "is_variant").unwrap()
        }
        Pattern::List { before, rest, after } => {
            let function = ctx.builder.get_insert_block().and_then(|b| b.get_parent()).unwrap();
            let element = list_element(ctx, ty);
            let len = list_len(ctx, place, ty);
            let count = ctx.context.i64_type().const_int((before.len() + after.len()) as u64, false);
            let predicate = if rest.is_some() { IntPredicate::UGE } else { IntPredicate::EQ };
            let length_fits = ctx.builder.build_int_compare(predicate, len, count, "length_fits").unwrap();

            // The items are only looked at if there are enough of them
            let matched = ctx.build_entry_alloca(ctx.context.bool_type(), "list_matched");
            ctx.builder.build_store(matched, ctx.context.bool_type().const_zero()).unwrap();
            let items_bb = ctx.context.append_basic_block(function, "list_items");
            let done_bb = ctx.context.append_basic_block(function, "list_done");
            ctx.builder.build_conditional_branch(length_fits, items_bb, done_bb).unwrap();

            ctx.builder.position_at_end(items_bb);
            let mut all = always;
            for (index, item) in list_items(ctx, before, after, len) {
                let item_place = list_item(ctx, place, ty, index);
                let item_matches = pattern_test(ctx, item, item_place, &element);
                all = ctx.builder.build_and(all, item_matches, "items_match").unwrap();
            }
            ctx.builder.build_store(matched, all).unwrap();
            ctx.builder.build_unconditional_branch(done_bb).unwrap();

            ctx.builder.position_at_end(done_bb);
            ctx.builder.build_load(ctx.context.bool_type(), matched, "list_matches").unwrap().into_int_value()
        }
        // Every value matches its own pattern
        Pattern::Tuple(items) => {
            let mut all = always;
            for (i, (item, item_ty)) in items.iter().zip(tuple_types(ty)).enumerate() {
                let item_place = tuple_item(ctx, place, ty, i);
                let item_matches = pattern_test(ctx, item, item_place, &item_ty);
                all = ctx.builder.build_and(all, item_matches, "tuple_matches").unwrap();
            }
            all
        }
    }
}

/// The types of the values of the tuple type `ty`.
fn tuple_types(ty: &Type) -> Vec<Type> {
    match ty {
        Type::Tuple(types) => types.clone(),
        other => panic!("'{}' isn't a tuple, sema checks tuple patterns", other),
    }
}

/// The address of value `index` of the tuple of type `ty` at `place`.
fn tuple_item<'ctx>(ctx: &CodegenContext<'ctx>, place: PointerValue<'ctx>, ty: &Type, index: usize) -> PointerValue<'ctx> {
    let struct_ty = ty.to_llvm(ctx).into_struct_type();
    ctx.builder.build_struct_gep(struct_ty, place, index as u32, "tuple_item").unwrap()
}

/// Adds the names `pattern` binds to the scope. They look at the matched
/// value at `place` of type `ty`, which keeps owning it.
fn bind_pattern<'ctx>(ctx: &mut CodegenContext<'ctx>, pattern: &Pattern, place: PointerValue<'ctx>, ty: &Type) {
    match pattern {
        Pattern::Literal(Expr { kind: ExprKind::Identifier(name), .. }) => {
            ctx.push_var(name.clone(), ty.to_llvm(ctx), place, ty.class_name());
        }
        Pattern::EnumVariant { enum_name, variant, binding: Some(name) } => {
            let struct_ty = ctx.module.get_struct_type(enum_name).expect("enum struct not defined");
            let index = ctx.enums.get(enum_name).and_then(|variants| variants.iter().position(|(n, _)| n == variant))
                .expect("sema checked the variant");
            let payload_ty = ctx.enum_variant_types.get(enum_name).and_then(|types| types.get(index)).cloned()
                .expect("sema checked the variant");
            let payload_place = ctx.builder.build_struct_gep(struct_ty, place, 1, "payload_ptr").unwrap();
            ctx.push_var(name.clone(), payload_ty.to_llvm(ctx), payload_place, payload_ty.class_name());
        }
        Pattern::List { before, rest, after } => {
            let element = list_element(ctx, ty);
            let len = list_len(ctx, place, ty);
            for (index, item) in list_items(ctx, before, after, len) {
                let item_place = list_item(ctx, place, ty, index);
                bind_pattern(ctx, item, item_place, &element);
            }
            let Some(ListRest { binding: Some(name), ty: rest_ty, .. }) = rest else { return };
            let start = ctx.context.i64_type().const_int(before.len() as u64, false);
            let first = list_item(ctx, place, ty, start);
            match ty {
                // The rest of an array is a smaller array right inside it
                Type::Array(..) => ctx.push_var(name.clone(), rest_ty.to_llvm(ctx), first, None),
                // The rest of a vector is a vector looking at its items, without owning them
                _ => {
                    let count = ctx.context.i64_type().const_int((before.len() + after.len()) as u64, false);
                    let rest_len = ctx.builder.build_int_sub(len, count, "rest_len").unwrap();
                    let header = ctx.build_entry_alloca(ctx.buffer_header_type(), "rest_buffer");
                    ctx.builder.build_store(ctx.buffer_capacity_ptr(header), rest_len).unwrap();
                    ctx.builder.build_store(ctx.buffer_data_ptr(header), first).unwrap();

                    let class = ty.class_name().expect("a vector is a class");
                    let struct_ty = ctx.module.get_struct_type(&class).expect("vector struct not defined");
                    let view = ctx.build_entry_alloca(struct_ty, "rest");
                    ctx.builder.build_store(view, struct_ty.const_zero()).unwrap();
                    let (data_index, length_index) = vector_fields(ctx, &class);
                    let data_ptr = ctx.builder.build_struct_gep(struct_ty, view, data_index, "rest_data").unwrap();
                    ctx.builder.build_store(data_ptr, header).unwrap();
                    let length_ptr = ctx.builder.build_struct_gep(struct_ty, view, length_index, "rest_length").unwrap();
                    ctx.builder.build_store(length_ptr, rest_len).unwrap();
                    ctx.push_var(name.clone(), struct_ty.into(), view, Some(class));
                }
            }
        }
        Pattern::Tuple(items) => {
            for (i, (item, item_ty)) in items.iter().zip(tuple_types(ty)).enumerate() {
                let item_place = tuple_item(ctx, place, ty, i);
                bind_pattern(ctx, item, item_place, &item_ty);
            }
        }
        _ => {}
    }
}

/// The items a list pattern looks at with their index: the ones before the
/// `..` from the start, the ones after it from the end of a list of length `len`.
fn list_items<'a, 'ctx>(ctx: &CodegenContext<'ctx>, before: &'a [Pattern], after: &'a [Pattern], len: IntValue<'ctx>) -> Vec<(IntValue<'ctx>, &'a Pattern)> {
    let i64_ty = ctx.context.i64_type();
    let mut items: Vec<_> = before.iter().enumerate().map(|(i, item)| (i64_ty.const_int(i as u64, false), item)).collect();
    for (j, item) in after.iter().enumerate() {
        let from_end = i64_ty.const_int((after.len() - j) as u64, false);
        items.push((ctx.builder.build_int_sub(len, from_end, "index_from_end").unwrap(), item));
    }
    items
}

/// The item type of the array or vector type `ty`.
fn list_element(ctx: &CodegenContext<'_>, ty: &Type) -> Type {
    match ty {
        Type::Array(element, _) => (**element).clone(),
        _ => {
            let class = ty.class_name().expect("sema only allows arrays and vectors");
            let (data_index, _) = vector_fields(ctx, &class);
            match ctx.class_field_types.get(&class).and_then(|fields| fields.get(data_index as usize)) {
                Some(Type::Buffer(element)) => (**element).clone(),
                _ => panic!("'{}' has no Buffer of items", class),
            }
        }
    }
}

/// Indices of the fields `data` and `length` of the vector class `class`.
fn vector_fields(ctx: &CodegenContext<'_>, class: &str) -> (u32, u32) {
    let fields = ctx.classes.get(class).expect("vector class not declared");
    let index = |name: &str| fields.iter().position(|(n, _)| n == name).expect("a Vector has data and length") as u32;
    (index("data"), index("length"))
}

/// The number of items of the array or vector of type `ty` at `place`, as an `i64`.
fn list_len<'ctx>(ctx: &CodegenContext<'ctx>, place: PointerValue<'ctx>, ty: &Type) -> IntValue<'ctx> {
    match ty {
        Type::Array(_, len) => ctx.context.i64_type().const_int(*len, false),
        _ => {
            let class = ty.class_name().expect("sema only allows arrays and vectors");
            let struct_ty = ctx.module.get_struct_type(&class).expect("vector struct not defined");
            let (_, length_index) = vector_fields(ctx, &class);
            let length_ptr = ctx.builder.build_struct_gep(struct_ty, place, length_index, "length_ptr").unwrap();
            ctx.builder.build_load(ctx.context.i64_type(), length_ptr, "length").unwrap().into_int_value()
        }
    }
}

/// Address of the item at `index` of the array or vector of type `ty` at `place`.
fn list_item<'ctx>(ctx: &CodegenContext<'ctx>, place: PointerValue<'ctx>, ty: &Type, index: IntValue<'ctx>) -> PointerValue<'ctx> {
    match ty {
        Type::Array(..) => {
            let zero = ctx.context.i64_type().const_zero();
            unsafe { ctx.builder.build_in_bounds_gep(ty.to_llvm(ctx), place, &[zero, index], "item").unwrap() }
        }
        _ => {
            let class = ty.class_name().expect("sema only allows arrays and vectors");
            let struct_ty = ctx.module.get_struct_type(&class).expect("vector struct not defined");
            let (data_index, _) = vector_fields(ctx, &class);
            let data_ptr = ctx.builder.build_struct_gep(struct_ty, place, data_index, "data_ptr").unwrap();
            let ptr_ty = ctx.context.ptr_type(inkwell::AddressSpace::default());
            let header = ctx.builder.build_load(ptr_ty, data_ptr, "buffer").unwrap().into_pointer_value();
            ctx.buffer_slot(header, &list_element(ctx, ty), index)
        }
    }
}
