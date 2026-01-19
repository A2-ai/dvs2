//! Output formatting for the CLI.
//!
//! Handles human-readable and JSON output formats.

use crate::OutputFormat;

/// Output handler for CLI commands.
pub struct Output {
    format: OutputFormat,
    quiet: bool,
}

impl Output {
    /// Create a new output handler.
    pub fn new(format: OutputFormat, quiet: bool) -> Self {
        Self { format, quiet }
    }

    /// Check if quiet mode is enabled.
    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Print a line to stdout (respects quiet mode).
    pub fn println(&self, msg: &str) {
        if !self.quiet {
            match self.format {
                OutputFormat::Human => println!("{}", msg),
                OutputFormat::Json => {
                    // For JSON, we'd normally collect and output at the end
                    // For now, just print as-is
                    println!("{}", msg);
                }
            }
        }
    }

    /// Print a success message (green in human format).
    pub fn success(&self, msg: &str) {
        if !self.quiet {
            match self.format {
                OutputFormat::Human => println!("\x1b[32m{}\x1b[0m", msg),
                OutputFormat::Json => println!("{{\"type\":\"success\",\"message\":\"{}\"}}", escape_json(msg)),
            }
        }
    }

    /// Print an info message.
    pub fn info(&self, msg: &str) {
        if !self.quiet {
            match self.format {
                OutputFormat::Human => println!("{}", msg),
                OutputFormat::Json => println!("{{\"type\":\"info\",\"message\":\"{}\"}}", escape_json(msg)),
            }
        }
    }

    /// Print a warning message (yellow in human format).
    pub fn warn(&self, msg: &str) {
        if !self.quiet {
            match self.format {
                OutputFormat::Human => eprintln!("\x1b[33m{}\x1b[0m", msg),
                OutputFormat::Json => eprintln!("{{\"type\":\"warning\",\"message\":\"{}\"}}", escape_json(msg)),
            }
        }
    }

    /// Print an error message (red in human format, always shown).
    pub fn error(&self, msg: &str) {
        match self.format {
            OutputFormat::Human => eprintln!("\x1b[31merror: {}\x1b[0m", msg),
            OutputFormat::Json => eprintln!("{{\"type\":\"error\",\"message\":\"{}\"}}", escape_json(msg)),
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
