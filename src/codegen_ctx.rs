use std::{collections::HashMap, path::Path};

use inkwell::{builder::Builder, context::Context, module::Module, targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine}, values::PointerValue};

pub struct CodegenContext<'ctx>  {
    pub context: &'ctx Context,
    pub module: Module<'ctx>,
    pub builder: Builder<'ctx>,
    pub machine: TargetMachine,

    pub scope: HashMap<String, PointerValue<'ctx>>
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
            scope: HashMap::new()
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

    pub fn compile(&self) {
        let exit_code = self.context.i32_type().const_int(0, false);
        self.builder.build_return(Some(&exit_code)).unwrap();
        self.machine.write_to_file(&self.module, FileType::Object, Path::new("./main.o")).unwrap();
    }
}
