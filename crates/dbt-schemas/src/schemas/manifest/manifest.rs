use chrono::{DateTime, Utc};
use dbt_common::{Span, adapter::AdapterType, io_args::StaticAnalysisKind};
use dbt_serde_yaml::{Spanned, UntaggedEnumDeserialize};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, str::FromStr as _, sync::Arc};
// Type aliases for clarity
type YmlValue = dbt_serde_yaml::Value;

use crate::{
    dbt_utils::get_dbt_schema_version,
    schemas::{
        CommonAttributes, DbtFunction, DbtFunctionAttr, DbtModel, DbtModelAttr, DbtSeed,
        DbtSnapshot, DbtSource, DbtTest, DbtUnitTest, DbtUnitTestAttr, IntrospectionKind,
        NodeBaseAttributes, Nodes, TimeSpine, TimeSpinePrimaryColumn,
        common::{
            Access, DbtChecksum, DbtMaterialization, DbtQuoting, NodeDependsOn, normalize_sql,
        },
        manifest::{
            ManifestExposure, ManifestGroup, ManifestSavedQuery, ManifestUnitTest,
            manifest_nodes::{
                ManifestAnalysis, ManifestDataTest, ManifestFunction, ManifestModel,
                ManifestOperation, ManifestSeed, ManifestSnapshot,
            },
            saved_query::DbtSavedQueryAttr,
            semantic_model::NodeRelation,
        },
        nodes::{
            AdapterAttr, DbtAnalysis, DbtAnalysisAttr, DbtGroup, DbtGroupAttr, DbtSeedAttr,
            DbtSnapshotAttr, DbtSourceAttr, DbtTestAttr,
        },
    },
    state::ResolverState,
};

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, UntaggedEnumDeserialize)]
#[serde(tag = "resource_type")]
#[serde(rename_all = "snake_case")]
pub enum DbtNode {
    Model(ManifestModel),
    Test(ManifestDataTest),
    Snapshot(ManifestSnapshot),
    Seed(ManifestSeed),
    Operation(ManifestOperation),
    Analysis(ManifestAnalysis),
    Function(ManifestFunction),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct ManifestMetadata {
    pub __base__: BaseMetadata,
    #[serde(default)]
    pub project_name: String,
    /// The MD5 hash of the project name.
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
        self.__base__.env == other.__base__.env
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

// Re-export the current version (V12) as the default
pub use super::v12::DbtManifestV12;

// Type aliases for backwards compatibility
pub type DbtManifest = DbtManifestV12;

pub fn serialize_with_resource_type(mut value: YmlValue, resource_type: &str) -> YmlValue {
    if let YmlValue::Mapping(ref mut map, _) = value {
        map.insert(
            YmlValue::string("resource_type".to_string()),
            YmlValue::string(resource_type.to_string()),
        );
    }
    value
}

pub fn build_manifest(invocation_id: &str, resolver_state: &ResolverState) -> DbtManifest {
    let (parent_map, child_map) = build_parent_and_child_maps(&resolver_state.nodes);
    let group_map = build_group_map(&resolver_state.nodes);

    let disabled = build_disabled_map(resolver_state);
    DbtManifest {
        metadata: ManifestMetadata {
            __base__: BaseMetadata {
                dbt_schema_version: get_dbt_schema_version("manifest", 20),
                dbt_version: env!("CARGO_PKG_VERSION").to_string(),
                generated_at: Utc::now(),
                invocation_id: Some(invocation_id.to_string()),
                ..Default::default()
            },
            project_name: resolver_state.root_project_name.clone(),
            adapter_type: resolver_state
                .dbt_profile
                .db_config
                .adapter_type()
                .to_string(),
            project_id: Some(format!(
                "{:x}",
                md5::compute(resolver_state.root_project_name.as_bytes())
            )),
            ..Default::default()
        },
        nodes: resolver_state
            .nodes
            .models
            .iter()
            .map(|(id, node)| {
                (id.clone(), {
                    let mut model_node: ManifestModel = (**node).clone().into();

                    // External public models should not have a path or original_file_path
                    if model_node.access == Some(Access::Public)
                        && resolver_state.root_project_name
                            != model_node.__common_attr__.package_name
                    {
                        model_node.__common_attr__.path = PathBuf::new();
                        model_node.__common_attr__.original_file_path = PathBuf::new();
                    }
                    DbtNode::Model(model_node)
                })
            })
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
                    .analyses
                    .iter()
                    .map(|(id, node)| (id.clone(), DbtNode::Analysis((**node).clone().into()))),
            )
            // Note: Functions are now handled separately in the functions field, not in nodes
            .chain(resolver_state.operations.on_run_start.iter().map(|node| {
                (
                    node.__common_attr__.unique_id.clone(),
                    DbtNode::Operation((*node).clone().into_inner().into()),
                )
            }))
            .chain(resolver_state.operations.on_run_end.iter().map(|node| {
                (
                    node.__common_attr__.unique_id.clone(),
                    DbtNode::Operation((*node).clone().into_inner().into()),
                )
            }))
            .collect(),
        sources: resolver_state
            .nodes
            .sources
            .iter()
            .map(|(id, source)| (id.clone(), (**source).clone().into()))
            .collect(),
        exposures: resolver_state
            .nodes
            .exposures
            .iter()
            .map(|(id, exposure)| (id.clone(), (**exposure).clone().into()))
            .collect(),
        semantic_models: resolver_state
            .nodes
            .semantic_models
            .iter()
            .map(|(id, semantic_model)| (id.clone(), (**semantic_model).clone().into()))
            .collect(),
        metrics: resolver_state
            .nodes
            .metrics
            .iter()
            .map(|(id, metric)| (id.clone(), (**metric).clone().into()))
            .collect(),
        saved_queries: resolver_state
            .nodes
            .saved_queries
            .iter()
            .map(|(id, saved_query)| (id.clone(), (**saved_query).clone().into()))
            .collect(),
        unit_tests: resolver_state
            .nodes
            .unit_tests
            .iter()
            .map(|(id, unit_test)| (id.clone(), (**unit_test).clone().into()))
            .collect(),
        macros: resolver_state.macros.macros.clone(),
        functions: resolver_state
            .nodes
            .functions
            .iter()
            .map(|(id, function)| (id.clone(), (**function).clone().into()))
            .collect(),
        groups: resolver_state
            .nodes
            .groups
            .iter()
            .map(|(id, group)| (id.clone(), (**group).clone().into()))
            .collect(),
        selectors: resolver_state.manifest_selectors.clone(),
        docs: resolver_state.macros.docs_macros.clone(),
        parent_map,
        child_map,
        group_map,
        disabled,
    }
}

fn build_disabled_map(resolver_state: &ResolverState) -> BTreeMap<String, Vec<YmlValue>> {
    let disabled: BTreeMap<String, Vec<YmlValue>> = resolver_state
        .disabled_nodes
        .models
        .iter()
        .map(|(id, model)| {
            (
                id.clone(),
                vec![
                    dbt_serde_yaml::to_value(ManifestModel::from((**model).clone()))
                        .unwrap_or_default(),
                ],
            )
        })
        .chain(
            resolver_state
                .disabled_nodes
                .tests
                .iter()
                .map(|(id, test)| {
                    (
                        id.clone(),
                        vec![
                            dbt_serde_yaml::to_value(ManifestDataTest::from((**test).clone()))
                                .unwrap_or_default(),
                        ],
                    )
                }),
        )
        .chain(
            resolver_state
                .disabled_nodes
                .snapshots
                .iter()
                .map(|(id, snapshot)| {
                    (
                        id.clone(),
                        vec![
                            dbt_serde_yaml::to_value(ManifestSnapshot::from((**snapshot).clone()))
                                .unwrap_or_default(),
                        ],
                    )
                }),
        )
        .chain(
            resolver_state
                .disabled_nodes
                .seeds
                .iter()
                .map(|(id, seed)| {
                    (
                        id.clone(),
                        vec![
                            dbt_serde_yaml::to_value(ManifestSeed::from((**seed).clone()))
                                .unwrap_or_default(),
                        ],
                    )
                }),
        )
        .chain(
            resolver_state
                .disabled_nodes
                .analyses
                .iter()
                .map(|(id, analysis)| {
                    (
                        id.clone(),
                        vec![
                            dbt_serde_yaml::to_value(ManifestAnalysis::from((**analysis).clone()))
                                .unwrap_or_default(),
                        ],
                    )
                }),
        )
        .chain(
            resolver_state
                .disabled_nodes
                .functions
                .iter()
                .map(|(id, function)| {
                    (
                        id.clone(),
                        vec![
                            dbt_serde_yaml::to_value(ManifestFunction::from((**function).clone()))
                                .unwrap_or_default(),
                        ],
                    )
                }),
        )
        .chain(
            resolver_state
                .disabled_nodes
                .exposures
                .iter()
                .map(|(id, exposure)| {
                    (
                        id.clone(),
                        vec![
                            dbt_serde_yaml::to_value(ManifestExposure::from((**exposure).clone()))
                                .unwrap_or_default(),
                        ],
                    )
                }),
        )
        .chain(
            resolver_state
                .disabled_nodes
                .saved_queries
                .iter()
                .map(|(id, saved_query)| {
                    (
                        id.clone(),
                        vec![
                            dbt_serde_yaml::to_value(ManifestSavedQuery::from(
                                (**saved_query).clone(),
                            ))
                            .unwrap_or_default(),
                        ],
                    )
                }),
        )
        .chain(
            resolver_state
                .disabled_nodes
                .unit_tests
                .iter()
                .map(|(id, unit_test)| {
                    (
                        id.clone(),
                        vec![
                            dbt_serde_yaml::to_value(ManifestUnitTest::from((**unit_test).clone()))
                                .unwrap_or_default(),
                        ],
                    )
                }),
        )
        .chain(
            resolver_state
                .disabled_nodes
                .groups
                .iter()
                .map(|(id, group)| {
                    (
                        id.clone(),
                        vec![
                            dbt_serde_yaml::to_value(ManifestGroup::from((**group).clone()))
                                .unwrap_or_default(),
                        ],
                    )
                }),
        )
        //.chain(resolver_state.disabled_nodes.metrics.iter().map(|(id, metric)| (id.clone(), vec![dbt_serde_yaml::to_value(ManifestMetric::from((**metric).clone())).unwrap_or_default()])))
        //.chain(resolver_state.disabled_nodes.semantic_models.iter().map(|(id, semantic_model)| (id.clone(), vec![dbt_serde_yaml::to_value(ManifestSemanticModel::from((**semantic_model).clone())).unwrap_or_default()])))
        .collect();
    disabled
}

// Build map of group names to nodes in the group
fn build_group_map(nodes: &Nodes) -> BTreeMap<String, Vec<String>> {
    let mut group_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, model) in &nodes.models {
        if let Some(group) = &model.__model_attr__.group {
            group_map.entry(group.clone()).or_default().push(id.clone());
        }
    }
    for (id, semantic_model) in &nodes.semantic_models {
        if let Some(group) = &semantic_model.__semantic_model_attr__.group {
            group_map.entry(group.clone()).or_default().push(id.clone());
        }
    }
    for (id, metric) in &nodes.metrics {
        if let Some(group) = &metric.__metric_attr__.group {
            group_map.entry(group.clone()).or_default().push(id.clone());
        }
    }
    for (id, saved_query) in &nodes.saved_queries {
        if let Some(group) = &saved_query.__saved_query_attr__.group {
            group_map.entry(group.clone()).or_default().push(id.clone());
        }
    }
    group_map
}

