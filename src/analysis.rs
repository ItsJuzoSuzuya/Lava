//! An index of every name in a checked program: where it's declared, where
//! it's used and what type it has.
//!
//! Built from the [`CheckedModule`]s by [`Index::build`]. The language server
//! answers hover, go to definition, references, rename, completion and
//! outline requests with it, the linter finds unused and needlessly mutable
//! names with it.

use std::{collections::HashMap, path::PathBuf};

use crate::{declaration::{Declaration, DeclarationKind, TypeParam, Visibility}, expression::{Expr, ExprKind},
    module::ModuleGraph, param::Param, program::CheckedModule, span::Span, statement::{Pattern, Stmt, StmtKind},
    r#type::Type};

/// What a [`Def`] declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Module,
    Function,
    Method,
    Constructor,
    Class,
    Enum,
    Variant,
    Trait,
    Field,
    Param,
    Variable,
    /// Something built into the language, like `print` or `Buffer.get`.
    Builtin,
}

/// A declared name.
#[derive(Debug, Clone)]
pub struct Def {
    pub name: String,
    pub kind: DefKind,
    /// Index into [`Index::files`], `None` for built-ins.
    pub file: Option<usize>,
    /// Position of the name.
    pub span: Span,
    pub ty: Type,
    /// The declaration as code, like `func add(a: int32, b: int32) -> int32`.
    pub signature: String,
    /// The class, enum, trait or impl owner this is a member of.
    pub container: Option<usize>,
    /// The function a parameter or variable is declared in, `None` at the top level.
    pub function: Option<usize>,
    pub public: bool,
    pub mutable: bool,
    /// Whether a method takes `mut &self`.
    pub mut_self: bool,
    /// Whether this is a method of a trait or impl, whose signature is fixed by the trait.
    pub in_trait: bool,
    pub has_body: bool,
    /// Last line of the declaration, for declarations with a body.
    pub end_line: usize,
    /// Last line a parameter or variable can be used on.
    pub scope_end: usize,
    pub reads: usize,
    pub writes: usize,
    /// Changes through it that need it to be `mut`, like `p.x = 1` or `v.push(1)`.
    pub mutations: usize,
    /// Whether a reference `&x` to it is taken.
    pub borrowed: bool,
    /// Whether it was only seen in a generic template that's never used, so
    /// sema didn't check it and types are unknown.
    pub unchecked: bool,
}

/// How a [`Reference`] uses its [`Def`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Read,
    /// `x = value`
    Write,
}

/// A use of a [`Def`].
#[derive(Debug, Clone)]
pub struct Reference {
    pub file: usize,
    pub span: Span,
    pub def: usize,
    pub kind: RefKind,
}

/// A source file of the program.
pub struct FileInfo {
    pub path: PathBuf,
    pub name: String,
    pub source: String,
    lines: Vec<Vec<char>>,
    /// The modules this file imports with the position of each `import`.
    pub imports: Vec<(String, Span)>,
}

impl FileInfo {
    fn new(path: PathBuf, name: String, source: String, imports: Vec<(String, Span)>) -> Self {
        let lines = source.lines().map(|l| l.chars().collect()).collect();
        Self { path, name, source, lines, imports }
    }

    /// Text of line `line`, empty past the end.
    pub fn line(&self, line: usize) -> &[char] {
        self.lines.get(line).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Position of the whole word `word` at or after `from`, within a few lines.
    fn word_after(&self, from: &Span, word: &str) -> Option<Span> {
        let word: Vec<char> = word.chars().collect();
        for line in from.line..(from.line + 20).min(self.lines.len()) {
            let start = if line == from.line { from.col } else { 0 };
            let text = self.line(line);
            for col in start..text.len() {
                if is_word_at(text, col, &word) {
                    return Some(Span { line, col, len: word.len() });
                }
            }
        }
        None
    }

    /// Position of the whole word `word` before `before`, within a few lines.
    fn word_before(&self, before: &Span, word: &str) -> Option<Span> {
        let word: Vec<char> = word.chars().collect();
        for line in (before.line.saturating_sub(5)..=before.line).rev() {
            let text = self.line(line);
            let end = if line == before.line { before.col.min(text.len()) } else { text.len() };
            for col in (0..end).rev() {
                if is_word_at(text, col, &word) {
                    return Some(Span { line, col, len: word.len() });
                }
            }
        }
        None
    }

    /// Whether the word at `span` is `word`.
    fn has_word(&self, span: &Span, word: &str) -> bool {
        let word: Vec<char> = word.chars().collect();
        is_word_at(self.line(span.line), span.col, &word)
    }

    /// Position of the `}` closing the first `{` at or after `from`. Strings
    /// and comments are skipped.
    pub fn closing_brace(&self, from: &Span) -> Option<(usize, usize)> {
        let mut depth = 0usize;
        let mut started = false;
        for line in from.line..self.lines.len() {
            let text = self.line(line);
            let mut col = if line == from.line { from.col } else { 0 };
            let mut in_string = false;
            while col < text.len() {
                let c = text[col];
                if in_string {
                    if c == '"' { in_string = false; }
                } else if c == '"' {
                    in_string = true;
                } else if c == '/' && text.get(col + 1) == Some(&'/') {
                    break;
                } else if c == '{' {
                    depth += 1;
                    started = true;
                } else if c == '}' && started {
                    depth -= 1;
                    if depth == 0 {
                        return Some((line, col));
                    }
                }
                col += 1;
            }
        }
        None
    }
}

/// Whether the whole word `word` starts at `col` of `text`.
fn is_word_at(text: &[char], col: usize, word: &[char]) -> bool {
    let is_word_char = |c: &char| c.is_alphanumeric() || *c == '_';
    text.len() >= col + word.len()
        && &text[col..col + word.len()] == word
        && (col == 0 || !is_word_char(&text[col - 1]))
        && text.get(col + word.len()).is_none_or(|c| !is_word_char(c))
}

/// Every declared name of a program and every use of them.
pub struct Index {
    pub files: Vec<FileInfo>,
    pub defs: Vec<Def>,
    pub refs: Vec<Reference>,
    /// Global names like `fn:Point_area` or `class:Point` to their definition.
    by_key: HashMap<String, usize>,
    /// Definitions by the position of their name.
    def_at: HashMap<(usize, usize, usize), usize>,
    /// References by their position.
    ref_at: HashMap<(usize, usize, usize), usize>,
    /// Methods of trait impls by the name of the type they're for.
    impl_methods: HashMap<String, Vec<usize>>,
    /// Keys standing for the definition of another key, with the type the
    /// impl is for: default trait methods copied into impls.
    aliases: Vec<(String, String, String)>,
}

/// The name of a generic's template for one of its instances, like `Box` for
/// `Box<int32>` and `Box_get` for `Box<int32>_get`.
pub fn template_name(name: &str) -> String {
    let mut depth = 0;
    name.chars().filter(|c| {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            _ => return depth == 0,
        }
        false
    }).collect()
}

