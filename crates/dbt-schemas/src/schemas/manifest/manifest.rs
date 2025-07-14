use chrono::{DateTime, Utc};
use dbt_common::io_args::StaticAnalysisKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use crate::{
    dbt_utils::get_dbt_schema_version,
    schemas::{
        common::{DbtChecksum, DbtMaterialization, DbtQuoting, NodeDependsOn},
        macros::{DbtDocsMacro, DbtMacro},
        manifest::manifest_nodes::{
            ManifestDataTest, ManifestModel, ManifestSeed, ManifestSnapshot, ManifestSource,
            ManifestUnitTest,
        },
        nodes::{DbtSeedAttr, DbtSnapshotAttr, DbtSourceAttr, DbtTestAttr},
        CommonAttributes, DbtModel, DbtModelAttr, DbtSeed, DbtSnapshot, DbtSource, DbtTest,
        DbtUnitTest, DbtUnitTestAttr, InternalDbtNode, IntrospectionKind, NodeBaseAttributes,
        Nodes,
    },
    state::ResolverState,
};

use super::{
    DbtExposure, DbtGroup, DbtMetric, DbtOperation, DbtSavedQuery, DbtSelector, DbtSemanticModel,
};

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "resource_type")]
#[serde(rename_all = "snake_case")]
pub enum DbtNode {
    Model(ManifestModel),
    Test(ManifestDataTest),
    Snapshot(ManifestSnapshot),
    Seed(ManifestSeed),
    Operation(DbtOperation),
    Analysis(ManifestModel),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct ManifestMetadata {
    #[serde(flatten)]
    pub base: BaseMetadata,
    #[serde(default)]
    pub project_name: String,
    pub project_id: Option<String>,
    pub user_id: Option<String>,
    pub send_anonymous_usage_stats: Option<bool>,
    #[serde(default)]
    pub adapter_type: String,
    pub quoting: Option<DbtQuoting>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BaseMetadata {
    pub dbt_schema_version: String,
    pub dbt_version: String,
    pub generated_at: DateTime<Utc>,
    pub invocation_id: Option<String>,
    pub invocation_started_at: Option<DateTime<Utc>>,
    pub env: BTreeMap<String, String>,
}

impl PartialEq for ManifestMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.base.env == other.base.env
            && self.project_name == other.project_name
            && self.send_anonymous_usage_stats == other.send_anonymous_usage_stats
            && self.adapter_type == other.adapter_type
        // Note: We intentionally skip comparing the following right now:
        // - generated_at (timestamp)
        // - invocation_id (changes each run)
        // - user_id (may change between environments)
        // - dbt_schema_version (changes between versions)
        // - dbt_version (changes between versions)
        // - project_id (changes between environments)
    }
}

impl Eq for ManifestMetadata {}

/// External representation of the manifest, internal we use Nodes
#[derive(Debug, Default, Deserialize)]
pub struct DbtManifest {
    pub metadata: ManifestMetadata,
    pub nodes: BTreeMap<String, DbtNode>,
    pub sources: BTreeMap<String, ManifestSource>,
    pub macros: BTreeMap<String, DbtMacro>,
    pub unit_tests: BTreeMap<String, ManifestUnitTest>,
    pub docs: BTreeMap<String, DbtDocsMacro>,
    pub semantic_models: BTreeMap<String, DbtSemanticModel>,
    pub saved_queries: BTreeMap<String, DbtSavedQuery>,
    pub exposures: BTreeMap<String, DbtExposure>,
    pub metrics: BTreeMap<String, DbtMetric>,
    pub child_map: BTreeMap<String, Vec<String>>,
    pub parent_map: BTreeMap<String, Vec<String>>,
    pub group_map: BTreeMap<String, Vec<String>>,
    pub disabled: BTreeMap<String, Vec<Value>>,
    pub selectors: BTreeMap<String, DbtSelector>,
    pub groups: BTreeMap<String, DbtGroup>,
}

impl DbtManifest {
    pub fn into_map_compiled_sql(self) -> HashMap<String, Option<String>> {
        self.nodes
            .into_iter()
            .filter_map(|(id, node)| match node {
                DbtNode::Model(model) => Some((id, model.base_attr.compiled_code)),
                DbtNode::Test(test) => Some((id, test.base_attr.compiled_code)),
                DbtNode::Snapshot(snapshot) => Some((id, snapshot.base_attr.compiled_code)),
                DbtNode::Seed(seed) => Some((id, seed.base_attr.compiled_code)),
                DbtNode::Operation(_operation) => None,
                DbtNode::Analysis(analysis) => Some((id, analysis.base_attr.compiled_code)),
            })
            .collect::<HashMap<_, _>>()
    }
}

