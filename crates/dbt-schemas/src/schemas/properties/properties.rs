use crate::schemas::common::DocsConfig;
use crate::schemas::common::Versions;
use crate::schemas::serde::{FloatOrString, bool_or_string_bool, string_or_array};
use dbt_serde_yaml::JsonSchema;
use dbt_serde_yaml::Verbatim;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::BTreeMap;

use super::DataTestProperties;
use super::ExposureProperties;
use super::MetricsProperties;
use super::ModelProperties;
use super::SavedQueriesProperties;
use super::SeedProperties;
use super::SemanticModelsProperties;
use super::SnapshotProperties;
use super::SourceProperties;
use super::unit_test_properties::UnitTestProperties;

#[derive(Deserialize, Debug)]
pub struct DbtPropertiesFileValues {
    pub version: Option<FloatOrString>,
    pub analyses: Option<Vec<dbt_serde_yaml::Value>>,
    pub exposures: Option<Vec<dbt_serde_yaml::Value>>,
    pub groups: Option<Vec<dbt_serde_yaml::Value>>,
    pub macros: Option<Vec<dbt_serde_yaml::Value>>,
    pub metrics: Option<Vec<dbt_serde_yaml::Value>>,
    pub models: Option<Vec<dbt_serde_yaml::Value>>,
    pub saved_queries: Option<Vec<dbt_serde_yaml::Value>>,
    pub seeds: Option<Vec<dbt_serde_yaml::Value>>,
    pub semantic_models: Option<Vec<dbt_serde_yaml::Value>>,
    pub snapshots: Option<Vec<dbt_serde_yaml::Value>>,
    pub sources: Option<Vec<dbt_serde_yaml::Value>>,
    pub unit_tests: Option<Vec<dbt_serde_yaml::Value>>,
    pub tests: Option<Vec<dbt_serde_yaml::Value>>,
    pub data_tests: Option<Vec<dbt_serde_yaml::Value>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MinimalSchemaValue {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<FloatOrString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versions: Option<Vec<Versions>>,
    pub tables: Verbatim<Option<Vec<dbt_serde_yaml::Value>>>,
    pub __additional_properties__: Verbatim<BTreeMap<String, dbt_serde_yaml::Value>>,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MinimalTableValue {
    pub name: String,
    pub __additional_properties__: Verbatim<BTreeMap<String, dbt_serde_yaml::Value>>,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct DbtPropertiesFile {
    pub models: Option<Vec<ModelProperties>>,
    pub snapshots: Option<Vec<SnapshotProperties>>,
    pub seeds: Option<Vec<SeedProperties>>,
    pub saved_queries: Option<Vec<SavedQueriesProperties>>,
    pub sources: Option<Vec<SourceProperties>>,
    pub unit_tests: Option<Vec<UnitTestProperties>>,
    pub tests: Option<Vec<DataTestProperties>>,
    pub data_tests: Option<Vec<DataTestProperties>>,
    pub analyses: Option<Vec<AnalysesProperties>>,
    pub exposures: Option<Vec<ExposureProperties>>,
    pub groups: Option<Vec<GroupsProperties>>,
    pub macros: Option<Vec<MacrosProperties>>,
    pub metrics: Option<Vec<MetricsProperties>>,
    pub semantic_models: Option<Vec<SemanticModelsProperties>>,
    pub version: Option<FloatOrString>,
}

// -- Additional Properties

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct AnalysesProperties {
    pub columns: Option<Vec<AnalysesColumns>>,
    pub config: Option<AnalysesConfig>,
    pub description: Option<String>,
    pub name: String,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct AnalysesColumns {
    pub data_type: Option<String>,
    pub description: Option<String>,
    pub name: String,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct AnalysesConfig {
    #[serde(default, deserialize_with = "string_or_array")]
    pub tags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "bool_or_string_bool")]
    pub enabled: Option<bool>,
    pub meta: Option<BTreeMap<String, serde_json::Value>>,
    pub docs: Option<DocsConfig>,
    pub group: Option<String>,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct GroupsProperties {
    pub name: String,
    pub owner: GroupsOwner,
    pub description: Option<String>,
    pub config: Option<GroupsConfig>,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct GroupsOwner {
    pub email: Option<String>,
    pub name: Option<String>,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct GroupsConfig {
    pub meta: Option<serde_json::Value>,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct MacrosProperties {
    pub arguments: Option<Vec<MacrosArguments>>,
    pub description: Option<String>,
    pub docs: Option<DocsConfig>,
    pub name: String,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct MacrosArguments {
    pub description: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub type_: Option<String>,
}

pub trait GetConfig<T>: DeserializeOwned + Send + Sync {
    fn get_config(&self) -> Option<&T>;
}
