//! This module contains the scope for materializing nodes

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use dbt_agate::AgateTable;
use dbt_common::ErrorCode;
use dbt_common::adapter::AdapterType;
use dbt_common::constants::DBT_COMPILED_DIR_NAME;
use dbt_common::constants::DBT_RUN_DIR_NAME;
use dbt_common::io_args::IoArgs;
use dbt_common::serde_utils::convert_yml_to_value_map;

use dbt_common::tokiofs;
use dbt_common::tracing::emit::emit_warn_log_message;
use dbt_fusion_adapter::load_store::ResultStore;
use dbt_fusion_adapter::relation_object::create_relation;
use dbt_schemas::schemas::CommonAttributes;
use dbt_schemas::schemas::NodeBaseAttributes;
use dbt_schemas::schemas::telemetry::NodeType;
use minijinja::State;
use minijinja::constants::CURRENT_PATH;
use minijinja::constants::CURRENT_SPAN;
use minijinja::listener::RenderingEventListener;
use minijinja::machinery::Span;
use minijinja::{Error, ErrorKind, Value as MinijinjaValue, value::Object};
use serde::Serialize;

use crate::phases::MacroLookupContext;

use super::run_config::RunConfig;

type YmlValue = dbt_serde_yaml::Value;

/// Build model-specific context (model, common_attr, alias, quoting, config, resource_type, sql_header)
#[allow(clippy::too_many_arguments)]
async fn extend_with_model_context<S: Serialize>(
    base_context: &mut BTreeMap<String, MinijinjaValue>,
    model: YmlValue,
    common_attr: &CommonAttributes,
    base_attr: &NodeBaseAttributes,
    deprecated_config: &S,
    adapter_type: AdapterType,
    io_args: &IoArgs,
    resource_type: NodeType,
    sql_header: Option<MinijinjaValue>,
) {
    // Create a relation for 'this' using config values
    let this_relation = create_relation(
        adapter_type,
        base_attr.database.clone(),
        base_attr.schema.clone(),
        Some(base_attr.alias.clone()),
        None,
        base_attr.quoting,
    )
    .unwrap()
    .as_value();

    base_context.insert("this".to_owned(), this_relation);
    base_context.insert(
        "database".to_owned(),
        MinijinjaValue::from(base_attr.database.clone()),
    );
    base_context.insert(
        "schema".to_owned(),
        MinijinjaValue::from(base_attr.schema.clone()),
    );
    base_context.insert(
        "identifier".to_owned(),
        MinijinjaValue::from(common_attr.name.clone()),
    );

    let config_yml =
        dbt_serde_yaml::to_value(deprecated_config).expect("Failed to serialize object");

    if let Some(pre_hook) = config_yml.get("pre_hook") {
        let values: Vec<HookConfig> = match pre_hook {
            YmlValue::String(_, _) | YmlValue::Mapping(_, _) => {
                parse_hook_item(pre_hook).into_iter().collect()
            }
            YmlValue::Sequence(arr, _) => arr.iter().filter_map(parse_hook_item).collect(),
            YmlValue::Null(_) => vec![],
            _ => {
                emit_warn_log_message(
                    ErrorCode::Generic,
                    format!("Unknown pre-hook type: {:?}", pre_hook),
                    io_args.status_reporter.as_ref(),
                );

                vec![]
            }
        };
        let pre_hooks_vals: MinijinjaValue = values
            .iter()
            .map(|hook| MinijinjaValue::from_object(hook.clone()))
            .collect::<Vec<MinijinjaValue>>()
            .into();
        base_context.insert("pre_hooks".to_owned(), pre_hooks_vals);
    }
    if let Some(post_hook) = config_yml.get("post_hook") {
        let values: Vec<HookConfig> = match post_hook {
            YmlValue::String(_, _) | YmlValue::Mapping(_, _) => {
                parse_hook_item(post_hook).into_iter().collect()
            }
            YmlValue::Sequence(arr, _) => arr.iter().filter_map(parse_hook_item).collect(),
            YmlValue::Null(_) => vec![],
            _ => {
                emit_warn_log_message(
                    ErrorCode::Generic,
                    format!("Unknown post-hook type: {:?}", post_hook),
                    io_args.status_reporter.as_ref(),
                );

                vec![]
            }
        };
        let post_hooks_vals: MinijinjaValue = values
            .iter()
            .map(|hook| MinijinjaValue::from_object(hook.clone()))
            .collect::<Vec<MinijinjaValue>>()
            .into();
        base_context.insert("post_hooks".to_owned(), post_hooks_vals);
    }

    let mut config_map = convert_yml_to_value_map(config_yml);
    if let Some(sql_header) = sql_header {
        config_map.insert("sql_header".to_string(), sql_header);
    }

    let mut model_map = convert_yml_to_value_map(model);

    // We are reading the raw_sql here for snapshots and models
    let raw_sql_path = match resource_type {
        NodeType::Snapshot => Some(io_args.out_dir.join(common_attr.original_file_path.clone())),
        NodeType::Model => Some(io_args.in_dir.join(common_attr.original_file_path.clone())),
        _ => None,
    };
    if let Some(raw_sql_path) = raw_sql_path {
        if let Ok(raw_sql) = tokiofs::read_to_string(&raw_sql_path).await {
            model_map.insert("raw_sql".to_owned(), MinijinjaValue::from(raw_sql));
        } else {
            emit_warn_log_message(
                ErrorCode::Generic,
                format!("Failed to read raw_sql: {}", raw_sql_path.display()),
                io_args.status_reporter.as_ref(),
            );
        };
    }

    let node_config = RunConfig {
        model_config: config_map,
        model: model_map.clone(),
    };

    base_context.insert(
        "config".to_owned(),
        MinijinjaValue::from_object(node_config),
    );

    base_context.insert(
        "model".to_owned(),
        MinijinjaValue::from_object(model_map.clone()),
    );
    base_context.insert("node".to_owned(), MinijinjaValue::from_object(model_map));
    base_context.insert("connection_name".to_owned(), MinijinjaValue::from(""));
}

