//! Module containing the entrypoint for the resolve phase.
use crate::dbt_project_config::{RootProjectConfigs, init_project_config};
use crate::utils::get_node_fqn;

use dbt_common::io_args::{IoArgs, StaticAnalysisKind};
use dbt_common::{ErrorCode, FsResult, err, show_error};
use dbt_jinja_utils::jinja_environment::JinjaEnv;
use dbt_jinja_utils::refs_and_sources::RefsAndSources;
use dbt_jinja_utils::serde::{Omissible, into_typed_with_jinja};
use dbt_jinja_utils::utils::generate_relation_name;
use dbt_schemas::schemas::common::{
    DbtChecksum, DbtMaterialization, DbtQuoting, FreshnessDefinition, FreshnessRules,
    NodeDependsOn, merge_meta, merge_tags, normalize_quoting,
};
use dbt_schemas::schemas::dbt_column::process_columns;
use dbt_schemas::schemas::project::{DefaultTo, SourceConfig};
use dbt_schemas::schemas::properties::{SourceProperties, Tables};
use dbt_schemas::schemas::{CommonAttributes, DbtSource, DbtSourceAttr, NodeBaseAttributes};
use dbt_schemas::state::{DbtAsset, DbtPackage, ModelStatus, RefsAndSourcesTracker};
use minijinja::Value as MinijinjaValue;
use regex::Regex;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;

