//! Hints as data. The library says what to do next. Each front end renders the
//! commands in its own syntax: the CLI as `dvs add b.csv`, R as
//! `dvs_add("b.csv")`.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::OnConflict;

/// A command a hint suggests. Paths are relative to the current directory.
/// Serialized for the CLI's JSON `next_steps`, tagged by `action`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum NextStep {
    /// Make the local files the tracked version.
    Add { paths: Vec<PathBuf> },
    /// Get these files, with a conflict policy when one is given.
    Get {
        paths: Vec<PathBuf>,
        on_conflict: Option<OnConflict>,
    },
    /// The same get again, with these options added.
    RerunWith {
        on_conflict: Option<OnConflict>,
        recursive: bool,
    },
    /// The same get again, as it was.
    Retry,
    /// List the tracked files.
    Status,
    /// Check the login, then log in. A terminal command, also for R users.
    Login,
}

/// One piece of a hint: prose, a path in the prose, or a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintPart {
    Text(String),
    /// Relative to the current directory
    Path(PathBuf),
    Step(NextStep),
}

/// How a front end writes the commands and paths of a hint.
pub trait HintSyntax {
    fn step(&self, step: &NextStep) -> String;
    fn path(&self, path: &Path) -> String;
}

/// A hint: prose with commands in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hint(pub Vec<HintPart>);

impl Hint {
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.0.push(HintPart::Text(text.into()));
        self
    }

    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.0.push(HintPart::Path(path.into()));
        self
    }

    pub fn step(mut self, step: NextStep) -> Self {
        self.0.push(HintPart::Step(step));
        self
    }

    /// Appends another hint.
    pub fn then(mut self, other: Hint) -> Self {
        self.0.extend(other.0);
        self
    }

    /// The commands, in order.
    pub fn steps(&self) -> impl Iterator<Item = &NextStep> {
        self.0.iter().filter_map(|part| match part {
            HintPart::Step(step) => Some(step),
            HintPart::Text(_) | HintPart::Path(_) => None,
        })
    }

    /// The text, with each command and path written in `syntax`.
    pub fn render(&self, syntax: &impl HintSyntax) -> String {
        self.0
            .iter()
            .map(|part| match part {
                HintPart::Text(text) => text.clone(),
                HintPart::Path(path) => syntax.path(path),
                HintPart::Step(step) => syntax.step(step),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Upper;
    impl HintSyntax for Upper {
        fn step(&self, step: &NextStep) -> String {
            match step {
                NextStep::Add { .. } => "ADD".into(),
                _ => "OTHER".into(),
            }
        }
        fn path(&self, path: &Path) -> String {
            path.display().to_string().to_uppercase()
        }
    }

    #[test]
    fn renders_steps_in_place_and_lists_them_in_order() {
        let hint = Hint::default()
            .text("keep: ")
            .step(NextStep::Add {
                paths: vec!["b.csv".into()],
            })
            .text("; or ")
            .step(NextStep::Status)
            .text(" for ")
            .path("b.csv");
        assert_eq!(hint.render(&Upper), "keep: ADD; or OTHER for B.CSV");
        assert_eq!(hint.steps().count(), 2);
        assert_eq!(
            serde_json::to_value(hint.steps().next().unwrap()).unwrap(),
            serde_json::json!({"action": "add", "paths": ["b.csv"]})
        );
        assert_eq!(
            serde_json::to_value(NextStep::RerunWith {
                on_conflict: Some(OnConflict::Skip),
                recursive: false
            })
            .unwrap(),
            serde_json::json!({"action": "rerun_with", "on_conflict": "skip", "recursive": false})
        );
    }
}
