//! A language server for Lava, speaking the Language Server Protocol over
//! stdin and stdout.
//!
//! Every change to an open file re-checks the program the file belongs to:
//! the program starting at `main.lv` in the file's directory, or the file
//! itself if there's no `main.lv` or it isn't imported from there. Compile
//! errors and lint warnings are published as diagnostics, the rest of the
//! features answer from the [`Index`] of the last check.
//!
//! Supported: diagnostics, hover, go to definition, references, highlights,
//! rename, completion, signature help, document and workspace symbols and
//! quick fixes for lint warnings.

use std::{collections::{HashMap, HashSet}, io::{self, BufRead, Write}, path::{Path, PathBuf}, process::ExitCode};

use serde_json::{Value, json};

use crate::{analysis::{DefKind, Index, RefKind, show_type, template_name}, declaration::{DeclarationKind, ExternItem, Visibility},
    diagnostic::{self, CompileError}, lint::{self, Warning}, module::{self, ModuleGraph}, parser::Parser, program::check_modules,
    span::Span, statement::StmtKind, r#type::Type};

/// Runs the server until the client sends `exit`.
pub fn run() -> ExitCode {
    // Errors are reported as diagnostics, nothing may be printed to stdout
    diagnostic::silence_panics();
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut server = Server::default();

    while let Some(message) = read_message(&mut input) {
        server.handle(message);
        if let Some(code) = server.exit_code {
            return ExitCode::from(code);
        }
    }
    ExitCode::SUCCESS
}

/// Reads one message: `Content-Length` headers, an empty line and the JSON body.
fn read_message(input: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn send(message: Value) {
    let body = message.to_string();
    let mut out = io::stdout().lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = out.flush();
}

/// The result of checking one program.
struct Analysis {
    /// The program's root file, like `main.lv`.
    root: PathBuf,
    index: Index,
    errors: Vec<CompileError>,
    warnings: Vec<Warning>,
    /// Files that parsed, so the index knows their declarations.
    parsed: HashSet<PathBuf>,
}

impl Analysis {
    fn contains(&self, path: &Path) -> bool {
        self.index.files.iter().any(|f| f.path == path)
    }
}

#[derive(Default)]
struct Server {
    /// Text of the open files, which may differ from the disk.
    documents: HashMap<PathBuf, String>,
    analyses: Vec<Analysis>,
    /// Earlier analyses of programs whose files now have syntax errors, to
    /// keep answering about them while typing.
    fallbacks: Vec<Analysis>,
    /// Files that got diagnostics, to clear them once they're fixed.
    published: HashSet<PathBuf>,
    shutdown: bool,
    exit_code: Option<u8>,
}

impl Server {
    fn handle(&mut self, message: Value) {
        let method = message["method"].as_str().unwrap_or("").to_string();
        let params = message["params"].clone();
        let id = message.get("id").cloned();

        let Some(id) = id else {
            self.notification(&method, &params);
            return;
        };
        if method.is_empty() {
            return; // a response to something the server never asks
        }

        let result = diagnostic::catch(|| self.request(&method, &params));
        let response = match result {
            Ok(Ok(result)) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Ok(Err((code, message))) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
            Err(crash) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": crash.message } }),
        };
        send(response);
    }

    fn notification(&mut self, method: &str, params: &Value) {
        match method {
            "textDocument/didOpen" => {
                let Some(path) = uri_path(&params["textDocument"]["uri"]) else { return };
                let text = params["textDocument"]["text"].as_str().unwrap_or("").to_string();
                self.documents.insert(path.clone(), text);
                self.changed(&path);
            }
            "textDocument/didChange" => {
                let Some(path) = uri_path(&params["textDocument"]["uri"]) else { return };
                // Full sync, the last change holds the whole text
                if let Some(text) = params["contentChanges"].as_array().and_then(|c| c.last()).and_then(|c| c["text"].as_str()) {
                    self.documents.insert(path.clone(), text.to_string());
                }
                self.changed(&path);
            }
            "textDocument/didSave" => {
                if let Some(path) = uri_path(&params["textDocument"]["uri"]) {
                    self.changed(&path);
                }
            }
            "textDocument/didClose" => {
                if let Some(path) = uri_path(&params["textDocument"]["uri"]) {
                    self.documents.remove(&path);
                    self.changed(&path);
                }
            }
            "exit" => self.exit_code = Some(if self.shutdown { 0 } else { 1 }),
            _ => {}
        }
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        let result = match method {
            "initialize" => json!({
                "capabilities": {
                    "textDocumentSync": { "openClose": true, "change": 1, "save": { "includeText": false } },
                    "hoverProvider": true,
                    "definitionProvider": true,
                    "referencesProvider": true,
                    "documentHighlightProvider": true,
                    "documentSymbolProvider": true,
                    "workspaceSymbolProvider": true,
                    "renameProvider": { "prepareProvider": true },
                    "completionProvider": { "triggerCharacters": [".", ":"] },
                    "signatureHelpProvider": { "triggerCharacters": ["(", ","], "retriggerCharacters": [","] },
                    "codeActionProvider": { "codeActionKinds": ["quickfix"] },
                },
                "serverInfo": { "name": "lava-lsp", "version": env!("CARGO_PKG_VERSION") },
            }),
            "shutdown" => {
                self.shutdown = true;
                Value::Null
            }
            "textDocument/hover" => self.hover(params),
            "textDocument/definition" => self.definition(params),
            "textDocument/references" => self.references(params),
            "textDocument/documentHighlight" => self.highlights(params),
            "textDocument/prepareRename" => self.prepare_rename(params),
            "textDocument/rename" => self.rename(params)?,
            "textDocument/documentSymbol" => self.document_symbols(params),
            "workspace/symbol" => self.workspace_symbols(params),
            "textDocument/completion" => self.completion(params),
            "textDocument/signatureHelp" => self.signature_help(params),
            "textDocument/codeAction" => self.code_actions(params),
            _ => return Err((-32601, format!("method '{}' isn't supported", method))),
        };
        Ok(result)
    }

    // --- Checking ---

    /// Re-checks every program `path` belongs to and publishes the diagnostics.
    fn changed(&mut self, path: &Path) {
        let mut roots: Vec<PathBuf> = self.analyses.iter()
            .filter(|a| a.contains(path))
            .map(|a| a.root.clone())
            .collect();
        let root = self.root_for(path);
        if !roots.contains(&root) {
            roots.push(root);
        }

        for root in roots {
            let analysis = self.analyze(&root);
            // A file not imported from `main.lv` is a program of its own
            if !analysis.contains(path) && root != path {
                let own = self.analyze(path);
                self.store(own);
            }
            self.store(analysis);
        }
        // Programs that no open file belongs to anymore aren't needed
        let documents = &self.documents;
        self.analyses.retain(|a| documents.keys().any(|d| a.contains(d)));
        self.publish();
    }

    fn store(&mut self, analysis: Analysis) {
        let Some(position) = self.analyses.iter().position(|a| a.root == analysis.root) else {
            self.analyses.push(analysis);
            return;
        };
        let old = std::mem::replace(&mut self.analyses[position], analysis);
        // An analysis where every file parsed stays around while one has a syntax error
        let all_parsed = |a: &Analysis| a.index.files.iter().all(|f| a.parsed.contains(&f.path));
        if all_parsed(&old) && !all_parsed(&self.analyses[position]) {
            self.fallbacks.retain(|a| a.root != old.root);
            self.fallbacks.push(old);
        } else if all_parsed(&self.analyses[position]) {
            let root = self.analyses[position].root.clone();
            self.fallbacks.retain(|a| a.root != root);
        }
    }

    /// The root of the program `path` most likely belongs to.
    fn root_for(&self, path: &Path) -> PathBuf {
        let main = path.with_file_name("main.lv");
        if main.exists() || self.documents.contains_key(&main) { main } else { path.to_path_buf() }
    }

    /// Checks the program starting at `root`, with the open documents' text.
    fn analyze(&self, root: &Path) -> Analysis {
        let mut overlay = self.documents.clone();
        let mut errors = Vec::new();
        // Files with lines blanked out because they don't parse, and those lines
        let mut recovered: HashSet<PathBuf> = HashSet::new();
        let mut blanked_lines: Vec<String> = Vec::new();
        // Whether a blanked line opened or closed a block, so the code after it doesn't fit anymore
        let mut unbalanced = false;
        let (graph, modules) = loop {
            let graph = match diagnostic::catch(|| ModuleGraph::with_overlay(root, overlay.clone())) {
                Ok(graph) => graph,
                Err(mut error) => {
                    error.file.get_or_insert_with(|| root.to_path_buf());
                    errors.push(error);
                    break (ModuleGraph::empty(), vec![]);
                }
            };
            let (modules, mut found) = check_modules(&graph);
            // Checking stops at a syntax error, it's the last one
            let syntax_error = found.last().is_some_and(|error| {
                error.file.as_ref().is_some_and(|file| !modules.iter().any(|m| &m.path == file))
            });
            if !syntax_error {
                // Leaving lines out can take away a declaration, the code using
                // it then has an unknown name. Other errors are real
                let from_blanked = |error: &CompileError| {
                    error.message.to_lowercase().contains("unknown") && quoted_name(&error.message)
                        .is_some_and(|name| blanked_lines.iter().any(|line| contains_word(line, name)))
                };
                errors.extend(found.into_iter().filter(|e| !from_blanked(e)));
                break (graph, modules);
            }
            let error = found.pop().expect("a syntax error was found");
            // Later syntax errors are real too, unless a blanked line opened or closed a block
            let report = !unbalanced;

            // A syntax error, like in a line being typed, would leave the whole file
            // without types. Its line is left out and the rest checked again, the
            // error is still reported as it is
            let unparsed = error.file.clone().filter(|file| !modules.iter().any(|m| &m.path == file));
            let retry = match (unparsed, &error.span) {
                (Some(file), Some(span)) if blanked_lines.len() < 20 => {
                    let text = overlay.get(&file).cloned().unwrap_or_else(|| self.text_of(&file));
                    let mut lines: Vec<&str> = text.lines().collect();
                    // An error at the end of the file, like after a `words.` being typed,
                    // is after the line causing it
                    let mut at = span.line.min(lines.len().saturating_sub(1));
                    while at > 0 && lines.get(at).is_some_and(|line| line.trim().is_empty()) {
                        at -= 1;
                    }
                    // So is an error at the start of a line after an unfinished one, like `text.`
                    let starts_line = lines.get(at).is_some_and(|line| line.len() - line.trim_start().len() >= span.col);
                    let previous = (0..at).rev().find(|&i| !lines[i].trim().is_empty());
                    if let (true, Some(previous)) = (starts_line && at == span.line, previous) {
                        let end = lines[previous].trim_end();
                        if !(end.ends_with(';') || end.ends_with('{') || end.ends_with('}') || end.ends_with(',') || end.trim_start().starts_with("//")) {
                            at = previous;
                        }
                    }
                    match lines.get_mut(at) {
                        Some(line) if !line.trim().is_empty() => {
                            let count = |c: char| line.chars().filter(|&x| x == c).count();
                            unbalanced |= count('{') != count('}') || count('(') != count(')') || count('[') != count(']');
                            blanked_lines.push(line.to_string());
                            *line = "";
                            let blanked = lines.join("\n");
                            overlay.insert(file.clone(), blanked);
                            recovered.insert(file);
                            true
                        }
                        _ => false,
                    }
                }
                _ => false,
            };
            if report {
                errors.push(error);
            }
            if !retry {
                // The modules checked before the one that doesn't parse
                errors.extend(found);
                break (graph, modules);
            }
        };
        // A file only parsed after leaving lines out counts as not parsed
        let parsed = modules.iter().map(|m| m.path.clone()).filter(|path| !recovered.contains(path)).collect();

        let built = diagnostic::catch(|| {
            let index = Index::build(&graph, &modules);
            let mut warnings = lint::lint(&index, &modules);
            // What's used in lines left out isn't unused
            warnings.retain(|w| !(w.code.starts_with("unused") && quoted_name(&w.message)
                .is_some_and(|name| blanked_lines.iter().any(|line| contains_word(line, name)))));
            (index, warnings)
        });
        let (index, warnings) = match built {
            Ok(built) => built,
            Err(mut crash) => {
                crash.message = format!("the language server crashed while analyzing: {}", crash.message);
                crash.file = Some(root.to_path_buf());
                crash.span = None;
                errors.push(crash);
                (Index::build(&graph, &[]), vec![])
            }
        };
        Analysis { root: root.to_path_buf(), index, errors, warnings, parsed }
    }

    /// Sends the diagnostics of every checked file, and empty ones to files
    /// that had some before.
    fn publish(&mut self) {
        let mut by_file: HashMap<PathBuf, Vec<Value>> = HashMap::new();
        let mut seen: HashSet<(PathBuf, usize, usize, String)> = HashSet::new();

        for analysis in &self.analyses {
            for file in &analysis.index.files {
                by_file.entry(file.path.clone()).or_default();
            }
            for error in &analysis.errors {
                let path = error.file.clone().unwrap_or_else(|| analysis.root.clone());
                let text = self.text_of(&path);
                let range = match &error.span {
                    Some(span) => word_range(&text, span),
                    None => json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }),
                };
                let key = (path.clone(), error.span.as_ref().map_or(0, |s| s.line), error.span.as_ref().map_or(0, |s| s.col), error.message.clone());
                if seen.insert(key) {
                    by_file.entry(path).or_default().push(json!({
                        "range": range, "severity": 1, "source": "lava", "message": error.message,
                    }));
                }
            }
            for warning in &analysis.warnings {
                let file = &analysis.index.files[warning.file];
                let key = (file.path.clone(), warning.span.line, warning.span.col, warning.message.clone());
                if !seen.insert(key) {
                    continue;
                }
                let mut diagnostic = json!({
                    "range": span_range(&file.source, &warning.span),
                    "severity": 2,
                    "source": "lava",
                    "code": warning.code,
                    "message": warning.message,
                });
                if warning.code.starts_with("unused") || warning.code == "unreachable_code" {
                    diagnostic["tags"] = json!([1]);
                }
                by_file.entry(file.path.clone()).or_default().push(diagnostic);
            }
        }

        for path in self.published.drain().collect::<Vec<_>>() {
            by_file.entry(path).or_default();
        }
        for (path, diagnostics) in by_file {
            if !diagnostics.is_empty() {
                self.published.insert(path.clone());
            }
            send(json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": { "uri": path_uri(&path), "diagnostics": diagnostics },
            }));
        }
    }

    /// The current text of `path`, from the editor or the disk.
    fn text_of(&self, path: &Path) -> String {
        self.documents.get(path).cloned()
            .or_else(|| std::fs::read_to_string(path).ok())
            .unwrap_or_default()
    }

    /// The analysis to answer requests about `path` with: the current one,
    /// which leaves out lines with syntax errors and has everything else.
    /// One from when every file parsed only if the current one lost the file.
    fn analysis_for(&self, path: &Path) -> Option<(&Analysis, usize)> {
        let path = path.to_path_buf();
        let current = self.analyses.iter()
            .filter(|a| a.contains(&path))
            .find_map(|a| a.index.file_index(&path).map(|file| (a, file)));
        current.or_else(|| self.fallbacks.iter()
            .filter(|a| a.parsed.contains(&path))
            .find_map(|a| a.index.file_index(&path).map(|file| (a, file))))
    }

    /// The document, position and analysis a request is about, with the
    /// position as line and character index.
    fn at<'a>(&'a self, params: &Value) -> Option<(PathBuf, &'a Analysis, usize, usize, usize)> {
        let path = uri_path(&params["textDocument"]["uri"])?;
        let line = params["position"]["line"].as_u64()? as usize;
        let character = params["position"]["character"].as_u64()? as usize;
        let text = self.text_of(&path);
        let col = utf16_to_col(text.lines().nth(line).unwrap_or(""), character);
        let (analysis, file) = self.analysis_for(&path)?;
        Some((path, analysis, file, line, col))
    }

    // --- Requests ---

    fn hover(&self, params: &Value) -> Value {
        let Some((_, analysis, file, line, col)) = self.at(params) else { return Value::Null };
        let index = &analysis.index;
        let Some(def) = index.def_at_position(file, line, col) else { return Value::Null };
        json!({ "contents": { "kind": "markdown", "value": index.describe(def) } })
    }

    fn definition(&self, params: &Value) -> Value {
        let Some((_, analysis, file, line, col)) = self.at(params) else { return Value::Null };
        let index = &analysis.index;
        let Some(def) = index.def_at_position(file, line, col) else { return Value::Null };
        let def = &index.defs[def];
        let Some(def_file) = def.file else { return Value::Null };
        let path = &index.files[def_file].path;
        json!({ "uri": path_uri(path), "range": span_range(&self.text_of(path), &def.span) })
    }

    /// Every position of the symbol at the request's position, by file.
    fn occurrences(&self, analysis: &Analysis, def: usize) -> Vec<(usize, Span, bool)> {
        let index = &analysis.index;
        let mut defs = vec![def];
        let d = &index.defs[def];
        // A class's name is also used by calls of its constructor
        if d.kind == DefKind::Class {
            defs.extend(index.key(&format!("fn:{}", d.name)).filter(|&c| index.defs[c].kind == DefKind::Constructor));
        }
        let mut found: Vec<(usize, Span, bool)> = Vec::new();
        for &def in &defs {
            let is_constructor = index.defs[def].kind == DefKind::Constructor;
            for (file, span) in index.occurrences(def) {
                // The `Constructor` keyword isn't the class's name
                if is_constructor && def != defs[0] && index.defs[def].file == Some(file) && index.defs[def].span.line == span.line && index.defs[def].span.col == span.col {
                    continue;
                }
                let write = index.refs.iter().any(|r| r.file == file && r.span.line == span.line && r.span.col == span.col && r.kind == RefKind::Write);
                if !found.iter().any(|(f, s, _)| *f == file && s.line == span.line && s.col == span.col) {
                    found.push((file, span, write));
                }
            }
        }
        found
    }

    /// The definition at the request's position, a constructor standing for its class.
    fn symbol_at(&self, params: &Value) -> Option<(&Analysis, usize)> {
        let (_, analysis, file, line, col) = self.at(params)?;
        let index = &analysis.index;
        let def = index.def_at_position(file, line, col)?;
        let def = match index.defs[def].kind {
            DefKind::Constructor => index.key(&format!("class:{}", index.defs[def].name)).unwrap_or(def),
            _ => def,
        };
        Some((analysis, def))
    }

    fn references(&self, params: &Value) -> Value {
        let Some((analysis, def)) = self.symbol_at(params) else { return Value::Null };
        let include_declaration = params["context"]["includeDeclaration"].as_bool().unwrap_or(true);
        let index = &analysis.index;
        let declaration = index.defs[def].file.map(|f| (f, index.defs[def].span.clone()));
        let locations: Vec<Value> = self.occurrences(analysis, def).into_iter()
            .filter(|(file, span, _)| include_declaration || !is_same(&declaration, *file, span))
            .map(|(file, span, _)| {
                let path = &index.files[file].path;
                json!({ "uri": path_uri(path), "range": span_range(&self.text_of(path), &span) })
            })
            .collect();
        json!(locations)
    }

    fn highlights(&self, params: &Value) -> Value {
        let Some((_, _, file, _, _)) = self.at(params) else { return Value::Null };
        let Some((analysis, def)) = self.symbol_at(params) else { return Value::Null };
        let path = &analysis.index.files[file].path;
        let text = self.text_of(path);
        let highlights: Vec<Value> = self.occurrences(analysis, def).into_iter()
            .filter(|(f, _, _)| *f == file)
            .map(|(_, span, write)| json!({ "range": span_range(&text, &span), "kind": if write { 3 } else { 2 } }))
            .collect();
        json!(highlights)
    }

    fn prepare_rename(&self, params: &Value) -> Value {
        let Some((analysis, def)) = self.symbol_at(params) else { return Value::Null };
        if !renamable(&analysis.index, def) {
            return Value::Null;
        }
        let Some((path, _, _, line, col)) = self.at(params) else { return Value::Null };
        let text = self.text_of(&path);
        let Some(span) = word_at(text.lines().nth(line).unwrap_or(""), line, col) else { return Value::Null };
        span_range(&text, &span)
    }

    fn rename(&self, params: &Value) -> Result<Value, (i64, String)> {
        let new_name = params["newName"].as_str().unwrap_or("");
        let valid = new_name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
            && new_name.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !valid {
            return Err((-32602, format!("'{}' isn't a valid name", new_name)));
        }
        let Some((analysis, def)) = self.symbol_at(params) else { return Ok(Value::Null) };
        if !renamable(&analysis.index, def) {
            return Err((-32602, "this can't be renamed".to_string()));
        }

        let index = &analysis.index;
        let mut changes: HashMap<String, Vec<Value>> = HashMap::new();
        for (file, span, _) in self.occurrences(analysis, def) {
            let path = &index.files[file].path;
            changes.entry(path_uri(path)).or_default()
                .push(json!({ "range": span_range(&self.text_of(path), &span), "newText": new_name }));
        }
        Ok(json!({ "changes": changes }))
    }

    fn document_symbols(&self, params: &Value) -> Value {
        let Some(path) = uri_path(&params["textDocument"]["uri"]) else { return Value::Null };
        let Some((analysis, file)) = self.analysis_for(&path) else { return json!([]) };
        let index = &analysis.index;
        let text = self.text_of(&path);

        let symbol = |def: usize, children: Vec<Value>| {
            let d = &index.defs[def];
            let end = Span { line: d.end_line.max(d.span.line), col: 0, len: 0 };
            let end_char = text.lines().nth(end.line).map(|l| l.encode_utf16().count()).unwrap_or(0);
            let start = span_range(&text, &d.span)["start"].clone();
            json!({
                "name": d.name,
                "detail": d.signature.lines().next().unwrap_or(""),
                "kind": symbol_kind(d.kind),
                "range": { "start": start, "end": { "line": end.line, "character": end_char } },
                "selectionRange": span_range(&text, &d.span),
                "children": children,
            })
        };

        let top: Vec<usize> = (0..index.defs.len())
            .filter(|&i| {
                let d = &index.defs[i];
                d.file == Some(file) && d.container.is_none() && (
                    matches!(d.kind, DefKind::Function | DefKind::Class | DefKind::Enum | DefKind::Trait | DefKind::Method)
                    || d.kind == DefKind::Variable && d.function.is_none()
                )
            })
            .collect();
        let mut symbols: Vec<(Span, Value)> = top.into_iter().map(|def| {
            let children = (0..index.defs.len())
                .filter(|&i| index.defs[i].container == Some(def) && index.defs[i].file == Some(file))
                .map(|i| symbol(i, vec![]))
                .collect();
            (index.defs[def].span.clone(), symbol(def, children))
        }).collect();
        symbols.sort_by_key(|(span, _)| (span.line, span.col));
        json!(symbols.into_iter().map(|(_, s)| s).collect::<Vec<_>>())
    }

    fn workspace_symbols(&self, params: &Value) -> Value {
        let query = params["query"].as_str().unwrap_or("").to_lowercase();
        let mut seen = HashSet::new();
        let mut symbols = Vec::new();
        for analysis in &self.analyses {
            let index = &analysis.index;
            for d in &index.defs {
                let global = matches!(d.kind, DefKind::Function | DefKind::Class | DefKind::Enum | DefKind::Trait
                    | DefKind::Method | DefKind::Field | DefKind::Variant | DefKind::Constructor);
                let Some(file) = d.file.filter(|_| global) else { continue };
                if !d.name.to_lowercase().contains(&query) {
                    continue;
                }
                let path = &index.files[file].path;
                if !seen.insert((path.clone(), d.span.line, d.span.col)) {
                    continue;
                }
                let container = d.container.map(|c| index.defs[c].name.clone());
                symbols.push(json!({
                    "name": d.name,
                    "kind": symbol_kind(d.kind),
                    "location": { "uri": path_uri(path), "range": span_range(&self.text_of(path), &d.span) },
                    "containerName": container,
                }));
            }
        }
        json!(symbols)
    }

    fn completion(&self, params: &Value) -> Value {
        let Some(path) = uri_path(&params["textDocument"]["uri"]) else { return Value::Null };
        let line = params["position"]["line"].as_u64().unwrap_or(0) as usize;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as usize;
        let text = self.text_of(&path);
        let line_text = text.lines().nth(line).unwrap_or("");
        let col = utf16_to_col(line_text, character);
        let before: Vec<char> = line_text.chars().take(col).collect();

        // The word being typed and what's in front of it
        let word_start = before.iter().rposition(|c| !(c.is_alphanumeric() || *c == '_')).map_or(0, |p| p + 1);
        let prefix: String = before[..word_start].iter().collect();

        // `import std::` lists the modules of the standard library. The
        // unfinished import keeps the program from loading, so it's done first
        if prefix.trim() == "import std::" {
            let modules: Vec<Value> = module::STD.iter()
                .map(|(module, _)| json!({ "label": module, "kind": 9, "detail": format!("module std::{}", module) }))
                .collect();
            return json!(modules);
        }

        let Some((analysis, file)) = self.analysis_for(&path) else { return json!(keyword_items()) };
        let index = &analysis.index;
        // From the text as it is now, the analysis can be older while typing
        let imports = module::imports_of(&text);
        let imported: Vec<String> = imports.iter().map(|i| i.name.clone()).collect();
        // A new import goes after the last one
        let import_line = imports.iter().map(|i| i.span.line + 1).max().unwrap_or(0);

        if let Some(receiver) = prefix.strip_suffix('.') {
            let chain = trailing_chain(receiver);
            // Private members are only usable inside their own class
            let class_here = index.function_at(file, line).and_then(|f| index.defs[f].container);
            let items: Vec<Value> = resolve_chain(index, file, line, &chain)
                .map(|ty| index.members(&ty).into_iter()
                    // After `.` come fields and methods, an enum's variants and the
                    // functions without `self` come after `::`
                    .filter(|&m| index.defs[m].kind != DefKind::Variant)
                    .filter(|&m| index.defs[m].kind != DefKind::Method || index.defs[m].takes_self)
                    .filter(|&m| index.defs[m].public || index.defs[m].container == class_here && class_here.is_some())
                    .map(|m| item(index, m))
                    .collect())
                .unwrap_or_default();
            return json!(items);
        }
        if let Some(owner) = prefix.strip_suffix("::") {
            let chain = trailing_chain(owner);
            let Some(name) = chain.last() else { return json!([]) };
            // The items of a standard library module, which is imported if it isn't yet
            if index.key(&format!("enum:{}", name)).is_none() && module::std_source(name).is_some() {
                let module_name = format!("std::{}", name);
                let edit = (!imported.contains(&module_name))
                    .then(|| insertion(import_line, 0, format!("import {};\n", module_name)));
                let items: Vec<Value> = self.importables(&path, &index.files[file].name).iter()
                    .filter(|i| i.module == module_name)
                    .map(|i| importable_item(i, edit.clone()))
                    .collect();
                return json!(items);
            }
            // After `Name::` the variants and the functions without `self`, methods are called on values
            let items: Vec<Value> = index.key(&format!("enum:{}", name))
                .or_else(|| index.key(&format!("class:{}", name)))
                .map(|owner| index.members(&index.defs[owner].name).into_iter()
                    .filter(|&m| index.defs[m].kind == DefKind::Variant
                        || index.defs[m].kind == DefKind::Method && !index.defs[m].takes_self)
                    .map(|m| item(index, m))
                    .collect())
                .unwrap_or_default();
            return json!(items);
        }

        let mut items = keyword_items();
        // Inner variables shadow outer ones, the latest declaration wins. One
        // declared on this line doesn't exist yet in its own value, like the
        // first `args` in `let args = args();`
        let mut locals: Vec<usize> = index.locals_at(file, line).into_iter()
            .filter(|&d| index.defs[d].span.line < line || index.defs[d].kind == DefKind::Param)
            .collect();
        locals.sort_by_key(|&d| std::cmp::Reverse((index.defs[d].span.line, index.defs[d].span.col)));
        let mut local_names = HashSet::new();
        for def in locals {
            if local_names.insert(index.defs[def].name.clone()) {
                items.push(item(index, def));
            }
        }
        // Functions and types are looked up apart from variables, so a variable
        // doesn't hide a function with its name
        let mut names = HashSet::new();
        for def in index.globals_for(file) {
            // Standard library items like `fs::read` come below, written by their own name
            if index.defs[def].name.contains("::") {
                continue;
            }
            if names.insert(index.defs[def].name.clone()) {
                items.push(item(index, def));
            }
        }
        // `Ok`, `Err`, `Some` and `None` need no `Result::` or `Option::`, of a program's
        // own enum or the one of `std::result` and `std::option`
        for (enum_name, variant) in [("Result", "Ok"), ("Result", "Err"), ("Option", "Some"), ("Option", "None")] {
            let std_name = format!("{}::{}", enum_name.to_lowercase(), enum_name);
            let def = index.key(&format!("variant:{}::{}", enum_name, variant))
                .or_else(|| index.key(&format!("variant:{}::{}", std_name, variant)));
            if let Some(def) = def {
                if names.insert(variant.to_string()) {
                    items.push(item(index, def));
                }
            }
        }
        // What other modules have. Picking it adds its import if needed: the
        // item itself for the standard library, like `import std::env::{args};`,
        // the whole module for the program's own modules
        for importable in self.importables(&path, &index.files[file].name) {
            let edit = if importable.module.starts_with("std::") {
                item_import(&text, &imports, &importable, import_line)
            } else if imported.contains(&importable.module) {
                // Its items are offered above already
                continue;
            } else {
                Some(insertion(import_line, 0, format!("import {};\n", importable.module)))
            };
            if names.insert(short_name(&importable.name).to_string()) {
                items.push(importable_item(&importable, edit));
            }
        }
        json!(items)
    }

    /// The public top-level declarations of the modules the file at `path`,
    /// the module `own`, could import: the standard library modules and the
    /// other `.lv` files in its directory.
    ///
    /// They're only parsed, not checked, so they can be suggested without
    /// becoming part of the program before they're imported.
    fn importables(&self, path: &Path, own: &str) -> Vec<Importable> {
        let mut modules: Vec<module::Module> = module::STD.iter()
            .map(|(name, source)| module::Module {
                imports: vec![],
                name: format!("std::{}", name),
                path: module::std_path(name),
                source: source.to_string(),
            })
            .collect();
        let builtin = |p: &Path| {
            let canonical = module::canonical(p);
            canonical.is_some() && (canonical == module::canonical(module::prelude_path())
                || module::STD.iter().any(|(name, _)| canonical == module::canonical(module::std_path(name))))
        };
        let siblings = path.parent().and_then(|dir| std::fs::read_dir(dir).ok()).into_iter().flatten().flatten();
        for sibling in siblings.map(|entry| entry.path()) {
            let Some(stem) = sibling.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            // `main` is the program itself, nothing imports it
            if sibling.extension().is_none_or(|e| e != "lv") || sibling == path || stem == "main" || builtin(&sibling) {
                continue;
            }
            modules.push(module::Module { imports: vec![], name: stem, source: self.text_of(&sibling), path: sibling });
        }

        modules.into_iter()
            .filter(|m| m.name != own)
            .flat_map(|m| {
                let parsed = diagnostic::catch(|| Parser::with_renames(m.source.clone(), m.renames()).parse_file());
                // A file being written may not parse yet, its declarations are still found by name
                let Ok(stmts) = parsed else { return scanned_importables(&m) };
                stmts.into_iter().filter_map(|stmt| {
                    let StmtKind::Declaration(decl) = stmt.kind else { return None };
                    if !matches!(decl.visibility, Visibility::Public) {
                        return None;
                    }
                    let (name, kind, signature) = match decl.kind {
                        DeclarationKind::Function { name, params, return_type, .. } => {
                            let params: Vec<String> = params.iter().map(|p| format!("{}: {}", p.name, show_type(&p.ty))).collect();
                            let ret = return_type.filter(|t| *t != Type::Void).map(|t| format!(" -> {}", show_type(&t))).unwrap_or_default();
                            let signature = format!("{}({}){}", short_name(&name), params.join(", "), ret);
                            (name, DefKind::Function, signature)
                        }
                        DeclarationKind::Class { name, .. } => (name.clone(), DefKind::Class, format!("class {}", short_name(&name))),
                        DeclarationKind::Enum { name, .. } => (name.clone(), DefKind::Enum, format!("enum {}", short_name(&name))),
                        DeclarationKind::Trait { name, .. } => (name.clone(), DefKind::Trait, format!("trait {}", short_name(&name))),
                        DeclarationKind::Extern(ExternItem::Type { name, .. }) =>
                            (name.clone(), DefKind::Class, format!("extern type {}", short_name(&name))),
                        DeclarationKind::Impl { .. } | DeclarationKind::Extern(ExternItem::Link(_)) => return None,
                    };
                    Some(Importable { name, kind, signature, module: m.name.clone() })
                }).collect()
            })
            .collect()
    }

    fn signature_help(&self, params: &Value) -> Value {
        let Some(path) = uri_path(&params["textDocument"]["uri"]) else { return Value::Null };
        let line = params["position"]["line"].as_u64().unwrap_or(0) as usize;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as usize;
        let text = self.text_of(&path);
        let line_text = text.lines().nth(line).unwrap_or("");
        let before: Vec<char> = line_text.chars().take(utf16_to_col(line_text, character)).collect();

        // The innermost unclosed `(` and how many arguments come before the cursor
        let mut depth = 0i32;
        let mut commas = 0;
        let mut open = None;
        for (i, c) in before.iter().enumerate().rev() {
            match c {
                ')' => depth += 1,
                '(' if depth == 0 => { open = Some(i); break; }
                '(' => depth -= 1,
                ',' if depth == 0 => commas += 1,
                _ => {}
            }
        }
        let Some(open) = open else { return Value::Null };
        let callee: String = before[..open].iter().collect();
        let chain = trailing_chain(&callee);
        let Some(name) = chain.last().cloned() else { return Value::Null };

        let Some((analysis, file)) = self.analysis_for(&path) else { return Value::Null };
        let index = &analysis.index;
        let def = if chain.len() > 1 {
            resolve_chain(index, file, line, &chain[..chain.len() - 1])
                .and_then(|ty| index.members(&ty).into_iter().find(|&m| index.defs[m].name == name))
        } else {
            index.key(&format!("fn:{}", name))
                .or_else(|| index.function_at(file, line)
                    .and_then(|f| index.defs[f].container)
                    .and_then(|c| index.members(&index.defs[c].name).into_iter().find(|&m| index.defs[m].name == name)))
        };
        let Some(def) = def else { return Value::Null };

        let label = index.defs[def].signature.lines().next().unwrap_or("").to_string();
        // `self` isn't passed in the parentheses
        let parameters: Vec<String> = signature_params(&label).into_iter().filter(|p| !p.ends_with("&self")).collect();
        json!({
            "signatures": [{
                "label": label,
                "parameters": parameters.iter().map(|p| json!({ "label": p })).collect::<Vec<_>>(),
            }],
            "activeSignature": 0,
            "activeParameter": commas.min(parameters.len().saturating_sub(1)),
        })
    }

    fn code_actions(&self, params: &Value) -> Value {
        let Some(path) = uri_path(&params["textDocument"]["uri"]) else { return json!([]) };
        let Some((analysis, file)) = self.analysis_for(&path) else { return json!([]) };
        let start = params["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
        let end = params["range"]["end"]["line"].as_u64().unwrap_or(0) as usize;
        let text = self.text_of(&path);
        let uri = path_uri(&path);
        let index = &analysis.index;

        let mut actions = Vec::new();
        for warning in analysis.warnings.iter().filter(|w| w.file == file && w.span.line >= start && w.span.line <= end) {
            let edits: Vec<Value> = match warning.code {
                "unused_variable" | "unused_parameter" => {
                    // Every write to it gets the new name too
                    let def = index.def_at_position(file, warning.span.line, warning.span.col);
                    def.map(|d| index.occurrences(d).into_iter()
                        .filter(|(f, _)| *f == file)
                        .map(|(_, span)| json!({ "range": span_range(&text, &Span { len: 0, ..span }), "newText": "_" }))
                        .collect())
                        .unwrap_or_default()
                }
                "unused_mut" => {
                    let line: Vec<char> = text.lines().nth(warning.span.line).unwrap_or("").chars().collect();
                    let before: String = line[..warning.span.col.min(line.len())].iter().collect();
                    match before.rfind("mut") {
                        // `mut x = ...` becomes `let x = ...`, `mut x: T` in a parameter list loses `mut `
                        Some(at) => {
                            let at = before[..at].chars().count();
                            let is_variable = before.trim_end().ends_with("mut") && before[..before.rfind("mut").unwrap()].trim().is_empty();
                            let span = Span { line: warning.span.line, col: at, len: if is_variable { 3 } else { 4 } };
                            vec![json!({ "range": span_range(&text, &span), "newText": if is_variable { "let" } else { "" } })]
                        }
                        None => vec![],
                    }
                }
                "naming" => {
                    let Some(suggestion) = warning.message.rsplit("like '").next().and_then(|s| s.strip_suffix('\'')) else { continue };
                    let def = index.def_at_position(file, warning.span.line, warning.span.col);
                    def.map(|d| self.occurrences(analysis, d).into_iter()
                        .filter(|(f, _, _)| *f == file)
                        .map(|(_, span, _)| json!({ "range": span_range(&text, &span), "newText": suggestion }))
                        .collect())
                        .unwrap_or_default()
                }
                _ => vec![],
            };
            if edits.is_empty() {
                continue;
            }
            let title = match warning.code {
                "unused_variable" | "unused_parameter" => "Prefix the name with '_'".to_string(),
                "unused_mut" => "Remove 'mut'".to_string(),
                _ => "Rename as suggested".to_string(),
            };
            actions.push(json!({
                "title": title,
                "kind": "quickfix",
                "edit": { "changes": { uri.clone(): edits } },
            }));
        }
        json!(actions)
    }
}

