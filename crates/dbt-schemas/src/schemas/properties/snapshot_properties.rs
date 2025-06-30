use dbt_serde_yaml::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::schemas::data_tests::DataTests;
use crate::schemas::dbt_column::ColumnProperties;
use crate::schemas::project::SnapshotConfig;
use crate::schemas::properties::GetConfig;

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct SnapshotProperties {
    pub name: String,
    pub relation: Option<String>,
    pub columns: Option<Vec<ColumnProperties>>,
    pub config: Option<SnapshotConfig>,
    pub data_tests: Option<Vec<DataTests>>,
    pub description: Option<String>,
    pub tests: Option<Vec<DataTests>>,
}

impl GetConfig<SnapshotConfig> for SnapshotProperties {
    fn get_config(&self) -> Option<&SnapshotConfig> {
        self.config.as_ref()
    }
}

impl SnapshotProperties {
    pub fn empty(name: String) -> Self {
        Self {
            name,
            relation: None,
            columns: None,
            config: None,
            data_tests: None,
            description: None,
            tests: None,
        }
    }
}
