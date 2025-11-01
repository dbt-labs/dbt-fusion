use crate::args::ResolveArgs;
use crate::dbt_project_config::{RootProjectConfigs, init_project_config};
use crate::utils::{get_node_fqn, get_original_file_path, get_unique_id};

use dbt_common::io_args::{StaticAnalysisKind, StaticAnalysisOffReason};
use dbt_common::tracing::emit::emit_error_log_from_fs_error;
use dbt_common::{ErrorCode, FsResult, fs_err};
use dbt_jinja_utils::jinja_environment::JinjaEnv;
use dbt_jinja_utils::serde::into_typed_with_jinja;
use dbt_jinja_utils::utils::dependency_package_name_from_ctx;
use dbt_schemas::schemas::common::{DbtChecksum, DbtMaterialization, NodeDependsOn};
use dbt_schemas::schemas::manifest::saved_query::{
    self, DbtSavedQuery, DbtSavedQueryAttr, SavedQueryExportConfig, SavedQueryParams,
};
use dbt_schemas::schemas::project::{DefaultTo, SavedQueryConfig};
use dbt_schemas::schemas::properties::SavedQueriesProperties;
use dbt_schemas::schemas::{CommonAttributes, NodeBaseAttributes};
use dbt_schemas::state::DbtPackage;
use minijinja::value::Value as MinijinjaValue;
use regex::Regex;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::resolve_properties::MinimalPropertiesEntry;