/// Whether `declaration` is at `span` of `file`.
fn is_same(declaration: &Option<(usize, Span)>, file: usize, span: &Span) -> bool {
    declaration.as_ref().is_some_and(|(f, s)| *f == file && s.line == span.line && s.col == span.col)
}

/// Whether the definition can be renamed: built-ins, modules and constructors can't.
fn renamable(index: &Index, def: usize) -> bool {
    let d = &index.defs[def];
    d.file.is_some() && !matches!(d.kind, DefKind::Builtin | DefKind::Module | DefKind::Constructor) && d.name != "self"
}

/// LSP `SymbolKind` of a definition.
fn symbol_kind(kind: DefKind) -> u32 {
    match kind {
        DefKind::Module => 2,
        DefKind::Class => 5,
        DefKind::Method => 6,
        DefKind::Field => 8,
        DefKind::Constructor => 9,
        DefKind::Enum => 10,
        DefKind::Trait => 11,
        DefKind::Function | DefKind::Builtin => 12,
        DefKind::Variable | DefKind::Param => 13,
        DefKind::Variant => 22,
    }
}

/// LSP `CompletionItemKind` of a definition.
fn completion_kind(kind: DefKind) -> u32 {
    match kind {
        DefKind::Module => 9,
        DefKind::Class => 7,
        DefKind::Method => 2,
        DefKind::Field => 5,
        DefKind::Constructor => 4,
        DefKind::Enum => 13,
        DefKind::Trait => 8,
        DefKind::Function | DefKind::Builtin => 3,
        DefKind::Variable | DefKind::Param => 6,
        DefKind::Variant => 20,
    }
}

