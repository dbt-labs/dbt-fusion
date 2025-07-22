use dbt_serde_yaml::{JsonSchema, ShouldBe};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::BTreeMap;

use crate::{
    default_to,
    schemas::{
        project::{DefaultTo, configs::common::default_meta_and_tags},
        serde::{StringOrArrayOfStrings, bool_or_string_bool},
    },
};

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct ProjectExposureConfig {
    #[serde(rename = "+meta")]
    pub meta: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(rename = "+tags")]
    pub tags: Option<StringOrArrayOfStrings>,
    #[serde(default, rename = "+enabled", deserialize_with = "bool_or_string_bool")]
    pub enabled: Option<bool>,
    #[serde(flatten)]
    pub __additional_properties__: BTreeMap<String, ShouldBe<ProjectExposureConfig>>,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Debug, Clone, Default, JsonSchema)]
pub struct ExposureConfig {
    #[serde(default, deserialize_with = "bool_or_string_bool")]
    pub enabled: Option<bool>,
    pub meta: Option<BTreeMap<String, serde_json::Value>>,
    pub tags: Option<StringOrArrayOfStrings>,
}

impl From<ProjectExposureConfig> for ExposureConfig {
    fn from(config: ProjectExposureConfig) -> Self {
        Self {
            enabled: config.enabled,
            meta: config.meta,
            tags: config.tags,
        }
    }
}

impl From<ExposureConfig> for ProjectExposureConfig {
    fn from(config: ExposureConfig) -> Self {
        Self {
            meta: config.meta,
            tags: config.tags,
            enabled: config.enabled,
            __additional_properties__: BTreeMap::new(),
        }
    }
}

impl DefaultTo<ExposureConfig> for ExposureConfig {
    fn get_enabled(&self) -> Option<bool> {
        self.enabled
    }

    fn default_to(&mut self, parent: &ExposureConfig) {
        let ExposureConfig {
            meta,
            tags,
            enabled,
        } = self;

        #[allow(unused, clippy::let_unit_value)]
        let meta = default_meta_and_tags(meta, &parent.meta, tags, &parent.tags);
        #[allow(unused)]
        let tags = ();

        default_to!(parent, [enabled]);
    }
}
