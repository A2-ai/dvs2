//! Test scenario definitions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::runner::Op;
use crate::snapshot::ObjectPresence;

/// A test scenario for conformance testing.
///
/// Defines the setup, operations, and expected outcomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    /// Scenario name.
    pub name: String,

    /// Description of what this scenario tests.
    pub description: String,

    /// Setup: files to create before running operations.
    pub setup: Setup,

    /// Steps: operations to run in order.
    pub steps: Vec<Step>,

    /// Expected outcomes after all steps.
    pub expect: Expectation,
}

/// Setup phase: files and config to create.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Setup {
    /// Files to create: path -> contents.
    pub files: BTreeMap<String, FileContent>,

    /// Whether to run dvs init as part of setup.
    pub init: Option<InitSetup>,
}

/// Content for a setup file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FileContent {
    /// Text content.
    Text(String),
    /// Binary content (base64 encoded in YAML/JSON).
    Binary(Vec<u8>),
}

impl FileContent {
    /// Get the content as bytes.
    pub fn as_bytes(&self) -> Vec<u8> {
        match self {
            FileContent::Text(s) => s.as_bytes().to_vec(),
            FileContent::Binary(b) => b.clone(),
        }
    }
}

/// Init setup configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitSetup {
    /// Storage directory (relative to repo root).
    pub storage_dir: String,

    /// Optional permissions.
    pub permissions: Option<u32>,

    /// Optional group.
    pub group: Option<String>,
}

/// A step in a scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    /// Step name/description.
    pub name: String,

    /// Operation to run.
    pub op: Op,

    /// Whether this step should fail.
    #[serde(default)]
    pub expect_failure: bool,

    /// Expected error type (if expect_failure is true).
    pub expect_error_type: Option<String>,
}

impl Step {
    /// Create a new step.
    pub fn new(name: &str, op: Op) -> Self {
        Self {
            name: name.to_string(),
            op,
            expect_failure: false,
            expect_error_type: None,
        }
    }

    /// Create a step that expects failure.
    pub fn expect_failure(name: &str, op: Op, error_type: Option<&str>) -> Self {
        Self {
            name: name.to_string(),
            op,
            expect_failure: true,
            expect_error_type: error_type.map(String::from),
        }
    }
}

/// Expected outcomes after running a scenario.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Expectation {
    /// Expected tracked files.
    pub tracked_files: Vec<TrackedFileExpectation>,

    /// Expected storage object count.
    pub storage_object_count: Option<usize>,

    /// Expected config presence.
    pub config_present: Option<bool>,

    /// Expected gitignore contains *.dvs.
    pub gitignore_has_dvs: Option<bool>,
}

/// Expectation for a tracked file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedFileExpectation {
    /// File path (relative to repo root).
    pub path: PathBuf,

    /// Expected data file exists.
    pub data_exists: Option<bool>,

    /// Expected storage object presence.
    pub storage_exists: Option<ObjectPresence>,
}

impl Scenario {
    /// Create a new scenario builder.
    pub fn builder(name: &str) -> ScenarioBuilder {
        ScenarioBuilder::new(name)
    }
}

/// Builder for creating scenarios.
pub struct ScenarioBuilder {
    name: String,
    description: String,
    setup: Setup,
    steps: Vec<Step>,
    expect: Expectation,
}

impl ScenarioBuilder {
    /// Create a new builder.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            description: String::new(),
            setup: Setup::default(),
            steps: Vec::new(),
            expect: Expectation::default(),
        }
    }

    /// Set the description.
    pub fn description(mut self, desc: &str) -> Self {
        self.description = desc.to_string();
        self
    }

    /// Add a setup file.
    pub fn setup_file(mut self, path: &str, content: &str) -> Self {
        self.setup
            .files
            .insert(path.to_string(), FileContent::Text(content.to_string()));
        self
    }

    /// Add init to setup.
    pub fn setup_init(mut self, storage_dir: &str) -> Self {
        self.setup.init = Some(InitSetup {
            storage_dir: storage_dir.to_string(),
            permissions: None,
            group: None,
        });
        self
    }

    /// Add a step.
    pub fn step(mut self, name: &str, op: Op) -> Self {
        self.steps.push(Step::new(name, op));
        self
    }

    /// Add a step that expects failure.
    pub fn step_expect_failure(mut self, name: &str, op: Op, error_type: Option<&str>) -> Self {
        self.steps.push(Step::expect_failure(name, op, error_type));
        self
    }

    /// Expect a tracked file.
    pub fn expect_tracked(mut self, path: &str) -> Self {
        self.expect.tracked_files.push(TrackedFileExpectation {
            path: PathBuf::from(path),
            data_exists: Some(true),
            storage_exists: Some(ObjectPresence::Present),
        });
        self
    }

    /// Expect storage object count.
    pub fn expect_storage_count(mut self, count: usize) -> Self {
        self.expect.storage_object_count = Some(count);
        self
    }

    /// Expect config present.
    pub fn expect_config(mut self, present: bool) -> Self {
        self.expect.config_present = Some(present);
        self
    }

    /// Expect gitignore has *.dvs.
    pub fn expect_gitignore_dvs(mut self, has: bool) -> Self {
        self.expect.gitignore_has_dvs = Some(has);
        self
    }

    /// Build the scenario.
    pub fn build(self) -> Scenario {
        Scenario {
            name: self.name,
            description: self.description,
            setup: self.setup,
            steps: self.steps,
            expect: self.expect,
        }
    }
}