/// A completion item for `def`. Functions get a snippet with parentheses.
/// A public declaration of a module a file doesn't import yet, see
/// [`Server::importables`].
struct Importable {
    /// How it's written: `fs::read` for an item of the standard library,
    /// `add` for one of the program's own modules.
    name: String,
    kind: DefKind,
    /// Like `read(path: String) -> Result<String, String>`.
    signature: String,
    /// The module to import, like `std::fs` or `math`.
    module: String,
}

/// The completion item of `importable`, written by its own name like `read`.
/// Picking it also applies `import`, the edit importing it, if given.
fn importable_item(importable: &Importable, import: Option<Value>) -> Value {
    let short = short_name(&importable.name);
    let mut item = json!({
        "label": short,
        "kind": completion_kind(importable.kind),
        "detail": importable.signature,
        "labelDetails": { "description": importable.module },
        "insertText": short,
    });
    if importable.kind == DefKind::Function {
        item["insertText"] = json!(format!("{}($0)", short));
        item["insertTextFormat"] = json!(2);
    }
    if let Some(import) = import {
        item["labelDetails"]["description"] = json!(format!("import {}", importable.module));
        item["additionalTextEdits"] = json!([import]);
    }
    item
}

/// The edit importing the standard library item `importable` by its name,
/// or `None` if one of `imports`, those of `text`, does already.
///
/// An import list of its module on one line, like `import std::env::{Args};`,
/// gets it added: `import std::env::{Args, args};`. Otherwise a new import
/// like `import std::env::{args};` goes to `import_line`.
fn item_import(text: &str, imports: &[module::Import], importable: &Importable, import_line: usize) -> Option<Value> {
    let short = short_name(&importable.name);
    let of_module: Vec<&module::Import> = imports.iter().filter(|i| i.name == importable.module).collect();
    if of_module.iter().any(|i| i.items.iter().any(|(item, _)| item == short)) {
        return None;
    }
    for import in &of_module {
        let Some((_, last)) = import.items.last() else { continue };
        let line = text.lines().nth(last.line).unwrap_or("");
        let end = last.col + last.len;
        let rest: String = line.chars().skip(end).collect();
        if rest.trim_start().trim_start_matches(',').trim_start().starts_with('}') {
            let character = line.chars().take(end).map(char::len_utf16).sum();
            return Some(insertion(last.line, character, format!(", {}", short)));
        }
    }
    Some(insertion(import_line, 0, format!("import {}::{{{}}};\n", importable.module, short)))
}