impl Serialize for DbtManifest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = BTreeMap::new();
        map.insert(
            "metadata".to_string(),
            serde_json::to_value(&self.metadata).map_err(serde::ser::Error::custom)?,
        );
        map.insert(
            "nodes".to_string(),
            serde_json::to_value(&self.nodes).map_err(serde::ser::Error::custom)?,
        );

        // Serialize sources using InternalDbtNode trait
        let sources_serialized: BTreeMap<String, Value> = self
            .sources
            .iter()
            .map(|(k, v)| {
                Ok((
                    k.clone(),
                    serialize_with_resource_type(
                        serde_json::to_value(v).map_err(serde::ser::Error::custom)?,
                        "source",
                    ),
                ))
            })
            .collect::<Result<_, _>>()?;
        map.insert(
            "sources".to_string(),
            serde_json::to_value(sources_serialized).map_err(serde::ser::Error::custom)?,
        );

        // Serialize macros using InternalDbtNode trait
        let macros_serialized: BTreeMap<String, Value> = self
            .macros
            .iter()
            .map(|(k, v)| (k.clone(), InternalDbtNode::serialize(v)))
            .collect();
        map.insert(
            "macros".to_string(),
            serde_json::to_value(macros_serialized).map_err(serde::ser::Error::custom)?,
        );

        // Serialize unit_tests using InternalDbtNode trait
        let unit_tests_serialized: BTreeMap<String, Value> = self
            .unit_tests
            .iter()
            .map(|(k, v)| {
                Ok((
                    k.clone(),
                    serialize_with_resource_type(
                        serde_json::to_value(v).map_err(serde::ser::Error::custom)?,
                        "unit_test",
                    ),
                ))
            })
            .collect::<Result<_, _>>()?;
        map.insert(
            "unit_tests".to_string(),
            serde_json::to_value(unit_tests_serialized).map_err(serde::ser::Error::custom)?,
        );

        map.insert(
            "docs".to_string(),
            serde_json::to_value(&self.docs).map_err(serde::ser::Error::custom)?,
        );

        // Serialize semantic_models using InternalDbtNode trait
        let semantic_models_serialized: BTreeMap<String, Value> = self
            .semantic_models
            .iter()
            .map(|(k, v)| (k.clone(), InternalDbtNode::serialize(v)))
            .collect();
        map.insert(
            "semantic_models".to_string(),
            serde_json::to_value(semantic_models_serialized).map_err(serde::ser::Error::custom)?,
        );

        // Serialize saved_queries using InternalDbtNode trait
        let saved_queries_serialized: BTreeMap<String, Value> = self
            .saved_queries
            .iter()
            .map(|(k, v)| (k.clone(), InternalDbtNode::serialize(v)))
            .collect();
        map.insert(
            "saved_queries".to_string(),
            serde_json::to_value(saved_queries_serialized).map_err(serde::ser::Error::custom)?,
        );

        // Serialize exposures using InternalDbtNode trait
        let exposures_serialized: BTreeMap<String, Value> = self
            .exposures
            .iter()
            .map(|(k, v)| (k.clone(), InternalDbtNode::serialize(v)))
            .collect();
        map.insert(
            "exposures".to_string(),
            serde_json::to_value(exposures_serialized).map_err(serde::ser::Error::custom)?,
        );

        // Serialize metrics using InternalDbtNode trait
        let metrics_serialized: BTreeMap<String, Value> = self
            .metrics
            .iter()
            .map(|(k, v)| (k.clone(), InternalDbtNode::serialize(v)))
            .collect();
        map.insert(
            "metrics".to_string(),
            serde_json::to_value(metrics_serialized).map_err(serde::ser::Error::custom)?,
        );

        map.insert(
            "child_map".to_string(),
            serde_json::to_value(&self.child_map).map_err(serde::ser::Error::custom)?,
        );
        map.insert(
            "parent_map".to_string(),
            serde_json::to_value(&self.parent_map).map_err(serde::ser::Error::custom)?,
        );
        map.insert(
            "group_map".to_string(),
            serde_json::to_value(&self.group_map).map_err(serde::ser::Error::custom)?,
        );
        map.insert(
            "disabled".to_string(),
            serde_json::to_value(&self.disabled).map_err(serde::ser::Error::custom)?,
        );
        map.insert(
            "selectors".to_string(),
            serde_json::to_value(&self.selectors).map_err(serde::ser::Error::custom)?,
        );
        map.insert(
            "groups".to_string(),
            serde_json::to_value(&self.groups).map_err(serde::ser::Error::custom)?,
        );

        map.serialize(serializer)
    }
}

