use std::{collections::HashMap, path::Path};

use inkwell::{builder::Builder, context::Context, module::Module, targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine}, types::BasicTypeEnum, values::{BasicValueEnum, PointerValue}};

use crate::{field::Field, param::Param, span::Span, sym::Symbol, r#type::Type};

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
    pub enums: HashMap<String, Vec<(String, BasicTypeEnum<'ctx>)>>,
    pub current_class: Option<String>,
    pub current_class_fields: Option<Vec<Field>>
}

impl<'ctx> CodegenContext<'ctx> {
    pub fn new(context: &'ctx Context, source: String) -> Self {
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
        Self {
            source,
            context,
            module: context.create_module("mod"),
            builder: context.create_builder(),
            machine,
            scope: HashMap::new(),
            functions: HashMap::new(),
            classes: HashMap::new(),
            enums: HashMap::new(),
            current_class: None,
            current_class_fields: None,
        }
    }

    pub fn build_entry(&self) {
        let i32_type = self.context.i32_type();
        let main_fn_type = i32_type.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_fn_type, None);
        let entry_block = self.context.append_basic_block(main_fn, "entry_block");
        self.builder.position_at_end(entry_block);
    }

    pub fn push_var(&mut self, name: String, ty: BasicTypeEnum<'ctx>, ptr: PointerValue<'ctx>, parent: Option<String>) {
        let symbol = Symbol { ty, ptr, mutable: true, class_name: parent};
        self.scope.insert(name, symbol);
    }

    pub fn push_func(&mut self, name: String, params: Vec<Param>) {
        self.functions.insert(name, params);
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
        self.functions.get(name).unwrap()[0].name == "self"
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
        let exit_code = self.context.i32_type().const_int(0, false);
        self.builder.build_return(Some(&exit_code)).unwrap();
        if let Err(e) = self.module.verify() {
            self.module.print_to_stderr();
            panic!("LLVM module verification failed:\n{}", e.to_string());
        }
        self.machine.write_to_file(&self.module, FileType::Object, Path::new("./main.o")).unwrap();
    }
}
