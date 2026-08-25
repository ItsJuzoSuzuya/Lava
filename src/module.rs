use std::fs::{self};

use crate::parser::Parser;

enum ModuleError {
    UndefinedModule,
}

impl ModuleError {
    fn throw(&self, source: &str) -> ! {
        match self {
            ModuleError::UndefinedModule => panic!("Module {} couldn't be found.", source)
        }
    }
}

pub struct ModuleID(pub u64);

pub enum ModuleState { Discovered, Parsing, Parsed, Failed }

pub struct Module {
    id: ModuleID,
    imports: Vec<Module>,
    state: ModuleState
}

pub struct ModuleGraph {

}

impl ModuleGraph {
    pub fn new(root_path: &'static str) -> ModuleGraph {
       let parser = Parser::new(root_path);
    }

    fn load(path: &'static str) {

    }
}

fn load_file(src: &str) -> String {
    match fs::read_to_string(src) {
        Ok(f) => f,
        Err(e) => ModuleError::UndefinedModule {}.throw(&e.to_string())
    }
}