/// `ty` as it's written in code.
pub fn show_type(ty: &Type) -> String {
    match ty {
        Type::Unknown => "?".to_string(),
        Type::Void => "void".to_string(),
        Type::Object(name) | Type::Class(name) | Type::Enum(name) | Type::Param(name) => name.clone(),
        Type::Generic { name, args } => {
            let args: Vec<String> = args.iter().map(show_type).collect();
            format!("{}<{}>", name, args.join(", "))
        }
        Type::Unique(inner) => format!("Unique<{}>", show_type(inner)),
        Type::Buffer(inner) => format!("Buffer<{}>", show_type(inner)),
        Type::Ref(inner) => format!("&{}", show_type(inner)),
        other => other.to_string(),
    }
}

/// `params` as they're written in a declaration.
fn show_params(params: &[Param]) -> String {
    params.iter().map(|p| {
        let prefix = if p.mutable { "mut " } else { "" };
        if p.name == "self" {
            format!("{}&self", prefix)
        } else {
            format!("{}{}: {}", prefix, p.name, show_type(&p.ty))
        }
    }).collect::<Vec<_>>().join(", ")
}

/// `type_params` as they're written in a declaration, empty if there are none.
fn show_type_params(type_params: &[TypeParam]) -> String {
    let shown: Vec<String> = type_params.iter()
        .filter(|p| !p.is_hidden())
        .map(|p| if p.bounds.is_empty() { p.name.clone() } else { format!("{}: {}", p.name, p.bounds.join(" + ")) })
        .collect();
    if shown.is_empty() { String::new() } else { format!("<{}>", shown.join(", ")) }
}

/// The built-in functions and methods with their signatures.
const BUILTINS: &[(&str, &str, &str)] = &[
    ("builtin:print", "print", "print(value)\n\nPrints a number, bool or String and a newline."),
    ("builtin:unique", "unique", "unique<T>(value: T) -> Unique<T>\n\nMoves the value onto the heap. It's freed when its owner goes out of scope."),
    ("builtin:Buffer", "Buffer", "Buffer<T>(capacity: uint64) -> Buffer<T>\n\nA block of `capacity` elements on the heap."),
    ("fn:sqrt", "sqrt", "sqrt(x: float64) -> float64\n\nThe square root of `x`."),
    ("fn:panic", "panic", "panic(message: String)\n\nPrints `error: message` and stops the program."),
    ("fn:Buffer.get", "get", "get(&self, index: uint64) -> T\n\nA copy of the element at `index`. Stops the program if it's out of bounds."),
    ("fn:Buffer.set", "set", "set(mut &self, index: uint64, value: T)\n\nReplaces the element at `index`. Stops the program if it's out of bounds."),
    ("fn:Buffer.capacity", "capacity", "capacity(&self) -> uint64\n\nHow many elements fit into the Buffer."),
    ("fn:Buffer.resize", "resize", "resize(mut &self, capacity: uint64)\n\nGrows or shrinks the Buffer, keeping the elements that still fit."),
];

impl Index {
    /// Indexes `modules`, the result of checking the modules of `graph`.
    pub fn build(graph: &ModuleGraph, modules: &[CheckedModule]) -> Index {
        let files = graph.modules.iter().map(|m| {
            let imports = m.imports.iter().map(|i| (i.name.clone(), i.span.clone())).collect();
            FileInfo::new(m.path.clone(), m.name.clone(), m.source.clone(), imports)
        }).collect();
        let mut index = Index {
            files,
            defs: Vec::new(),
            refs: Vec::new(),
            by_key: HashMap::new(),
            def_at: HashMap::new(),
            ref_at: HashMap::new(),
            impl_methods: HashMap::new(),
            aliases: Vec::new(),
        };

        for (key, name, signature) in BUILTINS {
            let mut def = Def::new(name, DefKind::Builtin, None, Span { line: 0, col: 0, len: 0 });
            def.signature = signature.to_string();
            def.public = true;
            def.mut_self = signature.contains("mut &self");
            let id = index.push_def(def);
            index.by_key.insert(key.to_string(), id);
        }

        for file in 0..index.files.len() {
            let name = index.files[file].name.clone();
            let mut def = Def::new(&name, DefKind::Module, Some(file), Span { line: 0, col: 0, len: 0 });
            def.signature = format!("module {}", name);
            def.public = true;
            def.end_line = index.files[file].lines.len();
            let id = index.push_def(def);
            index.by_key.insert(format!("module:{}", name), id);
        }

        // Declarations first, so uses in earlier modules and earlier lines find them
        let file_of = |path: &PathBuf| graph.modules.iter().position(|m| &m.path == path);
        for module in modules {
            let Some(file) = file_of(&module.path) else { continue };
            for stmt in &module.stmts[..module.own_stmts] {
                if let StmtKind::Declaration(decl) = &stmt.kind {
                    index.declare(file, decl, None);
                }
            }
        }
        for (key, target, owner) in std::mem::take(&mut index.aliases) {
            if let Some(def) = index.key(&target) {
                index.by_key.insert(key, def);
                index.impl_methods.entry(owner).or_default().push(def);
            }
        }

        for module in modules {
            let Some(file) = file_of(&module.path) else { continue };
            for (name, span) in index.files[file].imports.clone() {
                if let Some(&module_def) = index.by_key.get(&format!("module:{}", name)) {
                    index.add_ref(file, span, module_def, RefKind::Read);
                }
            }

            let mut walker = Walker::new(&mut index, file, module.checked);
            for stmt in &module.stmts[..module.own_stmts] {
                walker.stmt(stmt);
            }
            walker.close_top_level();

            // Instances of generics are checked copies of templates in the file declaring them
            for stmt in &module.stmts[module.own_stmts..] {
                let StmtKind::Declaration(decl) = &stmt.kind else { continue };
                let Some(template) = index.template_of(decl) else { continue };
                let Some(template_file) = index.defs[template].file else { continue };
                index.defs[template].unchecked = false;
                let mut walker = Walker::new(&mut index, template_file, true);
                walker.decl(decl);
            }
        }
        index
    }

