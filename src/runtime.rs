//! Functions built into the language, written directly in LLVM IR on top of
//! the C library. Only what Lava code can't do itself is here, the rest is
//! in the prelude and the standard library, calling C with `extern func`.
//!
//! [`builtins`] lists them for sema, codegen and the language server. Their
//! LLVM functions are named `lava.<name>` (see [`symbol`]).
//!
//! `String` is a class of the prelude. What it can't do in Lava, copying and
//! comparing bytes between Buffers in one go, are the built-ins `copy_bytes`
//! and `compare_bytes`, like `memmove` and `memcmp` with bounds checks.

use inkwell::{AddressSpace, IntPredicate, module::Linkage, values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue}};

use crate::{codegen_ctx::CodegenContext, param::Param, r#type::Type};

/// A function built into the language.
pub struct Builtin {
    pub name: &'static str,
    pub params: Vec<(&'static str, Type)>,
    pub ret: Type,
    /// One line describing what it does, for the language server.
    pub doc: &'static str,
}

const UINT64: Type = Type::Int { bits: 64, signed: false };

/// Every function built into the language, besides `print`, `unique` and
/// `Buffer`, which are handled on their own.
pub fn builtins() -> Vec<Builtin> {
    let bytes = || Type::Ref(Box::new(Type::Buffer(Box::new(Type::Char))));
    vec![
        Builtin { name: "copy_bytes",
            params: vec![("into", bytes()), ("at", UINT64), ("from", bytes()), ("start", UINT64), ("count", UINT64)], ret: Type::Void,
            doc: "Copies `count` bytes of `from`, beginning at `start`, into `into` at `at`. Stops the program if either is too short." },
        Builtin { name: "compare_bytes",
            params: vec![("a", bytes()), ("a_start", UINT64), ("b", bytes()), ("b_start", UINT64), ("count", UINT64)], ret: Type::INT32,
            doc: "How `count` bytes of `a` from `a_start` and of `b` from `b_start` compare: below, at or above zero like C's `memcmp`. Stops the program if either is too short." },
        Builtin { name: "arg_count", params: vec![], ret: Type::INT32,
            doc: "How many command line arguments the program got, the program's own name included." },
        Builtin { name: "arg_bytes", params: vec![("index", Type::INT32)], ret: Type::Buffer(Box::new(Type::Char)),
            doc: "The bytes of the command line argument at `index`, 0 is the program's name. Stops the program if it's out of range." },
    ]
}

/// Whether the built-in `name` is only for the prelude and the standard
/// library, which build `String` and `env::args()` with them.
pub fn is_internal(name: &str) -> bool {
    matches!(name, "copy_bytes" | "compare_bytes" | "arg_count" | "arg_bytes")
}

/// The LLVM name of the built-in `name`.
pub fn symbol(name: &str) -> String {
    format!("lava.{}", name)
}

/// Keeps the command line arguments of `main` in globals for `arg_count` and `arg`.
pub fn store_arguments<'ctx>(ctx: &CodegenContext<'ctx>, main: FunctionValue<'ctx>) {
    let (argc, argv) = argument_globals(ctx);
    ctx.builder.build_store(argc, main.get_nth_param(0).unwrap()).unwrap();
    ctx.builder.build_store(argv, main.get_nth_param(1).unwrap()).unwrap();
}

fn argument_globals<'ctx>(ctx: &CodegenContext<'ctx>) -> (PointerValue<'ctx>, PointerValue<'ctx>) {
    let global = |name: &str, ty: inkwell::types::BasicTypeEnum<'ctx>| {
        ctx.module.get_global(name).unwrap_or_else(|| {
            let global = ctx.module.add_global(ty, None, name);
            global.set_linkage(Linkage::Private);
            global.set_initializer(&ty.const_zero());
            global
        }).as_pointer_value()
    };
    (global("lava.argc", ctx.context.i32_type().into()), global("lava.argv", ctx.context.ptr_type(AddressSpace::default()).into()))
}

/// Defines every built-in of [`builtins`] in the module.
pub fn define(ctx: &mut CodegenContext<'_>) {
    for builtin in builtins() {
        let params: Vec<Param> = builtin.params.iter().map(|(name, ty)| Param::builtin(name, ty.clone())).collect();
        let param_types: Vec<_> = builtin.params.iter()
            .map(|(name, ty)| if *name == "self" { ctx.context.ptr_type(AddressSpace::default()).into() } else { ty.to_llvm(ctx).into() })
            .collect();
        let fn_type = match &builtin.ret {
            Type::Void => ctx.context.void_type().fn_type(&param_types, false),
            ty => ty.to_fn_type(&param_types, ctx),
        };
        let function = ctx.module.add_function(&symbol(builtin.name), fn_type, Some(Linkage::Private));
        let entry = ctx.context.append_basic_block(function, "entry");
        ctx.builder.position_at_end(entry);
        define_body(ctx, builtin.name, function);
        ctx.push_func(builtin.name.to_string(), params);
    }
}

/// Calls the C function `name` and returns its result, if it has one.
fn call<'ctx>(ctx: &CodegenContext<'ctx>, name: &str, args: &[BasicMetadataValueEnum<'ctx>]) -> Option<BasicValueEnum<'ctx>> {
    ctx.builder.build_call(ctx.libc_function(name), args, name).unwrap().try_as_basic_value().basic()
}

fn int<'ctx>(ctx: &CodegenContext<'ctx>, bits: u32, value: u64) -> IntValue<'ctx> {
    let ty = match bits {
        32 => ctx.context.i32_type(),
        _ => ctx.context.i64_type(),
    };
    ty.const_int(value, false)
}