pub fn serialize_with_resource_type(mut value: Value, resource_type: &str) -> Value {
    if let Value::Object(ref mut map) = value {
        map.insert(
            "resource_type".to_string(),
            Value::String(resource_type.to_string()),
        );
    }
    value
}

pub fn build_manifest(invocation_id: &str, resolver_state: &ResolverState) -> DbtManifest {
    DbtManifest {
        metadata: ManifestMetadata {
            base: BaseMetadata {
                dbt_schema_version: get_dbt_schema_version("manifest", 20),
                dbt_version: env!("CARGO_PKG_VERSION").to_string(),
                generated_at: Utc::now(),
                invocation_id: Some(invocation_id.to_string()),
                ..Default::default()
            },
            project_name: resolver_state.root_project_name.clone(),
            adapter_type: resolver_state.dbt_profile.db_config.adapter_type(),
            ..Default::default()
        },
        nodes: resolver_state
            .nodes
            .models
            .iter()
            .map(|(id, node)| (id.clone(), DbtNode::Model((**node).clone().into())))
            .chain(
                resolver_state
                    .nodes
                    .tests
                    .iter()
                    .map(|(id, node)| (id.clone(), DbtNode::Test((**node).clone().into()))),
            )
            .chain(
                resolver_state
                    .nodes
                    .snapshots
                    .iter()
                    .map(|(id, node)| (id.clone(), DbtNode::Snapshot((**node).clone().into()))),
            )
            .chain(
                resolver_state
                    .nodes
                    .seeds
                    .iter()
                    .map(|(id, node)| (id.clone(), DbtNode::Seed((**node).clone().into()))),
            )
            .chain(
                resolver_state
                    .nodes
                    .tests
                    .iter()
                    .map(|(id, node)| (id.clone(), DbtNode::Test((**node).clone().into()))),
            )
            .chain(resolver_state.operations.on_run_start.iter().map(|node| {
                (
                    node.common_attr.unique_id.clone(),
                    DbtNode::Operation(node.clone()),
                )
            }))
            .chain(resolver_state.operations.on_run_end.iter().map(|node| {
                (
                    node.common_attr.unique_id.clone(),
                    DbtNode::Operation(node.clone()),
                )
            }))
            .collect(),
        sources: resolver_state
            .nodes
            .sources
            .iter()
            .map(|(id, source)| (id.clone(), (**source).clone().into()))
            .collect(),
        unit_tests: resolver_state
            .nodes
            .unit_tests
            .iter()
            .map(|(id, unit_test)| (id.clone(), (**unit_test).clone().into()))
            .collect(),
        macros: resolver_state.macros.macros.clone(),
        docs: resolver_state.macros.docs_macros.clone(),
        ..Default::default()
    }
}