    fn push_def(&mut self, def: Def) -> usize {
        let id = self.defs.len();
        if let Some(file) = def.file {
            self.def_at.insert((file, def.span.line, def.span.col), id);
        }
        self.defs.push(def);
        id
    }

    /// The definition with the name at `span` of `file`, creating it with
    /// `make` the first time. Templates and their instances share definitions.
    fn def_at_or(&mut self, file: usize, span: &Span, make: impl FnOnce() -> Def) -> usize {
        match self.def_at.get(&(file, span.line, span.col)) {
            Some(&id) => id,
            None => self.push_def(make()),
        }
    }

    /// Records a use of `def` at `span`. A position is only counted once, even
    /// if a template and its instances use it.
    fn add_ref(&mut self, file: usize, span: Span, def: usize, kind: RefKind) {
        let key = (file, span.line, span.col);
        if self.ref_at.contains_key(&key) || self.def_at.get(&key) == Some(&def) {
            return;
        }
        match kind {
            RefKind::Read => self.defs[def].reads += 1,
            RefKind::Write => self.defs[def].writes += 1,
        }
        self.ref_at.insert(key, self.refs.len());
        self.refs.push(Reference { file, span, def, kind });
    }

    pub fn key(&self, key: &str) -> Option<usize> {
        self.by_key.get(key).copied()
    }

    /// The template definition of the instance `decl` of a generic.
    fn template_of(&self, decl: &Declaration) -> Option<usize> {
        match &decl.kind {
            DeclarationKind::Function { name, .. } => self.key(&format!("fn:{}", template_name(name))),
            DeclarationKind::Class { name, .. } => self.key(&format!("class:{}", template_name(name))),
            DeclarationKind::Enum { name, .. } => self.key(&format!("enum:{}", template_name(name))),
            _ => None,
        }
    }

