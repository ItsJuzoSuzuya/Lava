//! End-to-end tests for the Lava compiler.
//!
//! Every suite in `tests/suites/*.test` holds many cases. Each case is compiled
//! and run with the real compiler binary in its own temporary directory, then
//! its output or compile error is compared against the expectation.
//!
//! Suite format:
//!
//! ```text
//! === name of the case
//! <source of main.lv>
//! --- file math.lv
//! <source of another module, optional and repeatable>
//! --- output
//! <exact expected stdout of the program>
//! ```
//!
//! Instead of `--- output` a case can end with `--- error` followed by text the
//! compiler's error message has to contain.

use std::{fs, path::{Path, PathBuf}, process::Command};

/// What a case expects from compiling and running it.
enum Expected {
    /// The program compiles, runs and prints exactly this.
    Output(String),
    /// Compiling fails with an error message containing this.
    Error(String),
}

struct Case {
    name: String,
    /// File name and source of every module, `main.lv` first.
    files: Vec<(String, String)>,
    expected: Expected,
}

/// Splits a suite file into its cases.
///
/// # Panics
///
/// Panics if a case has no `--- output` or `--- error` section.
fn parse_suite(text: &str) -> Vec<Case> {
    let mut cases = Vec::new();

    for block in text.split("\n=== ").map(|b| b.trim_start_matches("=== ")) {
        if block.trim().is_empty() {
            continue;
        }
        let (name, body) = block.split_once('\n').unwrap_or((block, ""));
        let name = name.trim().to_string();

        // Sections start with a "--- " line, the first one is main.lv without a header
        let mut sections: Vec<(String, String)> = vec![("file main.lv".to_string(), String::new())];
        for line in body.lines() {
            match line.strip_prefix("--- ") {
                Some(header) => sections.push((header.trim().to_string(), String::new())),
                None => {
                    let content = &mut sections.last_mut().unwrap().1;
                    content.push_str(line);
                    content.push('\n');
                }
            }
        }

        let mut files = Vec::new();
        let mut expected = None;
        for (header, content) in sections {
            if let Some(file) = header.strip_prefix("file ") {
                files.push((file.trim().to_string(), content));
            } else if header == "output" {
                expected = Some(Expected::Output(content));
            } else if header == "error" {
                expected = Some(Expected::Error(content.trim().to_string()));
            } else {
                panic!("case '{}': unknown section '--- {}'", name, header);
            }
        }

        let expected = expected.unwrap_or_else(|| panic!("case '{}' has no '--- output' or '--- error' section", name));
        cases.push(Case { name, files, expected });
    }

    cases
}

/// Compiles and runs `case`, returns a description of what went wrong if it
/// didn't behave as expected.
fn run_case(suite: &str, index: usize, case: &Case) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("lava-test-{}-{}-{}", std::process::id(), suite, index));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (file, source) in &case.files {
        fs::write(dir.join(file), source).unwrap();
    }

    let result = Command::new(env!("CARGO_BIN_EXE_Letter"))
        .current_dir(&dir)
        .output()
        .expect("failed to start the compiler");
    let _ = fs::remove_dir_all(&dir);

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);

    match &case.expected {
        Expected::Output(expected) => {
            if !result.status.success() {
                return Err(format!("expected the program to run, but compiling failed:\n{}", stderr.trim()));
            }
            if normalize(&stdout) != normalize(expected) {
                return Err(format!("expected output:\n{}\ngot:\n{}", normalize(expected), normalize(&stdout)));
            }
        }
        Expected::Error(expected) => {
            if result.status.success() {
                return Err(format!("expected error containing '{}', but it compiled and printed:\n{}", expected, stdout.trim()));
            }
            if !stderr.contains(expected.as_str()) {
                return Err(format!("expected error containing '{}', got:\n{}", expected, stderr.trim()));
            }
        }
    }
    Ok(())
}

/// Drops trailing whitespace of every line and trailing empty lines.
fn normalize(text: &str) -> String {
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_end().to_string()
}

/// Runs every case of `tests/suites/<suite>.test` and fails with a report of
/// all cases that didn't behave as expected.
fn run_suite(suite: &str) {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/suites").join(format!("{}.test", suite));
    let text = fs::read_to_string(&path).unwrap_or_else(|_| panic!("can't read {}", path.display()));
    let cases = parse_suite(&text);
    assert!(!cases.is_empty(), "suite '{}' has no cases", suite);

    let failures: Vec<String> = cases.iter().enumerate()
        .filter_map(|(i, case)| run_case(suite, i, case).err().map(|e| format!("--- {}\n{}", case.name, e)))
        .collect();

    if !failures.is_empty() {
        panic!("{} of {} cases in '{}' failed:\n\n{}", failures.len(), cases.len(), suite, failures.join("\n\n"));
    }
}

#[test] fn arithmetic()  { run_suite("arithmetic"); }
#[test] fn basics()      { run_suite("basics"); }
#[test] fn classes()     { run_suite("classes"); }
#[test] fn conversions() { run_suite("conversions"); }
#[test] fn enums()       { run_suite("enums"); }
#[test] fn errors()      { run_suite("errors"); }
#[test] fn functions()   { run_suite("functions"); }
#[test] fn matching()    { run_suite("matching"); }
#[test] fn modules()     { run_suite("modules"); }
#[test] fn negation()    { run_suite("negation"); }
#[test] fn numbers()     { run_suite("numbers"); }
#[test] fn refs()        { run_suite("refs"); }
