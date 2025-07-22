use std::path::PathBuf;

use dbt_test_primitives::is_update_golden_files_mode;

pub mod profiles;
pub mod task;
pub mod testing;

mod schema;
pub use schema::random_schema;

/// Return a flag to use for record&replay runs (depending on the
/// update mode). If we are running update AND warehouse enabled, then
/// we set recording mode; otherwise it is replay mode.
///
/// The given path is the path to the directory to be used for
/// storying/reading.
pub fn record_or_replay_flag(abs_project_path: impl Into<PathBuf>) -> String {
    if is_update_mode_with_warehouse() {
        format!("--fs-record {}", abs_project_path.into().display())
    } else {
        format!("--fs-replay {}", abs_project_path.into().display())
    }
}

/// Return schema name for record&replay runs (depending on the update
/// mode).
pub fn record_or_replay_schema(base_schema_name: impl Into<String>) -> String {
    if is_update_mode_with_warehouse() {
        random_schema(&base_schema_name.into())
    } else {
        base_schema_name.into()
    }
}

/// Return true if this run requested use of warehouse during test
/// run.
fn is_use_warehouse() -> bool {
    // TODO: not the best name for a variable any longer (but we want
    // to be consistent). We should rename globally.
    std::env::var("ADAPTER_RECORD").is_ok()
}

/// Return true if this is an update run with warehouse.
pub fn is_update_mode_with_warehouse() -> bool {
    is_update_golden_files_mode() && is_use_warehouse()
}