    /// Adds the definitions of `decl` and its members. `container` is the
    /// class, trait or impl it's declared in.
    fn declare(&mut self, file: usize, decl: &Declaration, container: Option<(usize, String, Option<String>)>) {
        let public = matches!(decl.visibility, Visibility::Public);
        match &decl.kind {
            DeclarationKind::Function { name, type_params, params, body, return_type, visibility, .. } => {
                let (kind, key, span) = match &container {
                    Some((_, class, None)) if class == name =>
                        (DefKind::Constructor, format!("fn:{}", class), Span { len: "Constructor".len(), ..decl.span.clone() }),
                    Some((_, owner, None)) =>
                        (DefKind::Method, format!("fn:{}_{}", owner, name), self.name_after(file, &decl.span, name)),
                    Some((_, owner, Some(trait_name))) if owner.is_empty() =>
                        (DefKind::Method, format!("traitfn:{}.{}", trait_name, name), self.name_after(file, &decl.span, name)),
                    Some((_, owner, Some(trait_name))) =>
                        (DefKind::Method, format!("fn:{}_{}_{}", owner, trait_name, name), self.name_after(file, &decl.span, name)),
                    None => (DefKind::Function, format!("fn:{}", name), self.name_after(file, &decl.span, name)),
                };

                let ret = return_type.as_ref()
                    .filter(|t| **t != Type::Void && kind != DefKind::Constructor)
                    .map(|t| format!(" -> {}", show_type(t)))
                    .unwrap_or_default();
                let shown_params = show_params(params);
                let signature = match (&kind, &container) {
                    (DefKind::Constructor, _) => format!("constructor {}({})", name, shown_params),
                    (DefKind::Method, Some((_, owner, trait_name))) => {
                        let owner = match trait_name {
                            Some(t) if owner.is_empty() => t.clone(),
                            _ => owner.clone(),
                        };
                        let vis = if matches!(visibility, Visibility::Public) { "pub " } else { "" };
                        format!("{}func {}.{}{}({}){}", vis, owner, name, show_type_params(type_params), shown_params, ret)
                    }
                    _ => {
                        let vis = if public { "pub " } else { "" };
                        format!("{}func {}{}({}){}", vis, name, show_type_params(type_params), shown_params, ret)
                    }
                };

                let mut def = Def::new(name, kind, Some(file), span);
                def.signature = signature;
                def.ty = return_type.clone().unwrap_or(Type::Void);
                def.container = container.as_ref().map(|(id, _, _)| *id);
                def.public = matches!(visibility, Visibility::Public) || public || kind == DefKind::Constructor;
                def.mut_self = params.iter().any(|p| p.name == "self" && p.mutable);
                def.in_trait = matches!(&container, Some((_, _, Some(_))));
                def.has_body = body.is_some();
                def.end_line = self.files[file].closing_brace(&def.span).map(|(l, _)| l).unwrap_or(def.span.line);
                let id = self.push_def(def);
                self.by_key.insert(key, id);
                if let Some((_, owner, Some(_))) = &container {
                    if !owner.is_empty() {
                        self.impl_methods.entry(owner.clone()).or_default().push(id);
                    }
                }
            }
            DeclarationKind::Class { name, type_params, fields, methods } => {
                let span = self.name_after(file, &decl.span, name);
                let mut def = Def::new(name, DefKind::Class, Some(file), span);
                let shown_fields: Vec<String> = fields.iter()
                    .map(|f| format!("    {}{}: {}", if matches!(f.visibility, Visibility::Public) { "pub " } else { "" }, f.name, show_type(&f.ty)))
                    .collect();
                def.signature = format!("{}class {}{} {{\n{}\n}}", if public { "pub " } else { "" }, name, show_type_params(type_params), shown_fields.join("\n"));
                def.ty = Type::Class(name.clone());
                def.public = public;
                def.end_line = self.files[file].closing_brace(&def.span).map(|(l, _)| l).unwrap_or(def.span.line);
                let class = self.push_def(def);
                self.by_key.insert(format!("class:{}", name), class);

                for field in fields {
                    let span = self.files[file].word_before(&field.ty_span, &field.name).unwrap_or(field.ty_span.clone());
                    let mut def = Def::new(&field.name, DefKind::Field, Some(file), span);
                    def.signature = format!("{}.{}: {}", name, field.name, show_type(&field.ty));
                    def.ty = field.ty.clone();
                    def.container = Some(class);
                    def.public = matches!(field.visibility, Visibility::Public);
                    let id = self.push_def(def);
                    self.by_key.insert(format!("field:{}.{}", name, field.name), id);
                }
                for method in methods {
                    self.declare(file, method, Some((class, name.clone(), None)));
                }
            }
            DeclarationKind::Enum { name, type_params, fields } => {
                let span = self.name_after(file, &decl.span, name);
                let mut def = Def::new(name, DefKind::Enum, Some(file), span);
                let variants: Vec<String> = fields.iter().map(|(v, _, ty, _)| format!("    {} => {}", v, show_type(ty))).collect();
                def.signature = format!("{}enum {}{} {{\n{}\n}}", if public { "pub " } else { "" }, name, show_type_params(type_params), variants.join(",\n"));
                def.ty = Type::Enum(name.clone());
                def.public = public;
                def.end_line = self.files[file].closing_brace(&def.span).map(|(l, _)| l).unwrap_or(def.span.line);
                let enum_def = self.push_def(def);
                self.by_key.insert(format!("enum:{}", name), enum_def);

                for (variant, _, ty, ty_span) in fields {
                    let span = self.files[file].word_before(ty_span, variant).unwrap_or(ty_span.clone());
                    let mut def = Def::new(variant, DefKind::Variant, Some(file), span);
                    def.signature = format!("{}::{} => {}", name, variant, show_type(ty));
                    def.ty = ty.clone();
                    def.container = Some(enum_def);
                    def.public = true;
                    let id = self.push_def(def);
                    self.by_key.insert(format!("variant:{}::{}", name, variant), id);
                }
            }
            DeclarationKind::Trait { name, methods } => {
                let span = self.name_after(file, &decl.span, name);
                let mut def = Def::new(name, DefKind::Trait, Some(file), span);
                def.public = public;
                def.end_line = self.files[file].closing_brace(&def.span).map(|(l, _)| l).unwrap_or(def.span.line);
                let trait_def = self.push_def(def);
                self.by_key.insert(format!("trait:{}", name), trait_def);
                for method in methods {
                    self.declare(file, method, Some((trait_def, String::new(), Some(name.clone()))));
                }
                let shown: Vec<String> = self.defs.iter()
                    .filter(|d| d.container == Some(trait_def))
                    .map(|d| format!("    {}", d.signature.replacen(&format!("{}.", name), "", 1)))
                    .collect();
                self.defs[trait_def].signature = format!("{}trait {} {{\n{}\n}}", if public { "pub " } else { "" }, name, shown.join("\n"));
            }
            DeclarationKind::Impl { trait_name, target, methods } => {
                let owner = template_name(&target.owner_name().unwrap_or_else(|| show_type(target)));
                let class = self.key(&format!("class:{}", owner)).or_else(|| self.key(&format!("enum:{}", owner)));
                for method in methods {
                    let DeclarationKind::Function { name, .. } = &method.kind else { continue };
                    if self.written_in(file, &decl.span, method, name) {
                        self.declare(file, method, Some((class.unwrap_or(usize::MAX), owner.clone(), Some(trait_name.clone()))));
                    } else {
                        // A default method copied from the trait, the trait's definition stands for it
                        self.aliases.push((format!("fn:{}_{}_{}", owner, trait_name, name), format!("traitfn:{}.{}", trait_name, name), owner.clone()));
                    }
                }
                // Impls for built-in types have no definition to be a member of
                if class.is_none() {
                    for def in self.defs.iter_mut().filter(|d| d.container == Some(usize::MAX)) {
                        def.container = None;
                    }
                }
            }
        }
    }

    /// Whether the method `method` named `name` is written inside the block
    /// starting at `block` of `file`, unlike the default methods sema copies
    /// from a trait into an impl.
    fn written_in(&self, file: usize, block: &Span, method: &Declaration, name: &str) -> bool {
        let Some((end_line, end_col)) = self.files[file].closing_brace(block) else { return false };
        let span = &method.span;
        let inside = (span.line, span.col) > (block.line, block.col) && (span.line, span.col) < (end_line, end_col);
        inside && self.files[file].has_word(&self.name_after(file, span, name), name)
    }

    /// Position of the declared name `name` after the keyword at `keyword`.
    fn name_after(&self, file: usize, keyword: &Span, name: &str) -> Span {
        let after_keyword = Span { col: keyword.col + 1, ..keyword.clone() };
        self.files[file].word_after(&after_keyword, name).unwrap_or(keyword.clone())
    }

    /// The file at `path`.
    pub fn file_index(&self, path: &PathBuf) -> Option<usize> {
        self.files.iter().position(|f| &f.path == path)
    }

    /// The definition whose name or a use of it is at `line`, `col` of `file`.
    pub fn def_at_position(&self, file: usize, line: usize, col: usize) -> Option<usize> {
        let contains = |span: &Span| span.line == line && col >= span.col && col <= span.col + span.len;
        self.refs.iter()
            .find(|r| r.file == file && contains(&r.span))
            .map(|r| r.def)
            .or_else(|| self.defs.iter().position(|d| d.file == Some(file) && d.kind != DefKind::Module && contains(&d.span)))
    }

