use crate::AdapterType;
use crate::base_adapter::BaseAdapter;
use crate::cast_util::downcast_value_to_dyn_base_relation;
use crate::databricks::relation::DEFAULT_DATABRICKS_DATABASE;
use crate::errors::AdapterResult;
use crate::errors::{AdapterError, AdapterErrorKind};
use crate::factory::create_static_relation;
use crate::formatter::SqlLiteralFormatter;
use crate::response::ResultObject;

use arrow::array::RecordBatch;
use dbt_agate::AgateTable;
use minijinja::arg_utils::ArgsIter;
use minijinja::listener::RenderingEventListener;
use minijinja::value::ValueKind;
use minijinja::value::mutable_vec::MutableVec;
use minijinja::{Error as MinijinjaError, ErrorKind as MinijinjaErrorKind, State, Value};
use minijinja_contrib::modules::py_datetime::date::PyDate;
use minijinja_contrib::modules::py_datetime::datetime::PyDateTime;
use serde::Deserialize;

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

/// Performs method dispatch on the given adapter.
pub fn dispatch_adapter_calls(
    adapter: &dyn BaseAdapter,
    state: &State,
    name: &str,
    args: &[Value],
    _listeners: &[Rc<dyn RenderingEventListener>],
) -> Result<Value, MinijinjaError> {
    match name {
        "dispatch" => adapter.dispatch(state, args),
        "execute" => {
            // sql: str, auto_begin: bool = False, fetch: bool = False, limit: Optional[int] = None
            let iter = ArgsIter::new(name, &["sql"], args);
            let sql = iter.next_arg::<&str>()?;
            let auto_begin = iter
                .next_kwarg::<Option<bool>>("auto_begin")?
                .unwrap_or(false);
            let fetch = iter.next_kwarg::<Option<bool>>("fetch")?.unwrap_or(false);
            let limit = iter.next_kwarg::<Option<i64>>("limit")?;
            let options = if let Some(value) = iter.next_kwarg::<Option<Value>>("options")? {
                Some(HashMap::<String, String>::deserialize(value).map_err(|e| {
                    MinijinjaError::new(MinijinjaErrorKind::SerdeDeserializeError, e.to_string())
                })?)
            } else {
                None
            };
            // TODO(harry): add iter.finish() and fix the tests

            let (response, table) =
                adapter.execute(state, sql, auto_begin, fetch, limit, options)?;
            Ok(Value::from_iter([
                Value::from_object(response),
                Value::from_object(table),
            ]))
        }
        "add_query" => {
            // sql: str,
            // auto_begin: bool = True,
            // bindings: Optional[Any] = None,
            // abridge_sql_log: bool = False,
            // retryable_exceptions: Tuple[Type[Exception], ...] = tuple(),
            // retry_limit: int = 1,
            let iter = ArgsIter::new(name, &["sql"], args);
            let sql = iter.next_arg::<&str>()?;
            let auto_begin = iter
                .next_kwarg::<Option<bool>>("auto_begin")?
                .unwrap_or(true);
            let bindings = iter.next_kwarg::<Option<&Value>>("bindings")?;
            let abridge_sql_log = iter
                .next_kwarg::<Option<bool>>("abridge_sql_log")?
                .unwrap_or(false);
            let _retryable_exceptions =
                iter.next_kwarg::<Option<&Value>>("retryable_exceptions")?;
            let _retry_limit = iter.next_kwarg::<Option<i64>>("retry_limit")?.unwrap_or(1);
            adapter.add_query(
                state,
                sql,
                auto_begin,
                bindings,
                abridge_sql_log,
                // _retryable_exceptions,
                // _retry_limit,
            )?;
            Ok(Value::from(()))
        }
        "get_relation" => {
            // database: str
            // schema: str
            // identifier: str
            let iter = ArgsIter::new(name, &["database", "schema", "identifier"], args);

            let database = iter.next_arg::<&str>().or_else(|e| {
                if adapter.adapter_type() == AdapterType::Databricks {
                    Ok(DEFAULT_DATABRICKS_DATABASE)
                } else {
                    Err(e)
                }
            })?;
            let schema = iter.next_arg::<&str>()?;
            let identifier = iter.next_arg::<&str>()?;
            iter.finish()?;

            adapter.get_relation(state, database, schema, identifier)
        }
        "get_columns_in_relation" => {
            // relation: BaseRelation
            let iter = ArgsIter::new(name, &["relation"], args);
            let relation = iter.next_arg::<&Value>()?;
            let relation = downcast_value_to_dyn_base_relation(relation)?;
            iter.finish()?;

            adapter.get_columns_in_relation(state, relation)
        }
        "build_catalog_relation" => {
            let iter = ArgsIter::new(name, &["model"], args);
            let model = iter.next_arg::<&Value>()?;
            iter.finish()?;

            // Case 1: caller passed a plain string (CLD name) -- this is a hack for
            // incremental polaris model. TODO: remove this when catalog_relation is
            // fully engineered to no longer require runtime attribute shim-based solutions
            if model.kind() == ValueKind::String {
                return adapter.build_catalog_relation(model);
            }

            // Case 2: caller passed a model object
            // TODO: When we remove case 1, we can serialize this as an InternalDbtNode (and add the necessary attributes to their respective resource-type specific attribute structs)
            // Ex: minijinja_value_to_typed_struct::<DbtModel>(model.clone()).is_ok();

            adapter.build_catalog_relation(model)
        }
        "get_catalog_integration" => adapter.get_catalog_integration(state, args),
        "type" => Ok(Value::from(adapter.adapter_type().to_string())),
        "get_hard_deletes_behavior" => adapter.get_hard_deletes_behavior(state, args),
        "cache_added" => {
            // relation: BaseRelation
            let iter = ArgsIter::new(name, &["relation"], args);
            let relation = iter.next_arg::<&Value>()?;
            let relation = downcast_value_to_dyn_base_relation(relation)?;
            iter.finish()?;

            adapter.cache_added(state, relation)
        }
        "cache_dropped" => {
            // relation: BaseRelation
            let iter = ArgsIter::new(name, &["relation"], args);
            let relation = iter.next_arg::<&Value>()?;
            let relation = downcast_value_to_dyn_base_relation(relation)?;
            iter.finish()?;

            adapter.cache_dropped(state, relation)
        }
        "cache_renamed" => {
            // from_relation: BaseRelation, to_relation: BaseRelation
            let iter = ArgsIter::new(name, &["from_relation", "to_relation"], args);
            let from_relation = iter.next_arg::<&Value>()?;
            let from_relation = downcast_value_to_dyn_base_relation(from_relation)?;
            let to_relation = iter.next_arg::<&Value>()?;
            let to_relation = downcast_value_to_dyn_base_relation(to_relation)?;
            iter.finish()?;

            adapter.cache_renamed(state, from_relation, to_relation)
        }
        "quote" => {
            // identifier: str
            let iter = ArgsIter::new(name, &["identifier"], args);

            let identifier = iter.next_arg::<&str>()?;
            iter.finish()?;

            adapter.quote(state, identifier)
        }
        "quote_as_configured" => {
            // identifier: str
            // quote_key: str
            let iter = ArgsIter::new(name, &["identifier", "quote_key"], args);

            let identifier = iter.next_arg::<&str>()?;
            let quote_key = iter.next_arg::<&str>()?;

            iter.finish()?;

            adapter.quote_as_configured(state, identifier, quote_key)
        }
        "quote_seed_column" => {
            // column: str
            // quote_config: Optional[bool]
            let iter = ArgsIter::new(name, &["column", "quote_config"], args);

            let column = iter.next_arg::<&str>()?;
            let quote_config = iter.next_kwarg::<Option<bool>>("quote_config")?;

            iter.finish()?;

            adapter.quote_seed_column(state, column, quote_config)
        }
        "drop_relation" => adapter.drop_relation(state, args),
        "truncate_relation" => adapter.truncate_relation(state, args),
        "rename_relation" => adapter.rename_relation(state, args),
        "expand_target_column_types" => {
            // from_relation: BaseRelation, to_relation: BaseRelation
            let iter = ArgsIter::new(name, &["from_relation", "to_relation"], args);
            let from_relation = iter.next_arg::<&Value>()?;
            let from_relation = downcast_value_to_dyn_base_relation(from_relation)?;
            let to_relation = iter.next_arg::<&Value>()?;
            let to_relation = downcast_value_to_dyn_base_relation(to_relation)?;
            iter.finish()?;

            adapter.expand_target_column_types(state, from_relation, to_relation)
        }
        "list_schemas" => adapter.list_schemas(state, args),
        "create_schema" => adapter.create_schema(state, args),
        "drop_schema" => adapter.drop_schema(state, args),
        "valid_snapshot_target" => adapter.valid_snapshot_target(state, args),
        "assert_valid_snapshot_target_given_strategy" => {
            adapter.assert_valid_snapshot_target_given_strategy(state, args)
        }
        "get_missing_columns" => adapter.get_missing_columns(state, args),
        "render_raw_model_constraints" => adapter.render_raw_model_constraints(state, args),
        "standardize_grants_dict" => adapter.standardize_grants_dict(state, args),
        "convert_type" => adapter.convert_type(state, args),
        "render_raw_columns_constraints" => adapter.render_raw_columns_constraints(state, args),
        "verify_database" => adapter.verify_database(state, args),
        "commit" => adapter.commit(args),
        "get_incremental_strategy_macro" => adapter.get_incremental_strategy_macro(state, args),
        "check_schema_exists" => adapter.check_schema_exists(state, args),
        "get_relations_by_pattern" => adapter.get_relations_by_pattern(state, args),
        // only available for Bigquery
        "nest_column_data_types" => adapter.nest_column_data_types(state, args),
        "add_time_ingestion_partition_column" => {
            adapter.add_time_ingestion_partition_column(state, args)
        }
        "parse_partition_by" => adapter.parse_partition_by(state, args),
        "is_replaceable" => adapter.is_replaceable(state, args),
        "list_relations_without_caching" => adapter.list_relations_without_caching(state, args),
        "copy_table" => adapter.copy_table(state, args),
        "update_columns" => adapter.update_columns(state, args),
        "update_table_description" => adapter.update_table_description(state, args),
        "alter_table_add_columns" => adapter.alter_table_add_columns(state, args),
        "load_dataframe" => {
            let iter = ArgsIter::new(
                name,
                &[
                    "database",
                    "schema",
                    "table_name",
                    "field_path",
                    "agate_table",
                    "field_delimiter",
                ],
                args,
            );
            let database = iter.next_arg::<&str>()?;
            let schema = iter.next_arg::<&str>()?;
            let table_name = iter.next_arg::<&str>()?;
            let file_path = iter.next_arg::<&str>()?;
            let agate_table = iter
                .next_arg::<&Value>()?
                .downcast_object::<AgateTable>()
                .ok_or_else(|| {
                    MinijinjaError::new(
                        MinijinjaErrorKind::InvalidOperation,
                        "agate_table must be an agate.Table",
                    )
                })?;
            let field_delimiter = iter.next_arg::<&str>()?;
            iter.finish()?;

            adapter.load_dataframe(
                state,
                database,
                schema,
                table_name,
                agate_table,
                file_path,
                field_delimiter,
            )
        }
        "upload_file" => adapter.upload_file(state, args),
        "get_bq_table" => adapter.get_bq_table(state, args),
        "describe_relation" => {
            // relation: BaseRelation
            let iter = ArgsIter::new(name, &["relation"], args);
            let relation = iter.next_arg::<&Value>()?;
            let relation = downcast_value_to_dyn_base_relation(relation)?;
            iter.finish()?;

            adapter.describe_relation(state, relation)
        }
        "grant_access_to" => adapter.grant_access_to(state, args),
        "get_dataset_location" => adapter.get_dataset_location(state, args),
        "get_column_schema_from_query" => adapter.get_column_schema_from_query(state, args),
        "get_columns_in_select_sql" => adapter.get_columns_in_select_sql(state, args),
        "get_common_options" => adapter.get_common_options(state, args),
        "get_table_options" => adapter.get_table_options(state, args),
        "get_view_options" => adapter.get_view_options(state, args),
        "get_partitions_metadata" => adapter.get_partitions_metadata(state, args),
        "get_relations_without_caching" => adapter.get_relations_without_caching(state, args),
        "parse_index" => adapter.parse_index(state, args),
        "redact_credentials" => adapter.redact_credentials(state, args),
        // only available for DataBricks
        "compare_dbr_version" => adapter.compare_dbr_version(state, args),
        "compute_external_path" => adapter.compute_external_path(state, args),
        "update_tblproperties_for_iceberg" => adapter.update_tblproperties_for_iceberg(state, args),
        "valid_incremental_strategies" => adapter.valid_incremental_strategies(state, args),
        "get_relation_config" => adapter.get_relation_config(state, args),
        "get_config_from_model" => adapter.get_config_from_model(state, args),
        "get_persist_doc_columns" => adapter.get_persist_doc_columns(state, args),
        "generate_unique_temporary_table_suffix" => {
            adapter.generate_unique_temporary_table_suffix(state, args)
        }
        "parse_columns_and_constraints" => adapter.parse_columns_and_constraints(state, args),
        "clean_sql" => adapter.clean_sql(args),
        _ => Err(MinijinjaError::new(
            MinijinjaErrorKind::InvalidOperation,
            format!("Unknown method on adapter object: '{name}'"),
        )),
    }
}

