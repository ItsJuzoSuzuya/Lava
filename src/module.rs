use std::{collections::HashMap, fs, path::{Path, PathBuf}};

use crate::{diagnostic::{self, CompileError}, lexer::Lexer, span::Span, token::Token};

/// A module imported by another one, like `import math;` or
/// `import std::fs::{read};` at `span`.
pub struct Import {
    /// Name of the imported module, `math` or `std::fs`.
    pub name: String,
    /// Items of a standard library module usable without its name, `read`
    /// in `import std::fs::{read};` or `import std::fs::read;`.
    pub items: Vec<(String, Span)>,
    pub span: Span,
}

/// A source file of the program.
pub struct Module {
    pub imports: Vec<Import>,
    /// Module name, the file name without `.lv` (`math` for `math.lv`), or
    /// like `std::fs` for a module of the standard library.
    pub name: String,
    /// Canonical path of the file.
    pub path: PathBuf,
    pub source: String,
}

impl Module {
    /// The name of the standard library module this is, `fs` for `std::fs`.
    pub fn std_name(&self) -> Option<&str> {
        self.name.strip_prefix("std::")
    }

    /// What the parser renames this module's names to. A standard library
    /// module's top-level declarations get its name, `read` in `std::fs`
    /// becomes `fs::read`, so modules can declare the same names and code
    /// using them says which one it means.
    pub fn renames(&self) -> HashMap<String, String> {
        let Some(std_name) = self.std_name() else { return HashMap::new() };
        top_level_names(&self.source).into_iter()
            .map(|name| (name.clone(), format!("{}::{}", std_name, name)))
            .collect()
    }
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
    /// Imports that couldn't be loaded, like of a module that doesn't exist.
    /// They're left out, so the rest of the program can still be checked.
    pub errors: Vec<CompileError>,
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
        Self { modules: vec![], overlay: HashMap::new(), seen: vec![], errors: vec![] }
    }

    /// Like [`ModuleGraph::new`], but reads files in `overlay` from there
    /// instead of the disk. Its paths have to be canonical.
    pub fn with_overlay(root_path: impl AsRef<Path>, overlay: HashMap<PathBuf, String>) -> Self {
        let mut graph = Self { modules: vec![], overlay, seen: vec![], errors: vec![] };

        // The prelude comes first, everything can use it
        let prelude_path = prelude_path();
        let source = graph.overlay.get(&prelude_path).cloned().unwrap_or_else(|| PRELUDE.to_string());
        graph.seen.push(prelude_path.clone());
        graph.modules.push(Module { imports: vec![], name: "prelude".to_string(), path: prelude_path, source });

        let root_path = root_path.as_ref();
        let path = canonical(root_path).unwrap_or_else(|| {
            diagnostic::throw(format!("module file '{}' couldn't be found", root_path.display()), None, "")
        });
        // A file of the standard library is checked as the module it is, like in the editor.
        // The prelude is loaded already
        let is_prelude = canonical(&graph.modules[0].path).as_ref() == Some(&path);
        match STD.iter().find(|(name, _)| canonical(std_path(name)).as_ref() == Some(&path)) {
            Some((name, _)) => graph.load_std_module(name),
            None if is_prelude => {}
            None => graph.load(path),
        }

        // The program's own declarations replace the prelude's with the same name
        let own_names: Vec<String> = graph.modules[1..].iter()
            .filter(|m| m.std_name().is_none())
            .flat_map(|m| top_level_names(&m.source))
            .collect();
        graph.modules[0].source = rename_shadowed(&graph.modules[0].source, &own_names);
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

        let mut imports = imports_of(&source);
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        // An import that can't be loaded is reported and left out
        imports.retain(|import| {
            if let Some(std_name) = import.name.strip_prefix("std::") {
                return self.load_std(std_name, import, &source, &path);
            }
            let file = dir.join(format!("{}.lv", import.name));
            let Some(import_path) = canonical(&file).or_else(|| self.overlay.contains_key(&file).then(|| file.clone())) else {
                self.import_error(format!("module '{}' couldn't be found", import.name), import, &source, &path);
                return false;
            };
            if !self.seen.contains(&import_path) {
                self.load(import_path);
            }
            true
        });

        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.modules.push(Module { imports, name, path, source });
    }

    /// Records the error `message` about `import` in `importer`, the source of
    /// the file at `importer_path`.
    fn import_error(&mut self, message: String, import: &Import, importer: &str, importer_path: &Path) {
        self.errors.push(CompileError {
            message,
            span: Some(import.span.clone()),
            source: importer.to_string(),
            file: Some(importer_path.to_path_buf()),
        });
    }

    /// Loads the standard library module `std::<name>` and the ones it
    /// imports, unless it's loaded already. `import` in `importer`, the source
    /// of the file at `importer_path`, is where it's imported.
    ///
    /// Returns whether there's such a module, the error is recorded if not.
    fn load_std(&mut self, name: &str, import: &Import, importer: &str, importer_path: &Path) -> bool {
        if std_source(name).is_none() {
            self.import_error(format!("there's no module 'std::{}'", name), import, importer, importer_path);
            return false;
        }
        self.load_std_module(name);
        true
    }

    /// Loads the standard library module `std::<name>`, which exists, and the
    /// ones it imports, see [`ModuleGraph::load_std`].
    fn load_std_module(&mut self, name: &str) {
        let builtin = std_source(name).expect("only existing standard library modules are loaded");
        // Canonical like the paths of the editor's unsaved buffers in the overlay
        let path = canonical(std_path(name)).unwrap_or_else(|| std_path(name));
        if self.seen.contains(&path) {
            return;
        }
        self.seen.push(path.clone());
        let source = self.overlay.get(&path).cloned().unwrap_or_else(|| builtin.to_string());

        let mut imports = imports_of(&source);
        imports.retain(|inner| match inner.name.strip_prefix("std::") {
            Some(inner_name) => self.load_std(inner_name, inner, &source, &path),
            None => {
                self.import_error("the standard library can only import its own modules".to_string(), inner, &source, &path);
                false
            }
        });
        self.modules.push(Module { imports, name: format!("std::{}", name), path, source });
    }
}

