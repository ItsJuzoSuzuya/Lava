//! End-to-end tests for the language server: they start `lava lsp`, talk
//! to it over stdin and stdout like an editor and check its answers.

use std::{fs, io::{BufRead, BufReader, Read, Write}, path::{Path, PathBuf}, process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver}, thread, time::Duration};

use serde_json::{Value, json};

const MAIN: &str = "import vector;

class Point {
  pub x: int32 = x;

  Constructor(x: int32);

  pub func double(&self) -> int32 {
    return self.x * 2;
  }
}

func add(a: int32, b: int32) -> int32 {
  return a + b;
}

mut v = Vector<int32>();
v.push(add(1, 2));
let p = Point { x: 3 };
print(p.double());
let unused = 1;
";

const VECTOR: &str = "pub class Vector<T> {
  data: Buffer<T> = Buffer<T>(4);
  length: uint64 = 0;

  Constructor();

  pub func push(mut &self, item: T) {
    if self.length == self.data.capacity() {
      self.data.resize(self.data.capacity() * 2);
    }
    self.data.set(self.length, item);
    self.length = self.length + 1;
  }

  pub func len(&self) -> uint64 {
    return self.length;
  }
}
";

/// A language server process and the messages it sent.
struct Client {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
    next_id: i64,
    notifications: Vec<Value>,
}

impl Client {
    fn start(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lava"))
            .arg("lsp")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start the language server");
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());

        // Reads on a thread, so a server that hangs fails the test instead of blocking it
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || {
            loop {
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if stdout.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim_end();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("Content-Length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                stdout.read_exact(&mut body).unwrap();
                if sender.send(serde_json::from_slice(&body).unwrap()).is_err() {
                    return;
                }
            }
        });
        Client { child, stdin, messages, next_id: 1, notifications: Vec::new() }
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.stdin.flush().unwrap();
    }

    fn receive(&mut self) -> Value {
        self.messages.recv_timeout(Duration::from_secs(20)).expect("the language server didn't answer")
    }

    /// Sends a request and returns its response, keeping notifications sent meanwhile.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.receive();
            if message["id"] == json!(id) {
                return message;
            }
            self.notifications.push(message);
        }
    }

    fn result(&mut self, method: &str, params: Value) -> Value {
        let response = self.request(method, params);
        assert!(response.get("error").is_none(), "{} failed: {}", method, response);
        response["result"].clone()
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// The latest diagnostics published for `uri`, waiting for them if needed.
    fn diagnostics(&mut self, uri: &str) -> Vec<Value> {
        // A request after the change makes sure its diagnostics were sent before
        self.result("workspace/symbol", json!({ "query": "" }));
        self.notifications.iter().rev()
            .find(|n| n["method"] == "textDocument/publishDiagnostics" && n["params"]["uri"] == uri)
            .map(|n| n["params"]["diagnostics"].as_array().unwrap().clone())
            .unwrap_or_default()
    }

    fn open(&mut self, uri: &str, text: &str) {
        self.notify("textDocument/didOpen", json!({
            "textDocument": { "uri": uri, "languageId": "lava", "version": 1, "text": text },
        }));
    }

    fn change(&mut self, uri: &str, version: i64, text: &str) {
        self.notify("textDocument/didChange", json!({
            "textDocument": { "uri": uri, "version": version },
            "contentChanges": [{ "text": text }],
        }));
    }
}

fn at(uri: &str, line: u64, character: u64) -> Value {
    json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } })
}

