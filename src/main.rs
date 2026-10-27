use std::{fs, path::{Path, PathBuf}, process::{Command, ExitCode}};

use inkwell::context::Context;
use crate::{analysis::Index, module::ModuleGraph, program::{Program, check_modules}};

mod analysis;
mod traits;
mod span;
mod r#type;
mod token;
mod declaration;
mod codegen_ctx;
mod diagnostic;
mod lexer;
mod lint;
mod lsp;
mod parser;
mod expression;
mod statement;
mod program;
mod runtime;
mod sema;
mod sym;
mod param;
mod field;
mod module;

const USAGE: &str = "\
lava, the Lava compiler

usage:
  lava run [file] [-- args]     build and run a program, passing it args
  lava build [file] [-o path]   build a program into an executable
  lava lint [file]              check a program and print warnings
  lava new <name>               create a project in a new directory
  lava lsp                      run the language server on stdin and stdout
  lava help                     show this help
  lava version                  show the version

[file] is the program's main file, `main.lv` in the current directory by
default. Builds go into a `build` directory next to it.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let result = match args.as_slice() {
        ["run", rest @ ..] => {
            // Everything after `--` goes to the program
            let split = rest.iter().position(|a| *a == "--").unwrap_or(rest.len());
            let program_args = rest.get(split + 1..).unwrap_or(&[]);
            main_file(&rest[..split]).and_then(|main| run(&main, program_args))
        }
        ["build", rest @ ..] => build_command(rest),
        ["lint", rest @ ..] => main_file(rest).and_then(|main| lint(&main)),
        ["new", name] => new_project(name),
        ["lsp"] => return lsp::run(),
        ["help" | "--help" | "-h"] | [] => {
            println!("{}", USAGE);
            Ok(ExitCode::SUCCESS)
        }
        ["version" | "--version" | "-V"] => {
            println!("lava {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        [command, ..] => Err(format!("unknown command '{}', see `lava help`", command)),
    };

    result.unwrap_or_else(|message| {
        if !message.is_empty() {
            eprintln!("error: {}", message);
        }
        ExitCode::FAILURE
    })
}

/// The main file given in `args`, or `main.lv` in the current directory.
/// A directory stands for the `main.lv` in it.
fn main_file(args: &[&str]) -> Result<PathBuf, String> {
    let path = match args {
        [] => PathBuf::from("main.lv"),
        [file] => PathBuf::from(file),
        _ => return Err("expected at most one file, see `lava help`".to_string()),
    };
    let path = if path.is_dir() { path.join("main.lv") } else { path };
    if !path.is_file() {
        return Err(match args {
            [] => "there's no main.lv here, pass a file or create a project with `lava new <name>`".to_string(),
            _ => format!("'{}' doesn't exist", path.display()),
        });
    }
    Ok(path)
}

/// `lava build [file] [-o path]`
fn build_command(args: &[&str]) -> Result<ExitCode, String> {
    let (files, output) = match args {
        [rest @ .., "-o", output] => (rest, Some(PathBuf::from(output))),
        rest => (rest, None),
    };
    let main = main_file(files)?;
    let executable = build(&main, output)?;
    println!("built {}", executable.display());
    Ok(ExitCode::SUCCESS)
}

/// Builds the program starting at `main` into an executable, by default
/// `build/<name>` next to `main`, and returns its path.
///
/// Compile errors are printed, the returned error is then empty.
fn build(main: &Path, output: Option<PathBuf>) -> Result<PathBuf, String> {
    let dir = main.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = main.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "main".to_string());
    let build_dir = dir.join("build");
    fs::create_dir_all(&build_dir).map_err(|e| format!("can't create '{}': {}", build_dir.display(), e))?;
    let object = build_dir.join(format!("{}.o", name));
    let executable = output.unwrap_or_else(|| build_dir.join(&name));

    // Errors are printed here with their file, not by the panic hook
    diagnostic::silence_panics();
    let compiled = diagnostic::catch(|| {
        let context = Context::create();
        let mut graph = ModuleGraph::new(main);
        let mut program = Program::new(&context, &mut graph);
        program.build(&object)
    });
    let libraries = match compiled {
        Ok(Ok(libraries)) => libraries,
        Ok(Err(errors)) => {
            let rendered: Vec<String> = errors.iter().map(|e| e.render().trim().to_string()).collect();
            eprintln!("{}", rendered.join("\n\n"));
            return Err(String::new());
        }
        Err(error) => {
            eprintln!("{}", error.render().trim());
            return Err(String::new());
        }
    };

    let linked = Command::new("cc").arg(&object).arg("-o").arg(&executable).arg("-lm")
        .args(libraries.iter().map(|library| format!("-l{}", library))).status()
        .map_err(|e| format!("can't run the C compiler `cc` to link the program: {}", e))?;
    if !linked.success() {
        return Err("linking the program failed".to_string());
    }
    Ok(executable)
}

/// `lava run`: builds the program and runs it with the arguments `args`,
/// exiting with its exit code.
fn run(main: &Path, args: &[&str]) -> Result<ExitCode, String> {
    let executable = build(main, None)?;
    // A path without a directory would be looked up in PATH
    let executable = if executable.is_relative() { Path::new(".").join(executable) } else { executable };
    let status = Command::new(&executable).args(args).status()
        .map_err(|e| format!("can't run '{}': {}", executable.display(), e))?;
    Ok(match status.code() {
        Some(code) => ExitCode::from(code as u8),
        None => {
            eprintln!("error: the program was killed by a signal");
            ExitCode::FAILURE
        }
    })
}

/// `lava lint`: checks the program and prints its error and lint warnings.
/// Fails if the program doesn't compile.
fn lint(main: &Path) -> Result<ExitCode, String> {
    diagnostic::silence_panics();
    let graph = match diagnostic::catch(|| ModuleGraph::new(main)) {
        Ok(graph) => graph,
        Err(error) => {
            eprintln!("{}", error.render().trim());
            return Err(String::new());
        }
    };

    let (modules, errors) = check_modules(&graph);
    let index = Index::build(&graph, &modules);
    let warnings = lint::lint(&index, &modules);
    for warning in &warnings {
        println!("{}", warning.render(&index));
    }
    for error in &errors {
        eprintln!("{}\n", error.render().trim());
    }

    let count = |n: usize, what: &str| format!("{} {}{}", n, what, if n == 1 { "" } else { "s" });
    println!("{}, {}", count(errors.len(), "error"), count(warnings.len(), "warning"));
    Ok(if errors.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// `lava new <name>`: creates the directory `name` with a hello world program.
fn new_project(name: &str) -> Result<ExitCode, String> {
    let dir = PathBuf::from(name);
    if dir.exists() {
        return Err(format!("'{}' already exists", name));
    }
    fs::create_dir_all(&dir).map_err(|e| format!("can't create '{}': {}", name, e))?;
    let write = |file: &str, content: &str| fs::write(dir.join(file), content)
        .map_err(|e| format!("can't write '{}': {}", file, e));
    write("main.lv", "// Run with `lava run`, arguments after `--` come from `args()`\nfunc main() {\n  print(\"Hello, world!\");\n}\n")?;
    write(".gitignore", "build/\n")?;
    println!("created {}, run it with `cd {} && lava run`", name, name);
    Ok(ExitCode::SUCCESS)
}