use super::resolve_properties::MinimalPropertiesEntry;
use super::resolve_tests::persist_generic_data_tests::{TestableNodeTrait, TestableTable};

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn resolve_sources(
    io_args: &IoArgs,
    package: &DbtPackage,
    package_quoting: DbtQuoting,
    root_project_configs: &RootProjectConfigs,
    source_properties: BTreeMap<(String, String), MinimalPropertiesEntry>,
    database: &str,
    adapter_type: &str,
    base_ctx: &BTreeMap<String, MinijinjaValue>,
    jinja_env: &JinjaEnv,
    collected_tests: &mut Vec<DbtAsset>,
    refs_and_sources: &mut RefsAndSources,
) -> FsResult<(
    HashMap<String, Arc<DbtSource>>,
    HashMap<String, Arc<DbtSource>>,
)> {
    let mut sources: HashMap<String, Arc<DbtSource>> = HashMap::new();
    let mut disabled_sources: HashMap<String, Arc<DbtSource>> = HashMap::new();
    let package_name = package.dbt_project.name.as_ref();

    let special_chars = Regex::new(r"[^a-zA-Z0-9_]").unwrap();

    let local_project_config = init_project_config(
        io_args,
        &package.dbt_project.sources,
        SourceConfig {
            enabled: Some(true),
            quoting: Some(package_quoting),
            ..Default::default()
        },
    )?;
    for ((source_name, table_name), mpe) in source_properties.into_iter() {
        let source: SourceProperties = into_typed_with_jinja(
            Some(io_args),
            mpe.schema_value,
            false,
            jinja_env,
            base_ctx,
            &[],
        )?;

        let table: Tables = into_typed_with_jinja(
            Some(io_args),
            mpe.table_value.unwrap(),
            false,
            jinja_env,
            base_ctx,
            &[],
        )?;
        let database: String = source
            .database
            .clone()
            .or_else(|| source.catalog.clone())
            .unwrap_or(database.to_owned());
        let schema = source.schema.clone().unwrap_or(source.name.clone());

        // sources exist within the context of the model paths so we need to pass this into get config for path to get the config relative to the model paths
        let model_resource_paths = package
            .dbt_project
            .model_paths
            .as_ref()
            .unwrap_or(&vec![])
            .clone();

        let global_config = local_project_config.get_config_for_path(
            &mpe.relative_path,
            package_name,
            &model_resource_paths,
        );

        let mut project_config = root_project_configs
            .sources
            .get_config_for_path(&mpe.relative_path, package_name, &model_resource_paths)
            .clone();
        project_config.default_to(global_config);

        let source_properties_config = if let Some(properties) = &source.config {
            let mut properties_config: SourceConfig = properties.clone();
            properties_config.default_to(&project_config);
            properties_config
        } else {
            project_config
        };

        let table_config = table.config.clone().unwrap_or_default();

        let is_enabled = table_config
            .enabled
            .unwrap_or(source_properties_config.get_enabled().unwrap_or(true));

        let normalized_table_name = special_chars.replace_all(&table_name, "__");
        let unique_id = format!(
            "source.{}.{}.{}",
            &package_name, source_name, &normalized_table_name
        );
        let fqn = get_node_fqn(
            package_name,
            mpe.relative_path.clone(),
            vec![source_name.to_owned(), table_name.to_owned()],
        );

        let merged_loaded_at_field = Some(
            table_config.loaded_at_field.clone().unwrap_or(
                source_properties_config
                    .loaded_at_field
                    .clone()
                    .unwrap_or("".to_string()),
            ),
        );
        let merged_loaded_at_query = Some(
            table_config.loaded_at_query.clone().unwrap_or(
                source_properties_config
                    .loaded_at_query
                    .clone()
                    .unwrap_or("".to_string()),
            ),
        );
        if !merged_loaded_at_field.as_ref().unwrap().is_empty()
            && !merged_loaded_at_query.as_ref().unwrap().is_empty()
        {
            return err!(
                ErrorCode::Unexpected,
                "loaded_at_field and loaded_at_query cannot be set at the same time"
            );
        }
        let merged_freshness = merge_freshness(
            source_properties_config.freshness.as_ref(),
            &table_config.freshness,
        );

        // This should be set due to propagation from the resolved root project
        let properties_quoting = source_properties_config
            .quoting
            .expect("quoting should be set");

        let mut source_quoting = source.quoting.unwrap_or_default();
        source_quoting.default_to(&properties_quoting);

        let mut table_quoting = table.quoting.unwrap_or_default();
        table_quoting.default_to(&source_quoting);
        let quoting_ignore_case = table_quoting.snowflake_ignore_case.unwrap_or(false);

        let (database, schema, identifier, quoting) = normalize_quoting(
            &table_quoting.try_into()?,
            adapter_type,
            &database,
            &schema,
            &table.identifier.clone().unwrap_or(table_name.to_owned()),
        );

        let parse_adapter = jinja_env
            .get_parse_adapter()
            .expect("Failed to get parse adapter");

        let relation_name =
            generate_relation_name(parse_adapter, &database, &schema, &identifier, quoting)?;

        let source_tags: Option<Vec<String>> = source_properties_config
            .tags
            .clone()
            .map(|tags| tags.into());
        let table_tags: Option<Vec<String>> = table_config.tags.clone().map(|tags| tags.into());

        let merged_tags = merge_tags(source_tags, table_tags);
        let merged_meta = merge_meta(
            source_properties_config.meta.clone(),
            table_config.meta.clone(),
        );

        let columns = if let Some(ref cols) = table.columns {
            process_columns(
                Some(cols),
                source_properties_config.meta.clone(),
                source_properties_config
                    .tags
                    .clone()
                    .map(|tags| tags.into()),
            )?
        } else {
            BTreeMap::new()
        };

        if let Some(freshness) = merged_freshness.as_ref() {
            FreshnessRules::validate(freshness.error_after.as_ref())?;
            FreshnessRules::validate(freshness.warn_after.as_ref())?;
        }

        let dbt_source = DbtSource {
            common_attr: CommonAttributes {
                name: table_name.to_owned(),
                package_name: package_name.to_owned(),
                // original_file_path: dbt_asset.base_path.join(&dbt_asset.path),
                // path: dbt_asset.base_path.join(&dbt_asset.path),
                original_file_path: mpe.relative_path.clone(),
                path: mpe.relative_path.clone(),
                unique_id: unique_id.to_owned(),
                fqn,
                description: table.description.to_owned(),
                // todo: columns code gen missing
                patch_path: Some(mpe.relative_path.clone()),
                meta: merged_meta.unwrap_or_default(),
                tags: merged_tags.unwrap_or_default(),
                raw_code: None,
                checksum: DbtChecksum::default(),
                language: None,
            },
            base_attr: NodeBaseAttributes {
                database: database.to_owned(),
                schema: schema.to_owned(),
                alias: identifier.to_owned(),
                relation_name: Some(relation_name),
                quoting,
                quoting_ignore_case,
                enabled: is_enabled,
                extended_model: false,
                materialized: DbtMaterialization::External,
                static_analysis: source_properties_config
                    .static_analysis
                    .unwrap_or(StaticAnalysisKind::On),
                columns,
                refs: vec![],
                sources: vec![],
                depends_on: NodeDependsOn::default(),
                metrics: vec![],
            },
            source_attr: DbtSourceAttr {
                freshness: merged_freshness.clone(),
                identifier,
                source_name: source_name.to_owned(),
                source_description: source.description.clone().unwrap_or("".to_string()), // needs to be some or empty string per dbt spec
                loader: source.loader.clone().unwrap_or("".to_string()),
                loaded_at_field: merged_loaded_at_field.clone(),
                loaded_at_query: merged_loaded_at_query.clone(),
            },
            deprecated_config: source_properties_config.clone(),
            other: BTreeMap::new(),
        };
        let status = if is_enabled {
            ModelStatus::Enabled
        } else {
            ModelStatus::Disabled
        };

        match refs_and_sources.insert_source(package_name, &dbt_source, adapter_type, status) {
            Ok(_) => (),
            Err(e) => {
                show_error!(&io_args, e.with_location(mpe.relative_path.clone()));
            }
        }

        match status {
            ModelStatus::Enabled => {
                sources.insert(unique_id, Arc::new(dbt_source));

                TestableTable {
                    source_name: source_name.clone(),
                    table: &table.clone(),
                }
                .as_testable()
                .persist(package_name, collected_tests, adapter_type, io_args)?;
            }
            ModelStatus::Disabled => {
                disabled_sources.insert(unique_id, Arc::new(dbt_source));
            }
            ModelStatus::ParsingFailed => {}
        }
    }
    Ok((sources, disabled_sources))
}

