use std::{collections::HashMap, fs, path::{Path, PathBuf}};

use crate::{diagnostic, lexer::Lexer, span::Span, token::Token};

/// A module imported by another one, `import name;` at `span`.
pub struct Import {
    pub name: String,
    pub span: Span,
}

/// A source file of the program.
pub struct Module {
    pub imports: Vec<Import>,
    /// Module name, the file name without `.lv` (`math` for `math.lv`).
    pub name: String,
    /// Canonical path of the file.
    pub path: PathBuf,
    pub source: String,
}

/// All modules reachable from a root source file.
///
/// Built by [`ModuleGraph::new`], which loads the root module and then
/// recursively loads every module it imports. An import `foo` resolves to the
/// file `foo.lv` next to the importing file.
///
/// Each file is loaded only once, even if several modules import it, and
/// import cycles are allowed. [`ModuleGraph::modules`] lists imports before
/// the modules importing them.
pub struct ModuleGraph {
    pub modules: Vec<Module>,
    /// Sources that replace the content of files on disk, like the unsaved
    /// buffers of an editor.
    overlay: HashMap<PathBuf, String>,
    seen: Vec<PathBuf>,
}

impl ModuleGraph {
    /// Builds the graph starting from the module at `root_path`.
    ///
    /// # Panics
    ///
    /// Panics if the root module or any imported module can't be read, or
    /// contains a syntax error.
    pub fn new(root_path: impl AsRef<Path>) -> Self {
        Self::with_overlay(root_path, HashMap::new())
    }

    /// A graph without modules, for tools when the root can't be loaded.
    pub fn empty() -> Self {
        Self { modules: vec![], overlay: HashMap::new(), seen: vec![] }
    }

    /// Like [`ModuleGraph::new`], but reads files in `overlay` from there
    /// instead of the disk. Its paths have to be canonical.
    pub fn with_overlay(root_path: impl AsRef<Path>, overlay: HashMap<PathBuf, String>) -> Self {
        let mut graph = Self { modules: vec![], overlay, seen: vec![] };
        let root_path = root_path.as_ref();
        let path = canonical(root_path).unwrap_or_else(|| {
            diagnostic::throw(format!("module file '{}' couldn't be found", root_path.display()), None, "")
        });
        graph.load(path);
        graph
    }

    /// Loads the module at `path`, then each of its imports, and adds them
    /// to [`ModuleGraph::modules`], imports first.
    ///
    /// Modules that are already loaded are skipped, so shared imports are
    /// loaded once and import cycles terminate.
    fn load(&mut self, path: PathBuf) {
        self.seen.push(path.clone());
        let source = self.overlay.get(&path).cloned()
            .or_else(|| fs::read_to_string(&path).ok())
            .unwrap_or_else(|| diagnostic::throw(format!("module file '{}' couldn't be read", path.display()), None, ""));

        let imports = imports_of(&source);
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for import in &imports {
            let file = dir.join(format!("{}.lv", import.name));
            let Some(import_path) = canonical(&file).or_else(|| self.overlay.contains_key(&file).then(|| file.clone())) else {
                diagnostic::throw_in_file(format!("module '{}' couldn't be found", import.name), Some(import.span.clone()), &source, path.clone());
            };
            if !self.seen.contains(&import_path) {
                self.load(import_path);
            }
        }

        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.modules.push(Module { imports, name, path, source });
    }
}

/// `path` in its canonical absolute form, so different spellings of the same
/// file (`math.lv`, `./math.lv`) compare equal. `None` if it doesn't exist.
pub fn canonical(path: impl AsRef<Path>) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}

/// The modules `source` imports.
///
/// Only looks at the tokens of `import` statements, so a syntax error
/// elsewhere doesn't hide the imports. It's reported when the module is
/// parsed. The module of `import a::b;` is `b`.
fn imports_of(source: &str) -> Vec<Import> {
    let mut lexer = Lexer::new(source.to_string());
    let mut imports = Vec::new();
    while let Ok((token, _)) = lexer.get_next_token() {
        match token {
            Token::EOF => break,
            Token::Import => {
                let mut import = None;
                while let Ok((Token::Identifier(name), span)) = lexer.get_next_token() {
                    import = Some(Import { name, span: import.map_or(span, |i: Import| i.span) });
                    if !matches!(lexer.peek(), Ok((Token::ColonColon, _))) {
                        break;
                    }
                    lexer.get_next_token().ok();
                    // `import a::{x, y}` imports from `a`
                    if matches!(lexer.peek(), Ok((Token::LBrace, _))) {
                        break;
                    }
                }
                imports.extend(import);
            }
            _ => {}
        }
    }
    imports
}