#[allow(clippy::too_many_arguments)]
pub async fn resolve_saved_queries(
    arg: &ResolveArgs,
    package: &DbtPackage,
    _root_package_name: &str,
    root_project_configs: &RootProjectConfigs,
    saved_query_properties: &mut BTreeMap<String, MinimalPropertiesEntry>,
    database: &str,
    schema: &str,
    package_name: &str,
    env: Arc<JinjaEnv>,
    base_ctx: &BTreeMap<String, MinijinjaValue>,
) -> FsResult<(
    HashMap<String, Arc<DbtSavedQuery>>,
    HashMap<String, Arc<DbtSavedQuery>>,
)> {
    let mut saved_queries: HashMap<String, Arc<DbtSavedQuery>> = HashMap::new();
    let mut disabled_saved_queries: HashMap<String, Arc<DbtSavedQuery>> = HashMap::new();

    // Return early if no saved queries to process
    if saved_query_properties.is_empty() {
        return Ok((saved_queries, disabled_saved_queries));
    }

    let dependency_package_name = dependency_package_name_from_ctx(&env, base_ctx);
    let local_project_config = init_project_config(
        &arg.io,
        &package.dbt_project.saved_queries,
        SavedQueryConfig {
            enabled: Some(true),
            ..Default::default()
        },
        dependency_package_name,
    )?;

    // Validate saved query names with regex (similar to exposures)
    let saved_query_name_re = Regex::new(r"[\w-]+$").unwrap();

    for (saved_query_name, mpe) in saved_query_properties.iter_mut() {
        if !mpe.schema_value.is_null() {
            // Validate saved query name
            if !saved_query_name_re.is_match(saved_query_name) {
                let e = fs_err!(
                    code => ErrorCode::InvalidConfig,
                    loc => mpe.relative_path.clone(),
                    "Saved query name '{}' can only contain letters, numbers, and underscores.",
                    saved_query_name
                );
                emit_error_log_from_fs_error(&e, arg.io.status_reporter.as_ref());

                continue;
            }

            let unique_id = get_unique_id(saved_query_name, package_name, None, "saved_query");
            let fqn = get_node_fqn(
                package_name,
                mpe.relative_path.clone(),
                vec![saved_query_name.to_owned()],
                &package.dbt_project.all_source_paths(),
            );

            let schema_value =
                std::mem::replace(&mut mpe.schema_value, dbt_serde_yaml::Value::null());

            // Parse the saved query properties from YAML
            let saved_query_props: SavedQueriesProperties = into_typed_with_jinja(
                &arg.io,
                schema_value,
                false,
                &env,
                base_ctx,
                &[],
                dependency_package_name,
                true,
            )?;

            // Get combined config from project config and saved query config
            let global_config = local_project_config.get_config_for_fqn(&fqn);
            let mut project_config = root_project_configs
                .saved_queries
                .get_config_for_fqn(&fqn)
                .clone();
            project_config.default_to(global_config);

            let saved_query_config = if let Some(config) = &saved_query_props.config {
                let mut final_config = config.clone();
                final_config.default_to(&project_config);
                final_config
            } else {
                project_config.clone()
            };

            let props_query_params = &saved_query_props.query_params;

            // Create default query params and exports since we're doing minimal implementation
            let query_params = SavedQueryParams {
                metrics: props_query_params.metrics.clone().unwrap_or_default(),
                group_by: props_query_params.group_by.clone().unwrap_or_default(),
                where_: props_query_params
                    .where_
                    .clone()
                    .map(|where_clause| where_clause.into()),
                order_by: vec![],
                limit: None, // TODO? not sure where to source from
            };

            let unique_ids_of_nodes_depends_on: Vec<String> = query_params
                .metrics
                .iter()
                .map(|name| get_unique_id(name, package_name, None, "metric"))
                .collect();

            // TODO: we should probably also try to resolve semantic_models for dimensions,
            let depends_on = NodeDependsOn {
                macros: vec![],
                nodes: unique_ids_of_nodes_depends_on,
                nodes_with_ref_location: vec![],
            };

            // schema can be overriden from either Export config or Saved Query config
            // TODO: we should also allow overriding the database in the same manner
            let saved_query_schema = saved_query_config
                .schema
                .clone()
                .unwrap_or_else(|| schema.to_string());

            let exports = saved_query_props
                .exports
                .unwrap_or_default()
                .iter()
                .map(|export| {
                    let config = export.config.clone().unwrap_or_default();

                    saved_query::SavedQueryExport {
                        name: export.name.clone(),
                        config: SavedQueryExportConfig {
                            export_as: config.export_as.unwrap_or_default(),
                            schema_name: Some(
                                config
                                    .schema
                                    .unwrap_or_else(|| saved_query_schema.to_string()),
                            ),
                            alias: Some(config.alias.unwrap_or_else(|| export.name.clone())),
                            database: Some(database.to_string()),
                        },
                        unrendered_config: BTreeMap::new(), // TODO
                    }
                })
                .collect::<Vec<saved_query::SavedQueryExport>>();

            // FIXME: this is likely not completely correct, we should figure out the "right" solution
            // use the first export destination for the saved query values,
            // if there's no exports then the saved query doesn't technially materialize
            let schema = exports
                .first()
                .map(|export| export.config.schema_name.clone())
                .unwrap_or_default();
            let database = exports
                .first()
                .map(|export| export.config.database.clone())
                .unwrap_or_default();
            let alias = exports
                .first()
                .map(|export| export.config.alias.clone())
                .unwrap_or_default();

            let dbt_saved_query = DbtSavedQuery {
                __common_attr__: CommonAttributes {
                    name: saved_query_name.clone(),
                    package_name: package_name.to_string(),
                    path: mpe.relative_path.clone(),
                    original_file_path: get_original_file_path(
                        &package.package_root_path,
                        &arg.io.in_dir,
                        &mpe.relative_path,
                    ),
                    name_span: dbt_common::Span::from_serde_span(
                        mpe.name_span.clone(),
                        mpe.relative_path.clone(),
                    ),
                    patch_path: Some(mpe.relative_path.clone()),
                    unique_id: unique_id.clone(),
                    fqn,
                    description: saved_query_props.description,
                    checksum: DbtChecksum::default(),
                    raw_code: None,
                    language: None,
                    tags: saved_query_config
                        .tags
                        .clone()
                        .map(|tags| tags.into())
                        .unwrap_or_default(),
                    meta: saved_query_config.meta.clone().unwrap_or_default(),
                },
                __base_attr__: NodeBaseAttributes {
                    database: database.unwrap_or_default(),
                    schema: schema.unwrap_or_default(),
                    alias: alias.unwrap_or_default(),
                    relation_name: None,         // TODO: what should this be?
                    quoting: Default::default(), // TODO: what should this be?
                    materialized: DbtMaterialization::Unknown("export".to_string()),
                    static_analysis: StaticAnalysisKind::Off.into(),
                    static_analysis_off_reason: Some(StaticAnalysisOffReason::UnableToFetchSchema),
                    enabled: true,
                    extended_model: false,
                    persist_docs: None,
                    columns: Default::default(),
                    refs: vec![],
                    sources: vec![],
                    functions: vec![],
                    metrics: vec![],
                    depends_on,
                    quoting_ignore_case: false,
                },
                __saved_query_attr__: DbtSavedQueryAttr {
                    query_params,
                    exports,
                    label: saved_query_props.label,
                    metadata: None,
                    unrendered_config: BTreeMap::new(),
                    group: saved_query_config.group.clone(),
                    created_at: chrono::Utc::now().timestamp() as f64,
                },
                deprecated_config: saved_query_config.clone(),
                __other__: BTreeMap::new(),
            };

            // Check if saved query is enabled (following exposures pattern)
            if saved_query_config.enabled.unwrap_or(true) {
                saved_queries.insert(unique_id, Arc::new(dbt_saved_query));
            } else {
                disabled_saved_queries.insert(unique_id, Arc::new(dbt_saved_query));
            }
        }
    }

    Ok((saved_queries, disabled_saved_queries))
}
