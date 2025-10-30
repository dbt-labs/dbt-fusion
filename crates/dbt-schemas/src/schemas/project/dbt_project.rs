// This code was generated from dbt-make-dbt-schemas/json_schemas/dbt_project.json on 2025-03-31T06:22:06. Do not edit.

use std::collections::HashMap;
use std::collections::btree_map::Iter;
use std::fmt::Debug;

use dbt_serde_yaml::JsonSchema;

// Type aliases for clarity
type YmlValue = dbt_serde_yaml::Value;
use dbt_serde_yaml::ShouldBe;
use dbt_serde_yaml::Spanned;
use dbt_serde_yaml::UntaggedEnumDeserialize;
use dbt_serde_yaml::Verbatim;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use strum::{Display, EnumString};

use crate::schemas::common::DbtQuoting;
use crate::schemas::project::ProjectAnalysisConfig;
use crate::schemas::project::ProjectSemanticModelConfig;
use crate::schemas::project::configs::saved_query_config::ProjectSavedQueryConfig;
use crate::schemas::serde::FloatOrString;
use crate::schemas::serde::SpannedStringOrArrayOfStrings;
use crate::schemas::serde::StringOrArrayOfStrings;
use crate::schemas::serde::StringOrInteger;

use super::ProjectDataTestConfig;
use super::ProjectExposureConfig;
use super::ProjectFunctionConfig;
use super::ProjectMetricConfigs;
use super::ProjectModelConfig;
use super::ProjectSeedConfig;
use super::ProjectSnapshotConfig;
use super::ProjectSourceConfig;
use super::ProjectUnitTestConfig;