pub fn nodes_from_dbt_manifest(manifest: DbtManifest, dbt_quoting: DbtQuoting) -> Nodes {
    let mut nodes = Nodes::default();
    // Do not put disabled nodes into the nodes, because all things in Nodes object should be enabled.
    for (unique_id, node) in manifest.nodes {
        match node {
            DbtNode::Model(model) => {
                nodes.models.insert(
                    unique_id,
                    Arc::new(DbtModel {
                        common_attr: CommonAttributes {
                            unique_id: model.common_attr.unique_id,
                            name: model.common_attr.name,
                            package_name: model.common_attr.package_name,
                            path: model.common_attr.path,
                            original_file_path: model.common_attr.original_file_path,
                            patch_path: model.common_attr.patch_path,
                            fqn: model.common_attr.fqn,
                            description: model.common_attr.description,
                            raw_code: model.base_attr.raw_code,
                            checksum: model.base_attr.checksum,
                            language: model.base_attr.language,
                            tags: model
                                .config
                                .tags
                                .clone()
                                .map(|tags| tags.into())
                                .unwrap_or_default(),
                            meta: model.config.meta.clone().unwrap_or_default(),
                        },
                        base_attr: NodeBaseAttributes {
                            database: model.common_attr.database,
                            schema: model.common_attr.schema,
                            alias: model.base_attr.alias,
                            relation_name: model.base_attr.relation_name,
                            materialized: model
                                .config
                                .materialized
                                .clone()
                                .unwrap_or(DbtMaterialization::View),
                            static_analysis: StaticAnalysisKind::On,
                            enabled: model.config.enabled.unwrap_or(true),
                            extended_model: false,
                            quoting: model
                                .config
                                .quoting
                                .map(|mut quoting| {
                                    quoting.default_to(&dbt_quoting);
                                    quoting
                                })
                                .unwrap_or(dbt_quoting)
                                .try_into()
                                .expect("DbtQuoting should be set"),
                            columns: model.base_attr.columns,
                            depends_on: model.base_attr.depends_on,
                            refs: model.base_attr.refs,
                            sources: model.base_attr.sources,
                            metrics: model.base_attr.metrics,
                        },
                        model_attr: DbtModelAttr {
                            access: model.config.access.clone().unwrap_or_default(),
                            group: model.config.group.clone(),
                            contract: model.config.contract.clone(),
                            incremental_strategy: model.config.incremental_strategy.clone(),
                            freshness: model.config.freshness.clone(),
                            introspection: IntrospectionKind::None,
                            version: model.version,
                            latest_version: model.latest_version,
                            constraints: model.constraints.unwrap_or_default(),
                            deprecation_date: model.deprecation_date,
                            primary_key: model.primary_key.unwrap_or_default(),
                            time_spine: model.time_spine,
                            event_time: model.config.event_time.clone(),
                        },
                        deprecated_config: model.config,
                        other: model.other,
                    }),
                );
            }
            DbtNode::Test(test) => {
                nodes.tests.insert(
                    unique_id,
                    Arc::new(DbtTest {
                        common_attr: CommonAttributes {
                            unique_id: test.common_attr.unique_id,
                            name: test.common_attr.name,
                            package_name: test.common_attr.package_name,
                            path: test.common_attr.path,
                            original_file_path: test.common_attr.original_file_path,
                            patch_path: test.common_attr.patch_path,
                            fqn: test.common_attr.fqn,
                            description: test.common_attr.description,
                            raw_code: test.base_attr.raw_code,
                            checksum: test.base_attr.checksum,
                            language: test.base_attr.language,
                            tags: test
                                .config
                                .tags
                                .clone()
                                .map(|tags| tags.into())
                                .unwrap_or_default(),
                            meta: test.config.meta.clone().unwrap_or_default(),
                        },
                        base_attr: NodeBaseAttributes {
                            database: test.common_attr.database,
                            schema: test.common_attr.schema,
                            alias: test.base_attr.alias,
                            relation_name: test.base_attr.relation_name,
                            materialized: DbtMaterialization::Test,
                            static_analysis: StaticAnalysisKind::On,
                            enabled: test.config.enabled.unwrap_or(true),
                            extended_model: false,
                            quoting: test
                                .config
                                .quoting
                                .map(|mut quoting| {
                                    quoting.default_to(&dbt_quoting);
                                    quoting
                                })
                                .unwrap_or(dbt_quoting)
                                .try_into()
                                .expect("DbtQuoting should be set"),
                            columns: test.base_attr.columns,
                            depends_on: test.base_attr.depends_on,
                            refs: test.base_attr.refs,
                            sources: test.base_attr.sources,
                            metrics: test.base_attr.metrics,
                        },
                        test_attr: DbtTestAttr {
                            column_name: test.column_name,
                            attached_node: test.attached_node,
                            test_metadata: test.test_metadata,
                            file_key_name: test.file_key_name,
                        },
                        deprecated_config: test.config,
                        other: test.other,
                    }),
                );
            }
            DbtNode::Snapshot(snapshot) => {
                nodes.snapshots.insert(
                    unique_id,
                    Arc::new(DbtSnapshot {
                        common_attr: CommonAttributes {
                            unique_id: snapshot.common_attr.unique_id,
                            name: snapshot.common_attr.name,
                            package_name: snapshot.common_attr.package_name,
                            path: snapshot.common_attr.path,
                            original_file_path: snapshot.common_attr.original_file_path,
                            patch_path: snapshot.common_attr.patch_path,
                            fqn: snapshot.common_attr.fqn,
                            description: snapshot.common_attr.description,
                            raw_code: snapshot.base_attr.raw_code,
                            checksum: snapshot.base_attr.checksum,
                            language: snapshot.base_attr.language,
                            tags: snapshot
                                .config
                                .tags
                                .clone()
                                .map(|tags| tags.into())
                                .unwrap_or_default(),
                            meta: snapshot.config.meta.clone().unwrap_or_default(),
                        },
                        base_attr: NodeBaseAttributes {
                            database: snapshot.common_attr.database,
                            schema: snapshot.common_attr.schema,
                            alias: snapshot.base_attr.alias,
                            relation_name: snapshot.base_attr.relation_name,
                            enabled: snapshot.config.enabled.unwrap_or(true),
                            extended_model: false,
                            materialized: snapshot
                                .config
                                .materialized
                                .clone()
                                .unwrap_or(DbtMaterialization::Table),
                            static_analysis: StaticAnalysisKind::On,
                            quoting: snapshot
                                .config
                                .quoting
                                .map(|mut quoting| {
                                    quoting.default_to(&dbt_quoting);
                                    quoting
                                })
                                .unwrap_or(dbt_quoting)
                                .try_into()
                                .expect("DbtQuoting should be set"),
                            columns: snapshot.base_attr.columns,
                            depends_on: snapshot.base_attr.depends_on,
                            refs: snapshot.base_attr.refs,
                            sources: snapshot.base_attr.sources,
                            metrics: snapshot.base_attr.metrics,
                        },
                        snapshot_attr: DbtSnapshotAttr {
                            snapshot_meta_column_names: snapshot
                                .config
                                .snapshot_meta_column_names
                                .clone()
                                .unwrap_or_default(),
                        },
                        deprecated_config: snapshot.config,
                        compiled: snapshot.base_attr.compiled,
                        compiled_code: snapshot.base_attr.compiled_code,
                        other: snapshot.other,
                    }),
                );
            }
            DbtNode::Seed(seed) => {
                nodes.seeds.insert(
                    unique_id,
                    Arc::new(DbtSeed {
                        common_attr: CommonAttributes {
                            unique_id: seed.common_attr.unique_id,
                            name: seed.common_attr.name,
                            package_name: seed.common_attr.package_name,
                            path: seed.common_attr.path,
                            original_file_path: seed.common_attr.original_file_path,
                            patch_path: seed.common_attr.patch_path,
                            fqn: seed.common_attr.fqn,
                            description: seed.common_attr.description,
                            raw_code: seed.base_attr.raw_code,
                            checksum: seed.base_attr.checksum,
                            language: seed.base_attr.language,
                            tags: seed
                                .config
                                .tags
                                .clone()
                                .map(|tags| tags.into())
                                .unwrap_or_default(),
                            meta: seed.config.meta.clone().unwrap_or_default(),
                        },
                        base_attr: NodeBaseAttributes {
                            database: seed.common_attr.database,
                            schema: seed.common_attr.schema,
                            alias: seed.base_attr.alias,
                            relation_name: seed.base_attr.relation_name,
                            materialized: DbtMaterialization::Table,
                            static_analysis: StaticAnalysisKind::On,
                            enabled: seed.config.enabled.unwrap_or(true),
                            quoting: seed
                                .config
                                .quoting
                                .map(|mut quoting| {
                                    quoting.default_to(&dbt_quoting);
                                    quoting
                                })
                                .unwrap_or(dbt_quoting)
                                .try_into()
                                .expect("DbtQuoting should be set"),
                            extended_model: false,
                            columns: seed.base_attr.columns,
                            depends_on: seed.base_attr.depends_on,
                            refs: seed.base_attr.refs,
                            sources: seed.base_attr.sources,
                            metrics: seed.base_attr.metrics,
                        },
                        seed_attr: DbtSeedAttr {
                            quote_columns: seed.config.quote_columns.unwrap_or_default(),
                            column_types: seed.config.column_types.clone(),
                            delimiter: seed.config.delimiter.clone().map(|d| d.into_inner()),
                            root_path: seed.root_path,
                        },
                        deprecated_config: seed.config,
                        other: seed.other,
                    }),
                );
            }
            DbtNode::Operation(_) => {}
            DbtNode::Analysis(_) => {}
        }
    }
    for (unique_id, source) in manifest.sources {
        nodes.sources.insert(
            unique_id,
            Arc::new(DbtSource {
                common_attr: CommonAttributes {
                    unique_id: source.common_attr.unique_id,
                    name: source.common_attr.name,
                    package_name: source.common_attr.package_name,
                    path: source.common_attr.path,
                    original_file_path: source.common_attr.original_file_path,
                    patch_path: source.common_attr.patch_path,
                    fqn: source.common_attr.fqn,
                    description: source.common_attr.description,
                    raw_code: None,
                    checksum: DbtChecksum::default(),
                    language: None,
                    tags: source
                        .config
                        .tags
                        .clone()
                        .map(|tags| tags.into())
                        .unwrap_or_default(),
                    meta: source.config.meta.clone().unwrap_or_default(),
                },
                base_attr: NodeBaseAttributes {
                    database: source.common_attr.database,
                    schema: source.common_attr.schema,
                    alias: source.identifier.clone(),
                    relation_name: source.relation_name,
                    materialized: DbtMaterialization::Table,
                    static_analysis: StaticAnalysisKind::On,
                    enabled: source.config.enabled.unwrap_or(true),
                    extended_model: false,
                    quoting: source
                        .quoting
                        .map(|mut quoting| {
                            quoting.default_to(&dbt_quoting);
                            quoting
                        })
                        .unwrap_or(dbt_quoting)
                        .try_into()
                        .expect("DbtQuoting should be set"),
                    columns: source.columns,
                    depends_on: NodeDependsOn::default(),
                    refs: vec![],
                    sources: vec![],
                    metrics: vec![],
                },
                source_attr: DbtSourceAttr {
                    identifier: source.identifier,
                    source_name: source.source_name,
                    source_description: source.source_description,
                    loader: source.loader,
                    loaded_at_field: source.loaded_at_field,
                    loaded_at_query: source.loaded_at_query,
                    freshness: source.freshness,
                },
                deprecated_config: source.config,
                other: source.other,
            }),
        );
    }
    for (unique_id, unit_test) in manifest.unit_tests {
        nodes.unit_tests.insert(
            unique_id,
            Arc::new(DbtUnitTest {
                common_attr: CommonAttributes {
                    unique_id: unit_test.common_attr.unique_id,
                    name: unit_test.common_attr.name,
                    package_name: unit_test.common_attr.package_name,
                    path: unit_test.common_attr.path,
                    original_file_path: unit_test.common_attr.original_file_path,
                    patch_path: unit_test.common_attr.patch_path,
                    fqn: unit_test.common_attr.fqn,
                    description: unit_test.common_attr.description,
                    raw_code: unit_test.base_attr.raw_code,
                    checksum: unit_test.base_attr.checksum,
                    language: unit_test.base_attr.language,
                    tags: unit_test
                        .config
                        .tags
                        .clone()
                        .map(|tags| tags.into())
                        .unwrap_or_default(),
                    meta: unit_test.config.meta.clone().unwrap_or_default(),
                },
                base_attr: NodeBaseAttributes {
                    database: unit_test.common_attr.database,
                    schema: unit_test.common_attr.schema,
                    alias: unit_test.base_attr.alias,
                    relation_name: unit_test.base_attr.relation_name,
                    materialized: DbtMaterialization::Table,
                    static_analysis: StaticAnalysisKind::On,
                    quoting: dbt_quoting.try_into().expect("DbtQuoting should be set"),
                    enabled: unit_test.config.enabled.unwrap_or(true),
                    extended_model: false,
                    columns: unit_test.base_attr.columns,
                    depends_on: unit_test.base_attr.depends_on,
                    refs: unit_test.base_attr.refs,
                    sources: unit_test.base_attr.sources,
                    metrics: unit_test.base_attr.metrics,
                },
                unit_test_attr: DbtUnitTestAttr {
                    model: unit_test.model,
                    given: unit_test.given,
                    expect: unit_test.expect,
                    versions: unit_test.versions,
                    version: unit_test.version,
                    overrides: unit_test.overrides,
                },
                deprecated_config: unit_test.config,
            }),
        );
    }

    nodes
}