/// Prints `error: ` and the message made from `format` and `args`, then
/// stops the program with exit code 1.
fn build_fail<'ctx>(ctx: &CodegenContext<'ctx>, format: &str, args: &[BasicMetadataValueEnum<'ctx>]) {
    let format = ctx.builder.build_global_string_ptr(&format!("error: {}\n", format), "fail_format").unwrap();
    let mut all: Vec<BasicMetadataValueEnum> = vec![format.as_pointer_value().into()];
    all.extend_from_slice(args);
    call(ctx, "printf", &all);
    call(ctx, "exit", &[int(ctx, 32, 1).into()]);
    ctx.builder.build_unreachable().unwrap();
}

/// Builds code that fails with `format` and `args` if `failed` is true, and
/// goes on after it otherwise.
fn build_fail_if<'ctx>(ctx: &CodegenContext<'ctx>, function: FunctionValue<'ctx>, failed: IntValue<'ctx>, format: &str, args: &[BasicMetadataValueEnum<'ctx>]) {
    let fail_bb = ctx.context.append_basic_block(function, "fail");
    let ok_bb = ctx.context.append_basic_block(function, "ok");
    ctx.builder.build_conditional_branch(failed, fail_bb, ok_bb).unwrap();
    ctx.builder.position_at_end(fail_bb);
    build_fail(ctx, format, args);
    ctx.builder.position_at_end(ok_bb);
}

fn define_body<'ctx>(ctx: &CodegenContext<'ctx>, name: &str, function: FunctionValue<'ctx>) {
    let b = &ctx.builder;
    let i8_ty = ctx.context.i8_type();
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let param = |i: u32| function.get_nth_param(i).unwrap();
    // A `&Buffer` parameter points to the variable holding the Buffer
    let buffer = |i: u32| b.build_load(ptr_ty, param(i).into_pointer_value(), "buffer").unwrap().into_pointer_value();
    let capacity = |header| b.build_load(ctx.context.i64_type(), ctx.buffer_capacity_ptr(header), "capacity").unwrap().into_int_value();
    // Stops the program unless `count` bytes from `start` are in the Buffer `header`
    let check_range = |header, start: IntValue<'ctx>, count: IntValue<'ctx>| {
        let capacity = capacity(header);
        let start_past = b.build_int_compare(IntPredicate::UGT, start, capacity, "start_past").unwrap();
        let room = b.build_int_sub(capacity, start, "room").unwrap();
        let too_many = b.build_int_compare(IntPredicate::UGT, count, room, "too_many").unwrap();
        let out = b.build_or(start_past, too_many, "out_of_bounds").unwrap();
        build_fail_if(ctx, function, out, "%llu bytes from %llu are out of bounds for a Buffer of capacity %llu",
            &[count.into(), start.into(), capacity.into()]);
    };
    let at = |header, index| unsafe { b.build_in_bounds_gep(i8_ty, ctx.buffer_data(header), &[index], "at").unwrap() };

    match name {
        "copy_bytes" => {
            let (into, into_start) = (buffer(0), param(1).into_int_value());
            let (from, from_start) = (buffer(2), param(3).into_int_value());
            let count = param(4).into_int_value();
            check_range(into, into_start, count);
            check_range(from, from_start, count);
            // The two can be the same Buffer
            call(ctx, "memmove", &[at(into, into_start).into(), at(from, from_start).into(), count.into()]);
            b.build_return(None).unwrap();
        }
        "compare_bytes" => {
            let (a, a_start) = (buffer(0), param(1).into_int_value());
            let (b_buffer, b_start) = (buffer(2), param(3).into_int_value());
            let count = param(4).into_int_value();
            check_range(a, a_start, count);
            check_range(b_buffer, b_start, count);
            let order = call(ctx, "memcmp", &[at(a, a_start).into(), at(b_buffer, b_start).into(), count.into()]).unwrap();
            b.build_return(Some(&order)).unwrap();
        }
        "arg_count" => {
            let (argc, _) = argument_globals(ctx);
            let count = b.build_load(ctx.context.i32_type(), argc, "argc").unwrap();
            b.build_return(Some(&count)).unwrap();
        }
        "arg_bytes" => {
            let (argc, argv) = argument_globals(ctx);
            let index = param(0).into_int_value();
            let count = b.build_load(ctx.context.i32_type(), argc, "argc").unwrap().into_int_value();
            let negative = b.build_int_compare(IntPredicate::SLT, index, int(ctx, 32, 0), "negative").unwrap();
            let too_big = b.build_int_compare(IntPredicate::SGE, index, count, "too_big").unwrap();
            let out = b.build_or(negative, too_big, "out_of_range").unwrap();
            build_fail_if(ctx, function, out, "argument %d doesn't exist, there are %d", &[index.into(), count.into()]);

            let args = b.build_load(ptr_ty, argv, "argv").unwrap().into_pointer_value();
            let at = unsafe { b.build_in_bounds_gep(ptr_ty, args, &[index], "at").unwrap() };
            let arg = b.build_load(ptr_ty, at, "arg").unwrap().into_pointer_value();
            // A C string, copied into a Buffer of exactly its bytes
            let length = call(ctx, "strlen", &[arg.into()]).unwrap().into_int_value();
            let bytes = ctx.new_buffer(length, i8_ty.size_of());
            call(ctx, "memcpy", &[ctx.buffer_data(bytes).into(), arg.into(), length.into()]);
            b.build_return(Some(&bytes)).unwrap();
        }
        other => panic!("built-in '{}' has no body", other),
    }
}
