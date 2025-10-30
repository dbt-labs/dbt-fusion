use crate::args::ResolveArgs;
use crate::dbt_project_config::{RootProjectConfigs, init_project_config};
use crate::utils::{get_node_fqn, get_original_file_path, get_unique_id};

use dbt_common::FsResult;
use dbt_common::io_args::{StaticAnalysisKind, StaticAnalysisOffReason};
use dbt_jinja_utils::jinja_environment::JinjaEnv;
use dbt_jinja_utils::utils::dependency_package_name_from_ctx;
use dbt_schemas::schemas::common::{DbtChecksum, Dimension, DimensionTypeParams, NodeDependsOn};
use dbt_schemas::schemas::dbt_column::{
    ColumnPropertiesDimension, ColumnPropertiesDimensionConfig, Entity, EntityConfig,
};
use dbt_schemas::schemas::manifest::metric::{MeasureAggregationParameters, NonAdditiveDimension};
use dbt_schemas::schemas::manifest::semantic_model::{
    DbtSemanticModel, DbtSemanticModelAttr, NodeRelation, SemanticEntity, SemanticMeasure,
    SemanticModelDefaults,
};
use dbt_schemas::schemas::project::{DefaultTo, ModelConfig, SemanticModelConfig};
use dbt_schemas::schemas::properties::ModelProperties;
use dbt_schemas::schemas::properties::metrics_properties::{AggregationType, PercentileType};
use dbt_schemas::schemas::ref_and_source::DbtRef;
use dbt_schemas::schemas::semantic_layer::semantic_manifest::SemanticLayerElementConfig;
use dbt_schemas::schemas::{
    CommonAttributes, DbtModel, InternalDbtNodeAttributes, NodeBaseAttributes,
};
use dbt_schemas::state::DbtPackage;
use minijinja::value::Value as MinijinjaValue;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::resolve_properties::MinimalPropertiesEntry;

/// Helper to compute the effective semantic model config for a given semantic model
fn get_effective_semantic_model_config(
    semantic_model_fqn: &[String],
    root_project_configs: &RootProjectConfigs,
    resource_config: &SemanticModelConfig,
    model_props: &ModelProperties,
) -> SemanticModelConfig {
    let mut project_config = root_project_configs
        .semantic_models
        .get_config_for_fqn(semantic_model_fqn)
        .clone();
    project_config.default_to(resource_config);

    if let Some(config) = &model_props.semantic_model {
        let mut final_config = config.clone();
        final_config.default_to(&project_config);
        SemanticModelConfig {
            enabled: Some(final_config.enabled),
            group: final_config.group,
            meta: final_config.config.unwrap_or_default().meta,
            tags: project_config.tags,
        }
    } else {
        project_config
    }
}