    /// Every position of `def`'s name: its declaration first, then its uses.
    pub fn occurrences(&self, def: usize) -> Vec<(usize, Span)> {
        let mut found = Vec::new();
        if let Some(file) = self.defs[def].file {
            if self.defs[def].kind != DefKind::Module {
                found.push((file, self.defs[def].span.clone()));
            }
        }
        found.extend(self.refs.iter().filter(|r| r.def == def).map(|r| (r.file, r.span.clone())));
        found
    }

    /// Markdown describing `def`, for hovering over it.
    pub fn describe(&self, def: usize) -> String {
        let d = &self.defs[def];
        let code = match d.kind {
            DefKind::Variable => format!("{} {}: {}", if d.mutable { "mut" } else { "let" }, d.name, show_type(&d.ty)),
            DefKind::Param if d.name == "self" => format!("self: {}", show_type(&d.ty)),
            DefKind::Param => format!("{}{}: {}", if d.mutable { "mut " } else { "" }, d.name, show_type(&d.ty)),
            DefKind::Builtin => {
                let (code, docs) = d.signature.split_once("\n\n").unwrap_or((&d.signature, ""));
                return format!("```lava\n{}\n```\n\n{}\n\n*built in*", code, docs);
            }
            _ => d.signature.clone(),
        };
        let place = match d.kind {
            DefKind::Variable | DefKind::Param => "",
            _ => d.file.map(|f| self.files[f].name.as_str()).unwrap_or(""),
        };
        if place.is_empty() {
            format!("```lava\n{}\n```", code)
        } else {
            format!("```lava\n{}\n```\n\n*in module `{}`*", code, place)
        }
    }

    /// The fields and methods of the class or enum `type_name`, including
    /// those of its trait impls.
    pub fn members(&self, type_name: &str) -> Vec<usize> {
        let name = template_name(type_name);
        let owner = self.key(&format!("class:{}", name)).or_else(|| self.key(&format!("enum:{}", name)));
        let mut members: Vec<usize> = match owner {
            Some(owner) => (0..self.defs.len())
                .filter(|&i| self.defs[i].container == Some(owner) && self.defs[i].kind != DefKind::Constructor)
                .collect(),
            None => Vec::new(),
        };
        for &method in self.impl_methods.get(&name).into_iter().flatten() {
            if !members.contains(&method) {
                members.push(method);
            }
        }
        if name == "Buffer" {
            members.extend(["get", "set", "capacity", "resize"].iter().filter_map(|m| self.key(&format!("fn:Buffer.{}", m))));
        }
        members
    }

    /// The innermost function or method of `file` whose body contains `line`.
    pub fn function_at(&self, file: usize, line: usize) -> Option<usize> {
        (0..self.defs.len())
            .filter(|&i| {
                let d = &self.defs[i];
                d.file == Some(file)
                    && matches!(d.kind, DefKind::Function | DefKind::Method | DefKind::Constructor)
                    && d.has_body && d.span.line <= line && line <= d.end_line
            })
            .max_by_key(|&i| self.defs[i].span.line)
    }

    /// The parameters and variables usable at `line` of `file`.
    pub fn locals_at(&self, file: usize, line: usize) -> Vec<usize> {
        let function = self.function_at(file, line);
        (0..self.defs.len())
            .filter(|&i| {
                let d = &self.defs[i];
                matches!(d.kind, DefKind::Param | DefKind::Variable)
                    && d.file == Some(file)
                    && d.function == function
                    && d.span.line <= line && line <= d.scope_end
            })
            .collect()
    }

    /// The definitions usable from anywhere in `file`: its own top-level
    /// declarations and the public ones of the modules it imports.
    pub fn globals_for(&self, file: usize) -> Vec<usize> {
        let imported: Vec<usize> = self.files[file].imports.iter()
            .filter_map(|(name, _)| self.files.iter().position(|f| &f.name == name))
            .collect();
        (0..self.defs.len())
            .filter(|&i| {
                let d = &self.defs[i];
                let top_level = matches!(d.kind, DefKind::Function | DefKind::Class | DefKind::Enum | DefKind::Trait);
                match d.file {
                    Some(f) if top_level => f == file || (imported.contains(&f) && d.public),
                    Some(_) => d.kind == DefKind::Module && imported.contains(&d.file.unwrap()),
                    None => !d.name.is_empty() && !matches!(d.name.as_str(), "get" | "set" | "capacity" | "resize"),
                }
            })
            .collect()
    }
}

impl Def {
    fn new(name: &str, kind: DefKind, file: Option<usize>, span: Span) -> Self {
        Def {
            name: name.to_string(),
            kind,
            file,
            span: span.clone(),
            ty: Type::Unknown,
            signature: String::new(),
            container: None,
            function: None,
            public: false,
            mutable: false,
            mut_self: false,
            in_trait: false,
            has_body: false,
            end_line: span.line,
            scope_end: usize::MAX,
            reads: 0,
            writes: 0,
            mutations: 0,
            borrowed: false,
            unchecked: false,
        }
    }
}

/// Walks the statements of one file, resolving every name to its definition.
struct Walker<'a> {
    index: &'a mut Index,
    file: usize,
    /// Parameters and variables by name, the innermost scope last.
    scopes: Vec<HashMap<String, usize>>,
    /// The class or type whose methods are walked, for `self` and calls without receiver.
    class: Option<String>,
    /// The function whose body is walked.
    function: Option<usize>,
    /// Whether the walked code was checked by sema, so it has types. Generic
    /// templates aren't, only their instances.
    checked: bool,
}

impl<'a> Walker<'a> {
    fn new(index: &'a mut Index, file: usize, checked: bool) -> Self {
        Self { index, file, scopes: vec![HashMap::new()], class: None, function: None, checked }
    }

    /// Ends the scope of the top-level variables.
    fn close_top_level(&mut self) {
        self.scopes.truncate(1);
    }