/// Extend the base context with stateful functions
pub fn extend_base_context_stateful_fn(
    base_context: &mut BTreeMap<String, MinijinjaValue>,
    root_project_name: &str,
    packages: BTreeSet<String>,
) {
    let result_store = ResultStore::default();
    base_context.insert(
        "store_result".to_owned(),
        MinijinjaValue::from_function(result_store.store_result()),
    );
    base_context.insert(
        "load_result".to_owned(),
        MinijinjaValue::from_function(result_store.load_result()),
    );
    base_context.insert(
        "store_raw_result".to_owned(),
        MinijinjaValue::from_function(result_store.store_raw_result()),
    );

    // Add submit_python_job context function using a separate helper
    base_context.insert(
        "submit_python_job".to_owned(),
        MinijinjaValue::from_function(submit_python_job_context_fn()),
    );

    let mut packages = packages;
    packages.insert(root_project_name.to_string());

    base_context.insert(
        "context".to_owned(),
        MinijinjaValue::from_object(MacroLookupContext {
            root_project_name: root_project_name.to_string(),
            current_project_name: None,
            packages,
        }),
    );
}

/// Build a run context - parent function that orchestrates the context building
#[allow(clippy::too_many_arguments)]
pub async fn build_run_node_context<S: Serialize>(
    model: YmlValue,
    common_attr: &CommonAttributes,
    base_attr: &NodeBaseAttributes,
    deprecated_config: &S,
    adapter_type: AdapterType,
    agate_table: Option<AgateTable>,
    base_context: &BTreeMap<String, MinijinjaValue>,
    io_args: &IoArgs,
    resource_type: NodeType,
    sql_header: Option<MinijinjaValue>,
    packages: BTreeSet<String>,
) -> BTreeMap<String, MinijinjaValue> {
    // Build model-specific context
    let mut context = base_context.clone();
    extend_base_context_stateful_fn(&mut context, &common_attr.package_name, packages);

    extend_with_model_context(
        &mut context,
        model,
        common_attr,
        base_attr,
        deprecated_config,
        adapter_type,
        io_args,
        resource_type,
        sql_header,
    )
    .await;

    let model_name = common_attr.name.clone();
    // Add write function
    context.insert(
        "write".to_owned(),
        MinijinjaValue::from_object(WriteConfig {
            model_name,
            resource_type: resource_type.as_ref().to_string(),
            project_root: io_args.in_dir.clone(),
            target_path: io_args.out_dir.clone(),
        }),
    );

    if let Some(agate_table) = agate_table {
        context.insert(
            "load_agate_table".to_owned(),
            MinijinjaValue::from_function(move |_args: &[MinijinjaValue]| {
                MinijinjaValue::from_object(agate_table.clone())
            }),
        );
    }

    let mut base_builtins = if let Some(builtins) = context.get("builtins") {
        builtins
            .as_object()
            .unwrap()
            .downcast_ref::<BTreeMap<String, MinijinjaValue>>()
            .unwrap()
            .clone()
    } else {
        BTreeMap::new()
    };

    // Get the config from model context to pass to general context
    let node_config = context
        .get("config")
        .unwrap()
        .as_object()
        .unwrap()
        .downcast_ref::<RunConfig>()
        .unwrap();

    base_builtins.insert(
        "config".to_string(),
        MinijinjaValue::from_object(node_config.clone()),
    );

    // Register builtins as a global
    context.insert(
        "builtins".to_owned(),
        MinijinjaValue::from_object(base_builtins),
    );

    let relative_path = PathBuf::from(DBT_COMPILED_DIR_NAME).join(&common_attr.path);
    context.insert(
        CURRENT_PATH.to_string(),
        MinijinjaValue::from(relative_path.to_string_lossy()),
    );
    context.insert(
        CURRENT_SPAN.to_string(),
        MinijinjaValue::from_serialize(Span::default()),
    );

    context
}

