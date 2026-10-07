//! Compile errors as values.
//!
//! Errors stop compiling by panicking with a [`CompileError`] payload, see
//! [`throw`]. The compiler prints them through the hook from [`install_hook`],
//! tools like the linter and the language server catch them with [`catch`]
//! and keep going.

use std::{any::Any, panic::{self, AssertUnwindSafe}, path::PathBuf};

use crate::span::Span;

/// An error that stops compiling.
#[derive(Debug, Clone)]
pub struct CompileError {
    pub message: String,
    /// Where the error is in [`CompileError::source`], `None` if it has no position.
    pub span: Option<Span>,
    /// Source code the error is in, to show the line and to find its file.
    /// Empty if unknown.
    pub source: String,
    /// File the error is in, if the code throwing it knew it.
    pub file: Option<PathBuf>,
}

impl CompileError {
    /// The error as the compiler prints it, with the offending line and a
    /// `^` under the position.
    pub fn render(&self) -> String {
        let Some(span) = &self.span else {
            return format!("\nerror: {}\n", self.message);
        };
        let line_txt = self.source.lines().nth(span.line).unwrap_or("");
        let pointer = " ".repeat(span.col) + "^";
        let file = self.file.as_ref()
            .and_then(|f| f.file_name())
            .map(|f| format!("{}:", f.to_string_lossy()))
            .unwrap_or_default();
        format!("\nerror: {}\n  --> {}{}:{}\n   |\n   | {}\n   | {}\n",
            self.message, file, span.line + 1, span.col + 1, line_txt, pointer)
    }
}

/// Stops compiling with the error `message` at `span` in `source`.
pub fn throw(message: impl Into<String>, span: Option<Span>, source: &str) -> ! {
    panic::panic_any(CompileError { message: message.into(), span, source: source.to_string(), file: None })
}

/// Stops compiling with the error `message` at `span` in the file `file`.
pub fn throw_in_file(message: impl Into<String>, span: Option<Span>, source: &str, file: PathBuf) -> ! {
    panic::panic_any(CompileError { message: message.into(), span, source: source.to_string(), file: Some(file) })
}

/// Makes the compiler print [`CompileError`]s like `error: ...` with their
/// position instead of Rust's panic message. Other panics are printed as usual.
pub fn install_hook() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        match info.payload().downcast_ref::<CompileError>() {
            Some(error) => eprintln!("{}", error.render()),
            None => default(info),
        }
    }));
}

/// Stops printing anything when the compiler panics, for tools that report
/// the errors they [`catch`] themselves.
pub fn silence_panics() {
    panic::set_hook(Box::new(|_| {}));
}

/// Runs `f` and returns the error it stopped with instead of panicking.
///
/// Any other panic, like a bug in the compiler, becomes a [`CompileError`] too,
/// so a tool keeps running.
pub fn catch<T>(f: impl FnOnce() -> T) -> Result<T, CompileError> {
    panic::catch_unwind(AssertUnwindSafe(f)).map_err(from_panic)
}

/// Turns the payload of a panic into a [`CompileError`].
///
/// A plain message gets its position from a `--> line:col` in it, the
/// format errors were printed in.
fn from_panic(payload: Box<dyn Any + Send>) -> CompileError {
    let payload = match payload.downcast::<CompileError>() {
        Ok(error) => return *error,
        Err(payload) => payload,
    };
    let text = payload.downcast_ref::<String>().cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "the compiler crashed".to_string());

    let message = text.lines()
        .find_map(|line| line.trim().strip_prefix("error: "))
        .unwrap_or_else(|| text.trim())
        .to_string();
    let span = text.lines()
        .find_map(|line| line.trim().strip_prefix("--> "))
        .and_then(|position| {
            let (line, col) = position.split_once(':')?;
            let line: usize = line.trim().parse().ok()?;
            let col: usize = col.trim().parse().ok()?;
            Some(Span { line: line.saturating_sub(1), col: col.saturating_sub(1), len: 1 })
        });
    CompileError { message, span, source: String::new(), file: None }
}