/// An edit inserting `text` at `line` and the UTF-16 `character`.
fn insertion(line: usize, character: usize, text: String) -> Value {
    let position = json!({ "line": line, "character": character });
    json!({ "range": { "start": position, "end": position }, "newText": text })
}

/// The public top-level declarations of `module`, which doesn't parse, found
/// by their `pub class Name`, `pub func name` and the like.
fn scanned_importables(module: &module::Module) -> Vec<Importable> {
    use crate::token::Token;
    let mut lexer = crate::lexer::Lexer::new(module.source.clone());
    let mut found = Vec::new();
    let mut depth = 0usize;
    // The tokens since the last one that can't be part of a declaration's start
    let mut pending: Vec<Token> = Vec::new();
    let mut errors = 0;
    loop {
        // Characters Lava doesn't have, like in code copied from elsewhere, are skipped
        let token = match lexer.get_next_token() {
            Ok((token, _)) => token,
            Err(_) if errors < 1000 => {
                errors += 1;
                pending.clear();
                continue;
            }
            Err(_) => break,
        };
        match token {
            Token::EOF => break,
            Token::LBrace => depth += 1,
            Token::RBrace => depth = depth.saturating_sub(1),
            Token::Identifier(name) if depth == 0 => {
                let kind = match pending.as_slice() {
                    [.., Token::Pub, Token::Class] => Some((DefKind::Class, "class")),
                    [.., Token::Pub, Token::Enum] => Some((DefKind::Enum, "enum")),
                    [.., Token::Pub, Token::Trait] => Some((DefKind::Trait, "trait")),
                    [.., Token::Pub, Token::Func] => Some((DefKind::Function, "func")),
                    _ => None,
                };
                if let Some((kind, keyword)) = kind {
                    let signature = if keyword == "func" { format!("{}(…)", name) } else { format!("{} {}", keyword, name) };
                    found.push(Importable { name, kind, signature, module: module.name.clone() });
                }
                pending.clear();
                continue;
            }
            token @ (Token::Pub | Token::Class | Token::Enum | Token::Trait | Token::Func) => {
                pending.push(token);
                continue;
            }
            _ => {}
        }
        pending.clear();
    }
    found
}

