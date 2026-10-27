use std::{collections::HashMap, path::Path};

use inkwell::{AddressSpace, IntPredicate, basic_block::BasicBlock, builder::Builder, context::Context, module::{Linkage, Module}, targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine}, types::{BasicType, BasicTypeEnum, FunctionType, StructType}, values::{BasicValueEnum, FunctionValue, IntValue, PointerValue, StructValue}};

use crate::{field::Field, param::Param, runtime, span::Span, sym::{OwnedSlot, Symbol}, r#type::Type};

pub enum CodegenError {
    UndefinedClass(String),
    UndefinedFormatter(Type),
    UndefinedFunction(String),
    UndefinedTypeInit(String),
    UndefinedVariable(String),
    UnsupportedOperator(String),

    MissingArgument { index: usize, function: String },
    UndefinedField { class: String, field: String },
}

impl CodegenError {
    pub fn throw(self, source: &str, span: &Span) -> ! {
        let line_txt = source.lines().nth(span.line).unwrap_or("");
        let pointer = " ".repeat(span.col) + "^";
        let msg = match self {
            CodegenError::UndefinedClass(name) =>
                format!("undefined class '{}'", name),
            CodegenError::UndefinedFormatter(ty) =>
                format!("No formatter defined for {}", ty),
            CodegenError::UndefinedFunction(name) =>
                format!("undefined function '{}'", name),
            CodegenError::UndefinedTypeInit(name) =>
                format!("undefined type for variable '{}'", name),
            CodegenError::UndefinedVariable(name) =>
                format!("undefined variable '{}'", name),
            CodegenError::UnsupportedOperator(op) =>
                format!("unsupported binary operator '{}'", op),

            CodegenError::MissingArgument { index, function } =>
                format!("missing argument {} for '{}' and no default value is defined", index, function),
            CodegenError::UndefinedField { class, field } =>
                format!("class '{}' has no field '{}'", class, field),
        };
        panic!("\nerror: {}\n  --> {}:{}\n   |\n   | {}\n   | {}\n",
            msg, span.line + 1, span.col + 1, line_txt, pointer)
    }
}

pub struct CodegenContext<'ctx> {
    pub source: String,
    pub context: &'ctx Context,
    pub module: Module<'ctx>,
    pub builder: Builder<'ctx>,
    pub machine: TargetMachine,

    pub scope: HashMap<String, Symbol<'ctx>>,
    pub functions: HashMap<String, Vec<Param>>,
    pub classes: HashMap<String, Vec<(String, BasicTypeEnum<'ctx>)>>,
    /// Lava types of every class's fields in order, to find the ones owning heap memory.
    pub class_field_types: HashMap<String, Vec<Type>>,
    /// Lava payload types of every enum's variants in order, [`Type::Void`] for none.
    pub enum_variant_types: HashMap<String, Vec<Type>>,
    pub enums: HashMap<String, Vec<(String, BasicTypeEnum<'ctx>)>>,
    pub current_class: Option<String>,
    pub current_class_fields: Option<Vec<Field>>,
    /// Variables owning a `Unique` per nested scope of the current function, the
    /// innermost last. Each scope's values are freed when the scope ends.
    pub drop_scopes: Vec<Vec<OwnedSlot<'ctx>>>,
    /// Owning values an expression created but didn't keep, like the result of
    /// `make().x`, with their stack slots. They're freed when the expression ends.
    pub temporaries: Vec<(PointerValue<'ctx>, Type)>,
    /// The loops around the code being generated, the innermost last.
    pub loops: Vec<LoopTarget<'ctx>>,
    /// The C function of every `extern func` by mangled name, with the type
    /// it's called with. Several `extern func`s can call the same C function.
    pub externs: HashMap<String, (FunctionValue<'ctx>, FunctionType<'ctx>)>,
}

/// The LLVM name of the function `mangled`. Lava functions get their own
/// names, so they don't clash with C functions like `read` or with `main`,
/// the entry point running the top-level code.
pub fn function_symbol(mangled: &str) -> String {
    format!("lava.fn.{}", mangled)
}

/// The C functions the generated code uses itself, see [`CodegenContext::libc_function`].
const LIBC: &[&str] = &["calloc", "malloc", "memset", "realloc", "memcpy", "memmove", "memcmp", "free", "exit", "printf", "strlen"];

/// Where `break` and `continue` jump to in a loop.
#[derive(Clone, Copy)]
pub struct LoopTarget<'ctx> {
    pub continue_bb: BasicBlock<'ctx>,
    pub break_bb: BasicBlock<'ctx>,
    /// How many drop scopes there were outside the loop. Leaving a round frees
    /// the values of the scopes after them.
    pub drop_depth: usize,
    /// How many temporaries there were outside the loop, see `drop_depth`.
    pub temporary_depth: usize,
}

impl<'ctx> CodegenContext<'ctx> {
    pub fn new(context: &'ctx Context) -> Self {
        Target::initialize_native(&InitializationConfig::default())
            .expect("Failed to initialize native target");
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).expect("Unknown Target!");
        let machine = target.create_target_machine(
            &triple,
            "generic",
            "",
            inkwell::OptimizationLevel::None,
            RelocMode::PIC,
            CodeModel::Default,
        ).unwrap();
        let mut ctx = Self {
            source: "".to_string(),
            context,
            module: context.create_module("mod"),
            builder: context.create_builder(),
            machine,
            scope: HashMap::new(),
            functions: HashMap::new(),
            classes: HashMap::new(),
            class_field_types: HashMap::new(),
            enum_variant_types: HashMap::new(),
            enums: HashMap::new(),
            current_class: None,
            current_class_fields: None,
            // The top level of the program, freed at the end of `main`
            drop_scopes: vec![Vec::new()],
            temporaries: Vec::new(),
            loops: Vec::new(),
            externs: HashMap::new(),
        };
        // Declared first, so an `extern func` calling one of them finds it
        // instead of declaring it with its own types
        for name in LIBC {
            ctx.libc_function(name);
        }
        // Sema declares the same functions in `Sema::new`
        runtime::define(&mut ctx);
        ctx
    }

    /// Starts `main`, the code at the top level of the program goes into it.
    /// Its command line arguments are kept for the built-in `arg` functions.
    pub fn build_entry(&self) {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let main_fn_type = i32_type.fn_type(&[i32_type.into(), ptr_type.into()], false);
        let main_fn = self.module.add_function("main", main_fn_type, None);
        let entry_block = self.context.append_basic_block(main_fn, "entry_block");
        self.builder.position_at_end(entry_block);
        runtime::store_arguments(self, main_fn);
    }

    // --- Strings ---
    //
    // `String` is a class of the prelude, a `Buffer<uint8>` of bytes and their
    // length. Owning and copying strings works like for every class owning a
    // Buffer, its methods implement the string operators. The compiler only
    // makes literals with its constructor and reads its fields to print a
    // string or pass it to C.

    /// A new `String` with the text of the literal `text`, made by the
    /// constructor of the prelude's `String` from a Buffer of its bytes.
    pub fn string_literal(&self, text: &str) -> BasicValueEnum<'ctx> {
        let length = self.context.i64_type().const_int(text.len() as u64, false);
        let bytes = self.new_buffer(length, self.context.i8_type().size_of());

        // The text is copied from a constant of the program
        let constant = self.context.const_string(text.as_bytes(), false);
        let global = self.module.add_global(constant.get_type(), None, "literal");
        global.set_linkage(Linkage::Private);
        global.set_constant(true);
        global.set_initializer(&constant);
        let memcpy = self.libc_function("memcpy");
        self.builder.build_call(memcpy, &[self.buffer_data(bytes).into(), global.as_pointer_value().into(), length.into()], "").unwrap();
        self.string_from_bytes(bytes, length)
    }

    /// A new `String` of the first `length` bytes of the Buffer `bytes`, made
    /// by the constructor of the prelude's `String`, which takes the Buffer.
    fn string_from_bytes(&self, bytes: PointerValue<'ctx>, length: IntValue<'ctx>) -> BasicValueEnum<'ctx> {
        let constructor = self.module.get_function(&function_symbol("String"))
            .expect("the prelude declares the class String");
        self.builder.build_call(constructor, &[bytes.into(), length.into()], "string").unwrap()
            .try_as_basic_value().basic().unwrap()
    }

    /// The address of the bytes of `string`, a `String` value, and their
    /// length. The bytes belong to the string.
    ///
    /// Its field `bytes` is a `Buffer`, or a `Shared` one several strings use,
    /// and `start`, if it has one, says where its own bytes begin in it.
    pub fn string_parts(&self, string: StructValue<'ctx>) -> (PointerValue<'ctx>, IntValue<'ctx>) {
        let field = |name: &str| self.classes.get("String")
            .and_then(|fields| fields.iter().position(|(n, _)| n == name))
            .map(|i| i as u32);
        let required = |name: &str| field(name).unwrap_or_else(|| panic!("the prelude's String has no field '{}'", name));
        let bytes_field = required("bytes");
        let mut header = self.builder.build_extract_value(string, bytes_field, "bytes").unwrap().into_pointer_value();
        // A `Shared` points to where the Buffer is
        let shared = self.class_field_types.get("String")
            .and_then(|types| types.get(bytes_field as usize))
            .is_some_and(|ty| matches!(ty, Type::Shared(_)));
        if shared {
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            header = self.builder.build_load(ptr_ty, header, "buffer").unwrap().into_pointer_value();
        }
        let length = self.builder.build_extract_value(string, required("length"), "length").unwrap().into_int_value();
        let mut data = self.buffer_data(header);
        if let Some(start_field) = field("start") {
            let start = self.builder.build_extract_value(string, start_field, "start").unwrap().into_int_value();
            data = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), data, &[start], "own_bytes").unwrap() };
        }
        (data, length)
    }

    /// A copy of the text of `string`, a `String` value, on the heap with a
    /// zero byte after it, for C functions. The caller frees it.
    pub fn c_string(&self, string: StructValue<'ctx>) -> PointerValue<'ctx> {
        let (bytes, length) = self.string_parts(string);
        let size = self.builder.build_int_add(length, self.context.i64_type().const_int(1, false), "size").unwrap();
        let copy = self.builder.build_call(self.libc_function("malloc"), &[size.into()], "c_string").unwrap()
            .try_as_basic_value().basic().unwrap().into_pointer_value();
        self.builder.build_call(self.libc_function("memcpy"), &[copy.into(), bytes.into(), length.into()], "").unwrap();
        let end = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), copy, &[length], "end").unwrap() };
        self.builder.build_store(end, self.context.i8_type().const_zero()).unwrap();
        copy
    }

    /// Bytes before the value of a `Shared` holding its number of users. 16
    /// instead of 8, so the value stays aligned like `malloc`'s memory.
    const SHARED_HEADER: u64 = 16;

    /// A new `Shared` holding `value` of type `ty`, with one user. It points to
    /// the value like a `Unique`, the number of users is right before it.
    pub fn new_shared(&self, value: BasicValueEnum<'ctx>, ty: &Type) -> PointerValue<'ctx> {
        let i64_ty = self.context.i64_type();
        let size = ty.to_llvm(self).size_of().expect("shared value has no size");
        let total = self.builder.build_int_add(size, i64_ty.const_int(Self::SHARED_HEADER, false), "shared_size").unwrap();
        let block = self.builder.build_call(self.libc_function("malloc"), &[total.into()], "shared").unwrap()
            .try_as_basic_value().basic().unwrap().into_pointer_value();
        self.builder.build_store(block, i64_ty.const_int(1, false)).unwrap();
        let ptr = unsafe {
            self.builder.build_in_bounds_gep(self.context.i8_type(), block, &[i64_ty.const_int(Self::SHARED_HEADER, false)], "shared_value").unwrap()
        };
        self.builder.build_store(ptr, value).unwrap();
        ptr
    }

    /// The address of the number of users of the `Shared` pointing to `ptr`,
    /// which is also the start of its memory.
    fn shared_users(&self, ptr: PointerValue<'ctx>) -> PointerValue<'ctx> {
        let back = self.context.i64_type().const_int(Self::SHARED_HEADER, false).const_neg();
        unsafe { self.builder.build_gep(self.context.i8_type(), ptr, &[back], "shared_users").unwrap() }
    }

    /// A `String` with a copy of the text of `c_string`, a C string a C
    /// function returned. Empty if it's null. C keeps owning `c_string`.
    pub fn from_c_string(&self, c_string: PointerValue<'ctx>) -> StructValue<'ctx> {
        let empty = self.builder.build_global_string_ptr("", "empty").unwrap().as_pointer_value();
        let is_null = self.builder.build_is_null(c_string, "is_null").unwrap();
        let text = self.builder.build_select(is_null, empty, c_string, "text").unwrap().into_pointer_value();
        let length = self.builder.build_call(self.libc_function("strlen"), &[text.into()], "length").unwrap()
            .try_as_basic_value().basic().unwrap().into_int_value();
        let bytes = self.new_buffer(length, self.context.i8_type().size_of());
        self.builder.build_call(self.libc_function("memcpy"), &[self.buffer_data(bytes).into(), text.into(), length.into()], "").unwrap();
        self.string_from_bytes(bytes, length).into_struct_value()
    }

    /// A new `Buffer` with room for `capacity` elements of `element_size`
    /// bytes, all zero, so slots of owning elements start out empty.
    pub fn new_buffer(&self, capacity: IntValue<'ctx>, element_size: IntValue<'ctx>) -> PointerValue<'ctx> {
        let calloc = self.libc_function("calloc");
        let data = self.builder.build_call(calloc, &[capacity.into(), element_size.into()], "data").unwrap()
            .try_as_basic_value().basic().unwrap();
        let malloc = self.libc_function("malloc");
        let header_size = self.buffer_header_type().size_of().unwrap();
        let header = self.builder.build_call(malloc, &[header_size.into()], "buffer").unwrap()
            .try_as_basic_value().basic().unwrap().into_pointer_value();
        self.builder.build_store(self.buffer_capacity_ptr(header), capacity).unwrap();
        self.builder.build_store(self.buffer_data_ptr(header), data).unwrap();
        header
    }

    pub fn push_var(&mut self, name: String, ty: BasicTypeEnum<'ctx>, ptr: PointerValue<'ctx>, parent: Option<String>) {
        let symbol = Symbol { ty, ptr, class_name: parent, owned: None };
        self.scope.insert(name, symbol);
    }

    /// Adds the variable `name` owning a `Unique<inner>` stored in `slot`.
    ///
    /// The variable starts out owning its value and is freed at the end of the
    /// innermost scope, unless it gets moved before.
    pub fn push_owned_var(&mut self, name: String, slot: PointerValue<'ctx>, ty: Type) {
        let alive = self.build_entry_alloca(self.context.bool_type(), &format!("{}_alive", name));
        self.builder.build_store(alive, self.context.bool_type().const_int(1, false)).unwrap();

        let owned = OwnedSlot { slot, alive, ty: ty.clone() };
        self.drop_scopes.last_mut().expect("no scope to own the variable").push(owned.clone());
        let symbol = Symbol {
            // A `Unique` or `Buffer` slot holds a pointer, a class slot the object itself
            ty: ty.to_llvm(self),
            ptr: slot,
            class_name: ty.class_name(),
            owned: Some(owned),
        };
        self.scope.insert(name, symbol);
    }

    /// Whether values of `ty` own heap memory that has to be freed: a `Unique`,
    /// a `Buffer`, or a class or enum with a field or payload that does.
    pub fn owns_memory(&self, ty: &Type) -> bool {
        self.owns_memory_within(ty, 0)
    }

    /// [`CodegenContext::owns_memory`], giving up on types nested deeper than
    /// any real one, which can only be a type containing itself.
    fn owns_memory_within(&self, ty: &Type, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        match ty {
            Type::Unique(_) | Type::Shared(_) | Type::Buffer(_) => true,
            Type::Class(name) => self.class_field_types.get(name)
                .is_some_and(|fields| fields.iter().any(|f| self.owns_memory_within(f, depth + 1))),
            Type::Enum(name) => self.enum_variant_types.get(name)
                .is_some_and(|payloads| payloads.iter().any(|p| self.owns_memory_within(p, depth + 1))),
            Type::Array(element, _) => self.owns_memory_within(element, depth + 1),
            Type::Tuple(items) => items.iter().any(|item| self.owns_memory_within(item, depth + 1)),
            _ => false,
        }
    }

    /// Generates the function `lava.<kind>.<ty>` with `make` the first time and
    /// returns it. The builder goes back to where it was afterwards.
    ///
    /// Drop and clone code is generated once per type and called, so a type
    /// containing itself through a `Unique`, like a tree, doesn't expand forever.
    fn type_function(&self, kind: &str, ty: &Type, fn_type: FunctionType<'ctx>, make: impl FnOnce(FunctionValue<'ctx>)) -> FunctionValue<'ctx> {
        let name = format!("lava.{}.{}", kind, ty);
        if let Some(function) = self.module.get_function(&name) {
            return function;
        }
        let function = self.module.add_function(&name, fn_type, Some(Linkage::Private));
        let previous = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        make(function);
        if let Some(previous) = previous {
            self.builder.position_at_end(previous);
        }
        function
    }

    /// Emits a `switch` on the tag of the enum `name` at `place`, running
    /// `each` for every variant whose payload owns memory with the payload's
    /// place and type. Continues after the switch.
    fn for_owning_variant(&self, name: &str, place: PointerValue<'ctx>, mut each: impl FnMut(PointerValue<'ctx>, &Type)) {
        let function = self.current_function();
        let struct_ty = self.module.get_struct_type(name).expect("enum struct not defined");
        let tag_ptr = self.builder.build_struct_gep(struct_ty, place, 0, "tag_ptr").unwrap();
        let tag = self.builder.build_load(self.context.i32_type(), tag_ptr, "tag").unwrap().into_int_value();
        let done_bb = self.context.append_basic_block(function, "variants_done");

        let payloads = self.enum_variant_types.get(name).cloned().unwrap_or_default();
        let owning: Vec<(usize, Type)> = payloads.into_iter().enumerate().filter(|(_, p)| self.owns_memory(p)).collect();
        let blocks: Vec<_> = owning.iter().map(|_| self.context.append_basic_block(function, "owning_variant")).collect();
        let cases: Vec<_> = owning.iter().zip(&blocks)
            .map(|((i, _), bb)| (self.context.i32_type().const_int(*i as u64, false), *bb))
            .collect();
        self.builder.build_switch(tag, done_bb, &cases).unwrap();

        for ((_, payload), bb) in owning.iter().zip(blocks) {
            self.builder.position_at_end(bb);
            let payload_place = self.builder.build_struct_gep(struct_ty, place, 1, "payload").unwrap();
            each(payload_place, payload);
            self.builder.build_unconditional_branch(done_bb).unwrap();
        }
        self.builder.position_at_end(done_bb);
    }

    /// Frees the values still owned by `slots`, the most recently declared first.
    pub fn emit_drops(&self, slots: &[OwnedSlot<'ctx>]) {
        for owned in slots.iter().rev() {
            self.emit_drop(owned);
        }
    }

    /// Frees the values still owned in every scope of the current function,
    /// for leaving it with `return`.
    pub fn emit_all_drops(&self) {
        for scope in self.drop_scopes.iter().rev() {
            self.emit_drops(scope);
        }
    }

    /// Frees the value of `owned` if the variable still owns it.
    pub fn emit_drop(&self, owned: &OwnedSlot<'ctx>) {
        let function = self.builder.get_insert_block().and_then(|b| b.get_parent())
            .expect("dropping outside of a function");
        let free_bb = self.context.append_basic_block(function, "drop");
        let done_bb = self.context.append_basic_block(function, "dropped");

        let alive = self.builder.build_load(self.context.bool_type(), owned.alive, "alive").unwrap().into_int_value();
        self.builder.build_conditional_branch(alive, free_bb, done_bb).unwrap();

        self.builder.position_at_end(free_bb);
        self.drop_in_place(owned.slot, &owned.ty);
        self.builder.build_unconditional_branch(done_bb).unwrap();

        self.builder.position_at_end(done_bb);
    }

    /// Frees the heap memory owned by the value of type `ty` stored at `place`,
    /// leaving the value itself where it is.
    ///
    /// For a `Unique` or `Buffer` that's the memory its pointer points to, for
    /// a class the memory owned by its fields, for an enum the memory owned
    /// by the payload of its variant. A null pointer is an empty slot of a
    /// `Buffer` and owns nothing.
    pub fn drop_in_place(&self, place: PointerValue<'ctx>, ty: &Type) {
        if !self.owns_memory(ty) {
            return;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let fn_type = self.context.void_type().fn_type(&[ptr_ty.into()], false);
        let function = self.type_function("drop", ty, fn_type, |function| {
            let place = function.get_nth_param(0).unwrap().into_pointer_value();
            match ty {
                Type::Unique(_) | Type::Shared(_) | Type::Buffer(_) => {
                    let ptr = self.builder.build_load(ptr_ty, place, "owned").unwrap().into_pointer_value();
                    self.build_if_not_null(ptr, || self.free_owned(ptr, ty));
                }
                Type::Class(name) => {
                    let struct_ty = self.module.get_struct_type(name).expect("struct not defined");
                    let fields = self.class_field_types.get(name).cloned().unwrap_or_default();
                    for (i, field) in fields.iter().enumerate().filter(|(_, f)| self.owns_memory(f)) {
                        let field_place = self.builder.build_struct_gep(struct_ty, place, i as u32, "owned_field").unwrap();
                        self.drop_in_place(field_place, field);
                    }
                }
                Type::Enum(name) => self.for_owning_variant(name, place, |payload, payload_ty| self.drop_in_place(payload, payload_ty)),
                Type::Tuple(items) => {
                    let struct_ty = ty.to_llvm(self).into_struct_type();
                    for (i, item) in items.iter().enumerate().filter(|(_, item)| self.owns_memory(item)) {
                        let item_place = self.builder.build_struct_gep(struct_ty, place, i as u32, "owned_item").unwrap();
                        self.drop_in_place(item_place, item);
                    }
                }
                Type::Array(element, len) => {
                    let array_ty = ty.to_llvm(self);
                    let i64_ty = self.context.i64_type();
                    self.build_index_loop(i64_ty.const_zero(), i64_ty.const_int(*len, false), |i| {
                        let slot = unsafe { self.builder.build_in_bounds_gep(array_ty, place, &[i64_ty.const_zero(), i], "element").unwrap() };
                        self.drop_in_place(slot, element);
                    });
                }
                _ => {}
            }
            self.builder.build_return(None).unwrap();
        });
        self.builder.build_call(function, &[place.into()], "").unwrap();
    }

    /// Frees the heap memory `ptr` of the owning type `ty` points to.
    ///
    /// A `Unique` first frees what its value owns itself, like the inner one
    /// of a `Unique<Unique<int32>>`. A `Buffer` frees what its elements own,
    /// its elements' block and its header.
    pub fn free_owned(&self, ptr: PointerValue<'ctx>, ty: &Type) {
        let free = self.libc_function("free");
        match ty {
            Type::Unique(inner) => {
                self.drop_in_place(ptr, inner);
                self.builder.build_call(free, &[ptr.into()], "").unwrap();
            }
            // One user less, the last one frees the value
            Type::Shared(inner) => {
                let i64_ty = self.context.i64_type();
                let users_ptr = self.shared_users(ptr);
                let users = self.builder.build_load(i64_ty, users_ptr, "users").unwrap().into_int_value();
                let left = self.builder.build_int_sub(users, i64_ty.const_int(1, false), "users_left").unwrap();
                self.builder.build_store(users_ptr, left).unwrap();

                let function = self.current_function();
                let free_bb = self.context.append_basic_block(function, "last_user");
                let done_bb = self.context.append_basic_block(function, "shared_dropped");
                let last = self.builder.build_int_compare(IntPredicate::EQ, left, i64_ty.const_zero(), "last").unwrap();
                self.builder.build_conditional_branch(last, free_bb, done_bb).unwrap();
                self.builder.position_at_end(free_bb);
                self.drop_in_place(ptr, inner);
                self.builder.build_call(free, &[users_ptr.into()], "").unwrap();
                self.builder.build_unconditional_branch(done_bb).unwrap();
                self.builder.position_at_end(done_bb);
            }
            Type::Buffer(element) => {
                if self.owns_memory(element) {
                    let capacity = self.builder.build_load(self.context.i64_type(), self.buffer_capacity_ptr(ptr), "capacity")
                        .unwrap().into_int_value();
                    let start = self.context.i64_type().const_zero();
                    self.build_index_loop(start, capacity, |i| self.drop_in_place(self.buffer_slot(ptr, element, i), element));
                }
                let data = self.buffer_data(ptr);
                self.builder.build_call(free, &[data.into()], "").unwrap();
                self.builder.build_call(free, &[ptr.into()], "").unwrap();
            }
            other => panic!("'{}' doesn't own heap memory", other),
        }
    }

    /// Copies `value` of type `ty` including the heap memory it owns, so the
    /// copy owns memory of its own and both can be freed separately.
    ///
    /// A `Unique` gets a new allocation with a copy of its value, a `Buffer` a
    /// new block with all elements copied, and a class copies its owning
    /// fields and an enum the payload of its variant that way. A null `Unique`
    /// or `Buffer`, an empty slot of a `Buffer`, stays null. Other values are
    /// returned as they are.
    pub fn deep_clone(&self, value: BasicValueEnum<'ctx>, ty: &Type) -> BasicValueEnum<'ctx> {
        if !self.owns_memory(ty) {
            return value;
        }
        let llvm_ty = ty.to_llvm(self);
        let fn_type = llvm_ty.fn_type(&[llvm_ty.into()], false);
        let function = self.type_function("clone", ty, fn_type, |function| {
            let original = function.get_nth_param(0).unwrap();
            let copy = self.build_clone(original, ty);
            self.builder.build_return(Some(&copy)).unwrap();
        });
        self.builder.build_call(function, &[value.into()], "copy").unwrap().try_as_basic_value().basic().unwrap()
    }

    /// The body of the clone function for `ty`, see [`CodegenContext::deep_clone`].
    fn build_clone(&self, value: BasicValueEnum<'ctx>, ty: &Type) -> BasicValueEnum<'ctx> {
        let malloc = self.libc_function("malloc");
        match ty {
            Type::Unique(inner) => {
                let ptr = value.into_pointer_value();
                self.build_clone_unless_null(ptr, || {
                    let inner_ty = inner.to_llvm(self);
                    let inner_value = self.builder.build_load(inner_ty, ptr, "unique_value").unwrap();
                    let copied = self.deep_clone(inner_value, inner);
                    let size = inner_ty.size_of().expect("unique value has no size");
                    let heap = self.builder.build_call(malloc, &[size.into()], "unique_copy").unwrap()
                        .try_as_basic_value().basic().unwrap().into_pointer_value();
                    self.builder.build_store(heap, copied).unwrap();
                    heap
                }).into()
            }
            // The copy uses the same value, it has one user more
            Type::Shared(_) => {
                let ptr = value.into_pointer_value();
                self.build_clone_unless_null(ptr, || {
                    let i64_ty = self.context.i64_type();
                    let users_ptr = self.shared_users(ptr);
                    let users = self.builder.build_load(i64_ty, users_ptr, "users").unwrap().into_int_value();
                    let more = self.builder.build_int_add(users, i64_ty.const_int(1, false), "users_more").unwrap();
                    self.builder.build_store(users_ptr, more).unwrap();
                    ptr
                }).into()
            }
            Type::Buffer(element) => {
                let header = value.into_pointer_value();
                self.build_clone_unless_null(header, || {
                    let capacity = self.builder.build_load(self.context.i64_type(), self.buffer_capacity_ptr(header), "capacity")
                        .unwrap().into_int_value();
                    let element_size = element.to_llvm(self).size_of().expect("Buffer element has no size");
                    let bytes = self.builder.build_int_mul(capacity, element_size, "bytes").unwrap();

                    let data = self.builder.build_call(malloc, &[bytes.into()], "data_copy").unwrap()
                        .try_as_basic_value().basic().unwrap();
                    let memcpy = self.libc_function("memcpy");
                    self.builder.build_call(memcpy, &[data.into(), self.buffer_data(header).into(), bytes.into()], "").unwrap();

                    let header_size = self.buffer_header_type().size_of().unwrap();
                    let new_header = self.builder.build_call(malloc, &[header_size.into()], "buffer_copy").unwrap()
                        .try_as_basic_value().basic().unwrap().into_pointer_value();
                    self.builder.build_store(self.buffer_capacity_ptr(new_header), capacity).unwrap();
                    self.builder.build_store(self.buffer_data_ptr(new_header), data).unwrap();

                    // Elements owning memory get copies of it instead of sharing it with the original
                    if self.owns_memory(element) {
                        let element_ty = element.to_llvm(self);
                        let start = self.context.i64_type().const_zero();
                        self.build_index_loop(start, capacity, |i| {
                            let slot = self.buffer_slot(new_header, element, i);
                            let original = self.builder.build_load(element_ty, slot, "element").unwrap();
                            self.builder.build_store(slot, self.deep_clone(original, element)).unwrap();
                        });
                    }
                    new_header
                }).into()
            }
            Type::Tuple(items) => {
                let mut tuple = value.into_struct_value();
                for (i, item) in items.iter().enumerate().filter(|(_, item)| self.owns_memory(item)) {
                    let original = self.builder.build_extract_value(tuple, i as u32, "item").unwrap();
                    let copied = self.deep_clone(original, item);
                    tuple = self.builder.build_insert_value(tuple, copied, i as u32, "tuple_copy").unwrap().into_struct_value();
                }
                tuple.into()
            }
            Type::Class(name) if self.owns_memory(ty) => {
                let mut object = value.into_struct_value();
                let fields = self.class_field_types.get(name).cloned().unwrap_or_default();
                for (i, field) in fields.iter().enumerate().filter(|(_, f)| self.owns_memory(f)) {
                    let field_value = self.builder.build_extract_value(object, i as u32, "field").unwrap();
                    let copied = self.deep_clone(field_value, field);
                    object = self.builder.build_insert_value(object, copied, i as u32, "object_copy").unwrap()
                        .into_struct_value();
                }
                object.into()
            }
            Type::Array(element, len) => {
                // Every element is copied in place in a copy of the whole array
                let place = self.build_entry_alloca(value.get_type(), "array_copy");
                self.builder.build_store(place, value).unwrap();
                let i64_ty = self.context.i64_type();
                let element_ty = element.to_llvm(self);
                self.build_index_loop(i64_ty.const_zero(), i64_ty.const_int(*len, false), |i| {
                    let slot = unsafe { self.builder.build_in_bounds_gep(value.get_type(), place, &[i64_ty.const_zero(), i], "element").unwrap() };
                    let original = self.builder.build_load(element_ty, slot, "element").unwrap();
                    self.builder.build_store(slot, self.deep_clone(original, element)).unwrap();
                });
                self.builder.build_load(value.get_type(), place, "array_copied").unwrap()
            }
            Type::Enum(name) => {
                // The payload is behind the tag, it's copied through memory
                let place = self.build_entry_alloca(value.get_type(), "enum_copy");
                self.builder.build_store(place, value).unwrap();
                self.for_owning_variant(name, place, |payload, payload_ty| {
                    let original = self.builder.build_load(payload_ty.to_llvm(self), payload, "payload").unwrap();
                    self.builder.build_store(payload, self.deep_clone(original, payload_ty)).unwrap();
                });
                self.builder.build_load(value.get_type(), place, "enum_copied").unwrap()
            }
            _ => value,
        }
    }

    /// Layout of a `&[T]`: the address of its first element and how many there are.
    pub fn slice_type(&self) -> StructType<'ctx> {
        self.context.struct_type(&[self.context.ptr_type(AddressSpace::default()).into(), self.context.i64_type().into()], false)
    }

    /// Stops the program with an error if `index` isn't below `len`, the
    /// length of a `&[T]`.
    pub fn build_slice_bounds_check(&self, index: IntValue<'ctx>, len: IntValue<'ctx>) {
        self.build_index_check(index, len, "error: index %lld is out of bounds for a slice of length %llu\n");
    }

    /// Layout of a `Buffer`'s header on the heap: its capacity and a pointer to
    /// the block of elements. A `Buffer` value is a pointer to this header.
    pub fn buffer_header_type(&self) -> StructType<'ctx> {
        self.context.struct_type(&[self.context.i64_type().into(), self.context.ptr_type(AddressSpace::default()).into()], false)
    }

    /// Address of the capacity in the header of the `Buffer` at `header`.
    pub fn buffer_capacity_ptr(&self, header: PointerValue<'ctx>) -> PointerValue<'ctx> {
        self.builder.build_struct_gep(self.buffer_header_type(), header, 0, "capacity_ptr").unwrap()
    }

    /// Address of the elements pointer in the header of the `Buffer` at `header`.
    pub fn buffer_data_ptr(&self, header: PointerValue<'ctx>) -> PointerValue<'ctx> {
        self.builder.build_struct_gep(self.buffer_header_type(), header, 1, "data_ptr").unwrap()
    }

    /// The block of elements of the `Buffer` at `header`.
    pub fn buffer_data(&self, header: PointerValue<'ctx>) -> PointerValue<'ctx> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        self.builder.build_load(ptr_ty, self.buffer_data_ptr(header), "data").unwrap().into_pointer_value()
    }

    /// Address of the element at `index` of the `Buffer` at `header`, without
    /// a bounds check.
    pub fn buffer_slot(&self, header: PointerValue<'ctx>, element: &Type, index: IntValue<'ctx>) -> PointerValue<'ctx> {
        let data = self.buffer_data(header);
        unsafe { self.builder.build_in_bounds_gep(element.to_llvm(self), data, &[index], "slot").unwrap() }
    }

    /// Allocates a stack slot at the start of the current function, so a slot
    /// for a value inside a loop is made once instead of once per round.
    pub fn build_entry_alloca(&self, ty: impl BasicType<'ctx>, name: &str) -> PointerValue<'ctx> {
        let entry = self.current_function().get_first_basic_block().expect("function without an entry block");
        let builder = self.context.create_builder();
        match entry.get_first_instruction() {
            Some(first) => builder.position_before(&first),
            None => builder.position_at_end(entry),
        }
        builder.build_alloca(ty, name).unwrap()
    }

    /// Keeps the owning `value` of type `ty` until the current expression ends.
    pub fn push_temporary(&mut self, value: BasicValueEnum<'ctx>, ty: &Type) {
        let slot = self.build_entry_alloca(value.get_type(), "temporary");
        self.builder.build_store(slot, value).unwrap();
        self.temporaries.push((slot, ty.clone()));
    }

    /// Moves the value of type `ty` at `place` out: returns it and leaves the
    /// place empty, all zeroes. Every owning part of an empty value is a null
    /// pointer, so freeing its owner later frees nothing of the moved value.
    pub fn move_out(&self, place: PointerValue<'ctx>, ty: &Type) -> BasicValueEnum<'ctx> {
        let llvm_ty = ty.to_llvm(self);
        let value = self.builder.build_load(llvm_ty, place, "moved").unwrap();
        if self.owns_memory(ty) {
            self.builder.build_store(place, llvm_ty.const_zero()).unwrap();
        }
        value
    }

    /// Frees the temporaries kept since there were `mark` of them, for jumping
    /// out of the expressions they belong to with `return` or `break`. They
    /// stay kept, the code that doesn't jump frees them as usual.
    pub fn emit_temporary_drops(&self, mark: usize) {
        for (slot, ty) in self.temporaries[mark..].iter().rev() {
            self.drop_in_place(*slot, ty);
        }
    }

    /// Frees the temporaries kept since there were `mark` of them.
    pub fn drop_temporaries(&mut self, mark: usize) {
        for (slot, ty) in self.temporaries.split_off(mark).iter().rev() {
            self.drop_in_place(*slot, ty);
        }
    }

    /// The function the builder currently emits into.
    fn current_function(&self) -> FunctionValue<'ctx> {
        self.builder.get_insert_block().and_then(|b| b.get_parent()).expect("emitting code outside of a function")
    }

    /// Emits a loop running `body` for every `i64` index from `start` up to,
    /// but not including, `end`.
    pub fn build_index_loop(&self, start: IntValue<'ctx>, end: IntValue<'ctx>, body: impl FnOnce(IntValue<'ctx>)) {
        let function = self.current_function();
        let before_bb = self.builder.get_insert_block().unwrap();
        let cond_bb = self.context.append_basic_block(function, "loop_cond");
        let body_bb = self.context.append_basic_block(function, "loop_body");
        let done_bb = self.context.append_basic_block(function, "loop_done");
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        self.builder.position_at_end(cond_bb);
        let index = self.builder.build_phi(self.context.i64_type(), "index").unwrap();
        index.add_incoming(&[(&start, before_bb)]);
        let i = index.as_basic_value().into_int_value();
        let more = self.builder.build_int_compare(IntPredicate::ULT, i, end, "more").unwrap();
        self.builder.build_conditional_branch(more, body_bb, done_bb).unwrap();

        self.builder.position_at_end(body_bb);
        body(i);
        let next = self.builder.build_int_add(i, self.context.i64_type().const_int(1, false), "next").unwrap();
        // The body can end in another block than it started in
        index.add_incoming(&[(&next, self.builder.get_insert_block().unwrap())]);
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        self.builder.position_at_end(done_bb);
    }

    /// Emits `then` so it only runs if `ptr` isn't null.
    fn build_if_not_null(&self, ptr: PointerValue<'ctx>, then: impl FnOnce()) {
        let function = self.current_function();
        let then_bb = self.context.append_basic_block(function, "not_null");
        let done_bb = self.context.append_basic_block(function, "null_checked");
        let is_null = self.builder.build_is_null(ptr, "is_null").unwrap();
        self.builder.build_conditional_branch(is_null, done_bb, then_bb).unwrap();

        self.builder.position_at_end(then_bb);
        then();
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(done_bb);
    }

    /// Returns the pointer `clone` builds if `ptr` isn't null, else null.
    fn build_clone_unless_null(&self, ptr: PointerValue<'ctx>, clone: impl FnOnce() -> PointerValue<'ctx>) -> PointerValue<'ctx> {
        let function = self.current_function();
        let before_bb = self.builder.get_insert_block().unwrap();
        let clone_bb = self.context.append_basic_block(function, "clone");
        let done_bb = self.context.append_basic_block(function, "cloned");
        let is_null = self.builder.build_is_null(ptr, "is_null").unwrap();
        self.builder.build_conditional_branch(is_null, done_bb, clone_bb).unwrap();

        self.builder.position_at_end(clone_bb);
        let copy = clone();
        let clone_end_bb = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(done_bb).unwrap();

        self.builder.position_at_end(done_bb);
        let result = self.builder.build_phi(ptr.get_type(), "copy").unwrap();
        result.add_incoming(&[(&ptr, before_bb), (&copy, clone_end_bb)]);
        result.as_basic_value().into_pointer_value()
    }

    /// The C function `name` of [`LIBC`], like `malloc` (`size -> ptr`) or
    /// `free` (`ptr -> void`), declared the first time it's used.
    pub fn libc_function(&self, name: &str) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function(name) {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let fn_type = match name {
            "calloc" => ptr_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false),
            "malloc" => ptr_ty.fn_type(&[i64_ty.into()], false),
            "memset" => ptr_ty.fn_type(&[ptr_ty.into(), self.context.i32_type().into(), i64_ty.into()], false),
            "realloc" => ptr_ty.fn_type(&[ptr_ty.into(), i64_ty.into()], false),
            "memcpy" | "memmove" => ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false),
            "free" => self.context.void_type().fn_type(&[ptr_ty.into()], false),
            "exit" => self.context.void_type().fn_type(&[self.context.i32_type().into()], false),
            "printf" => self.context.i32_type().fn_type(&[ptr_ty.into()], true),
            "strlen" => i64_ty.fn_type(&[ptr_ty.into()], false),
            "memcmp" => self.context.i32_type().fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false),
            other => panic!("no libc function '{}' declared", other),
        };
        self.module.add_function(name, fn_type, Some(Linkage::External))
    }

    /// Stops the program with an error if `index` isn't below the capacity of
    /// the `Buffer` at `header`, like a Rust panic.
    pub fn build_bounds_check(&self, header: PointerValue<'ctx>, index: IntValue<'ctx>) {
        let capacity = self.builder.build_load(self.context.i64_type(), self.buffer_capacity_ptr(header), "capacity")
            .unwrap().into_int_value();
        self.build_index_check(index, capacity, "error: index %llu is out of bounds for a Buffer of capacity %llu\n");
    }

    /// Stops the program with an error if the 64 bit `index` isn't below `len`,
    /// the length of an array. A negative index is out of bounds too.
    pub fn build_array_bounds_check(&self, index: IntValue<'ctx>, len: u64) {
        let len = self.context.i64_type().const_int(len, false);
        self.build_index_check(index, len, "error: index %lld is out of bounds for an array of length %llu\n");
    }

    /// Stops the program, printing `format` with `index` and `len`, if `index`
    /// isn't below `len` when compared unsigned.
    fn build_index_check(&self, index: IntValue<'ctx>, len: IntValue<'ctx>, format: &str) {
        let out_of_bounds = self.builder.build_int_compare(IntPredicate::UGE, index, len, "out_of_bounds").unwrap();

        let function = self.current_function();
        let panic_bb = self.context.append_basic_block(function, "out_of_bounds");
        let ok_bb = self.context.append_basic_block(function, "in_bounds");
        self.builder.build_conditional_branch(out_of_bounds, panic_bb, ok_bb).unwrap();

        self.builder.position_at_end(panic_bb);
        let message = self.builder.build_global_string_ptr(format, "bounds_msg").unwrap();
        let printf = self.libc_function("printf");
        self.builder.build_call(printf, &[message.as_pointer_value().into(), index.into(), len.into()], "").unwrap();
        let exit = self.libc_function("exit");
        self.builder.build_call(exit, &[self.context.i32_type().const_int(1, false).into()], "").unwrap();
        self.builder.build_unreachable().unwrap();

        self.builder.position_at_end(ok_bb);
    }

    pub fn push_func(&mut self, name: String, params: Vec<Param>) {
        self.functions.insert(name, params);
    }

    /// Returns the struct type of the class or enum `name`, creating it without
    /// a body the first time. The body is set when the class or enum is declared.
    pub fn struct_type(&self, name: &str) -> StructType<'ctx> {
        self.module.get_struct_type(name)
            .unwrap_or_else(|| self.context.opaque_struct_type(name))
    }

    pub fn push_class(&mut self, name: String, fields: Vec<(String, BasicTypeEnum<'ctx>)>) {
        self.classes.insert(name, fields);
    }

    pub fn push_enum(&mut self, name: String, fields: Vec<(String, BasicTypeEnum<'ctx>)>) {
        self.enums.insert(name, fields);
    }

    pub fn load_func_params(&self, name: &str) -> Option<&Vec<Param>> {
        self.functions.get(name)
    }

    pub fn func_has_self(&self, name: &str) -> bool {
        let params = self.functions.get(name).unwrap();
        if params.len() == 0 {
            return false;
        }

        params[0].name == "self"
    }

    pub fn load_sym(&mut self, name: &str) -> Option<&Symbol<'ctx>> {
        self.scope.get(name)
    }

    pub fn load_var(&mut self, name: &str) -> Option<BasicValueEnum<'ctx>> {
        let symbol = self.scope.get(name)?;
        let ty = symbol.ty;
        let ptr = symbol.ptr;
        Some(self.builder.build_load(ty, ptr, name).unwrap())
    }

    pub fn load_var_ptr(&mut self, name: &str) -> Option<PointerValue<'ctx>> {
        match self.scope.get(name) {
            Some(sym) => Some(sym.ptr),
            None => None
        }
    }

    /// Finishes `main` and writes the object file to `object`.
    pub fn compile(&self, object: &Path) {
        // Top level `Unique`s live until the end of the program
        self.emit_all_drops();
        let exit_code = self.context.i32_type().const_int(0, false);
        self.builder.build_return(Some(&exit_code)).unwrap();
        if let Err(e) = self.module.verify() {
            self.module.print_to_stderr();
            panic!("LLVM module verification failed:\n{}", e.to_string());
        }
        self.machine.write_to_file(&self.module, FileType::Object, object)
            .unwrap_or_else(|e| panic!("error: can't write '{}': {}", object.display(), e));
    }
}
