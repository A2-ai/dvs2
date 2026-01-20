//! Output formatting for the CLI.
//!
//! Handles human-readable and JSON output formats, with support for
//! writing to different destinations (inherit/stdout, null, pipe, file).

use crate::{OutputDest, OutputFormat};
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;

/// Output handler for CLI commands.
pub struct Output {
    format: OutputFormat,
    dest: OutputDest,
    quiet: bool,
    /// File handle for file output (wrapped in Mutex for interior mutability)
    file: Option<Mutex<std::fs::File>>,
    /// Pipe process for pipe output mode (cat command that discards output)
    pipe: Option<Mutex<PipeOutput>>,
}

/// Holds the pipe output process and its stdin handle.
struct PipeOutput {
    stdin: ChildStdin,
    #[allow(dead_code)]
    child: Child,
}

impl Output {
    /// Create a new output handler.
    pub fn new(format: OutputFormat, dest: OutputDest, quiet: bool) -> Self {
        let file = if let OutputDest::File(path) = &dest {
            // Try to create/truncate the file
            match std::fs::File::create(path) {
                Ok(f) => Some(Mutex::new(f)),
                Err(e) => {
                    eprintln!("Warning: Failed to open output file {:?}: {}", path, e);
                    None
                }
            }
        } else {
            None
        };

        // For pipe mode, spawn a process that reads and discards input
        let pipe = if matches!(dest, OutputDest::Pipe) {
            // Use `cat > /dev/null` on Unix, or just cat on Windows (less ideal)
            #[cfg(unix)]
            let result = Command::new("cat")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();

            #[cfg(not(unix))]
            let result = Command::new("cmd")
                .args(["/C", "type", "CON", ">", "NUL"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();

            match result {
                Ok(mut child) => {
                    if let Some(stdin) = child.stdin.take() {
                        Some(Mutex::new(PipeOutput { stdin, child }))
                    } else {
                        eprintln!("Warning: Failed to get stdin for pipe process");
                        None
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to spawn pipe process: {}", e);
                    None
                }
            }
        } else {
            None
        };

        Self {
            format,
            dest,
            quiet,
            file,
            pipe,
        }
    }

    /// Check if quiet mode is enabled.
    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Check if JSON output is requested.
    pub fn is_json(&self) -> bool {
        matches!(self.format, OutputFormat::Json)
    }

    /// Check if output is going to null or pipe (discarded).
    pub fn is_null(&self) -> bool {
        matches!(self.dest, OutputDest::Null | OutputDest::Pipe)
    }

    /// Get the output file path if writing to a file.
    #[allow(dead_code)]
    pub fn output_path(&self) -> Option<&PathBuf> {
        if let OutputDest::File(path) = &self.dest {
            Some(path)
        } else {
            None
        }
    }

    /// Write a line to the output destination.
    fn write_line(&self, msg: &str) {
        match &self.dest {
            OutputDest::Null => {} // Discard silently
            OutputDest::Inherit => println!("{}", msg),
            OutputDest::Pipe => {
                // Write to pipe process stdin
                if let Some(pipe) = &self.pipe {
                    if let Ok(mut p) = pipe.lock() {
                        let _ = writeln!(p.stdin, "{}", msg);
                    }
                }
            }
            OutputDest::File(_) => {
                if let Some(file) = &self.file {
                    if let Ok(mut f) = file.lock() {
                        let _ = writeln!(f, "{}", msg);
                    }
                }
            }
        }
    }

    /// Write a line to stderr (errors/warnings always go to stderr).
    fn write_err(&self, msg: &str) {
        // Errors and warnings go to stderr, not to the output destination
        eprintln!("{}", msg);
    }

    /// Output a structured JSON value (only in JSON mode).
    ///
    /// This is the preferred way to output structured data in JSON mode.
    /// In human mode, this does nothing - use human-readable methods instead.
    pub fn json<T: Serialize>(&self, value: &T) {
        if self.is_json() && !self.is_null() {
            match serde_json::to_string(value) {
                Ok(json) => self.write_line(&json),
                Err(e) => self.write_err(&format!("{{\"error\":\"failed to serialize: {}\"}}", e)),
            }
        }
    }

    /// Output a structured JSON value with pretty formatting (only in JSON mode).
    #[allow(dead_code)]
    pub fn json_pretty<T: Serialize>(&self, value: &T) {
        if self.is_json() && !self.is_null() {
            match serde_json::to_string_pretty(value) {
                Ok(json) => self.write_line(&json),
                Err(e) => self.write_err(&format!("{{\"error\":\"failed to serialize: {}\"}}", e)),
            }
        }
    }

    /// Print a line to output (respects quiet mode).
    pub fn println(&self, msg: &str) {
        if !self.quiet && !self.is_null() {
            match self.format {
                OutputFormat::Human => self.write_line(msg),
                OutputFormat::Json => {
                    // For JSON, we'd normally collect and output at the end
                    // For now, just print as-is
                    self.write_line(msg);
                }
            }
        }
    }

    /// Print a success message (green in human format).
    pub fn success(&self, msg: &str) {
        if !self.quiet && !self.is_null() {
            match self.format {
                OutputFormat::Human => self.write_line(&format!("\x1b[32m{}\x1b[0m", msg)),
                OutputFormat::Json => self.write_line(&format!(
                    "{{\"type\":\"success\",\"message\":\"{}\"}}",
                    escape_json(msg)
                )),
            }
        }
    }

    /// Print an info message.
    pub fn info(&self, msg: &str) {
        if !self.quiet && !self.is_null() {
            match self.format {
                OutputFormat::Human => self.write_line(msg),
                OutputFormat::Json => self.write_line(&format!(
                    "{{\"type\":\"info\",\"message\":\"{}\"}}",
                    escape_json(msg)
                )),
            }
        }
    }

    /// Print a warning message (yellow in human format).
    pub fn warn(&self, msg: &str) {
        if !self.quiet {
            // Warnings go to stderr
            match self.format {
                OutputFormat::Human => self.write_err(&format!("\x1b[33m{}\x1b[0m", msg)),
                OutputFormat::Json => self.write_err(&format!(
                    "{{\"type\":\"warning\",\"message\":\"{}\"}}",
                    escape_json(msg)
                )),
            }
        }
    }

    /// Print an error message (red in human format, always shown).
    pub fn error(&self, msg: &str) {
        // Errors always go to stderr
        match self.format {
            OutputFormat::Human => self.write_err(&format!("\x1b[31merror: {}\x1b[0m", msg)),
            OutputFormat::Json => self.write_err(&format!(
                "{{\"type\":\"error\",\"message\":\"{}\"}}",
                escape_json(msg)
            )),
        }
    }
}

/// Escape a string for JSON output.
fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_json() {
        assert_eq!(escape_json("hello"), "hello");
        assert_eq!(escape_json("hello\"world"), "hello\\\"world");
        assert_eq!(escape_json("line1\nline2"), "line1\\nline2");
    }
}
