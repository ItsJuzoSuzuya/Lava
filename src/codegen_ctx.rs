use std::{collections::HashMap, path::Path};

use inkwell::{AddressSpace, IntPredicate, builder::Builder, context::Context, intrinsics::Intrinsic, module::{Linkage, Module}, targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine}, types::{BasicType, BasicTypeEnum, StructType}, values::{BasicValueEnum, FunctionValue, IntValue, PointerValue}};

use crate::{field::Field, param::Param, span::Span, sym::{OwnedSlot, Symbol}, r#type::Type};

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
    pub enums: HashMap<String, Vec<(String, BasicTypeEnum<'ctx>)>>,
    pub current_class: Option<String>,
    pub current_class_fields: Option<Vec<Field>>,
    /// Variables owning a `Unique` per nested scope of the current function, the
    /// innermost last. Each scope's values are freed when the scope ends.
    pub drop_scopes: Vec<Vec<OwnedSlot<'ctx>>>,
    /// Owning values an expression created but didn't keep, like the result of
    /// `make().x`, with their stack slots. They're freed when the expression ends.
    pub temporaries: Vec<(PointerValue<'ctx>, Type)>,
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
            enums: HashMap::new(),
            current_class: None,
            current_class_fields: None,
            // The top level of the program, freed at the end of `main`
            drop_scopes: vec![Vec::new()],
            temporaries: Vec::new(),
        };
        ctx.define_builtins();
        ctx
    }

    /// Defines the functions built into the language.
    ///
    /// Sema declares the same functions in [`Sema::new`](crate::sema::Sema::new).
    fn define_builtins(&mut self) {
        let f64_type = self.context.f64_type();

        // sqrt(x: float64) -> float64. Private linkage keeps it from clashing
        // with libm's `sqrt(double)` when linking.
        let sqrt_fn = self.module.add_function("sqrt", f64_type.fn_type(&[f64_type.into()], false), Some(Linkage::Private));
        let entry = self.context.append_basic_block(sqrt_fn, "entry");
        self.builder.position_at_end(entry);

        let x = sqrt_fn.get_nth_param(0).unwrap().into_float_value();
        let llvm_sqrt = Intrinsic::find("llvm.sqrt").unwrap()
            .get_declaration(&self.module, &[f64_type.into()]).unwrap();
        let root = self.builder.build_call(llvm_sqrt, &[x.into()], "root").unwrap()
            .try_as_basic_value().basic().unwrap();
        self.builder.build_return(Some(&root)).unwrap();

        self.push_func("sqrt".to_string(), vec![Param::builtin("x", Type::Float { bits: 64 })]);
    }

    pub fn build_entry(&self) {
        let i32_type = self.context.i32_type();
        let main_fn_type = i32_type.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_fn_type, None);
        let entry_block = self.context.append_basic_block(main_fn, "entry_block");
        self.builder.position_at_end(entry_block);
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
    /// a `Buffer`, or a class with a field that does.
    pub fn owns_memory(&self, ty: &Type) -> bool {
        match ty {
            Type::Unique(_) | Type::Buffer(_) => true,
            Type::Class(name) => self.class_field_types.get(name)
                .is_some_and(|fields| fields.iter().any(|f| self.owns_memory(f))),
            _ => false,
        }
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
    /// a class the memory owned by its fields. A null pointer is an empty slot
    /// of a `Buffer` and owns nothing.
    pub fn drop_in_place(&self, place: PointerValue<'ctx>, ty: &Type) {
        match ty {
            Type::Unique(_) | Type::Buffer(_) => {
                let ptr_ty = self.context.ptr_type(AddressSpace::default());
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
            _ => {}
        }
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
    /// fields that way. A null `Unique` or `Buffer`, an empty slot of a
    /// `Buffer`, stays null. Other values are returned as they are.
    pub fn deep_clone(&self, value: BasicValueEnum<'ctx>, ty: &Type) -> BasicValueEnum<'ctx> {
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
            _ => value,
        }
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

    /// Declares the C function `malloc` (`size -> ptr`) or `free` (`ptr -> void`)
    /// the first time it's used.
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
            "memcpy" => ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false),
            "free" => self.context.void_type().fn_type(&[ptr_ty.into()], false),
            "exit" => self.context.void_type().fn_type(&[self.context.i32_type().into()], false),
            "printf" => self.context.i32_type().fn_type(&[ptr_ty.into()], true),
            other => panic!("no libc function '{}' declared", other),
        };
        self.module.add_function(name, fn_type, Some(Linkage::External))
    }

    /// Stops the program with an error if `index` isn't below the capacity of
    /// the `Buffer` at `header`, like a Rust panic.
    pub fn build_bounds_check(&self, header: PointerValue<'ctx>, index: IntValue<'ctx>) {
        let capacity = self.builder.build_load(self.context.i64_type(), self.buffer_capacity_ptr(header), "capacity")
            .unwrap().into_int_value();
        let out_of_bounds = self.builder.build_int_compare(IntPredicate::UGE, index, capacity, "out_of_bounds").unwrap();

        let function = self.builder.get_insert_block().and_then(|b| b.get_parent())
            .expect("bounds check outside of a function");
        let panic_bb = self.context.append_basic_block(function, "out_of_bounds");
        let ok_bb = self.context.append_basic_block(function, "in_bounds");
        self.builder.build_conditional_branch(out_of_bounds, panic_bb, ok_bb).unwrap();

        self.builder.position_at_end(panic_bb);
        let message = self.builder
            .build_global_string_ptr("error: index %llu is out of bounds for a Buffer of capacity %llu\n", "bounds_msg")
            .unwrap();
        let printf = self.libc_function("printf");
        self.builder.build_call(printf, &[message.as_pointer_value().into(), index.into(), capacity.into()], "").unwrap();
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

    pub fn compile(&self) {
        // Top level `Unique`s live until the end of the program
        self.emit_all_drops();
        let exit_code = self.context.i32_type().const_int(0, false);
        self.builder.build_return(Some(&exit_code)).unwrap();
        if let Err(e) = self.module.verify() {
            self.module.print_to_stderr();
            panic!("LLVM module verification failed:\n{}", e.to_string());
        }
        self.machine.write_to_file(&self.module, FileType::Object, Path::new("./main.o")).unwrap();
    }
}