    fn lookup(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name).copied())
    }

    /// Declares the parameter or variable `name` at `span`, visible until `scope_end`.
    fn declare_local(&mut self, name: &str, span: Span, kind: DefKind, ty: &Type, mutable: bool, scope_end: usize) -> usize {
        let file = self.file;
        let function = self.function;
        let id = self.index.def_at_or(file, &span, || {
            let mut def = Def::new(name, kind, Some(file), span.clone());
            def.mutable = mutable;
            def.function = function;
            def.scope_end = scope_end;
            def.unchecked = true;
            def
        });
        if self.checked {
            self.index.defs[id].unchecked = false;
        }
        // Templates leave types open, their checked instances fill them in
        if self.index.defs[id].ty == Type::Unknown {
            self.index.defs[id].ty = ty.clone();
        }
        self.scopes.last_mut().unwrap().insert(name.to_string(), id);
        id
    }

    fn reference(&mut self, span: Span, def: usize, kind: RefKind) {
        self.index.add_ref(self.file, span, def, kind);
    }

    /// Marks the variable at the root of `expr`, like `p` in `p.a.b`, as changed.
    fn mutate_root(&mut self, expr: &Expr) {
        if let Some(def) = self.root_def(expr) {
            self.index.defs[def].mutations += 1;
        }
    }

    fn root_def(&self, expr: &Expr) -> Option<usize> {
        match &expr.kind {
            ExprKind::Identifier(name) => self.lookup(name),
            ExprKind::FieldAccess { object, .. } => self.root_def(object),
            ExprKind::FunctionCall { receiver: Some(receiver), .. } => self.root_def(receiver),
            _ => None,
        }
    }

    /// Records a use of the class or enum named in the type annotation `ty` at `span`.
    fn type_ref(&mut self, ty: &Type, span: &Span) {
        let name = match ty {
            Type::Class(n) | Type::Enum(n) | Type::Generic { name: n, .. } => template_name(n),
            Type::Ref(inner) | Type::Unique(inner) => return self.type_ref(inner, span),
            _ => return,
        };
        if !self.index.files[self.file].has_word(span, &name) {
            return;
        }
        let def = self.index.key(&format!("class:{}", name)).or_else(|| self.index.key(&format!("enum:{}", name)));
        if let Some(def) = def {
            self.reference(Span { len: name.chars().count(), ..span.clone() }, def, RefKind::Read);
        }
    }

    fn block(&mut self, stmts: &[Stmt], scope_end: usize) {
        self.scopes.push(HashMap::new());
        for stmt in stmts {
            self.stmt(stmt);
        }
        self.close_scope(scope_end);
    }

    /// Ends the innermost scope, its locals stay visible until `scope_end`.
    fn close_scope(&mut self, scope_end: usize) {
        if let Some(scope) = self.scopes.pop() {
            for id in scope.into_values() {
                let def = &mut self.index.defs[id];
                def.scope_end = def.scope_end.min(scope_end);
            }
        }
    }

    /// Line of the `}` ending the block that starts at or after `from`.
    fn block_end(&self, from: &Span) -> usize {
        self.index.files[self.file].closing_brace(from).map(|(l, _)| l).unwrap_or(from.line)
    }

    fn decl(&mut self, decl: &Declaration) {
        match &decl.kind {
            DeclarationKind::Function { type_params, .. } if !type_params.is_empty() && self.checked => {
                // Sema only checks the instances of a generic template
                self.checked = false;
                self.decl(decl);
                self.checked = true;
            }
            DeclarationKind::Class { type_params, .. } if !type_params.is_empty() && self.checked => {
                self.checked = false;
                self.decl(decl);
                self.checked = true;
            }
            DeclarationKind::Function { name, params, body, return_type, .. } => {
                let file = self.file;
                // Instances are named like `max<int32>`, the code says `max`
                let name = template_name(name);
                let span = if self.class.as_deref() == Some(name.as_str()) && self.index.files[file].has_word(&decl.span, "Constructor") {
                    decl.span.clone()
                } else {
                    self.index.name_after(file, &decl.span, &name)
                };
                let function = self.index.def_at.get(&(file, span.line, span.col)).copied();
                let end = self.block_end(&span);

                let outer_function = std::mem::replace(&mut self.function, function);
                self.scopes.push(HashMap::new());
                self.params(params, end);
                if let Some(ret) = return_type {
                    // Return types have no position, they're found after the parameters
                    if let Some(name) = ret.class_name() {
                        if let Some(span) = self.index.files[file].word_after(&span, &template_name(&name)) {
                            self.type_ref(ret, &span);
                        }
                    }
                }
                if let Some(body) = body {
                    self.block(body, end);
                }
                self.close_scope(end);
                self.function = outer_function;
            }
            DeclarationKind::Class { name, fields, methods, .. } => {
                let outer = self.class.replace(template_name(name));

                // Field defaults can use the constructor's parameters
                let constructor = methods.iter().find_map(|m| match &m.kind {
                    DeclarationKind::Function { name: n, params, .. } if template_name(n) == template_name(name) =>
                        Some((m.span.clone(), params.clone())),
                    _ => None,
                });
                self.scopes.push(HashMap::new());
                if let Some((span, params)) = &constructor {
                    let file = self.file;
                    let function = self.index.def_at.get(&(file, span.line, span.col)).copied();
                    let outer_function = std::mem::replace(&mut self.function, function);
                    let end = self.block_end(span).max(span.line);
                    self.params(params, end);
                    self.function = outer_function;
                }
                for field in fields {
                    self.type_ref(&field.ty, &field.ty_span);
                    if let Some(value) = &field.value {
                        self.expr(value);
                    }
                }
                self.scopes.pop();

                for method in methods {
                    self.decl(method);
                }
                self.class = outer;
            }
            DeclarationKind::Enum { fields, .. } => {
                for (_, _, ty, span) in fields {
                    self.type_ref(ty, span);
                }
            }
            DeclarationKind::Trait { methods, .. } => {
                for method in methods {
                    self.decl(method);
                }
            }
            DeclarationKind::Impl { trait_name, target, methods } => {
                let file = self.file;
                if let Some(trait_def) = self.index.key(&format!("trait:{}", trait_name)) {
                    if let Some(span) = self.index.files[file].word_after(&decl.span, trait_name) {
                        self.reference(span, trait_def, RefKind::Read);
                    }
                }
                if let Some(owner) = target.owner_name() {
                    if let Some(span) = self.index.files[file].word_after(&decl.span, &template_name(&owner)) {
                        self.type_ref(target, &span);
                    }
                }

                let outer = self.class.replace(template_name(&target.owner_name().unwrap_or_default()));
                for method in methods {
                    // Default methods copied from the trait are walked with the trait
                    let DeclarationKind::Function { name, .. } = &method.kind else { continue };
                    if self.index.written_in(file, &decl.span, method, name) {
                        self.decl(method);
                    }
                }
                self.class = outer;
            }
        }
    }

    fn params(&mut self, params: &[Param], scope_end: usize) {
        let file = self.file;
        for param in params {
            let span = if param.name == "self" {
                param.ty_span.clone()
            } else {
                self.index.files[file].word_before(&param.ty_span, &param.name).unwrap_or(param.ty_span.clone())
            };
            if param.name != "self" {
                self.type_ref(&param.ty, &param.ty_span);
            }
            if let Some(value) = &param.value {
                self.expr(value);
            }
            self.declare_local(&param.name, span, DefKind::Param, &param.ty, param.mutable, scope_end);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        let file = self.file;
        match &stmt.kind {
            StmtKind::Declaration(decl) => self.decl(decl),
            StmtKind::Expression(expr) | StmtKind::Return(expr) => self.expr(expr),
            StmtKind::Import(_) => {}
            StmtKind::Instantiation { name, typedef, value, mutable } => {
                self.expr(value);
                let after_keyword = Span { col: stmt.span.col + 1, ..stmt.span.clone() };
                let span = self.index.files[file].word_after(&after_keyword, name).unwrap_or(stmt.span.clone());
                if let Some(ty) = typedef {
                    if let Some(type_name) = ty.class_name().or_else(|| match ty { Type::Generic { name, .. } => Some(name.clone()), _ => None }) {
                        if let Some(type_span) = self.index.files[file].word_after(&span, &template_name(&type_name)) {
                            if type_span.line == span.line {
                                self.type_ref(ty, &type_span);
                            }
                        }
                    }
                }
                let ty = match &value.ty {
                    Type::Unknown => typedef.clone().unwrap_or(Type::Unknown),
                    ty => ty.clone(),
                };
                self.declare_local(name, span, DefKind::Variable, &ty, *mutable, usize::MAX);
            }
            StmtKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    let arm_end = arm.body.iter().map(|s| s.span.line).max().unwrap_or(arm.span.line);
                    let arm_end = if self.index.files[file].line(arm.span.line).contains(&'{') { self.block_end(&arm.span).max(arm_end) } else { arm_end };
                    match &arm.pattern {
                        Pattern::EnumVariant { enum_name, variant, binding } => {
                            let enum_key = template_name(enum_name);
                            if let Some(enum_def) = self.index.key(&format!("enum:{}", enum_key)) {
                                if self.index.files[file].has_word(&arm.span, enum_name) {
                                    self.reference(Span { len: enum_name.chars().count(), ..arm.span.clone() }, enum_def, RefKind::Read);
                                }
                            }
                            let variant_def = self.index.key(&format!("variant:{}::{}", enum_key, variant));
                            let variant_span = self.index.files[file].word_after(&Span { col: arm.span.col + 1, ..arm.span.clone() }, variant);
                            if let (Some(def), Some(span)) = (variant_def, variant_span.clone()) {
                                self.reference(span, def, RefKind::Read);
                            }
                            if let (Some(binding), Some(after)) = (binding, variant_span) {
                                let after = Span { col: after.col + after.len, ..after };
                                if let Some(span) = self.index.files[file].word_after(&after, binding) {
                                    let ty = variant_def.map(|d| self.index.defs[d].ty.clone()).unwrap_or(Type::Unknown);
                                    self.declare_local(binding, span, DefKind::Variable, &ty, false, arm_end);
                                }
                            }
                        }
                        Pattern::Literal(expr) => self.expr(expr),
                        Pattern::Wildcard => {}
                    }
                    for stmt in &arm.body {
                        self.stmt(stmt);
                    }
                    self.close_scope(arm_end);
                }
            }
            StmtKind::If { condition, then_body, else_body } => {
                self.expr(condition);
                let then_end = self.index.files[file].closing_brace(&stmt.span);
                self.block(then_body, then_end.map(|(l, _)| l).unwrap_or(stmt.span.line));
                if let Some(else_body) = else_body {
                    let else_start = then_end.map(|(line, col)| Span { line, col: col + 1, len: 0 }).unwrap_or(stmt.span.clone());
                    let else_end = self.block_end(&else_start);
                    self.block(else_body, else_end);
                }
            }
            StmtKind::While { condition, body } => {
                self.expr(condition);
                let end = self.block_end(&stmt.span);
                self.block(body, end);
            }
        }
    }

    fn expr(&mut self, expr: &Expr) {
        let file = self.file;
        match &expr.kind {
            ExprKind::Identifier(name) => {
                if let Some(def) = self.lookup(name) {
                    self.reference(expr.span.clone(), def, RefKind::Read);
                } else if let Some(def) = self.global(name) {
                    self.reference(expr.span.clone(), def, RefKind::Read);
                }
            }
            ExprKind::FunctionCall { name, params, receiver, mangled, .. } => {
                if let Some(receiver) = receiver {
                    self.expr(receiver);
                }
                let callee = self.callee(name, receiver.as_deref(), mangled.as_deref());
                if let Some(def) = callee {
                    let span = Span { len: name.chars().count(), ..expr.span.clone() };
                    if self.index.files[file].has_word(&span, name) {
                        self.reference(span, def, RefKind::Read);
                    }
                    if self.index.defs[def].mut_self {
                        match receiver {
                            Some(receiver) => self.mutate_root(receiver),
                            // `self.change()` written as `change()` inside a method
                            None => if let Some(self_def) = self.lookup("self") {
                                self.index.defs[self_def].mutations += 1;
                            },
                        }
                    }
                }
                for param in params {
                    self.expr(param);
                }
            }
            ExprKind::FieldAccess { object, field } => {
                self.expr(object);
                if let Some(def) = self.field_def(object, field) {
                    self.reference(expr.span.clone(), def, RefKind::Read);
                }
            }
            ExprKind::Assignment { lhs, rhs } => {
                self.expr(rhs);
                match &lhs.kind {
                    ExprKind::Identifier(name) => {
                        if let Some(def) = self.lookup(name) {
                            self.reference(lhs.span.clone(), def, RefKind::Write);
                        }
                    }
                    ExprKind::FieldAccess { object, field } => {
                        self.expr(object);
                        if let Some(def) = self.field_def(object, field) {
                            self.reference(lhs.span.clone(), def, RefKind::Write);
                        }
                        self.mutate_root(object);
                    }
                    _ => self.expr(lhs),
                }
            }
            ExprKind::BinaryExpr { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Convert { value, target } => {
                self.expr(value);
                if let Some(name) = target.class_name() {
                    if let Some(span) = self.index.files[file].word_after(&value.span, &name) {
                        self.type_ref(target, &span);
                    }
                }
            }
            ExprKind::Negation(inner) => self.expr(inner),
            ExprKind::Ref(inner) => {
                self.expr(inner);
                if let Some(def) = self.root_def(inner) {
                    self.index.defs[def].borrowed = true;
                }
            }
            ExprKind::Print(inner) => {
                self.builtin(&expr.span, "print", "builtin:print");
                self.expr(inner);
            }
            ExprKind::Unique(inner) => {
                self.builtin(&expr.span, "unique", "builtin:unique");
                self.expr(inner);
            }
            ExprKind::NewBuffer(capacity) => {
                self.builtin(&expr.span, "Buffer", "builtin:Buffer");
                self.expr(capacity);
            }
            ExprKind::ColonAccess { name, tag } => self.variant_ref(expr, name, tag),
            ExprKind::EnumInit { name, tag, value, .. } => {
                self.variant_ref(expr, name, tag);
                self.expr(value);
            }
            ExprKind::StructInit { class, fields, .. } => {
                let class_name = template_name(class);
                if let Some(def) = self.index.key(&format!("class:{}", class_name)) {
                    if self.index.files[file].has_word(&expr.span, &class_name) {
                        self.reference(Span { len: class_name.chars().count(), ..expr.span.clone() }, def, RefKind::Read);
                    }
                }
                for (field, value) in fields {
                    if let Some(def) = self.index.key(&format!("field:{}.{}", class_name, field)) {
                        if let Some(span) = self.index.files[file].word_before(&value.span, field) {
                            self.reference(span, def, RefKind::Read);
                        }
                    }
                    self.expr(value);
                }
            }
            ExprKind::ModuleImport { .. } | ExprKind::Boolean(_) | ExprKind::FloatLiteral(_)
            | ExprKind::IntLiteral(_) | ExprKind::StringLiteral(_) => {}
        }
    }

    /// A function, class or enum named `name` at the top level.
    fn global(&self, name: &str) -> Option<usize> {
        ["fn", "class", "enum", "trait"].iter().find_map(|kind| self.index.key(&format!("{}:{}", kind, name)))
    }

    /// The definition of the function or method a call of `name` calls.
    fn callee(&self, name: &str, receiver: Option<&Expr>, mangled: Option<&str>) -> Option<usize> {
        if let Some(mangled) = mangled {
            if let Some(def) = self.index.key(&format!("fn:{}", template_name(mangled))) {
                return Some(def);
            }
        }
        let owner = match receiver {
            Some(receiver) => self.type_name_of(receiver),
            None => self.class.clone(),
        };
        if let Some(owner) = owner.map(|o| template_name(&o)) {
            if let Some(def) = self.index.key(&format!("fn:{}_{}", owner, name)) {
                return Some(def);
            }
            let from_impl = self.index.impl_methods.get(&owner)
                .and_then(|methods| methods.iter().copied().find(|&m| self.index.defs[m].name == name));
            if from_impl.is_some() {
                return from_impl;
            }
        }
        if receiver.is_none() {
            return self.index.key(&format!("fn:{}", name)).or_else(|| self.index.key(&format!("class:{}", name)));
        }
        None
    }

    /// The definition of the field `field` of `object`.
    fn field_def(&self, object: &Expr, field: &str) -> Option<usize> {
        let class = self.type_name_of(object)?;
        self.index.key(&format!("field:{}.{}", template_name(&class), field))
    }

    /// Name of the type of `expr`, like `Point`. `self` in an unchecked
    /// template has no type yet, but it's the walked class.
    fn type_name_of(&self, expr: &Expr) -> Option<String> {
        if matches!(&expr.kind, ExprKind::Identifier(name) if name == "self") && expr.ty == Type::Unknown {
            return self.class.clone();
        }
        let ty = match &expr.ty {
            Type::Unknown => {
                // A field of `self` in a template, like `self.data`, has the field's declared type
                let ExprKind::FieldAccess { object, field } = &expr.kind else { return None };
                let def = self.field_def(object, field)?;
                self.index.defs[def].ty.clone()
            }
            ty => ty.clone(),
        };
        ty.class_name().or_else(|| ty.owner_name()).or_else(|| match ty {
            Type::Generic { name, .. } => Some(name),
            Type::Buffer(_) => Some("Buffer".to_string()),
            _ => None,
        })
    }

    fn builtin(&mut self, span: &Span, word: &str, key: &str) {
        if let Some(def) = self.index.key(key) {
            if self.index.files[self.file].has_word(span, word) {
                self.reference(Span { len: word.len(), ..span.clone() }, def, RefKind::Read);
            }
        }
    }

    /// Records the uses of the enum `name` and its variant `tag` in `expr`, like `Color::Red`.
    fn variant_ref(&mut self, expr: &Expr, name: &str, tag: &str) {
        let file = self.file;
        let enum_name = template_name(name);
        if let Some(def) = self.index.key(&format!("enum:{}", enum_name)) {
            if self.index.files[file].has_word(&expr.span, &enum_name) {
                self.reference(Span { len: enum_name.chars().count(), ..expr.span.clone() }, def, RefKind::Read);
            }
        }
        if let Some(def) = self.index.key(&format!("variant:{}::{}", enum_name, tag)) {
            if let Some(span) = self.index.files[file].word_after(&Span { col: expr.span.col + 1, ..expr.span.clone() }, tag) {
                self.reference(span, def, RefKind::Read);
            }
        }
    }
}