/// A project with `main.lv` and `vector.lv` in a fresh directory.
fn project(name: &str) -> (PathBuf, String, String) {
    let dir = std::env::temp_dir().join(format!("lava-lsp-{}-{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("main.lv"), MAIN).unwrap();
    fs::write(dir.join("vector.lv"), VECTOR).unwrap();
    let dir = fs::canonicalize(dir).unwrap();
    let uri = |file: &str| format!("file://{}", dir.join(file).display());
    let (main, vector) = (uri("main.lv"), uri("vector.lv"));
    (dir, main, vector)
}

fn started(name: &str) -> (Client, PathBuf, String, String) {
    let (dir, main, vector) = project(name);
    let mut client = Client::start(&dir);
    let init = client.result("initialize", json!({ "processId": null, "rootUri": format!("file://{}", dir.display()), "capabilities": {} }));
    assert_eq!(init["capabilities"]["hoverProvider"], true);
    client.notify("initialized", json!({}));
    client.open(&main, MAIN);
    (client, dir, main, vector)
}

fn stop(mut client: Client, dir: PathBuf) {
    client.result("shutdown", Value::Null);
    client.notify("exit", Value::Null);
    let status = client.child.wait().unwrap();
    assert!(status.success(), "the server didn't exit cleanly: {}", status);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn diagnostics_show_warnings_and_errors() {
    let (mut client, dir, main, _) = started("diagnostics");

    let diagnostics = client.diagnostics(&main);
    assert_eq!(diagnostics.len(), 1, "{:?}", diagnostics);
    assert_eq!(diagnostics[0]["message"], "variable 'unused' is never used");
    assert_eq!(diagnostics[0]["severity"], 2);
    assert_eq!(diagnostics[0]["range"]["start"], json!({ "line": 20, "character": 4 }));
    assert_eq!(diagnostics[0]["range"]["end"], json!({ "line": 20, "character": 10 }));

    // A type error is shown next to the warnings, the rest of the file is still linted
    client.change(&main, 2, &MAIN.replace("let unused = 1;", "let unused: String = 1;"));
    let diagnostics = client.diagnostics(&main);
    assert_eq!(diagnostics.len(), 2, "{:?}", diagnostics);
    assert_eq!(diagnostics[0]["severity"], 1);
    assert_eq!(diagnostics[1]["message"], "variable 'unused' is never used");
    assert!(diagnostics[0]["message"].as_str().unwrap().contains("does not match"));
    assert_eq!(diagnostics[0]["range"]["start"]["line"], 20);

    // Fixing it brings back a clean state
    client.change(&main, 3, &MAIN.replace("let unused = 1;\n", "print(v.len());\n"));
    assert!(client.diagnostics(&main).is_empty());

    stop(client, dir);
}

#[test]
fn syntax_errors_are_reported_and_features_keep_working() {
    let (mut client, dir, main, _) = started("syntax");

    let broken = MAIN.replace("let unused = 1;", "let unused = ;");
    client.change(&main, 2, &broken);
    let diagnostics = client.diagnostics(&main);
    assert_eq!(diagnostics.len(), 1, "{:?}", diagnostics);
    assert_eq!(diagnostics[0]["severity"], 1);
    assert_eq!(diagnostics[0]["range"]["start"]["line"], 20);

    // Hover still answers from the last check that parsed
    let hover = client.result("textDocument/hover", at(&main, 17, 0));
    assert!(hover["contents"]["value"].as_str().unwrap().contains("mut v: Vector<int32>"), "{}", hover);

    stop(client, dir);
}

#[test]
fn hover_shows_types_and_signatures() {
    let (mut client, dir, main, _) = started("hover");

    let hover = |client: &mut Client, line, character| {
        let result = client.result("textDocument/hover", at(&main, line, character));
        result["contents"]["value"].as_str().unwrap_or("").to_string()
    };
    assert!(hover(&mut client, 17, 0).contains("mut v: Vector<int32>"));
    assert!(hover(&mut client, 19, 9).contains("pub func Point.double(&self) -> int32"));
    assert!(hover(&mut client, 17, 7).contains("func add(a: int32, b: int32) -> int32"));
    assert!(hover(&mut client, 13, 9).contains("a: int32"));
    assert!(hover(&mut client, 8, 16).contains("Point.x: int32"));
    assert!(hover(&mut client, 17, 3).contains("pub func Vector.push(mut &self, item: T)"));
    assert!(hover(&mut client, 19, 0).contains("built in"));
    assert_eq!(client.result("textDocument/hover", at(&main, 1, 0)), Value::Null);

    stop(client, dir);
}

#[test]
fn go_to_definition_across_files() {
    let (mut client, dir, main, vector) = started("definition");

    let push = client.result("textDocument/definition", at(&main, 17, 3));
    assert_eq!(push["uri"], vector);
    assert_eq!(push["range"]["start"], json!({ "line": 6, "character": 11 }));

    let add = client.result("textDocument/definition", at(&main, 17, 8));
    assert_eq!(add["uri"], main);
    assert_eq!(add["range"]["start"], json!({ "line": 12, "character": 5 }));

    let module = client.result("textDocument/definition", at(&main, 0, 9));
    assert_eq!(module["uri"], vector);

    let class = client.result("textDocument/definition", at(&main, 18, 9));
    assert_eq!(class["range"]["start"], json!({ "line": 2, "character": 6 }), "a brace initializer goes to the class");

    stop(client, dir);
}

#[test]
fn references_and_rename() {
    let (mut client, dir, main, vector) = started("rename");

    let references = client.result("textDocument/references", json!({
        "textDocument": { "uri": main }, "position": { "line": 12, "character": 6 }, "context": { "includeDeclaration": true },
    }));
    assert_eq!(references.as_array().unwrap().len(), 2);

    let prepared = client.result("textDocument/prepareRename", at(&main, 17, 3));
    assert_eq!(prepared["start"], json!({ "line": 17, "character": 2 }));

    let edit = client.result("textDocument/rename", json!({
        "textDocument": { "uri": main }, "position": { "line": 17, "character": 3 }, "newName": "append",
    }));
    let changes = edit["changes"].as_object().unwrap();
    assert_eq!(changes[&main].as_array().unwrap().len(), 1);
    assert_eq!(changes[&vector].as_array().unwrap().len(), 1);

    // Renaming a class also renames its constructor calls
    let edit = client.result("textDocument/rename", json!({
        "textDocument": { "uri": main }, "position": { "line": 2, "character": 7 }, "newName": "Spot",
    }));
    assert_eq!(edit["changes"][&main].as_array().unwrap().len(), 2);

    let invalid = client.request("textDocument/rename", json!({
        "textDocument": { "uri": main }, "position": { "line": 12, "character": 6 }, "newName": "1abc",
    }));
    assert!(invalid.get("error").is_some());

    stop(client, dir);
}

#[test]
fn completion_and_signature_help() {
    let (mut client, dir, main, _) = started("completion");

    let labels = |items: &Value| -> Vec<String> {
        items.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap().to_string()).collect()
    };

    // Typing `v.` breaks the syntax, members come from the last good check
    client.change(&main, 2, &format!("{}v.", MAIN));
    let members = labels(&client.result("textDocument/completion", at(&main, 21, 2)));
    assert!(members.contains(&"push".to_string()), "{:?}", members);
    assert!(members.contains(&"len".to_string()));
    assert!(!members.contains(&"data".to_string()), "private fields aren't offered outside the class");

    client.change(&main, 3, &format!("{}p.", MAIN));
    let members = labels(&client.result("textDocument/completion", at(&main, 21, 2)));
    assert_eq!(members, vec!["x", "double"]);

    let everything = labels(&client.result("textDocument/completion", at(&main, 21, 0)));
    for expected in ["add", "Point", "Vector", "v", "p", "while", "int32", "print"] {
        assert!(everything.contains(&expected.to_string()), "missing {} in {:?}", expected, everything);
    }

    client.change(&main, 4, &format!("{}print(add(1, ", MAIN));
    let help = client.result("textDocument/signatureHelp", at(&main, 21, 13));
    assert_eq!(help["signatures"][0]["label"], "func add(a: int32, b: int32) -> int32");
    assert_eq!(help["activeParameter"], 1);

    stop(client, dir);
}

#[test]
fn completion_in_field_defaults() {
    let (mut client, dir, main, _) = started("field_defaults");

    let labels = |items: &Value| -> Vec<String> {
        items.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap().to_string()).collect()
    };

    // Field defaults run in the constructor, so they see its parameters
    let source = format!("{}class Holder {{\n  pub point: &Point;\n  pub y: int32 = point.double();\n}}\n", MAIN);
    client.change(&main, 2, &source);
    let line = source.lines().count() as u64 - 2;
    let everything = labels(&client.result("textDocument/completion", at(&main, line, 17)));
    assert!(everything.contains(&"point".to_string()), "{:?}", everything);

    let typed = source.replace("point.double();", "point.");
    client.change(&main, 3, &typed);
    let members = labels(&client.result("textDocument/completion", at(&main, line, 23)));
    assert!(members.contains(&"double".to_string()), "{:?}", members);

    stop(client, dir);
}

#[test]
fn completion_imports_what_it_suggests() {
    let (mut client, dir, main, _) = started("auto_import");
    fs::write(dir.join("shapes.lv"), "pub func area(w: int32, h: int32) -> int32 {\n  return w * h;\n}\nfunc hidden() {}\n").unwrap();
    let find = |items: &Value, label: &str| items.as_array().unwrap().iter().find(|i| i["label"] == label).cloned();
    let import_of = |item: &Value| item["additionalTextEdits"][0]["newText"].as_str().map(str::to_string);

    // A module of the program that isn't imported, its private functions aren't offered
    let everything = client.result("textDocument/completion", at(&main, 21, 0));
    let area = find(&everything, "area").expect("area of shapes.lv is suggested");
    assert_eq!(area["insertText"], "area($0)");
    assert_eq!(import_of(&area).as_deref(), Some("import shapes;\n"));
    assert_eq!(area["additionalTextEdits"][0]["range"]["start"]["line"], 1, "after the last import");
    assert!(find(&everything, "hidden").is_none());

    // A standard library item is written by its name and imported by it
    let read = find(&everything, "read").expect("fs::read is suggested");
    assert_eq!(read["insertText"], "read($0)");
    assert_eq!(import_of(&read).as_deref(), Some("import std::fs::{read};\n"));
    // The imported module's items need no import
    assert!(import_of(&find(&everything, "Vector").unwrap()).is_none());

    // An existing import list of the module gets the item added
    client.change(&main, 2, &format!("import std::env::{{Args}};\n{}", MAIN));
    let items = client.result("textDocument/completion", at(&main, 22, 0));
    let args = find(&items, "args").expect("env::args is suggested");
    assert_eq!(args["insertText"], "args($0)");
    assert_eq!(import_of(&args).as_deref(), Some(", args"));
    assert_eq!(args["additionalTextEdits"][0]["range"]["start"], json!({ "line": 0, "character": 22 }));
    let imported = find(&items, "Args").expect("the imported Args is suggested");
    assert!(import_of(&imported).is_none());

    // A variable with the same name, here the one being declared, doesn't hide the function
    client.change(&main, 3, &format!("{}let args = args", MAIN));
    let items = client.result("textDocument/completion", at(&main, 21, 15));
    let args: Vec<&Value> = items.as_array().unwrap().iter().filter(|i| i["label"] == "args").collect();
    assert_eq!(args.len(), 1, "only the function, the variable doesn't exist yet: {:?}", args);
    assert_eq!(args[0]["insertText"], "args($0)");
    assert_eq!(import_of(args[0]).as_deref(), Some("import std::env::{args};\n"));

    // After `fs::` only the name is written
    client.change(&main, 4, &format!("{}fs::", MAIN));
    let items = client.result("textDocument/completion", at(&main, 21, 4));
    let read = find(&items, "read").expect("fs:: offers read");
    assert_eq!(read["insertText"], "read($0)");
    assert_eq!(import_of(&read).as_deref(), Some("import std::fs;\n"));

    // Once imported, it's offered without adding the import again
    client.change(&main, 5, &format!("import std::fs;\n{}fs::", MAIN));
    let items = client.result("textDocument/completion", at(&main, 22, 4));
    let read = find(&items, "read").expect("fs:: offers read of the imported module");
    assert_eq!(read["insertText"], "read($0)");
    assert!(import_of(&read).is_none());

    // After `text.` the methods std::string adds to `String`, which import the module
    let with_text = format!("{}let text = \"a,b\";\n", MAIN);
    client.change(&main, 6, &with_text);
    client.diagnostics(&main);
    client.change(&main, 7, &format!("{}text.", with_text));
    let items = client.result("textDocument/completion", at(&main, 22, 5));
    let split = find(&items, "split").expect("split of std::string is offered after `text.`");
    assert_eq!(split["insertText"], "split($0)");
    assert_eq!(import_of(&split).as_deref(), Some("import std::string;\n"));
    assert_eq!(split["detail"], "split(separator: char) -> Vector<String>");
    assert!(find(&items, "read").is_none(), "fs::read doesn't take a String first");

    // `import std::` lists the modules
    client.change(&main, 8, &format!("import std::\n{}", MAIN));
    let modules = client.result("textDocument/completion", at(&main, 0, 12));
    assert!(find(&modules, "fs").is_some() && find(&modules, "math").is_some(), "{}", modules);

    stop(client, dir);
}

#[test]
fn symbols_and_quick_fixes() {
    let (mut client, dir, main, _) = started("symbols");

    let symbols = client.result("textDocument/documentSymbol", json!({ "textDocument": { "uri": main } }));
    let names: Vec<&str> = symbols.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Point", "add", "v", "p", "unused"]);
    let point_members: Vec<&str> = symbols[0]["children"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert!(point_members.contains(&"x") && point_members.contains(&"double"));

    let found = client.result("workspace/symbol", json!({ "query": "len" }));
    assert!(found.as_array().unwrap().iter().any(|s| s["name"] == "len"));

    let actions = client.result("textDocument/codeAction", json!({
        "textDocument": { "uri": main },
        "range": { "start": { "line": 20, "character": 4 }, "end": { "line": 20, "character": 4 } },
        "context": { "diagnostics": [] },
    }));
    let action = &actions[0];
    assert_eq!(action["title"], "Prefix the name with '_'");
    let edit = &action["edit"]["changes"][&main][0];
    assert_eq!(edit["newText"], "_");
    assert_eq!(edit["range"]["start"], json!({ "line": 20, "character": 4 }));

    // A method called without receiver gets `self.` put in front
    let quad = "    return self.x * 2;\n  }\n\n  pub func quad(&self) -> int32 {\n    return double() * 2;\n  }\n";
    client.change(&main, 2, &MAIN.replacen("    return self.x * 2;\n  }\n", quad, 1));
    let diagnostics = client.diagnostics(&main);
    assert!(diagnostics.iter().any(|d| d["severity"] == 1), "{:?}", diagnostics);
    let actions = client.result("textDocument/codeAction", json!({
        "textDocument": { "uri": main },
        "range": { "start": { "line": 12, "character": 11 }, "end": { "line": 12, "character": 11 } },
        "context": { "diagnostics": [] },
    }));
    let action = &actions[0];
    assert_eq!(action["title"], "Call it as `self.double(...)`", "{}", actions);
    let edit = &action["edit"]["changes"][&main][0];
    assert_eq!(edit["newText"], "self.");
    assert_eq!(edit["range"]["start"], json!({ "line": 12, "character": 11 }));

    stop(client, dir);
}

#[test]
fn unknown_requests_get_an_error() {
    let (mut client, dir, _, _) = started("unknown");
    let response = client.request("textDocument/somethingNew", json!({}));
    assert_eq!(response["error"]["code"], -32601);
    stop(client, dir);
}

#[test]
fn impl_blocks_in_symbols_and_completion() {
    let (mut client, dir, main, _) = started("impl_blocks");
    let source = format!("{}impl String {{\n  pub func shout(&self) -> String {{\n    return self.concat(\"!\");\n  }}\n}}\nlet s = \"hi\";\nprint(s.shout());\n", MAIN);
    client.change(&main, 2, &source);
    assert!(client.diagnostics(&main).iter().all(|d| d["severity"] != 1));

    // The block is listed as `impl String`, not as the trait it becomes
    let symbols = client.result("textDocument/documentSymbol", json!({ "textDocument": { "uri": main } }));
    let block = symbols.as_array().unwrap().iter().find(|s| s["name"] == "impl String").expect("impl block listed");
    assert_eq!(block["detail"], "impl String {");
    assert_eq!(block["children"][0]["name"], "shout");
    assert_eq!(block["children"][0]["detail"], "pub func String.shout(&self) -> String");

    let line = source.lines().count() as u64;
    let hover = client.result("textDocument/hover", at(&main, line - 1, 9));
    assert!(hover["contents"]["value"].as_str().unwrap().contains("pub func String.shout(&self) -> String"), "{}", hover);

    client.change(&main, 3, &format!("{}s.", source));
    let items = client.result("textDocument/completion", at(&main, line, 2));
    assert!(items.as_array().unwrap().iter().any(|i| i["label"] == "shout"), "{}", items);

    stop(client, dir);
}