pub fn dispatch_adapter_get_value(adapter: &dyn BaseAdapter, key: &Value) -> Option<Value> {
    match key.as_str() {
        Some("behavior") => Some(adapter.behavior()),
        // NOTE(serramatutu): BigQuery adapter calls `Relation` from `adapter.Relation`
        // instead of `api.Relation` when executing materialized views
        Some("Relation") => create_static_relation(adapter.adapter_type(), adapter.quoting()),
        _ => None,
    }
}

/// Execute a macro under the `dbt` package.
/// Unlike [`execute_macro`] that returns a `Value`,
/// this function returns a `RecordBatch` which may become handy when the result manipulation is necessary.
///
/// # Panics
///
/// This function will panic if the macro named `dbt.{macro_name}` does not exist in the template state.
pub fn execute_macro_wrapper(
    state: &State,
    args: &[Value],
    macro_name: &str,
) -> Result<Arc<RecordBatch>, AdapterError> {
    execute_macro_wrapper_with_package(state, args, macro_name, "dbt")
}

/// Execute a macro under "dbt" package.
/// If you need to manipulate the result, checkout [`execute_macro_wrapper`]
///
/// # Panics
///
/// This function will panic if the macro named `dbt.{macro_name}` does not exist in the template state.
pub fn execute_macro(
    state: &State,
    args: &[Value],
    macro_name: &str,
) -> Result<Value, AdapterError> {
    execute_macro_with_package(state, args, macro_name, "dbt")
}

