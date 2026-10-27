//! Builds and runs the programs in `examples/` and checks what they print.

use std::{fs, path::{Path, PathBuf}, process::Command};

/// A copy of `examples/<name>` in a fresh directory for the test `test`, so
/// building it doesn't write into the repository.
fn copy_example(name: &str, test: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(name);
    let dir = std::env::temp_dir().join(format!("lava-example-{}-{}-{}", std::process::id(), name, test));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for entry in fs::read_dir(&source).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "lv") {
            fs::copy(&path, dir.join(path.file_name().unwrap())).unwrap();
        }
    }
    dir
}

/// Runs `lava run` in `dir` with the program arguments `args`, returns its
/// stdout and whether it succeeded.
fn run(dir: &Path, args: &[&str]) -> (String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_lava"))
        .arg("run")
        .arg("--")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to start lava");
    (String::from_utf8_lossy(&output.stdout).into_owned(), output.status.success())
}

#[test]
fn main_gets_every_argument_and_sets_the_exit_code() {
    let dir = std::env::temp_dir().join(format!("lava-main-args-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("main.lv"), "import std::env;

func main() -> int32 {
  mut arguments = env::args();
  let count = arguments.len() as int32;
  // The first one is the program itself
  arguments.next();
  while true {
    match arguments.next() {
      Option::Some(argument) => print(argument),
      Option::None => break,
    }
  }
  return count;
}
").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_lava"))
        .args(["run", "--", "first", "two words", "--flag"])
        .current_dir(&dir)
        .output()
        .expect("failed to start lava");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "first\ntwo words\n--flag\n");
    assert_eq!(output.status.code(), Some(4));
    let _ = fs::remove_dir_all(dir);
}

/// Opened on its own, like in the editor, a standard library module may use
/// the built-ins only the standard library has.
#[test]
fn standard_library_files_check_on_their_own() {
    let std_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/std");
    for entry in fs::read_dir(&std_dir).unwrap() {
        let path = entry.unwrap().path();
        let output = Command::new(env!("CARGO_BIN_EXE_lava"))
            .arg("lint")
            .arg(&path)
            .output()
            .expect("failed to start lava");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success() && stdout.contains("0 errors, 0 warnings"), "{}:\n{}{}",
            path.display(), stdout, String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
fn selfhost_runs_a_program_with_its_own_lexer_parser_and_interpreter() {
    let dir = copy_example("selfhost", "run");
    let (out, ok) = run(&dir, &["run", "sample.lv"]);
    assert!(ok, "{}", out);
    assert_eq!(out, "25\n120\n1\n-20\n");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn selfhost_lexes_real_lava_code() {
    let dir = copy_example("selfhost", "tokens");
    let prelude = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/prelude.lv");
    let (out, ok) = run(&dir, &["tokens", prelude.to_str().unwrap()]);
    assert!(ok, "{}", out);
    // The prelude starts with `pub class String {`
    assert!(out.contains(":1: keyword 'pub'\n") && out.contains(":5: keyword 'class'\n") && out.contains(":11: identifier 'String'\n"), "{}", out);

    // Its own source too, strings with escapes and character literals included
    let (out, ok) = run(&dir, &["tokens", "lexer.lv"]);
    assert!(ok, "{}", out);
    assert!(out.starts_with("tokens: "), "{}", out);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn selfhost_reports_errors_explicitly() {
    let dir = copy_example("selfhost", "errors");
    fs::write(dir.join("bad.lv"), "let x = 1;\nprint(x + );\n").unwrap();
    fs::write(dir.join("div.lv"), "let x = 1;\nprint(x / (x - 1));\n").unwrap();
    fs::write(dir.join("chars.lv"), "let x = 1 @ 2;\n").unwrap();

    let (out, ok) = run(&dir, &["run", "bad.lv"]);
    assert!(!ok);
    assert_eq!(out, "error: 2:11: expected an expression, got ')'\n");

    let (out, ok) = run(&dir, &["run", "div.lv"]);
    assert!(!ok);
    assert_eq!(out, "error: division by zero\n");

    let (out, _) = run(&dir, &["run", "chars.lv"]);
    assert_eq!(out, "error: 1:11: unexpected character '@'\n");

    let (out, _) = run(&dir, &["run", "missing.lv"]);
    assert_eq!(out, "error: can't open the file 'missing.lv'\n");
    let _ = fs::remove_dir_all(dir);
}
