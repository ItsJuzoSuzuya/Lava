use std::{collections::HashMap, path::Path};

use inkwell::{builder::Builder, context::Context, module::Module, targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine}, types::BasicTypeEnum, values::{BasicValueEnum, PointerValue}};

use crate::{statement::Param};

pub struct Symbol<'ctx> {
  pub ty: BasicTypeEnum<'ctx>,
  pub ptr: PointerValue<'ctx>,
  pub mutable: bool,
}

pub struct CodegenContext<'ctx>  {
    pub context: &'ctx Context,
    pub module: Module<'ctx>,
    pub builder: Builder<'ctx>,
    pub machine: TargetMachine,

    pub scope: HashMap<String, Symbol<'ctx>>,
    pub functions: HashMap<String, Vec<Param>>,
    pub classes: HashMap<String, Vec<BasicTypeEnum<'ctx>>>
}

impl<'ctx> CodegenContext<'ctx> {
    pub fn new(context: &'ctx Context) -> Self {
        Target::initialize_native(&InitializationConfig::default())
            .expect("Failed to initialize native target");
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).expect("Unknown Target!");
        let machine = target.
            create_target_machine(
                &triple, 
                "generic", 
                "", 
                inkwell::OptimizationLevel::None, 
                RelocMode::Default, 
                CodeModel::Default
            ).unwrap();
        Self {
            context: context,
            module: context.create_module("mod"),
            builder: context.create_builder(),
            machine: machine,
            scope: HashMap::new(),
            functions: HashMap::new(),
            classes: HashMap::new()
        }
    }

    pub fn build_entry(&self){
        // Create Main Function
        let i32_type = self.context.i32_type();
        let main_fn_type = i32_type.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_fn_type, None);

        // Insert Basic Block
        let entry_block = self.context.append_basic_block(main_fn, "entry_block");
        self.builder.position_at_end(entry_block);
    }

    pub fn push_var(&mut self, name: String, ty: BasicTypeEnum<'ctx>, ptr: PointerValue<'ctx>){
        let symbol = Symbol{ ty:ty, ptr:ptr, mutable:true};
        self.scope.insert(name, symbol);
    }

    pub fn push_func(&mut self, name: String, params: Vec<Param>){
        self.functions.insert(name, params);
    }

    pub fn push_class(&mut self, name: String, fields: Vec<BasicTypeEnum<'ctx>>){
        self.classes.insert(name, fields);
    }

    pub fn load_func_params(&mut self, name: &str) -> Option<&Vec<Param>> {
        self.functions.get(name)
    }

    pub fn load_var(&mut self, name: &str) -> Option<BasicValueEnum<'ctx>> {
        let symbol = self.scope.get(name)?;
        let ty = symbol.ty;
        let ptr = symbol.ptr;
        Some(self.builder.build_load(ty, ptr, name).unwrap())
    }

    pub fn compile(&self) {
        let exit_code = self.context.i32_type().const_int(0, false);
        self.builder.build_return(Some(&exit_code)).unwrap();
        self.machine.write_to_file(&self.module, FileType::Object, Path::new("./main.o")).unwrap();
    }
}
