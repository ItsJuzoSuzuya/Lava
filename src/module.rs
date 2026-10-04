use std::{collections::HashMap, fs::{self}, path::{Path, PathBuf}};

use crate::{parser::Parser, statement::{StmtKind}};

/// Errors that can occur while building the [`ModuleGraph`].
enum ModuleError {
    UndefinedModule,
}

impl ModuleError {
    /// Panics with an error message for this error.
    fn throw(&self, source: &str) -> ! {
        match self {
            ModuleError::UndefinedModule => panic!("Module {} couldn't be found.", source)
        }
    }
}

#[derive(Hash, PartialEq, Eq, Clone)]
pub struct ModuleID(pub u64);

/// Possible states a module can be in.
///
/// Every module starts in the [`ModuleState::Discovered`] state once it is loaded.
pub enum ModuleState { Discovered, Parsing, Parsed, Failed }

/// Bundles [`ModuleState`] and every import.
pub struct Module {
    imports: Vec<StmtKind>,
    pub state: ModuleState,
    /// Module name, the file name without `.lv` (`math` for `math.lv`).
    pub name: String,
    pub path: String,
    pub source: String,
}

/// All modules reachable from a root source file.
///
/// Built by [`ModuleGraph::new`], which loads the root module and then
/// recursively loads every module it imports. An import `foo` resolves
/// to the file `foo.lv`.
///
/// Modules are stored by their [`ModuleID`]. Each file is loaded only once,
/// even if several modules import it, and import cycles are allowed.
pub struct ModuleGraph {
    loader: ModuleLoader,
    pub modules: Vec<Module>,
    by_path: HashMap<PathBuf, ModuleID>,
}

impl ModuleGraph {
    /// Builds the graph starting from the module at `root_path`.
    ///
    /// # Panics
    ///
    /// Panics if the root module or any imported module can't be read.
    pub fn new(root_path: &'static str) -> Self {
        let mut graph = Self { loader: ModuleLoader::new(), modules: vec![], by_path: HashMap::new() };
        let path = graph.loader.resolve(root_path);
        let (id, module) = graph.loader.load(&path);
        graph.traverse_module(path, module, id);
        graph
    }

    /// Adds `module` to [`ModuleGraph::modules`], then recursively loads and adds
    /// each of its imports.
    ///
    /// Imports that are already in the graph are skipped, so shared imports are
    /// loaded once and import cycles terminate.
    fn traverse_module(&mut self, path: PathBuf, module: Module, id: ModuleID) {
        let imports: Vec<String> = module.imports.iter().map(|i| i.to_string()).collect();
        self.by_path.insert(path, id.clone());

        for import in imports {
            let path = self.loader.resolve(&(import + ".lv"));
            if self.by_path.contains_key(&path) {
                continue;
            }
            let (id, import_module) = self.loader.load(&path);
            self.traverse_module(path, import_module, id);
        }

        self.modules.push(module);
    }
}

/// Reads module files from disk and assigns each one a unique [`ModuleID`].
///
/// IDs are assigned sequentially, starting at 0.
pub struct ModuleLoader {
    current_id: u64,
}

impl ModuleLoader {
    pub fn new() -> Self {
        Self { current_id: 0 }
    }

    /// Resolves `path` to its canonical absolute form, so that different
    /// spellings of the same file (`math.lv`, `./math.lv`) compare equal.
    ///
    /// # Panics
    ///
    /// Panics if the file doesn't exist.
    pub fn resolve(&self, path: &str) -> PathBuf {
        match fs::canonicalize(path) {
            Ok(p) => p,
            Err(_) => ModuleError::UndefinedModule {}.throw(path)
        }
    }

    /// Reads the file at `path` and parses only its import statements.
    ///
    /// Returns a [`Module`] in the [`ModuleState::Discovered`] state with a fresh ID.
    /// Calling this twice with the same path gives two different IDs.
    ///
    /// # Panics
    ///
    /// Panics if the file can't be read.
    pub fn load(&mut self, path: &Path) -> (ModuleID, Module) {
        let file_str = self.load_file(path);
        let imports = Parser::new(file_str.clone()).parse_imports();
        let id = ModuleID(self.current_id);
        self.current_id += 1;
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        (id, Module { imports, state: ModuleState::Discovered, name, path: path.display().to_string(), source: file_str })
    }

    /// Reads the file at `path` into a string.
    ///
    /// # Panics
    ///
    /// Panics if the file can't be read.
    fn load_file(&self, path: &Path) -> String {
        match fs::read_to_string(path) {
            Ok(f) => f,
            Err(_) => ModuleError::UndefinedModule {}.throw(&path.display().to_string())
        }
    }
}