#[allow(clippy::too_many_arguments, clippy::expect_fun_call)]
pub async fn resolve_semantic_models(
    args: &ResolveArgs,
    package: &DbtPackage,
    root_project_configs: &RootProjectConfigs,
    minimal_model_properties: &mut BTreeMap<String, MinimalPropertiesEntry>,
    typed_models_properties: &BTreeMap<String, ModelProperties>,
    resolved_models: BTreeMap<String, Arc<DbtModel>>,
    package_name: &str,
    env: &JinjaEnv,
    base_ctx: &BTreeMap<String, MinijinjaValue>,
) -> FsResult<(
    HashMap<String, Arc<DbtSemanticModel>>,
    HashMap<String, Arc<DbtSemanticModel>>,
)> {
    let mut semantic_models: HashMap<String, Arc<DbtSemanticModel>> = HashMap::new();
    let mut disabled_semantic_models: HashMap<String, Arc<DbtSemanticModel>> = HashMap::new();

    if minimal_model_properties.is_empty() {
        return Ok((semantic_models, disabled_semantic_models));
    }

    // TODO: what is the difference between 'package_name' and 'dependency_package_name'?
    let dependency_package_name = dependency_package_name_from_ctx(env, base_ctx);
    let _local_model_project_config = init_project_config(
        &args.io,
        &package.dbt_project.models,
        ModelConfig {
            enabled: Some(true),
            ..Default::default()
        },
        dependency_package_name,
    )?;
    let local_semantic_model_project_config = init_project_config(
        &args.io,
        &package.dbt_project.semantic_models,
        SemanticModelConfig {
            enabled: Some(false),
            ..Default::default()
        },
        dependency_package_name,
    )?;

    for (model_name, model_props) in typed_models_properties.iter() {
        // TODO: Do we need to validate semantic_model like how we validate
        // exposure names to only contain letters, numbers, and underscores?

        if model_props.semantic_model.is_none() {
            continue;
        }
        if !model_props.semantic_model.clone().unwrap().enabled {
            continue;
        }

        let mpe = minimal_model_properties
            .get(model_name)
            .unwrap_or_else(|| panic!("ModelPropertiesEntry must exist for model '{model_name}'"));

        // TODO: These are reused from resolve_models, can probably refactor to implement methods in MinimalPropertiesEntry
        let model_maybe_version = mpe.version_info.as_ref().map(|v| v.version.clone());
        let model_unique_id = get_unique_id(model_name, package_name, model_maybe_version, "model");

        // TODO: should we be panicking if model cannot be found?
        // This would for example happen if you declare model yaml properties but not the sql itself
        // or should we just silently skip hydrating the fields that depend on resolved_model?
        let resolved_model = resolved_models
            .get(&model_unique_id)
            .unwrap_or_else(|| panic!("Cannot find resolved model '{model_unique_id}'"));

        // TODO: semantic_model_name may not always be equal to model_name in the future
        // TODO: if the underlying model has versions, which version is the semantic_model tied to?
        let semantic_model_name = model_props
            .semantic_model
            .clone()
            .unwrap()
            .name
            .unwrap_or_else(|| model_name.clone());
        let semantic_model_unique_id =
            get_unique_id(&semantic_model_name, package_name, None, "semantic_model");
        let semantic_model_fqn = get_node_fqn(
            package_name,
            mpe.relative_path.clone(),
            vec![semantic_model_name.to_owned()],
            &package.dbt_project.all_source_paths(),
        );

        // Get combined config from project config and semantic_model config
        let semantic_model_resource_config =
            local_semantic_model_project_config.get_config_for_fqn(&semantic_model_fqn);
        let semantic_model_config = get_effective_semantic_model_config(
            &semantic_model_fqn,
            root_project_configs,
            semantic_model_resource_config,
            model_props,
        );

        let measures: Vec<SemanticMeasure> = model_props
            .metrics
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|metric| {
                SemanticMeasure {
                    name: metric.name.clone(),
                    expr: metric.expr.clone().map(String::from),
                    description: metric.description.clone(),
                    label: metric.label.clone(),
                    config: metric.config.as_ref().map(|c| SemanticLayerElementConfig {
                        meta: c.meta.clone(),
                    }),
                    agg: metric.agg.clone().unwrap_or(AggregationType::Sum), // FIXME: if metric.agg is optional what should it default to?
                    create_metric: Some(true), // TODO: confirm this: "old spec can declare measure without creating metric, but since this is a metric, always true"
                    agg_params: if metric.percentile.is_some() {
                        Some(MeasureAggregationParameters {
                            percentile: metric.percentile,
                            use_discrete_percentile: metric
                                .percentile_type
                                .as_ref()
                                .map(|pt| matches!(pt, PercentileType::Discrete)),
                            // TODO: confirm approximate == continuous percentile
                            use_approximate_percentile: metric
                                .percentile_type
                                .as_ref()
                                .map(|pt| matches!(pt, PercentileType::Continuous)),
                        })
                    } else {
                        None
                    },
                    non_additive_dimension: metric
                        .non_additive_dimension
                        .clone()
                        .map(NonAdditiveDimension::from),
                    agg_time_dimension: Some(
                        metric
                            .agg_time_dimension
                            .clone()
                            .or_else(|| model_props.agg_time_dimension.clone())
                            .unwrap_or_default(),
                    ),
                }
            })
            .collect();

        let dimensions = model_props_to_dimensions(model_props.clone());
        let node_relation = NodeRelation {
            database: Some(resolved_model.database()),
            schema_name: resolved_model.schema(),
            alias: resolved_model.alias(),
            relation_name: resolved_model.__base_attr__.relation_name.clone(),
        };

        let dbt_semantic_model = DbtSemanticModel {
            __common_attr__: CommonAttributes {
                name: semantic_model_name.clone(),
                package_name: package_name.to_string(),
                path: mpe.relative_path.clone(),
                original_file_path: get_original_file_path(
                    &package.package_root_path,
                    &args.io.in_dir,
                    &mpe.relative_path,
                ),
                name_span: dbt_common::Span::from_serde_span(
                    mpe.name_span.clone(),
                    mpe.relative_path.clone(),
                ),
                patch_path: Some(mpe.relative_path.clone()),
                unique_id: semantic_model_unique_id.clone(),
                fqn: semantic_model_fqn.clone(),
                description: model_props.description.clone(),
                checksum: DbtChecksum::default(),
                raw_code: None,
                language: None,
                tags: semantic_model_config
                    .tags
                    .clone()
                    .map(|tags| tags.into())
                    .unwrap_or_default(),
                meta: semantic_model_config.meta.clone().unwrap_or_default(),
            },
            __base_attr__: NodeBaseAttributes {
                database: "".to_string(),
                schema: "".to_string(),
                alias: "".to_string(),
                relation_name: None,
                quoting: Default::default(),
                materialized: Default::default(),
                static_analysis: StaticAnalysisKind::Off.into(),
                static_analysis_off_reason: Some(StaticAnalysisOffReason::UnableToFetchSchema),
                enabled: true,
                extended_model: false,
                persist_docs: None,
                columns: Default::default(),
                refs: vec![DbtRef {
                    // only name is hydrated for parity with Mantle
                    name: model_name.clone(),
                    package: None,
                    version: None,
                    location: None,
                }],
                sources: vec![],
                functions: vec![],
                metrics: vec![],
                depends_on: NodeDependsOn {
                    nodes: vec![model_unique_id.clone()],
                    macros: vec![],
                    nodes_with_ref_location: vec![],
                },
                quoting_ignore_case: false,
            },
            __semantic_model_attr__: DbtSemanticModelAttr {
                unrendered_config: BTreeMap::new(), // TODO: do we need to hydrate?
                group: semantic_model_config.group.clone(),
                created_at: chrono::Utc::now().timestamp() as f64,
                metadata: None, // deprioritized feature. always null for now.
                label: None,    // no semantic model level label (could maybe inherit from model?)
                model: format!("ref('{model_name}')"),
                node_relation: Some(node_relation),
                defaults: Some(SemanticModelDefaults {
                    agg_time_dimension: model_props.agg_time_dimension.clone(),
                }),
                entities: model_props_to_semantic_entities(model_props.clone()),
                measures,
                dimensions,
                primary_entity: model_props.primary_entity.clone(),
            },
            deprecated_config: semantic_model_config.clone(),
            __other__: BTreeMap::new(),
        };

        // Check if semantic_model is enabled (following exposures pattern)
        if semantic_model_config.enabled.unwrap_or(true) {
            semantic_models.insert(semantic_model_unique_id, Arc::new(dbt_semantic_model));
        } else {
            disabled_semantic_models.insert(semantic_model_unique_id, Arc::new(dbt_semantic_model));
        }
    }

    Ok((semantic_models, disabled_semantic_models))
}