/// Standard scenarios for conformance testing.
#[allow(dead_code)]
pub mod standard {
    use super::*;

    /// Basic init scenario.
    pub fn init_basic() -> Scenario {
        Scenario::builder("init_basic")
            .description("Initialize DVS in a git repository")
            .step("init", Op::init(".dvs-storage"))
            .expect_config(true)
            .expect_gitignore_dvs(true)
            .build()
    }

    /// Add a single file scenario.
    pub fn add_single_file() -> Scenario {
        Scenario::builder("add_single_file")
            .description("Add a single file to DVS")
            .setup_file("data.csv", "a,b,c\n1,2,3\n")
            .setup_init(".dvs-storage")
            .step("add file", Op::add(&["data.csv"]))
            .expect_tracked("data.csv")
            .expect_storage_count(1)
            .build()
    }

    /// Add multiple files scenario.
    pub fn add_multiple_files() -> Scenario {
        Scenario::builder("add_multiple_files")
            .description("Add multiple files to DVS")
            .setup_file("file1.txt", "content 1")
            .setup_file("file2.txt", "content 2")
            .setup_file("dir/file3.txt", "content 3")
            .setup_init(".dvs-storage")
            .step(
                "add files",
                Op::add(&["file1.txt", "file2.txt", "dir/file3.txt"]),
            )
            .expect_tracked("file1.txt")
            .expect_tracked("file2.txt")
            .expect_tracked("dir/file3.txt")
            .expect_storage_count(3)
            .build()
    }

    /// Add file that doesn't exist (should fail).
    pub fn add_nonexistent() -> Scenario {
        Scenario::builder("add_nonexistent")
            .description("Attempt to add a file that doesn't exist")
            .setup_init(".dvs-storage")
            .step_expect_failure(
                "add missing file",
                Op::add(&["missing.txt"]),
                Some("file_not_found"),
            )
            .build()
    }

    /// Get a file after adding it.
    pub fn get_after_add() -> Scenario {
        Scenario::builder("get_after_add")
            .description("Get a file that was previously added")
            .setup_file("data.csv", "a,b,c\n1,2,3\n")
            .setup_init(".dvs-storage")
            .step("add file", Op::add(&["data.csv"]))
            .step("get file", Op::get(&["data.csv"]))
            .expect_tracked("data.csv")
            .build()
    }

    /// Status of tracked files.
    pub fn status_tracked() -> Scenario {
        Scenario::builder("status_tracked")
            .description("Check status of tracked files")
            .setup_file("data.csv", "a,b,c\n1,2,3\n")
            .setup_init(".dvs-storage")
            .step("add file", Op::add(&["data.csv"]))
            .step("check status", Op::status(&[]))
            .expect_tracked("data.csv")
            .build()
    }

    /// All standard scenarios.
    pub fn all() -> Vec<Scenario> {
        vec![
            init_basic(),
            add_single_file(),
            add_multiple_files(),
            add_nonexistent(),
            get_after_add(),
            status_tracked(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scenario_builder() {
        let scenario = Scenario::builder("test")
            .description("A test scenario")
            .setup_file("data.csv", "content")
            .setup_init(".storage")
            .step("add", Op::add(&["data.csv"]))
            .expect_tracked("data.csv")
            .build();

        assert_eq!(scenario.name, "test");
        assert_eq!(scenario.setup.files.len(), 1);
        assert!(scenario.setup.init.is_some());
        assert_eq!(scenario.steps.len(), 1);
        assert_eq!(scenario.expect.tracked_files.len(), 1);
    }

    #[test]
    fn test_standard_scenarios() {
        let scenarios = standard::all();
        assert!(!scenarios.is_empty());

        for s in scenarios {
            assert!(!s.name.is_empty());
            assert!(!s.description.is_empty());
        }
    }
}