#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct ProjectDbtCloudConfig {
    #[serde(rename = "project-id")]
    pub project_id: Option<StringOrInteger>,
    #[serde(rename = "defer-env-id")]
    pub defer_env_id: Option<StringOrInteger>,

    // unsure if any of these other keys are actually used or expected
    pub account_id: Option<StringOrInteger>,
    #[serde(rename = "account-host")]
    pub account_host: Option<String>,
    #[serde(rename = "job-id")]
    pub job_id: Option<StringOrInteger>,
    #[serde(rename = "run-id")]
    pub run_id: Option<StringOrInteger>,
    pub api_key: Option<StringOrInteger>,
    pub application: Option<StringOrInteger>,
    pub environment: Option<StringOrInteger>,
    pub tenant_hostname: Option<String>,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct DbtProjectNameOnly {
    pub name: String,

    pub __ignored__: Verbatim<HashMap<String, dbt_serde_yaml::Value>>,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct DbtProjectSimplified {
    #[serde(rename = "packages-install-path")]
    pub packages_install_path: Option<String>,
    pub profile: Option<String>,
    #[serde(rename = "dbt-cloud")]
    pub dbt_cloud: Option<ProjectDbtCloudConfig>,

    // Deprecated paths
    // When present in the db_project.yml file we will raise an error
    #[serde(rename = "data-paths")]
    pub data_paths: Verbatim<Option<Vec<String>>>,
    #[serde(rename = "source-paths")]
    pub source_paths: Verbatim<Option<Vec<String>>>,
    #[serde(rename = "log-path")]
    pub log_path: Verbatim<Option<String>>,
    #[serde(rename = "target-path")]
    pub target_path: Verbatim<Option<String>>,

    pub __ignored__: Verbatim<HashMap<String, dbt_serde_yaml::Value>>,
}

#[derive(
    Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, EnumString, Display, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum LogPath {
    #[default]
    Logs,
}

#[derive(
    Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, EnumString, Display, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TargetPath {
    #[default]
    Target,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct DbtProject {
    pub name: String,
    pub version: Option<FloatOrString>,
    pub profile: Option<String>,
    // Paths
    #[serde(rename = "analysis-paths")]
    pub analysis_paths: Option<Vec<String>>,
    #[serde(rename = "asset-paths")]
    pub asset_paths: Option<Vec<String>>,
    #[serde(rename = "macro-paths")]
    pub macro_paths: Option<Vec<String>>,
    #[serde(rename = "model-paths")]
    pub model_paths: Option<Vec<String>>,
    #[serde(rename = "function-paths")]
    pub function_paths: Option<Vec<String>>,
    #[serde(rename = "seed-paths")]
    pub seed_paths: Option<Vec<String>>,
    #[serde(rename = "snapshot-paths")]
    pub snapshot_paths: Option<Vec<String>>,
    #[serde(rename = "test-paths")]
    pub test_paths: Option<Vec<String>>,
    #[serde(rename = "docs-paths")]
    pub docs_paths: Option<Vec<String>>,
    #[serde(rename = "target-path")]
    pub target_path: Option<TargetPath>,
    #[serde(rename = "log-path")]
    pub log_path: Option<LogPath>,
    #[serde(rename = "packages-install-path")]
    pub packages_install_path: Option<String>,
    // Configs
    pub metrics: Option<ProjectMetricConfigs>,
    pub models: Option<ProjectModelConfig>,
    pub functions: Option<ProjectFunctionConfig>,
    pub snapshots: Option<ProjectSnapshotConfig>,
    pub seeds: Option<ProjectSeedConfig>,
    pub sources: Option<ProjectSourceConfig>,
    pub tests: Option<ProjectDataTestConfig>,
    pub unit_tests: Option<ProjectUnitTestConfig>,
    pub data_tests: Option<ProjectDataTestConfig>,
    pub exposures: Option<ProjectExposureConfig>,
    pub analyses: Option<ProjectAnalysisConfig>,
    #[serde(rename = "saved-queries")]
    pub saved_queries: Option<ProjectSavedQueryConfig>,
    #[serde(rename = "semantic-models")]
    pub semantic_models: Option<ProjectSemanticModelConfig>,
    // Misc
    #[serde(rename = "clean-targets")]
    pub clean_targets: Option<Vec<String>>,
    #[serde(rename = "config-version")]
    pub config_version: Option<i32>,
    #[serde(rename = "dbt-cloud")]
    pub dbt_cloud: Option<ProjectDbtCloudConfig>,
    pub dispatch: Option<Vec<_Dispatch>>,
    pub flags: Option<YmlValue>,
    #[serde(rename = "on-run-end")]
    pub on_run_end: Verbatim<Option<SpannedStringOrArrayOfStrings>>,
    #[serde(rename = "on-run-start")]
    pub on_run_start: Verbatim<Option<SpannedStringOrArrayOfStrings>>,
    #[serde(rename = "query-comment")]
    pub query_comment: Verbatim<Option<QueryComment>>,
    pub quoting: Spanned<Option<DbtQuoting>>,
    #[serde(rename = "require-dbt-version")]
    pub require_dbt_version: Option<StringOrArrayOfStrings>,
    #[serde(rename = "restrict-access")]
    pub restrict_access: Option<bool>,
    pub vars: Verbatim<Option<dbt_serde_yaml::Value>>,
}

impl DbtProject {
    pub fn get_project_id(&self) -> String {
        /*
        Returns the hash of the project name. Can be used for telemetry.
        */
        // TODO: do we really need cryptographic hashing here?
        format!("{:x}", md5::compute(self.name.as_bytes()))
    }

    pub fn all_source_paths(&self) -> Vec<String> {
        /*
        Returns a vector of strings combining all path configurations:
        model_paths, function_paths, seed_paths, snapshot_paths, analysis_paths, macro_paths, and test_paths.
        */
        let mut paths = Vec::new();

        if let Some(ref model_paths) = self.model_paths {
            paths.extend(model_paths.clone());
        }
        if let Some(ref function_paths) = self.function_paths {
            paths.extend(function_paths.clone());
        }
        if let Some(ref seed_paths) = self.seed_paths {
            paths.extend(seed_paths.clone());
        }
        if let Some(ref snapshot_paths) = self.snapshot_paths {
            paths.extend(snapshot_paths.clone());
        }
        if let Some(ref analysis_paths) = self.analysis_paths {
            paths.extend(analysis_paths.clone());
        }
        if let Some(ref macro_paths) = self.macro_paths {
            paths.extend(macro_paths.clone());
        }
        if let Some(ref test_paths) = self.test_paths {
            paths.extend(test_paths.clone());
        }

        paths
    }
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct _Dispatch {
    pub macro_namespace: String,
    pub search_order: Vec<String>,
}

#[derive(UntaggedEnumDeserialize, Serialize, Debug, Clone, JsonSchema)]
#[serde(untagged)]
pub enum QueryComment {
    String(String),
    Object(YmlValue),
}

/// This trait is used to default fields in a config to the values of a parent config.
pub trait DefaultTo<T>:
    Serialize + DeserializeOwned + Default + Debug + Clone + Send + Sync
{
    fn default_to(&mut self, parent: &T);

    fn get_enabled(&self) -> Option<bool> {
        None
    }

    fn is_incremental(&self) -> bool {
        false
    }

    fn database(&self) -> Option<String> {
        None
    }

    fn schema(&self) -> Option<String> {
        None
    }

    fn alias(&self) -> Option<String> {
        None
    }

    fn get_pre_hook(&self) -> Option<&crate::schemas::common::Hooks> {
        None
    }

    fn get_post_hook(&self) -> Option<&crate::schemas::common::Hooks> {
        None
    }
}

// Improved macro for simple field defaulting with mutable references
#[macro_export]
macro_rules! default_to {
    ($parent:ident, [$($field:ident),* $(,)?]) => {
        $(
            if $field.is_none() {
                *$field = $parent.$field.clone();
            }
        )*
    };
}

pub trait IterChildren<T> {
    fn iter_children(&self) -> Iter<'_, String, ShouldBe<T>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_project_id() {
        let project = DbtProject {
            name: "fishtown_internal_analytics".to_string(),
            version: Some(FloatOrString::String("1.0".to_string())),
            profile: Some("garage-snowflake".to_string()),
            analysis_paths: Some(vec![]),
            asset_paths: Some(vec![]),
            macro_paths: Some(vec![]),
            model_paths: Some(vec![]),
            function_paths: Some(vec![]),
            seed_paths: Some(vec![]),
            snapshot_paths: Some(vec![]),
            test_paths: Some(vec![]),
            docs_paths: Some(vec![]),
            target_path: Some(TargetPath::Target),
            log_path: Some(LogPath::Logs),
            packages_install_path: Some("packages".to_string()),
            metrics: None,
            models: None,
            functions: None,
            snapshots: None,
            seeds: None,
            sources: None,
            tests: None,
            unit_tests: None,
            data_tests: None,
            saved_queries: None,
            semantic_models: None,
            exposures: None,
            analyses: None,
            clean_targets: None,
            config_version: None,
            dbt_cloud: None,
            dispatch: None,
            flags: None,
            on_run_end: Verbatim::from(None),
            on_run_start: Verbatim::from(None),
            query_comment: Verbatim::from(None),
            quoting: Spanned::new(None),
            require_dbt_version: None,
            restrict_access: None,
            vars: Verbatim::from(None),
        };
        assert_eq!(project.get_project_id(), "92c907bdbc0c4f27451b9b9fdb1bc8ec");
    }
}