pub fn model_props_to_semantic_entities(model_props: ModelProperties) -> Vec<SemanticEntity> {
    let mut entities: Vec<SemanticEntity> = vec![];

    for column in model_props.columns.unwrap_or_default() {
        if let Some(column_entity) = column.entity {
            let mut column_entity_config = match column_entity {
                Entity::EntityType(ref entity_type) => EntityConfig {
                    name: Some(column.name.clone()), // defaults to column.name if there is no column.entity.name
                    type_: entity_type.clone(),
                    description: column.description.clone(), // defaults to column.description if there is no column.entity.description
                    label: None,
                    config: None,
                },
                Entity::EntityConfig(ref config) => config.clone(),
            };
            if column_entity_config.name.is_none() {
                column_entity_config.name = Some(column.name.clone());
            }
            if column_entity_config.description.is_none() {
                column_entity_config.description = column.description.clone();
            }

            let semantic_entity = SemanticEntity {
                name: column_entity_config.name.unwrap_or_default(),
                entity_type: column_entity_config.type_,
                description: column_entity_config.description,
                expr: None, // only applicable for derived_semantics
                label: column_entity_config.label,
                config: column_entity_config.config,
                // fields below are always null (for now)
                role: None,
                metadata: None,
            };
            entities.push(semantic_entity);
        }
    }

    let derived_entities = model_props
        .derived_semantics
        .unwrap_or_default()
        .entities
        .unwrap_or_default();
    for derived_entity in derived_entities {
        let semantic_entity = SemanticEntity {
            name: derived_entity.name.clone(),
            expr: Some(derived_entity.expr.clone()),
            entity_type: derived_entity.type_.clone(),
            description: derived_entity.description.clone(),
            label: derived_entity.label.clone(),
            config: derived_entity.config.clone(),
            // fields below are always null (for now)
            role: None,
            metadata: None,
        };
        entities.push(semantic_entity);
    }

    entities
}