/// The first name in quotes in `message`, `x` of "unknown identifier 'x'".
fn quoted_name(message: &str) -> Option<&str> {
    let (_, rest) = message.split_once('\'')?;
    let (name, _) = rest.split_once('\'')?;
    Some(short_name(name))
}

/// Whether `line` contains `word` on its own, not as part of a longer name.
fn contains_word(line: &str, word: &str) -> bool {
    let is_name = |c: char| c.is_alphanumeric() || c == '_';
    line.split(|c: char| !is_name(c)).any(|part| part == word)
}

/// `name` without its module, `read` for `fs::read`.
fn short_name(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

fn item(index: &Index, def: usize) -> Value {
    let d = &index.defs[def];
    let detail = match d.kind {
        DefKind::Variable | DefKind::Param | DefKind::Field => show_type(&d.ty),
        _ => d.signature.lines().next().unwrap_or("").to_string(),
    };
    let callable = matches!(d.kind, DefKind::Function | DefKind::Method)
        || d.kind == DefKind::Builtin && d.signature.contains('(');
    // Editors show `detail` above the documentation, a one line signature would be there twice
    let documentation = if d.kind == DefKind::Builtin {
        let docs = d.signature.split_once("\n\n").map(|(_, docs)| docs).unwrap_or("");
        format!("{}\n\n*built in*", docs)
    } else if d.signature.lines().count() > 1 {
        // A class or trait shows its members
        index.describe(def)
    } else {
        d.file.filter(|_| !matches!(d.kind, DefKind::Variable | DefKind::Param))
            .map(|f| format!("*in module `{}`*", index.files[f].name))
            .unwrap_or_default()
    };
    let mut item = json!({
        "label": d.name,
        "kind": completion_kind(d.kind),
        "detail": detail,
        "documentation": { "kind": "markdown", "value": documentation },
    });
    if callable {
        item["insertText"] = json!(format!("{}($0)", d.name));
        item["insertTextFormat"] = json!(2);
    }
    item
}

const KEYWORDS: &[&str] = &[
    "as", "break", "class", "Constructor", "continue", "else", "enum", "extern", "false", "for", "func", "if", "impl", "import", "let",
    "match", "mut", "print", "pub", "return", "self", "trait", "true", "while",
];

const TYPES: &[&str] = &[
    "bool", "char", "byte", "String", "int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64",
    "float32", "float64", "Unique", "Shared", "Buffer",
];

fn keyword_items() -> Vec<Value> {
    KEYWORDS.iter().map(|k| json!({ "label": k, "kind": 14 }))
        .chain(TYPES.iter().map(|t| json!({ "label": t, "kind": 25, "detail": "built-in type" })))
        .collect()
}

/// The names of an access chain at the end of `text`, like `["self", "data"]`
/// for `print(self.data`. Calls in the chain are skipped, so `a.get(0).b`
/// gives `["a", "get", "b"]`.
fn trailing_chain(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut names = Vec::new();
    let mut end = chars.len();
    loop {
        // Skip a call's arguments `(...)`
        if end > 0 && chars[end - 1] == ')' {
            let mut depth = 0;
            let mut i = end;
            while i > 0 {
                i -= 1;
                match chars[i] {
                    ')' => depth += 1,
                    '(' => { depth -= 1; if depth == 0 { break; } }
                    _ => {}
                }
            }
            end = i;
        }
        let start = chars[..end].iter().rposition(|c| !(c.is_alphanumeric() || *c == '_')).map_or(0, |p| p + 1);
        if start == end {
            break;
        }
        names.push(chars[start..end].iter().collect::<String>());
        if start > 0 && chars[start - 1] == '.' {
            end = start - 1;
        } else if start > 1 && chars[start - 1] == ':' && chars[start - 2] == ':' {
            end = start - 2;
        } else {
            break;
        }
    }
    names.reverse();
    names
}

/// The name of the type the access chain `chain` evaluates to at `line` of `file`.
fn resolve_chain(index: &Index, file: usize, line: usize, chain: &[String]) -> Option<String> {
    // `fs::read(path).` starts with the qualified name `fs::read`
    let qualified = (chain.len() > 1).then(|| format!("{}::{}", chain[0], chain[1]));
    let chain: Vec<String> = match qualified {
        Some(name) if index.globals_for(file).into_iter().any(|d| index.defs[d].name == name) =>
            std::iter::once(name).chain(chain[2..].iter().cloned()).collect(),
        _ => chain.to_vec(),
    };
    let (first, rest) = chain.split_first()?;

    // A local variable, the latest declaration before the line wins
    let mut locals = index.locals_at(file, line);
    locals.sort_by_key(|&d| std::cmp::Reverse((index.defs[d].span.line, index.defs[d].span.col)));
    let mut ty = match locals.into_iter().find(|&d| &index.defs[d].name == first) {
        Some(def) => index.defs[def].ty.clone(),
        // A function's result, like `half(4).` or `fs::read(path).`, has the type it returns.
        // `Point.` isn't valid, an enum's name before `::` is handled by the caller
        None => {
            let suffix = format!("::{}", first);
            let function = index.globals_for(file).into_iter()
                .find(|&d| index.defs[d].kind == DefKind::Function && (index.defs[d].name == *first || index.defs[d].name.ends_with(&suffix)))?;
            index.defs[function].ty.clone()
        }
    };
    if first == "self" && ty == Type::Unknown {
        let function = index.function_at(file, line)?;
        ty = Type::Class(index.defs[index.defs[function].container?].name.clone());
    }

    for name in rest {
        let owner = type_owner(&ty)?;
        let member = index.members(&owner).into_iter().find(|&m| &index.defs[m].name == name)?;
        ty = index.defs[member].ty.clone();
    }
    type_owner(&ty)
}

/// Name of the type whose members a value of type `ty` has.
fn type_owner(ty: &Type) -> Option<String> {
    match ty {
        Type::Ref(inner) | Type::Unique(inner) | Type::Shared(inner) => type_owner(inner),
        Type::Buffer(_) => Some("Buffer".to_string()),
        // Their built-in `len`, see `Index::members`
        Type::Array(..) => Some("Array".to_string()),
        Type::Slice(_) => Some("Slice".to_string()),
        Type::Generic { name, .. } => Some(match name.as_str() {
            "Unique" | "Shared" => return None,
            other => other.to_string(),
        }),
        other => other.class_name().or_else(|| other.owner_name()).map(|n| template_name(&n)),
    }
}

/// The parameters in a signature like `func f(a: int32, b: T) -> T`.
fn signature_params(signature: &str) -> Vec<String> {
    let Some(open) = signature.find('(') else { return vec![] };
    let mut depth = 0;
    let mut params = Vec::new();
    let mut current = String::new();
    for c in signature[open + 1..].chars() {
        match c {
            '(' | '<' => { depth += 1; current.push(c); }
            ')' if depth == 0 => break,
            ')' | '>' => { depth -= 1; current.push(c); }
            ',' if depth == 0 => { params.push(current.trim().to_string()); current.clear(); }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        params.push(current.trim().to_string());
    }
    params
}

// --- Positions ---

/// The character index for the UTF-16 offset `utf16` of `line`, which LSP positions use.
fn utf16_to_col(line: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in line.chars().enumerate() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    line.chars().count()
}

/// The UTF-16 offset of the character index `col` of `line`.
fn col_to_utf16(line: &str, col: usize) -> usize {
    line.chars().take(col).map(char::len_utf16).sum()
}

/// The LSP range of `span` in `text`.
fn span_range(text: &str, span: &Span) -> Value {
    let line = text.lines().nth(span.line).unwrap_or("");
    json!({
        "start": { "line": span.line, "character": col_to_utf16(line, span.col) },
        "end": { "line": span.line, "character": col_to_utf16(line, span.col + span.len) },
    })
}

/// The LSP range of the word at `span` in `text`, at least one character.
fn word_range(text: &str, span: &Span) -> Value {
    let line = text.lines().nth(span.line).unwrap_or("");
    let chars: Vec<char> = line.chars().collect();
    let col = span.col.min(chars.len());
    let mut end = col;
    while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    let end = if end == col { (col + span.len.max(1)).min(chars.len().max(col + 1)) } else { end };
    span_range(text, &Span { line: span.line, col, len: end - col })
}

/// The span of the identifier around `col` of `line`.
fn word_at(line_text: &str, line: usize, col: usize) -> Option<Span> {
    let chars: Vec<char> = line_text.chars().collect();
    let is_word = |c: &char| c.is_alphanumeric() || *c == '_';
    let mut start = col.min(chars.len());
    while start > 0 && is_word(&chars[start - 1]) {
        start -= 1;
    }
    let mut end = col.min(chars.len());
    while end < chars.len() && is_word(&chars[end]) {
        end += 1;
    }
    (start < end).then(|| Span { line, col: start, len: end - start })
}

/// The file path of a `file://` URI, canonical if the file exists.
fn uri_path(uri: &Value) -> Option<PathBuf> {
    let uri = uri.as_str()?;
    let path = uri.strip_prefix("file://")?;
    let mut decoded = Vec::new();
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&path[i + 1..i + 3], 16) {
                decoded.push(byte);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let path = PathBuf::from(String::from_utf8(decoded).ok()?);
    Some(module::canonical(&path).unwrap_or(path))
}

/// The `file://` URI of `path`.
fn path_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => uri.push(byte as char),
            other => uri.push_str(&format!("%{:02X}", other)),
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chains() {
        assert_eq!(trailing_chain("print(self.data"), vec!["self", "data"]);
        assert_eq!(trailing_chain("  v.get(0).x"), vec!["v", "get", "x"]);
        assert_eq!(trailing_chain("Color::Red"), vec!["Color", "Red"]);
        assert_eq!(trailing_chain("x"), vec!["x"]);
        assert!(trailing_chain("1 + ").is_empty());
    }

    #[test]
    fn signature_parameters() {
        assert_eq!(signature_params("func f(a: int32, b: Box<int32, T>) -> T"), vec!["a: int32", "b: Box<int32, T>"]);
        assert!(signature_params("func f()").is_empty());
    }

    #[test]
    fn utf16_positions() {
        assert_eq!(utf16_to_col("a😀b", 3), 2);
        assert_eq!(col_to_utf16("a😀b", 2), 3);
    }

    #[test]
    fn uris() {
        let path = PathBuf::from("/tmp/my dir/main.lv");
        assert_eq!(path_uri(&path), "file:///tmp/my%20dir/main.lv");
        assert_eq!(uri_path(&json!("file:///tmp/my%20dir/main.lv")), Some(path));
    }
}