/// Build parent and child dependency maps from the nodes.
/// Returns a tuple of (parent_map, child_map) where:
/// - parent_map: maps each node ID to a list of node IDs it depends on
/// - child_map: maps each node ID to a list of node IDs that depend on it
fn build_parent_and_child_maps(
    nodes: &Nodes,
) -> (BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<String>>) {
    let mut parent_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut child_map: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // Collect all nodes with their dependencies
    let mut all_nodes: Vec<(String, NodeDependsOn)> = Vec::new();

    for (id, model) in &nodes.models {
        all_nodes.push((id.clone(), model.__base_attr__.depends_on.clone()));
    }

    for (id, test) in &nodes.tests {
        all_nodes.push((id.clone(), test.__base_attr__.depends_on.clone()));
    }

    for (id, seed) in &nodes.seeds {
        all_nodes.push((id.clone(), seed.__base_attr__.depends_on.clone()));
    }

    for (id, snapshot) in &nodes.snapshots {
        all_nodes.push((id.clone(), snapshot.__base_attr__.depends_on.clone()));
    }

    for (id, analysis) in &nodes.analyses {
        all_nodes.push((id.clone(), analysis.__base_attr__.depends_on.clone()));
    }

    for (id, exposure) in &nodes.exposures {
        all_nodes.push((id.clone(), exposure.__base_attr__.depends_on.clone()));
    }

    for (id, unit_test) in &nodes.unit_tests {
        all_nodes.push((id.clone(), unit_test.__base_attr__.depends_on.clone()));
    }

    for (id, semantic_model) in &nodes.semantic_models {
        all_nodes.push((id.clone(), semantic_model.__base_attr__.depends_on.clone()));
    }

    for (id, metric) in &nodes.metrics {
        all_nodes.push((id.clone(), metric.__base_attr__.depends_on.clone()));
    }

    for (id, saved_query) in &nodes.saved_queries {
        all_nodes.push((id.clone(), saved_query.__base_attr__.depends_on.clone()));
    }

    for (id, function) in &nodes.functions {
        all_nodes.push((id.clone(), function.__base_attr__.depends_on.clone()));
    }

    // Process all collected nodes
    for (node_id, depends_on) in all_nodes {
        // Initialize parent list for this node
        parent_map.entry(node_id.clone()).or_default();

        // Add parents and update child map
        for parent_id in &depends_on.nodes {
            // Add parent to this node's parent list
            parent_map
                .entry(node_id.clone())
                .or_default()
                .push(parent_id.clone());

            // Add this node as a child of the parent
            child_map
                .entry(parent_id.clone())
                .or_default()
                .push(node_id.clone());
        }
    }

    // Process sources (they typically don't have dependencies but can have children)
    for id in nodes.sources.keys() {
        // Sources usually don't depend on anything, but we ensure they exist in maps
        parent_map.entry(id.clone()).or_default();
        child_map.entry(id.clone()).or_default();
    }

    // Ensure all nodes that are referenced but don't have their own entry exist in the maps
    // This handles cases where a node is referenced as a parent but isn't in our nodes
    let all_parent_ids: Vec<String> = parent_map
        .values()
        .flat_map(|parents| parents.clone())
        .collect();

    for parent_id in all_parent_ids {
        parent_map.entry(parent_id.clone()).or_default();
        child_map.entry(parent_id).or_default();
    }

    (parent_map, child_map)
}