/// The source of the prelude, the module every program starts with: only
/// what the language itself needs, `String` and `panic`. The rest is in [`STD`].
pub const PRELUDE: &str = include_str!("prelude.lv");

/// Where the prelude's source is in this repository. Errors and the language
/// server point there, the compiler itself uses the copy built into it.
pub fn prelude_path() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src/prelude.lv"))
}

/// The modules of the standard library, imported like `import std::fs;`.
/// They're built into the compiler like the prelude.
pub const STD: &[(&str, &str)] = &[
    ("option", include_str!("std/option.lv")),
    ("result", include_str!("std/result.lv")),
    ("vector", include_str!("std/vector.lv")),
    ("string", include_str!("std/string.lv")),
    ("collections", include_str!("std/collections.lv")),
    ("hashmap", include_str!("std/hashmap.lv")),
    ("path", include_str!("std/path.lv")),
    ("env", include_str!("std/env.lv")),
    ("fs", include_str!("std/fs.lv")),
    ("io", include_str!("std/io.lv")),
    ("math", include_str!("std/math.lv")),
    ("process", include_str!("std/process.lv")),
];

/// The source of the standard library module `std::<name>`, if there's one.
pub fn std_source(name: &str) -> Option<&'static str> {
    STD.iter().find(|(n, _)| *n == name).map(|(_, source)| *source)
}

/// Where the source of `std::<name>` is in this repository, see [`prelude_path`].
pub fn std_path(name: &str) -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src/std")).join(format!("{}.lv", name))
}

/// Names of the functions, classes, enums and traits `source` declares at the
/// top level. Lexing errors end the list, they're reported when parsing.
pub fn top_level_names(source: &str) -> Vec<String> {
    let mut lexer = Lexer::new(source.to_string());
    let mut names = Vec::new();
    let mut depth = 0usize;
    let mut declaring = false;
    while let Ok((token, _)) = lexer.get_next_token() {
        match token {
            Token::EOF => break,
            Token::LBrace => depth += 1,
            Token::RBrace => depth = depth.saturating_sub(1),
            Token::Func | Token::Class | Token::Enum | Token::Trait if depth == 0 => declaring = true,
            Token::Identifier(name) if declaring => {
                names.push(name);
                declaring = false;
            }
            _ => declaring = false,
        }
    }
    names
}

/// `prelude` with every name it declares at the top level that's also in
/// `own_names` renamed to `prelude_<name>`, so a program's own declaration
/// can replace the prelude's without the two clashing.
fn rename_shadowed(prelude: &str, own_names: &[String]) -> String {
    let shadowed: Vec<String> = top_level_names(prelude).into_iter().filter(|n| own_names.contains(n)).collect();
    if shadowed.is_empty() {
        return prelude.to_string();
    }

    let mut renames: Vec<Span> = Vec::new();
    let mut lexer = Lexer::new(prelude.to_string());
    while let Ok((token, span)) = lexer.get_next_token() {
        match token {
            Token::EOF => break,
            Token::Identifier(name) if shadowed.contains(&name) => renames.push(span),
            _ => {}
        }
    }

    let mut lines: Vec<Vec<char>> = prelude.lines().map(|l| l.chars().collect()).collect();
    for span in renames.iter().rev() {
        lines[span.line].splice(span.col..span.col, "prelude_".chars());
    }
    lines.into_iter().map(|l| l.into_iter().collect::<String>()).collect::<Vec<_>>().join("\n")
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
/// parsed. The module of `import a::b;` is `b`, except in the standard
/// library: `import std::fs::read;` imports `read` of `std::fs`.
pub fn imports_of(source: &str) -> Vec<Import> {
    let mut lexer = Lexer::new(source.to_string());
    let mut imports = Vec::new();
    while let Ok((token, _)) = lexer.get_next_token() {
        match token {
            Token::EOF => break,
            Token::Import => {
                let mut path: Vec<(String, Span)> = Vec::new();
                let mut items = Vec::new();
                while let Ok((Token::Identifier(name), span)) = lexer.get_next_token() {
                    path.push((name, span));
                    if !matches!(lexer.peek(), Ok((Token::ColonColon, _))) {
                        break;
                    }
                    lexer.get_next_token().ok();
                    // `import a::{x, y}` imports from `a`
                    if matches!(lexer.peek(), Ok((Token::LBrace, _))) {
                        lexer.get_next_token().ok();
                        while let Ok((Token::Identifier(item), span)) = lexer.get_next_token() {
                            items.push((item, span));
                            if !matches!(lexer.peek(), Ok((Token::Comma, _))) {
                                break;
                            }
                            lexer.get_next_token().ok();
                        }
                        break;
                    }
                }
                imports.extend(import_of_path(path, items));
            }
            _ => {}
        }
    }
    imports
}

/// The import of the module `path` with the listed `items`, see [`imports_of`].
fn import_of_path(mut path: Vec<(String, Span)>, mut items: Vec<(String, Span)>) -> Option<Import> {
    let (first, span) = path.first().cloned()?;
    if first == "std" && path.len() > 1 {
        if path.len() > 2 && items.is_empty() {
            items = path.split_off(2);
        }
        return Some(Import { name: format!("std::{}", path[1].0), items, span });
    }
    let (name, _) = path.pop()?;
    Some(Import { name, items: vec![], span })
}
