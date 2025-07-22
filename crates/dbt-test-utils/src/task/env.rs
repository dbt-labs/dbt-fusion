use std::path::{Path, PathBuf};

use dbt_common::{FsResult, stdfs};
use tempfile::TempDir;

use crate::task::utils::strip_leading_relative;

use super::TestResult;
use super::utils::copy_dir_non_ignored;

// ------------------------------------------------------------------------------------------------
/// Project environment to help us isolate test execution, such that one test uses one directory.
#[derive(Debug)]
pub struct ProjectEnv {
    pub mutable: bool,
    // Root of the create to use to build the golden_dir
    // destination. Note that this is actually given by the caller, so
    // in theory one can set any value, but it is expected it would be
    // crate root when running test(s) reside.
    pub crate_root: PathBuf,
    // Relative path (to the crate) of an existing workspace that is copied
    pub project_dir: PathBuf,
    // Directory that contains a copy of the original workspace
    #[allow(dead_code)] // do NOT use, but kept for Drop
    pub temp_dir_to_drop: Option<TempDir>,
    // Absolute path to the workspace dir used in run
    pub absolute_project_dir: PathBuf,
}

impl ProjectEnv {
    /// Create a new (read-write) environment, which is a copy of the
    /// project with the given `name`. If `name` does not exist in
    /// `projects`, it will error. `project_dir` is found relative to
    /// the crate that has this file.
    pub fn mutable(project_dir: &str, keep_dir: bool) -> TestResult<Self> {
        ProjectEnv::mutable_with_crate(project_dir, keep_dir, "dbt-cli")
    }

    /// Create a new (read-write) environment, which is a copy of the
    /// project with the given `name`. If `name` does not exist in
    /// `projects`, it will error. `project_dir` is found relative to
    /// the crate that has this file.
    pub fn mutable_lsp(project_dir: &str, keep_dir: bool) -> TestResult<Self> {
        ProjectEnv::mutable_with_crate(project_dir, keep_dir, "dbt-lsp")
    }

    fn mutable_with_crate(project_dir: &str, keep_dir: bool, crate_name: &str) -> TestResult<Self> {
        // Ensure we have the source workspace.
        let def_crate_root = env!("CARGO_MANIFEST_DIR");
        // TODO: Make this more robust / parameterizeable
        for ancestor in PathBuf::from(def_crate_root).ancestors() {
            let crate_path = ancestor.join("crates").join(crate_name);
            if crate_path.exists() {
                return Self::mutable_from(&crate_path.to_string_lossy(), project_dir, keep_dir);
            }
        }

        Err(format!("Could not find crates/{crate_name} directory").into())
    }

    /// [`Self::mutable`] that finds the `project_dir` relative to the
    /// given root.
    pub fn mutable_from(root: &str, project_dir: &str, keep_dir: bool) -> TestResult<Self> {
        let absolute_project_dir = Path::new(root).join(project_dir);
        let simplified_project_dir = stdfs::canonicalize(absolute_project_dir.as_path())?;
        if !simplified_project_dir.is_dir() {
            return Err(format!("Project {} does not exist", &project_dir).into());
        }
        let project_dir = PathBuf::from(project_dir);

        // Copy the workspace content to the temporary directory.
        let temp_dir = tempfile::tempdir()?;
        let mutable_project_dir = temp_dir.path().join("root");
        copy_dir_non_ignored(&absolute_project_dir, mutable_project_dir.as_path())?;

        let temp_dir_to_drop = if keep_dir {
            std::mem::forget(temp_dir);
            None
        } else {
            Some(temp_dir)
        };
        Ok(ProjectEnv {
            crate_root: root.into(),
            mutable: true,
            project_dir,
            temp_dir_to_drop,
            absolute_project_dir: mutable_project_dir,
        })
    }

    /// Creates a new environment (read-only). (The environment does not enforce
    /// read-only at the moment.)  If `project_dir` does not exist, the run will
    /// error. `project_dir` is relative to the crate of this file.
    pub fn immutable(project_dir: &str) -> TestResult<Self> {
        let def_crate_root = env!("CARGO_MANIFEST_DIR");
        // TODO: Make this more robust / parameterizeable
        for ancestor in PathBuf::from(def_crate_root).ancestors() {
            let dbt_cli_path = ancestor.join("crates").join("dbt-cli");
            if dbt_cli_path.exists() {
                return Self::immutable_from(&dbt_cli_path.to_string_lossy(), project_dir);
            }
        }

        Err("Could not find dbt-cli directory".into())
    }

    /// [`Self::immutable`] that finds the `project_dir` relative to
    /// the given (crate) root.
    pub fn immutable_from(root: &str, project_dir: &str) -> TestResult<Self> {
        let absolute_project_dir = Path::new(root).join(project_dir);
        let simplified_project_dir = stdfs::canonicalize(absolute_project_dir.as_path())?;
        if !simplified_project_dir.is_dir() {
            return Err(format!("Project {} does not exist", &project_dir).into());
        }
        let project_dir = PathBuf::from(project_dir);

        // Copy the workspace content to the temporary directory.
        let temp_dir = tempfile::tempdir()?;
        let mutable_project_dir = temp_dir.path().join("root");
        copy_dir_non_ignored(&absolute_project_dir, mutable_project_dir.as_path())?;
        let temp_dir_to_drop = Some(temp_dir);

        Ok(ProjectEnv {
            crate_root: root.into(),
            mutable: false,
            // original project dir
            project_dir,
            temp_dir_to_drop,
            absolute_project_dir: mutable_project_dir,
        })
    }

    pub fn create_test_env(&self) -> FsResult<TestEnv> {
        let crate_root = self.crate_root.clone();
        let project_dir = self.project_dir.clone();

        // For regression tests, we go outside the project root, but the golden dirs here are always subdirectories
        // so we need to strip the leading relative path
        let stripped_project_dir = strip_leading_relative(&project_dir);
        // If the project_dir is a subdirectory of the crate_root, we can use the project_dir directly
        let golden_dir = if stripped_project_dir != project_dir {
            let mut new_path = crate_root.join("tests").join("golden");
            for component in stripped_project_dir.components() {
                new_path.push(component);
            }
            new_path
        } else {
            // replace /data with /golden
            let mut new_path = PathBuf::new();
            let mut replaced = false;
            for component in project_dir.components() {
                if !replaced && component.as_os_str() == "data" {
                    new_path.push("golden");
                    replaced = true;
                } else {
                    new_path.push(component);
                }
            }
            crate_root.join(new_path)
        };

        // Create golden directory
        stdfs::create_dir_all(golden_dir.as_path())?;

        // Create scratch directory
        let temp_dir = {
            let temp_dir = stdfs::canonicalize(
                self.absolute_project_dir
                    .parent()
                    .expect("absolute_project_dir should be a subdirectory of a temp dir"),
            )?
            .join("scratch");
            stdfs::create_dir_all(temp_dir.as_path())?;
            temp_dir
        };

        Ok(TestEnv {
            golden_dir,
            temp_dir,
        })
    }
}

// ------------------------------------------------------------------------------------------------
/// Environment in which test is run.
#[derive(Debug)]
pub struct TestEnv {
    // golden directory for the test
    pub golden_dir: PathBuf,
    // temporary directory that contains sdftarget
    pub temp_dir: PathBuf,
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = stdfs::remove_dir_all(self.temp_dir.as_path());
    }
}