/// Execute a macro under a given package.
/// Unlike [`execute_macro_with_package`] that returns a `Value`,
/// this function returns a `RecordBatch` which may become handy when the result manipulation is necessary.
///
/// # Panics
///
/// This function will panic if the macro named `{package}.{macro_name}` does not exist in the template state.
pub fn execute_macro_wrapper_with_package(
    state: &State,
    args: &[Value],
    macro_name: &str,
    package: &str,
) -> Result<Arc<RecordBatch>, AdapterError> {
    let result: Value = execute_macro_with_package(state, args, macro_name, package)?;
    convert_macro_result_to_record_batch(&result)
}

pub fn convert_macro_result_to_record_batch(
    result: &Value,
) -> Result<Arc<RecordBatch>, AdapterError> {
    // Depending on the macro impl, result can be either ResultObject or AgateTable
    let table = if let Some(result) = result.downcast_object::<ResultObject>() {
        result.table.as_ref().expect("AgateTable exists").to_owned()
    } else if let Some(result) = result.downcast_object::<AgateTable>() {
        result.as_ref().to_owned()
    } else {
        return Err(AdapterError::new(
            AdapterErrorKind::UnexpectedResult,
            format!("Unexpected result type {result}"),
        ));
    };

    let record_batch = table.original_record_batch();
    Ok(record_batch)
}