pub fn model_props_to_dimensions(model_props: ModelProperties) -> Vec<Dimension> {
    let mut dimensions: Vec<Dimension> = vec![];

    for column in model_props.columns.unwrap_or_default() {
        if let Some(column_dimension) = column.dimension {
            let mut column_dimension_config = match column_dimension {
                ColumnPropertiesDimension::DimensionType(ref dimension_type) => {
                    ColumnPropertiesDimensionConfig {
                        type_: dimension_type.clone(),
                        is_partition: Some(false),
                        name: Some(column.name.clone()), // defaults to column.name if there is no column.dimension.name
                        description: column.description.clone(), // defaults to column.description if there is no column.dimension.description
                        label: None,
                        config: None,
                        validity_params: None,
                    }
                }
                ColumnPropertiesDimension::DimensionConfig(ref config) => config.clone(),
            };
            if column_dimension_config.name.is_none() {
                column_dimension_config.name = Some(column.name.clone());
            }
            if column_dimension_config.description.is_none() {
                column_dimension_config.description = column.description.clone();
            }

            let mut type_params = None;
            if column.granularity.is_some() || column_dimension_config.validity_params.is_some() {
                type_params = Some(DimensionTypeParams {
                    time_granularity: column.granularity,
                    validity_params: column_dimension_config.validity_params,
                });
            }

            let dimension = Dimension {
                name: column_dimension_config.name.unwrap_or_default(),
                column_name: Some(column.name.clone()),
                dimension_type: column_dimension_config.type_.clone(),
                description: column_dimension_config.description,
                expr: None, // only applicable for derived_semantics
                label: column_dimension_config.label,
                is_partition: column_dimension_config.is_partition.unwrap_or(false),
                type_params,
                config: column_dimension_config.config.clone(),
                // fields below are always null (for now)
                metadata: None,
            };
            dimensions.push(dimension);
        }
    }

    let derived_dimensions = model_props
        .derived_semantics
        .unwrap_or_default()
        .dimensions
        .unwrap_or_default();
    for derived_dimension in derived_dimensions {
        let mut type_params = None;
        if derived_dimension.granularity.is_some() || derived_dimension.validity_params.is_some() {
            type_params = Some(DimensionTypeParams {
                time_granularity: derived_dimension.granularity,
                validity_params: derived_dimension.validity_params,
            });
        }

        let dimension = Dimension {
            name: derived_dimension.name.clone(),
            expr: Some(derived_dimension.expr.clone()),
            column_name: None,
            dimension_type: derived_dimension.type_.clone(),
            is_partition: derived_dimension.is_partition.unwrap_or(false),
            description: derived_dimension.description.clone(),
            type_params,
            label: derived_dimension.label.clone(),
            config: derived_dimension.config.clone(),
            // fields below are always null (for now)
            metadata: None,
        };
        dimensions.push(dimension);
    }

    dimensions
}