fn merge_freshness(
    base: Option<&FreshnessDefinition>,
    update: &Omissible<Option<FreshnessDefinition>>,
) -> Option<FreshnessDefinition> {
    match update {
        // A present but 'null' freshness does not inherit from the base and inhibits freshness by returning None.
        Omissible::Present(update) => update
            .as_ref()
            .and_then(|update| merge_freshness_unwrapped(base, Some(update))),
        // If there is no freshness present in the update then it is inherited (merged) from the base.
        Omissible::Omitted => merge_freshness_unwrapped(base, None),
    }
}

fn merge_freshness_unwrapped(
    base: Option<&FreshnessDefinition>,
    update: Option<&FreshnessDefinition>,
) -> Option<FreshnessDefinition> {
    match (base, update) {
        (Some(base), Some(update)) => {
            // First create a merged base
            let mut merged = FreshnessDefinition {
                error_after: base.error_after.clone(),
                warn_after: base.warn_after.clone(),
                filter: base.filter.clone(),
            };

            // Apply updates from the update object
            if update.error_after.is_some() {
                merged.error_after = update.error_after.clone();
            }
            if update.warn_after.is_some() {
                merged.warn_after = update.warn_after.clone();
            }

            if update.filter.is_some() {
                merged.filter = update.filter.clone();
            }

            Some(merged)
        }
        (None, Some(update)) => Some(update.clone()),
        (Some(base), None) => Some(base.clone()),
        (None, None) => None,
    }
}