/// Execute a macro under a given package.
/// If you need to manipulate the result, checkout [`execute_macro_wrapper_with_package`]
///
/// # Panics
///
/// This function will panic if the macro named `{package_name}.{macro_name}` does not exist in the template state.
pub fn execute_macro_with_package(
    state: &State,
    args: &[Value],
    macro_name: &str,
    package: &str,
) -> Result<Value, AdapterError> {
    let template_name = format!("{package}.{macro_name}");
    let template = state.env().get_template(&template_name)?;
    let base_ctx = state.get_base_context();
    let state = template.eval_to_state(base_ctx, &[])?;
    let func = state
        .lookup(macro_name)
        .ok_or_else(|| AdapterError::new(
            AdapterErrorKind::UnexpectedResult, 
            format!("Macro '{macro_name}' not found in template")
        ))?;
    func.call(&state, args, &[])
        .map_err(|err| AdapterError::new(AdapterErrorKind::UnexpectedResult, err.to_string()))
}

/// Returns a value that represents the absence of a value of a Object method return.
pub fn none_value() -> Value {
    Value::from(())
}

pub fn empty_string_value() -> Value {
    Value::from("")
}

pub fn empty_vec_value() -> Value {
    Value::from(Vec::<Value>::new())
}

pub fn empty_mutable_vec_value() -> Value {
    Value::from(MutableVec::<Value>::new())
}