pub fn nodes_from_dbt_manifest(manifest: DbtManifest, dbt_quoting: DbtQuoting) -> Nodes {
    let mut nodes = Nodes::default();
    // Do not put disabled nodes into the nodes, because all things in Nodes object should be enabled.
    for (unique_id, node) in manifest.nodes.clone() {
        match node {
            DbtNode::Model(model) => {
                nodes.models.insert(
                    unique_id,
                    Arc::new(manifest_model_to_dbt_model(model, &manifest, dbt_quoting)),
                );
            }
            DbtNode::Test(test) => {
                nodes.tests.insert(
                    unique_id,
                    Arc::new(DbtTest {
                        // TODO: persist the line/column info through the manifest as well
                        defined_at: Some(test.__common_attr__.original_file_path.clone().into()),

                        manifest_original_file_path: test
                            .__common_attr__
                            .original_file_path
                            .clone(),

                        __common_attr__: CommonAttributes {
                            unique_id: test.__common_attr__.unique_id,
                            name: test.__common_attr__.name,
                            package_name: test.__common_attr__.package_name,
                            path: test.__common_attr__.path,
                            name_span: Span::default(),

                            original_file_path: test.generated_sql_file.map_or_else(
                                // Note: for fusion generated manifests, the
                                // `generated_sql_file` field should really never be
                                // None (see [ManifestDataTest])
                                || test.__common_attr__.original_file_path.clone(),
                                PathBuf::from,
                            ),
                            patch_path: test.__common_attr__.patch_path,

                            fqn: test.__common_attr__.fqn,
                            description: test.__common_attr__.description,
                            raw_code: test.__base_attr__.raw_code,
                            checksum: test.__base_attr__.checksum,
                            language: test.__base_attr__.language,
                            tags: test
                                .config
                                .tags
                                .clone()
                                .map(|tags| tags.into())
                                .unwrap_or_default(),
                            meta: test.config.meta.clone().unwrap_or_default(),
                        },
                        __base_attr__: NodeBaseAttributes {
                            database: test.__common_attr__.database,
                            schema: test.__common_attr__.schema,
                            alias: test.__base_attr__.alias,
                            relation_name: test.__base_attr__.relation_name,
                            materialized: DbtMaterialization::Test,
                            static_analysis: StaticAnalysisKind::On.into(),
                            static_analysis_off_reason: None,
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
                            quoting_ignore_case: false,
                            persist_docs: None,
                            columns: test.__base_attr__.columns,
                            depends_on: test.__base_attr__.depends_on,
                            refs: test.__base_attr__.refs,
                            sources: test.__base_attr__.sources,
                            functions: test.__base_attr__.functions,
                            metrics: test.__base_attr__.metrics,
                        },
                        __test_attr__: DbtTestAttr {
                            column_name: test.column_name,
                            attached_node: test.attached_node,
                            test_metadata: test.test_metadata,
                            file_key_name: test.file_key_name,
                            introspection: IntrospectionKind::None,
                        },
                        deprecated_config: test.config,
                        __other__: test.__other__,
                    }),
                );
            }
            DbtNode::Snapshot(snapshot) => {
                // Recalculate checksum that eliminates whitespace and case differences.
                let normalized_raw_code = snapshot
                    .__base_attr__
                    .raw_code
                    .clone()
                    .map(|raw_code| normalize_sql(&raw_code));
                let recalculated_checksum = recalculate_checksum(
                    normalized_raw_code.as_deref(),
                    snapshot.__base_attr__.checksum.clone(),
                );

                nodes.snapshots.insert(
                    unique_id,
                    Arc::new(DbtSnapshot {
                        __common_attr__: CommonAttributes {
                            unique_id: snapshot.__common_attr__.unique_id,
                            name: snapshot.__common_attr__.name,
                            package_name: snapshot.__common_attr__.package_name,
                            path: snapshot.__common_attr__.path,
                            name_span: Span::default(),
                            original_file_path: snapshot.__common_attr__.original_file_path,
                            patch_path: snapshot.__common_attr__.patch_path,
                            fqn: snapshot.__common_attr__.fqn,
                            description: snapshot.__common_attr__.description,
                            raw_code: snapshot.__base_attr__.raw_code,
                            checksum: recalculated_checksum,
                            language: snapshot.__base_attr__.language,
                            tags: snapshot
                                .config
                                .tags
                                .clone()
                                .map(|tags| tags.into())
                                .unwrap_or_default(),
                            meta: snapshot.config.meta.clone().unwrap_or_default(),
                        },
                        __base_attr__: NodeBaseAttributes {
                            database: snapshot.__common_attr__.database,
                            schema: snapshot.__common_attr__.schema,
                            alias: snapshot.__base_attr__.alias,
                            relation_name: snapshot.__base_attr__.relation_name,
                            enabled: snapshot.config.enabled.unwrap_or(true),
                            extended_model: false,
                            materialized: snapshot
                                .config
                                .materialized
                                .clone()
                                .unwrap_or(DbtMaterialization::Table),
                            static_analysis: StaticAnalysisKind::On.into(),
                            static_analysis_off_reason: None,
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
                            quoting_ignore_case: false,
                            persist_docs: snapshot.config.persist_docs.clone(),
                            columns: snapshot.__base_attr__.columns,
                            depends_on: snapshot.__base_attr__.depends_on,
                            refs: snapshot.__base_attr__.refs,
                            sources: snapshot.__base_attr__.sources,
                            functions: snapshot.__base_attr__.functions,
                            metrics: snapshot.__base_attr__.metrics,
                        },
                        __snapshot_attr__: DbtSnapshotAttr {
                            snapshot_meta_column_names: snapshot
                                .config
                                .snapshot_meta_column_names
                                .clone()
                                .unwrap_or_default(),
                            introspection: IntrospectionKind::None,
                        },
                        __adapter_attr__: AdapterAttr::from_config_and_dialect(
                            &snapshot.config.__warehouse_specific_config__,
                            AdapterType::from_str(&manifest.metadata.adapter_type)
                                .expect("Unknown or unsupported adapter type"),
                        ),
                        deprecated_config: snapshot.config,
                        compiled: snapshot.__base_attr__.compiled,
                        compiled_code: snapshot.__base_attr__.compiled_code,
                        __other__: snapshot.__other__,
                    }),
                );
            }
            DbtNode::Seed(seed) => {
                nodes.seeds.insert(
                    unique_id,
                    Arc::new(DbtSeed {
                        __common_attr__: CommonAttributes {
                            unique_id: seed.__common_attr__.unique_id,
                            name: seed.__common_attr__.name,
                            package_name: seed.__common_attr__.package_name,
                            path: seed.__common_attr__.path,
                            name_span: Span::default(),
                            original_file_path: seed.__common_attr__.original_file_path,
                            patch_path: seed.__common_attr__.patch_path,
                            fqn: seed.__common_attr__.fqn,
                            description: seed.__common_attr__.description,
                            raw_code: seed.__base_attr__.raw_code,
                            checksum: seed.__base_attr__.checksum,
                            language: seed.__base_attr__.language,
                            tags: seed
                                .config
                                .tags
                                .clone()
                                .map(|tags| tags.into())
                                .unwrap_or_default(),
                            meta: seed.config.meta.clone().unwrap_or_default(),
                        },
                        __base_attr__: NodeBaseAttributes {
                            database: seed.__common_attr__.database,
                            schema: seed.__common_attr__.schema,
                            alias: seed.__base_attr__.alias,
                            relation_name: seed.__base_attr__.relation_name,
                            materialized: DbtMaterialization::Table,
                            static_analysis: StaticAnalysisKind::On.into(),
                            static_analysis_off_reason: None,
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
                            quoting_ignore_case: false,
                            extended_model: false,
                            persist_docs: seed.config.persist_docs.clone(),
                            columns: seed.__base_attr__.columns,
                            depends_on: seed.__base_attr__.depends_on,
                            refs: seed.__base_attr__.refs,
                            sources: seed.__base_attr__.sources,
                            functions: seed.__base_attr__.functions,
                            metrics: seed.__base_attr__.metrics,
                        },
                        __seed_attr__: DbtSeedAttr {
                            quote_columns: seed.config.quote_columns.unwrap_or_default(),
                            column_types: seed.config.column_types.clone(),
                            delimiter: seed.config.delimiter.clone().map(|d| d.into_inner()),
                            root_path: seed.root_path,
                        },
                        deprecated_config: seed.config.into(),
                        __other__: seed.__other__,
                    }),
                );
            }
            DbtNode::Operation(_) => {}
            DbtNode::Function(function) => {
                nodes.functions.insert(
                    unique_id,
                    Arc::new(DbtFunction {
                        __common_attr__: CommonAttributes {
                            unique_id: function.__common_attr__.unique_id,
                            name: function.__common_attr__.name,
                            package_name: function.__common_attr__.package_name,
                            path: function.__common_attr__.path,
                            name_span: Span::default(),
                            original_file_path: function.__common_attr__.original_file_path,
                            patch_path: function.__common_attr__.patch_path,
                            fqn: function.__common_attr__.fqn,
                            description: function.__common_attr__.description,
                            raw_code: None,
                            checksum: DbtChecksum::default(),
                            language: function.language.clone(),
                            tags: function
                                .config
                                .tags
                                .clone()
                                .map(|tags| tags.into())
                                .unwrap_or_default(),
                            meta: function.config.meta.clone().unwrap_or_default(),
                        },
                        __base_attr__: NodeBaseAttributes {
                            database: function.__common_attr__.database,
                            schema: function.__common_attr__.schema,
                            alias: function.__base_attr__.alias,
                            relation_name: function.__base_attr__.relation_name,
                            materialized: DbtMaterialization::Function,
                            static_analysis: StaticAnalysisKind::On.into(),
                            static_analysis_off_reason: None,
                            quoting: dbt_quoting.try_into().expect("DbtQuoting should be set"),
                            quoting_ignore_case: false,
                            enabled: function.config.enabled.unwrap_or(true),
                            extended_model: false,
                            persist_docs: None,
                            columns: vec![],
                            depends_on: function.__base_attr__.depends_on,
                            refs: function.__base_attr__.refs,
                            sources: function.__base_attr__.sources,
                            functions: function.__base_attr__.functions,
                            metrics: function.__base_attr__.metrics,
                        },
                        __function_attr__: DbtFunctionAttr {
                            access: function.access,
                            group: function.group,
                            language: function.language,
                            on_configuration_change: function.on_configuration_change,
                            returns: function.returns,
                            arguments: function.arguments,
                        },
                        deprecated_config: function.config,
                        __other__: function.__other__,
                    }),
                );
            }
            DbtNode::Analysis(analysis) => {
                let config = analysis.config;
                let tags = config
                    .tags
                    .clone()
                    .map(|tags| tags.into())
                    .unwrap_or_default();
                let meta = config.meta.clone().unwrap_or_default();
                let normalized_raw_code = analysis
                    .__base_attr__
                    .raw_code
                    .clone()
                    .map(|raw_code| normalize_sql(&raw_code));
                let recalculated_checksum = recalculate_checksum(
                    normalized_raw_code.as_deref(),
                    analysis.__base_attr__.checksum.clone(),
                );
                nodes.analyses.insert(
                    unique_id,
                    Arc::new(DbtAnalysis {
                        __common_attr__: CommonAttributes {
                            unique_id: analysis.__common_attr__.unique_id,
                            name: analysis.__common_attr__.name,
                            package_name: analysis.__common_attr__.package_name,
                            path: analysis.__common_attr__.path,
                            name_span: Span::default(),
                            original_file_path: analysis.__common_attr__.original_file_path,
                            patch_path: analysis.__common_attr__.patch_path,
                            fqn: analysis.__common_attr__.fqn,
                            description: analysis.__common_attr__.description,
                            raw_code: analysis.__base_attr__.raw_code,
                            checksum: recalculated_checksum,
                            language: analysis.__base_attr__.language,
                            tags,
                            meta,
                        },
                        __base_attr__: NodeBaseAttributes {
                            database: analysis.__common_attr__.database,
                            schema: analysis.__common_attr__.schema,
                            alias: analysis.__base_attr__.alias,
                            relation_name: analysis.__base_attr__.relation_name,
                            materialized: analysis.materialized,
                            static_analysis: Spanned::new(analysis.static_analysis),
                            enabled: analysis.enabled,
                            static_analysis_off_reason: None,
                            extended_model: false,
                            quoting: analysis
                                .quoting
                                .map(|mut quoting| {
                                    quoting.default_to(&dbt_quoting);
                                    quoting
                                })
                                .unwrap_or(dbt_quoting)
                                .try_into()
                                .expect("DbtQuoting should be set"),
                            quoting_ignore_case: analysis.quoting_ignore_case,
                            persist_docs: analysis.persist_docs.clone(),
                            columns: analysis.__base_attr__.columns,
                            depends_on: analysis.__base_attr__.depends_on,
                            refs: analysis.__base_attr__.refs,
                            sources: analysis.__base_attr__.sources,
                            metrics: analysis.__base_attr__.metrics,
                            functions: analysis.__base_attr__.functions,
                        },
                        __analysis_attr__: DbtAnalysisAttr::default(),
                        deprecated_config: config,
                        __other__: analysis.__other__,
                    }),
                );
            }
        }
    }
    for (unique_id, source) in manifest.sources {
        nodes.sources.insert(
            unique_id,
            Arc::new(DbtSource {
                __common_attr__: CommonAttributes {
                    unique_id: source.__common_attr__.unique_id,
                    name: source.__common_attr__.name,
                    package_name: source.__common_attr__.package_name,
                    path: source.__common_attr__.path,
                    name_span: Span::default(),
                    original_file_path: source.__common_attr__.original_file_path,
                    patch_path: source.__common_attr__.patch_path,
                    fqn: source.__common_attr__.fqn,
                    description: source.__common_attr__.description,
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
                __base_attr__: NodeBaseAttributes {
                    database: source.__common_attr__.database,
                    schema: source.__common_attr__.schema,
                    alias: source.identifier.clone(),
                    relation_name: source.relation_name,
                    materialized: DbtMaterialization::Table,
                    static_analysis: StaticAnalysisKind::On.into(),
                    static_analysis_off_reason: None,
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
                    quoting_ignore_case: false,
                    persist_docs: None,
                    columns: source.columns,
                    depends_on: NodeDependsOn::default(),
                    refs: vec![],
                    sources: vec![],
                    functions: vec![],
                    metrics: vec![],
                },
                __source_attr__: DbtSourceAttr {
                    identifier: source.identifier,
                    source_name: source.source_name,
                    source_description: source.source_description,
                    loader: source.loader,
                    loaded_at_field: source.loaded_at_field,
                    loaded_at_query: source.loaded_at_query,
                    freshness: source.freshness,
                },
                deprecated_config: source.config,
                __other__: source.__other__,
            }),
        );
    }
    for (unique_id, exposure) in manifest.exposures {
        nodes.exposures.insert(
            unique_id,
            Arc::new(crate::schemas::nodes::DbtExposure {
                __common_attr__: CommonAttributes {
                    name: exposure.__common_attr__.name,
                    package_name: exposure.__common_attr__.package_name,
                    path: exposure.__common_attr__.path,
                    name_span: Span::default(),
                    original_file_path: exposure.__common_attr__.original_file_path,
                    patch_path: None,
                    unique_id: exposure.__common_attr__.unique_id,
                    fqn: exposure.__common_attr__.fqn,
                    description: exposure.__common_attr__.description,
                    checksum: Default::default(),
                    language: None,
                    raw_code: None,
                    tags: vec![],
                    meta: BTreeMap::new(),
                },
                __base_attr__: NodeBaseAttributes {
                    database: "".to_string(),
                    schema: "".to_string(),
                    alias: "".to_string(),
                    relation_name: None,
                    quoting: Default::default(),
                    materialized: Default::default(),
                    static_analysis: Default::default(),
                    static_analysis_off_reason: None,
                    enabled: true,
                    extended_model: false,
                    persist_docs: None,
                    columns: vec![],
                    refs: exposure.__base_attr__.refs,
                    sources: exposure.__base_attr__.sources,
                    functions: vec![],
                    metrics: exposure.__base_attr__.metrics,
                    depends_on: exposure.__base_attr__.depends_on,
                    quoting_ignore_case: false,
                },
                __exposure_attr__: crate::schemas::nodes::DbtExposureAttr {
                    owner: exposure.owner,
                    label: exposure.label,
                    maturity: exposure.maturity,
                    type_: exposure.type_,
                    url: exposure.url,
                    unrendered_config: exposure.__base_attr__.unrendered_config,
                    created_at: exposure.__base_attr__.created_at,
                },
                deprecated_config: exposure.config,
            }),
        );
    }
    for (unique_id, unit_test) in manifest.unit_tests {
        nodes.unit_tests.insert(
            unique_id,
            Arc::new(DbtUnitTest {
                __common_attr__: CommonAttributes {
                    unique_id: unit_test.__common_attr__.unique_id,
                    name: unit_test.__common_attr__.name,
                    package_name: unit_test.__common_attr__.package_name,
                    path: unit_test.__common_attr__.path,
                    name_span: Span::default(),
                    original_file_path: unit_test.__common_attr__.original_file_path,
                    patch_path: unit_test.__common_attr__.patch_path,
                    fqn: unit_test.__common_attr__.fqn,
                    description: unit_test.__common_attr__.description,
                    raw_code: unit_test.__base_attr__.raw_code,
                    checksum: unit_test.__base_attr__.checksum,
                    language: unit_test.__base_attr__.language,
                    tags: unit_test
                        .config
                        .tags
                        .clone()
                        .map(|tags| tags.into())
                        .unwrap_or_default(),
                    meta: unit_test.config.meta.clone().unwrap_or_default(),
                },
                __base_attr__: NodeBaseAttributes {
                    database: unit_test.__common_attr__.database,
                    schema: unit_test.__common_attr__.schema,
                    alias: unit_test.__base_attr__.alias,
                    relation_name: unit_test.__base_attr__.relation_name,
                    materialized: DbtMaterialization::Table,
                    static_analysis: StaticAnalysisKind::On.into(),
                    static_analysis_off_reason: None,
                    quoting: dbt_quoting.try_into().expect("DbtQuoting should be set"),
                    quoting_ignore_case: false,
                    enabled: unit_test.config.enabled.unwrap_or(true),
                    extended_model: false,
                    persist_docs: None,
                    columns: unit_test.__base_attr__.columns,
                    depends_on: unit_test.__base_attr__.depends_on,
                    refs: unit_test.__base_attr__.refs,
                    sources: unit_test.__base_attr__.sources,
                    functions: unit_test.__base_attr__.functions,
                    metrics: unit_test.__base_attr__.metrics,
                },
                __unit_test_attr__: DbtUnitTestAttr {
                    model: unit_test.model,
                    given: unit_test.given,
                    expect: unit_test.expect,
                    versions: unit_test.versions,
                    version: unit_test.version,
                    overrides: unit_test.overrides,
                },
                field_event_status: unit_test.field_event_status,
                field_pre_injected_sql: unit_test.field_pre_injected_sql,
                tested_node_unique_id: unit_test.tested_node_unique_id,
                this_input_node_unique_id: unit_test.this_input_node_unique_id,
                deprecated_config: unit_test.config,
            }),
        );
    }
    for (_unique_id, _semantic_model) in manifest.semantic_models {
        // TODO: insert DbtSemanticModel into node.semantic_models
    }
    for (_unique_id, _metric) in manifest.metrics {
        // TODO: insert DbtMetric into node.metrics
    }
    for (unique_id, saved_query) in manifest.saved_queries {
        nodes.saved_queries.insert(
            unique_id,
            Arc::new(crate::schemas::manifest::DbtSavedQuery {
                __common_attr__: CommonAttributes {
                    unique_id: saved_query.__common_attr__.unique_id,
                    name: saved_query.__common_attr__.name,
                    package_name: saved_query.__common_attr__.package_name,
                    path: saved_query.__common_attr__.path,
                    original_file_path: saved_query.__common_attr__.original_file_path,
                    patch_path: None, // TODO: Add to ManifestSavedQueryCommonAttributes if needed
                    fqn: saved_query.__common_attr__.fqn,
                    description: saved_query.__common_attr__.description,
                    raw_code: None,
                    checksum: DbtChecksum::default(),
                    name_span: Span::default(),
                    language: None,
                    tags: saved_query
                        .config
                        .tags
                        .clone()
                        .map(|tags| tags.into())
                        .unwrap_or_default(),
                    meta: saved_query.config.meta.clone().unwrap_or_default(),
                },
                __base_attr__: NodeBaseAttributes {
                    database: "".to_string(),
                    schema: "".to_string(),
                    alias: "".to_string(),
                    relation_name: None,
                    quoting: Default::default(),
                    materialized: Default::default(),
                    static_analysis: Default::default(),
                    static_analysis_off_reason: None,
                    enabled: true,
                    extended_model: false,
                    persist_docs: None,
                    columns: Default::default(),
                    refs: saved_query.__base_attr__.refs,
                    sources: vec![],
                    functions: vec![],
                    metrics: vec![],
                    depends_on: saved_query.__base_attr__.depends_on,
                    quoting_ignore_case: false,
                },
                __saved_query_attr__: DbtSavedQueryAttr {
                    query_params: saved_query.query_params,
                    exports: saved_query.exports,
                    label: saved_query.label,
                    metadata: saved_query.metadata,
                    unrendered_config: saved_query.__base_attr__.unrendered_config,
                    created_at: saved_query.__base_attr__.created_at,
                    group: saved_query.group,
                },
                deprecated_config: saved_query.config,
                __other__: saved_query.__other__,
            }),
        );
    }
    for (unique_id, group) in manifest.groups {
        nodes.groups.insert(
            unique_id.clone(),
            Arc::new(DbtGroup {
                __common_attr__: CommonAttributes {
                    name: group.name.to_string(),
                    package_name: group.package_name.to_string(),
                    path: group.path.clone(),
                    name_span: Span::default(),
                    original_file_path: group.original_file_path.clone(),
                    unique_id: unique_id.clone(),
                    fqn: vec![],
                    description: Some(group.description.unwrap_or_default()),
                    patch_path: None,
                    checksum: Default::default(),
                    language: None,
                    raw_code: None,
                    tags: vec![],
                    meta: BTreeMap::new(),
                },
                __base_attr__: NodeBaseAttributes {
                    database: "".to_string(),
                    schema: "".to_string(),
                    alias: "".to_string(),
                    relation_name: None,
                    quoting: Default::default(),
                    materialized: Default::default(),
                    static_analysis: Default::default(),
                    static_analysis_off_reason: None,
                    enabled: true,
                    extended_model: false,
                    persist_docs: None,
                    columns: vec![],
                    depends_on: NodeDependsOn::default(),
                    quoting_ignore_case: false,
                    refs: vec![],
                    sources: vec![],
                    functions: vec![],
                    metrics: vec![],
                },
                __group_attr__: DbtGroupAttr { owner: group.owner },
            }),
        );
    }

    // Process functions from the separate manifest.functions field
    for (unique_id, function) in manifest.functions {
        nodes.functions.insert(
            unique_id,
            Arc::new(DbtFunction {
                __common_attr__: CommonAttributes {
                    unique_id: function.__common_attr__.unique_id,
                    name: function.__common_attr__.name,
                    package_name: function.__common_attr__.package_name,
                    path: function.__common_attr__.path,
                    name_span: Span::default(),
                    original_file_path: function.__common_attr__.original_file_path,
                    patch_path: None,
                    fqn: function.__common_attr__.fqn,
                    description: function.__common_attr__.description,
                    raw_code: None,
                    checksum: DbtChecksum::default(),
                    language: function.language.clone(),
                    tags: function
                        .config
                        .tags
                        .clone()
                        .map(|tags| tags.into())
                        .unwrap_or_default(),
                    meta: function.config.meta.clone().unwrap_or_default(),
                },
                __base_attr__: NodeBaseAttributes {
                    database: String::new(),
                    schema: String::new(),
                    alias: function.__base_attr__.alias,
                    relation_name: function.__base_attr__.relation_name,
                    materialized: DbtMaterialization::View,
                    static_analysis: StaticAnalysisKind::On.into(),
                    static_analysis_off_reason: None,
                    enabled: function.config.enabled.unwrap_or(true),
                    extended_model: false,
                    quoting: function
                        .config
                        .quoting
                        .map(|mut quoting| {
                            quoting.default_to(&dbt_quoting);
                            quoting
                        })
                        .unwrap_or(dbt_quoting)
                        .try_into()
                        .expect("DbtQuoting should be set"),
                    quoting_ignore_case: false,
                    persist_docs: None,
                    columns: function.__base_attr__.columns,
                    depends_on: function.__base_attr__.depends_on,
                    refs: function.__base_attr__.refs,
                    sources: function.__base_attr__.sources,
                    functions: function.__base_attr__.functions,
                    metrics: vec![],
                },
                __function_attr__: DbtFunctionAttr {
                    access: function.access,
                    group: function.group,
                    language: function.language,
                    on_configuration_change: function.on_configuration_change,
                    returns: function.returns,
                    arguments: function.arguments,
                },
                deprecated_config: function.config,
                __other__: function.__other__,
            }),
        );
    }

    nodes
}

/// Convert a ManifestModel to a DbtModel.
/// Inverse of From<DbtModel> for ManifestModel.
pub fn manifest_model_to_dbt_model(
    model: ManifestModel,
    manifest: &DbtManifest,
    dbt_quoting: DbtQuoting,
) -> DbtModel {
    let database = model.__common_attr__.database;
    let schema = model.__common_attr__.schema;
    let alias = model.__base_attr__.alias;
    let relation_name = model.__base_attr__.relation_name;

    let node_relation = NodeRelation {
        database: Some(database.clone()),
        schema_name: schema.clone(),
        alias: alias.clone(),
        relation_name: relation_name.clone(),
    };

    let time_spine = model.time_spine.map(|ts| TimeSpine {
        node_relation,
        primary_column: TimeSpinePrimaryColumn {
            name: ts.standard_granularity_column,
            time_granularity: Default::default(), // TODO: hydrate time_granularity by looking up the column's granularity, not sure if available in manifest.
        },
        custom_granularities: ts.custom_granularities.unwrap_or_default(),
    });

    let normalized_raw_code = model
        .__base_attr__
        .raw_code
        .clone()
        .map(|raw_code| normalize_sql(&raw_code));
    let recalculated_checksum = recalculate_checksum(
        normalized_raw_code.as_deref(),
        model.__base_attr__.checksum.clone(),
    );

    DbtModel {
        __common_attr__: CommonAttributes {
            unique_id: model.__common_attr__.unique_id,
            name: model.__common_attr__.name,
            package_name: model.__common_attr__.package_name,
            path: model.__common_attr__.path,
            name_span: Span::default(),
            original_file_path: model.__common_attr__.original_file_path,
            patch_path: model.__common_attr__.patch_path,
            fqn: model.__common_attr__.fqn,
            description: model.__common_attr__.description,
            raw_code: model.__base_attr__.raw_code,
            checksum: recalculated_checksum,
            language: model.__base_attr__.language,
            tags: model
                .config
                .tags
                .clone()
                .map(|tags| tags.into())
                .unwrap_or_default(),
            meta: model.config.meta.clone().unwrap_or_default(),
        },
        __base_attr__: NodeBaseAttributes {
            database,
            schema,
            alias,
            relation_name,
            materialized: model
                .config
                .materialized
                .clone()
                .unwrap_or(DbtMaterialization::View),
            static_analysis: StaticAnalysisKind::On.into(),
            static_analysis_off_reason: None,
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
            quoting_ignore_case: false,
            persist_docs: model.config.persist_docs.clone(),
            columns: model.__base_attr__.columns,
            depends_on: model.__base_attr__.depends_on,
            refs: model.__base_attr__.refs,
            sources: model.__base_attr__.sources,
            functions: model.__base_attr__.functions,
            metrics: model.__base_attr__.metrics,
        },
        __model_attr__: DbtModelAttr {
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
            time_spine,
            event_time: model.config.event_time.clone(),
            catalog_name: model.config.catalog_name.clone(),
            table_format: model.config.table_format.clone(),
        },
        __adapter_attr__: AdapterAttr::from_config_and_dialect(
            &model.config.__warehouse_specific_config__,
            AdapterType::from_str(&manifest.metadata.adapter_type)
                .expect("Unknown or unsupported adapter type"),
        ),
        deprecated_config: model.config.into(),
        __other__: model.__other__,
    }
}

/// Recalculate checksum for a snapshot/model based on normalized raw code.
/// If the normalized code is the placeholder, use the original checksum.
/// Otherwise, hash the normalized code.
pub fn recalculate_checksum(
    normalized_raw_code: Option<&str>,
    original_checksum: DbtChecksum,
) -> DbtChecksum {
    match normalized_raw_code {
        Some("--placeholder--") => original_checksum,
        Some(code) => DbtChecksum::hash(code.as_bytes()),
        None => original_checksum,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::{CommonAttributes, Nodes};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn create_test_nodes() -> Nodes {
        Nodes {
            models: BTreeMap::new(),
            tests: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            analyses: BTreeMap::new(),
            seeds: BTreeMap::new(),
            exposures: BTreeMap::new(),
            sources: BTreeMap::new(),
            unit_tests: BTreeMap::new(),
            semantic_models: BTreeMap::new(),
            metrics: BTreeMap::new(),
            saved_queries: BTreeMap::new(),
            groups: BTreeMap::new(),
            functions: BTreeMap::new(),
        }
    }

    fn create_test_model(id: &str, depends_on: Vec<String>) -> Arc<DbtModel> {
        Arc::new(DbtModel {
            __common_attr__: CommonAttributes {
                unique_id: id.to_string(),
                name: id.split('.').next_back().unwrap_or(id).to_string(),
                package_name: "test".to_string(),
                ..Default::default()
            },
            __base_attr__: NodeBaseAttributes {
                database: "db".to_string(),
                schema: "schema".to_string(),
                depends_on: NodeDependsOn {
                    nodes: depends_on,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        })
    }

    #[test]
    fn test_build_parent_and_child_maps_empty_nodes() {
        let nodes = create_test_nodes();
        let (parent_map, child_map) = build_parent_and_child_maps(&nodes);

        assert!(parent_map.is_empty());
        assert!(child_map.is_empty());
    }

    #[test]
    fn test_build_parent_and_child_maps_single_model_no_deps() {
        let mut nodes = create_test_nodes();
        nodes.models.insert(
            "model.test.model_a".to_string(),
            create_test_model("model.test.model_a", vec![]),
        );

        let (parent_map, child_map) = build_parent_and_child_maps(&nodes);

        assert_eq!(parent_map.len(), 1);
        assert!(parent_map.contains_key("model.test.model_a"));
        assert_eq!(parent_map.get("model.test.model_a").unwrap().len(), 0);

        // child_map should be empty since no dependencies
        assert!(child_map.is_empty());
    }

    #[test]
    fn test_build_parent_and_child_maps_simple_dependency() {
        let mut nodes = create_test_nodes();

        nodes.models.insert(
            "model.test.model_a".to_string(),
            create_test_model("model.test.model_a", vec![]),
        );
        nodes.models.insert(
            "model.test.model_b".to_string(),
            create_test_model("model.test.model_b", vec!["model.test.model_a".to_string()]),
        );

        let (parent_map, child_map) = build_parent_and_child_maps(&nodes);

        // Check parent_map
        assert_eq!(parent_map.len(), 2);
        assert_eq!(parent_map.get("model.test.model_a").unwrap().len(), 0);
        assert_eq!(
            parent_map.get("model.test.model_b").unwrap(),
            &vec!["model.test.model_a".to_string()]
        );

        // Check child_map
        assert_eq!(child_map.len(), 1);
        assert_eq!(
            child_map.get("model.test.model_a").unwrap(),
            &vec!["model.test.model_b".to_string()]
        );
    }

    #[test]
    fn test_build_parent_and_child_maps_multiple_dependencies() {
        let mut nodes = create_test_nodes();

        nodes.models.insert(
            "model.test.model_a".to_string(),
            create_test_model("model.test.model_a", vec![]),
        );
        nodes.models.insert(
            "model.test.model_b".to_string(),
            create_test_model("model.test.model_b", vec![]),
        );
        nodes.models.insert(
            "model.test.model_c".to_string(),
            create_test_model(
                "model.test.model_c",
                vec![
                    "model.test.model_a".to_string(),
                    "model.test.model_b".to_string(),
                ],
            ),
        );

        let (parent_map, child_map) = build_parent_and_child_maps(&nodes);

        // Check parent_map
        assert_eq!(parent_map.len(), 3);
        assert_eq!(parent_map.get("model.test.model_a").unwrap().len(), 0);
        assert_eq!(parent_map.get("model.test.model_b").unwrap().len(), 0);
        assert_eq!(
            parent_map.get("model.test.model_c").unwrap(),
            &vec![
                "model.test.model_a".to_string(),
                "model.test.model_b".to_string()
            ]
        );

        // Check child_map
        assert_eq!(child_map.len(), 2);
        assert_eq!(
            child_map.get("model.test.model_a").unwrap(),
            &vec!["model.test.model_c".to_string()]
        );
        assert_eq!(
            child_map.get("model.test.model_b").unwrap(),
            &vec!["model.test.model_c".to_string()]
        );
    }

    #[test]
    fn test_build_parent_and_child_maps_chain_dependency() {
        let mut nodes = create_test_nodes();

        nodes.models.insert(
            "model.test.model_a".to_string(),
            create_test_model("model.test.model_a", vec![]),
        );
        nodes.models.insert(
            "model.test.model_b".to_string(),
            create_test_model("model.test.model_b", vec!["model.test.model_a".to_string()]),
        );
        nodes.models.insert(
            "model.test.model_c".to_string(),
            create_test_model("model.test.model_c", vec!["model.test.model_b".to_string()]),
        );

        let (parent_map, child_map) = build_parent_and_child_maps(&nodes);

        // Check parent_map
        assert_eq!(parent_map.len(), 3);
        assert_eq!(parent_map.get("model.test.model_a").unwrap().len(), 0);
        assert_eq!(
            parent_map.get("model.test.model_b").unwrap(),
            &vec!["model.test.model_a".to_string()]
        );
        assert_eq!(
            parent_map.get("model.test.model_c").unwrap(),
            &vec!["model.test.model_b".to_string()]
        );

        // Check child_map
        assert_eq!(child_map.len(), 2);
        assert_eq!(
            child_map.get("model.test.model_a").unwrap(),
            &vec!["model.test.model_b".to_string()]
        );
        assert_eq!(
            child_map.get("model.test.model_b").unwrap(),
            &vec!["model.test.model_c".to_string()]
        );
    }

    #[test]
    fn test_build_parent_and_child_maps_with_source() {
        let mut nodes = create_test_nodes();

        nodes.sources.insert(
            "source.test.my_source.table1".to_string(),
            Arc::new(DbtSource {
                __common_attr__: CommonAttributes {
                    unique_id: "source.test.my_source.table1".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );

        nodes.models.insert(
            "model.test.model_a".to_string(),
            create_test_model(
                "model.test.model_a",
                vec!["source.test.my_source.table1".to_string()],
            ),
        );

        let (parent_map, child_map) = build_parent_and_child_maps(&nodes);

        // Check parent_map
        assert_eq!(parent_map.len(), 2);
        assert_eq!(
            parent_map.get("model.test.model_a").unwrap(),
            &vec!["source.test.my_source.table1".to_string()]
        );
        assert_eq!(
            parent_map
                .get("source.test.my_source.table1")
                .unwrap()
                .len(),
            0
        );

        // Check child_map
        assert_eq!(child_map.len(), 1);
        assert_eq!(
            child_map.get("source.test.my_source.table1").unwrap(),
            &vec!["model.test.model_a".to_string()]
        );
    }

    #[test]
    fn test_build_parent_and_child_maps_missing_dependency() {
        let mut nodes = create_test_nodes();

        nodes.models.insert(
            "model.test.model_b".to_string(),
            create_test_model("model.test.model_b", vec!["model.test.model_a".to_string()]),
        );

        let (parent_map, child_map) = build_parent_and_child_maps(&nodes);

        // Both the existing model and the missing dependency should have entries
        assert_eq!(parent_map.len(), 2);
        assert_eq!(
            parent_map.get("model.test.model_b").unwrap(),
            &vec!["model.test.model_a".to_string()]
        );
        assert_eq!(parent_map.get("model.test.model_a").unwrap().len(), 0); // Missing node gets empty entry

        // Child map should track the relationship
        assert_eq!(child_map.len(), 1);
        assert_eq!(
            child_map.get("model.test.model_a").unwrap(),
            &vec!["model.test.model_b".to_string()]
        );
    }
}