fn parse_hook_item(item: &YmlValue) -> Option<HookConfig> {
    match item {
        YmlValue::String(s, _) => Some(HookConfig {
            sql: s.to_string(),
            transaction: true,
        }),
        YmlValue::Mapping(map, _) => {
            let sql = map.get("sql")?.as_str()?.to_string();
            let transaction = map
                .get("transaction")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Some(HookConfig { sql, transaction })
        }
        _ => {
            eprintln!("Pre hook unknown type: {item:?}");
            None
        }
    }
}

#[derive(Clone)]
struct HookConfig {
    pub sql: String,

    pub transaction: bool,
}
impl Object for HookConfig {
    fn get_value(self: &Arc<Self>, key: &MinijinjaValue) -> Option<MinijinjaValue> {
        match key.as_str() {
            Some("sql") => Some(MinijinjaValue::from(self.sql.clone())),
            Some("transaction") => Some(MinijinjaValue::from(self.transaction)),
            _ => None,
        }
    }
    fn render(self: &Arc<Self>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.sql)
    }
}
// iplement std::fmt::Debug for HookConfig
impl std::fmt::Debug for HookConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HookConfig {{ sql: {} }}", self.sql)
    }
}

#[derive(Debug)]
pub struct WriteConfig {
    pub model_name: String,
    pub resource_type: String,
    pub project_root: PathBuf,
    pub target_path: PathBuf,
}

impl Object for WriteConfig {
    fn call(
        self: &Arc<Self>,
        _state: &State<'_, '_>,
        args: &[MinijinjaValue],
        _listeners: &[Rc<dyn RenderingEventListener>],
    ) -> Result<MinijinjaValue, Error> {
        if args.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidOperation,
                "write function requires payload argument".to_string(),
            ));
        }

        // Extract payload from args
        let payload = match args[0].as_str() {
            Some(s) => s,
            None => {
                return Err(Error::new(
                    ErrorKind::InvalidOperation,
                    "Failed to convert payload to string".to_string(),
                ));
            }
        };

        // Write the file
        match write_file(
            &self.project_root,
            &self.target_path,
            &self.model_name,
            &self.resource_type,
            payload,
        ) {
            Ok(_) => {}
            Err(e) => {
                return Err(Error::new(
                    ErrorKind::InvalidOperation,
                    format!("Failed to write file: {e}"),
                ));
            }
        }

        // Return empty string on success
        Ok(MinijinjaValue::from(""))
    }
}

/// Write a file to disk
fn write_file(
    project_root: &Path,
    target_path: &Path,
    model_name: &str,
    resource_type: &str,
    payload: &str,
) -> Result<(), Error> {
    // Check if model is a Macro or SourceDefinition
    if resource_type == "macro" || resource_type == "source" {
        return Err(Error::new(
            ErrorKind::InvalidOperation,
            "Macros and sources cannot be written to disk",
        ));
    }

    // Construct build path - simple implementation
    let build_path = target_path
        .join(DBT_RUN_DIR_NAME)
        .join(format!("{model_name}.sql"));
    let full_path = if build_path.is_absolute() {
        build_path
    } else {
        project_root.join(&build_path)
    };

    // Create parent directories if needed
    if let Some(parent) = full_path.parent()
        && !parent.exists()
        && let Err(e) = fs::create_dir_all(parent)
    {
        return Err(Error::new(
            ErrorKind::InvalidOperation,
            format!("Failed to create directory {}: {}", parent.display(), e),
        ));
    }

    match fs::write(&full_path, payload) {
        Ok(_) => Ok(()),
        Err(e) => Err(Error::new(
            ErrorKind::InvalidOperation,
            format!("Failed to write to {}: {}", full_path.display(), e),
        )),
    }
}

/// Returns the function used for the submit_python_job context.
fn submit_python_job_context_fn()
-> impl Fn(&State, &[MinijinjaValue]) -> Result<MinijinjaValue, Error> + Copy {
    |state: &State, args: &[MinijinjaValue]| {
        // Parse arguments: submit_python_job(parsed_model, compiled_code)
        if args.len() != 2 {
            return Err(Error::new(
                ErrorKind::InvalidOperation,
                format!("submit_python_job expects 2 arguments, got {}", args.len()),
            ));
        }
        let parsed_model = &args[0];
        let compiled_code = args[1].as_str().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidOperation,
                "compiled_code must be a string",
            )
        })?;

        // Note(Ani):
        // dbt-core validates:
        //   - macro_stack.depth == 2
        //   - call_stack[1] == "macro.dbt.statement"
        //   - "materialization" in call_stack[0]
        //
        // In fusion, we shouldn't need to do this because this funciton is only registered in the run node context
        // so if a user tries to use it outside of a statement.sql macro, in a materialization macro, it will fail earlier due to an unrecongized function call.

        // Get adapter from context and call submit_python_job
        let adapter = state
            .lookup("adapter")
            .ok_or_else(|| Error::new(ErrorKind::UndefinedError, "adapter not found in context"))?;
        adapter.call_method(
            state,
            "submit_python_job",
            &[parsed_model.clone(), MinijinjaValue::from(compiled_code)],
            &[],
        )
    }
}