pub fn empty_map_value() -> Value {
    Value::from(BTreeMap::<Value, Value>::new())
}

// Helper function to format SQL with bindings
pub fn format_sql_with_bindings(
    adapter_type: AdapterType,
    sql: &str,
    bindings: &Value,
) -> AdapterResult<String> {
    let formatter = SqlLiteralFormatter::new(adapter_type);
    let mut result = String::with_capacity(sql.len());
    // this placeholder char is seen from `get_binding_char` macro
    let mut parts = sql.split("%s");
    let mut binding_iter = bindings.as_object().unwrap().try_iter().unwrap();

    // Add the first part (before any %s)
    if let Some(first) = parts.next() {
        result.push_str(first);
    }

    // For each remaining part, insert a binding value before it
    for part in parts {
        match binding_iter.next() {
            Some(value) => {
                // Convert minijinja::Value to a SQL-safe string
                match value.kind() {
                    ValueKind::String => {
                        result.push_str(&formatter.format_str(value.as_str().unwrap()))
                    }
                    ValueKind::Bytes => result.push_str(&formatter.format_bytes(&value)),
                    ValueKind::None => result.push_str(&formatter.none_value()),
                    ValueKind::Bool => result.push_str(&formatter.format_bool(value.is_true())),
                    _ => {
                        // TODO: handle the SQL escaping of more data types
                        if let Some(date) = value.downcast_object::<PyDate>() {
                            result.push_str(&formatter.format_date(date.as_ref().clone()));
                        } else if let Some(datetime) = value.downcast_object::<PyDateTime>() {
                            result.push_str(&formatter.format_datetime(datetime.as_ref().clone()));
                        } else {
                            result.push_str(&value.to_string())
                        }
                    }
                }
            }
            None => {
                return Err(AdapterError::new(
                    AdapterErrorKind::Configuration,
                    "Not enough bindings provided for SQL template".to_string(),
                ));
            }
        }
        result.push_str(part);
    }

    // Check if we used all bindings
    if binding_iter.next().is_some() {
        return Err(AdapterError::new(
            AdapterErrorKind::Configuration,
            "Too many bindings provided for SQL template".to_string(),
        ));
    }

    Ok(result)
}
