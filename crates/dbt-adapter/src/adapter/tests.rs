use std::collections::BTreeMap;

use super::*;
use crate::adapter::Adapter;
use crate::adapter::adapter_impl::AdapterImpl;
use crate::sql_types::SATypeOpsImpl;
use crate::stmt_splitter::NaiveStmtSplitter;
use dbt_adapter_core::AdapterType;
use dbt_common::cancellation::never_cancels;
use dbt_schemas::schemas::relations::{DEFAULT_DBT_QUOTING, DEFAULT_RESOLVED_QUOTING};
use indexmap::IndexMap;
use minijinja::value::Kwargs;

/// Helper to call [Adapter::call_method_impl] with jinja-valued arguments.
fn dispatch_test(
    adapter: &Arc<Adapter>,
    name: &str,
    args: &[Value],
) -> Result<Value, minijinja::Error> {
    let env = minijinja::Environment::new();
    let state = State::new_for_env(&env);
    adapter.call_method_impl(&state, name, args, &[])
}

/// Create a Typed-phase DuckDB adapter backed by MockEngine.
fn make_duckdb_adapter() -> Arc<Adapter> {
    let concrete = AdapterImpl::new_mock(
        AdapterType::DuckDB,
        BTreeMap::new(),
        DEFAULT_RESOLVED_QUOTING,
        Arc::new(SATypeOpsImpl::new(AdapterType::DuckDB)),
        Arc::new(NaiveStmtSplitter),
    );
    let adapter = Adapter::new(Arc::new(concrete), None, never_cancels());
    Arc::new(adapter)
}

/// Create a parse-phase DuckDB adapter (returns defaults, no real execution).
fn make_duckdb_parse_adapter() -> Arc<Adapter> {
    let adapter = Adapter::new_parse_phase_adapter(
        AdapterType::DuckDB,
        dbt_yaml::Mapping::new(),
        DEFAULT_DBT_QUOTING,
        Arc::new(SATypeOpsImpl::new(AdapterType::DuckDB)),
        None,
    );
    Arc::new(adapter)
}

/// Create a parse-phase ClickHouse adapter.
fn make_clickhouse_parse_adapter() -> Arc<Adapter> {
    let adapter = Adapter::new_parse_phase_adapter(
        AdapterType::ClickHouse,
        dbt_yaml::Mapping::new(),
        DEFAULT_DBT_QUOTING,
        Arc::new(SATypeOpsImpl::new(AdapterType::ClickHouse)),
        None,
    );
    Arc::new(adapter)
}

/// Helper to build a minijinja dict Value from key-value pairs.
fn dict(pairs: &[(&str, &str)]) -> Value {
    let map: IndexMap<String, Value> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), Value::from(*v)))
        .collect();
    Value::from(map)
}

// -- external_root tests --------------------------------------------------

#[test]
fn test_external_root_default() {
    let adapter = make_duckdb_adapter();
    let result = dispatch_test(&adapter, "external_root", &[]).unwrap();
    assert_eq!(result.as_str().unwrap(), ".");
}

// TODO: test external_root with custom config once MockAdapter supports custom AdapterConfig

// -- external_write_options tests (ported from dbt-duckdb test_external_utils.py) --

#[test]
fn test_external_write_options_csv_inferred() {
    let adapter = make_duckdb_adapter();
    let result = dispatch_test(
        &adapter,
        "external_write_options",
        &[Value::from("/tmp/test.csv"), dict(&[])],
    )
    .unwrap();
    assert_eq!(result.as_str().unwrap(), "format csv, header 1");
}

#[test]
fn test_external_write_options_parquet_with_codec() {
    let adapter = make_duckdb_adapter();
    let result = dispatch_test(
        &adapter,
        "external_write_options",
        &[Value::from("./foo.parquet"), dict(&[("codec", "zstd")])],
    )
    .unwrap();
    assert_eq!(result.as_str().unwrap(), "codec zstd, format parquet");
}

#[test]
fn test_external_write_options_delimiter_infers_csv() {
    let adapter = make_duckdb_adapter();
    let result = dispatch_test(
        &adapter,
        "external_write_options",
        &[
            Value::from("bar"),
            dict(&[("delimiter", "|"), ("header", "0")]),
        ],
    )
    .unwrap();
    assert_eq!(
        result.as_str().unwrap(),
        "delimiter '|', header 0, format csv"
    );
}

#[test]
fn test_external_write_options_partition_by_single() {
    let adapter = make_duckdb_adapter();
    let result = dispatch_test(
        &adapter,
        "external_write_options",
        &[Value::from("a.parquet"), dict(&[("partition_by", "ds")])],
    )
    .unwrap();
    assert_eq!(result.as_str().unwrap(), "partition_by ds, format parquet");
}

#[test]
fn test_external_write_options_partition_by_multi_adds_parens() {
    let adapter = make_duckdb_adapter();
    let result = dispatch_test(
        &adapter,
        "external_write_options",
        &[
            Value::from("b.csv"),
            dict(&[("partition_by", "ds,category")]),
        ],
    )
    .unwrap();
    assert_eq!(
        result.as_str().unwrap(),
        "partition_by (ds,category), format csv, header 1"
    );
}

#[test]
fn test_external_write_options_null_quoted() {
    let adapter = make_duckdb_adapter();
    let result = dispatch_test(
        &adapter,
        "external_write_options",
        &[Value::from("/path/to/c.csv"), dict(&[("null", "\\N")])],
    )
    .unwrap();
    assert_eq!(result.as_str().unwrap(), "null '\\N', format csv, header 1");
}

// -- external_read_location tests (ported from dbt-duckdb test_external_utils.py) --

#[test]
fn test_external_read_location_no_partition() {
    let adapter = make_duckdb_adapter();
    let result = dispatch_test(
        &adapter,
        "external_read_location",
        &[
            Value::from("bar"),
            dict(&[("format", "csv"), ("delimiter", "|"), ("header", "0")]),
        ],
    )
    .unwrap();
    assert_eq!(result.as_str().unwrap(), "bar");
}

#[test]
fn test_external_read_location_single_partition() {
    let adapter = make_duckdb_adapter();
    let result = dispatch_test(
        &adapter,
        "external_read_location",
        &[
            Value::from("/tmp/a"),
            dict(&[("partition_by", "ds"), ("format", "parquet")]),
        ],
    )
    .unwrap();
    assert_eq!(result.as_str().unwrap(), "/tmp/a/*/*.parquet");
}

#[test]
fn test_external_read_location_multi_partition() {
    let adapter = make_duckdb_adapter();
    let result = dispatch_test(
        &adapter,
        "external_read_location",
        &[Value::from("b"), dict(&[("partition_by", "ds,category")])],
    )
    .unwrap();
    assert_eq!(result.as_str().unwrap(), "b/*/*/*.parquet");
}

// -- location_exists tests ------------------------------------------------

#[test]
fn test_location_exists_parse_mode_returns_false() {
    let adapter = make_duckdb_parse_adapter();
    let result = dispatch_test(
        &adapter,
        "location_exists",
        &[Value::from("/nonexistent/path")],
    )
    .unwrap();
    // Parse-mode adapter always returns false
    assert_eq!(result, Value::from(false));
}

// -- get_relation kwarg dispatch tests ------------------------------------

/// Builds a real ClickHouse relation, reads its `.database` attribute via 
/// the Jinja accessor, and  passes that value back to `adapter.get_relation`
/// Breaks on a regression at any of the three layers (relation factory, Jinja accessor, or dispatch).
#[test]
fn test_get_relation_with_clickhouse_relation_database_attr_round_trips() {
    use crate::relation::{RelationObject, do_create_relation};
    use dbt_schemas::schemas::relations::base::BaseRelation;

    // `""` matches what production passes here (the `relation.database` kwarg).
    let relation: Arc<dyn BaseRelation> = Arc::from(
        do_create_relation(
            AdapterType::ClickHouse,
            String::new(),
            "default".to_string(),
            Some("stg_customers".to_string()),
            None,
            DEFAULT_RESOLVED_QUOTING,
        )
        .unwrap(),
    );
    let relation_value = RelationObject::new(relation).into_value();

    let database = relation_value.get_attr("database").unwrap();
    let schema = relation_value.get_attr("schema").unwrap();
    let identifier = relation_value.get_attr("identifier").unwrap();

    // Real string (not Jinja `none`) AND falsy — matches upstream's
    // `__post_init__` so `{% if relation.database %}` stays false.
    assert_eq!(database.as_str(), Some(""));
    assert!(!database.is_true());

    let adapter = make_clickhouse_parse_adapter();
    let kwargs = Kwargs::from_iter([
        ("database", database),
        ("schema", schema),
        ("identifier", identifier),
    ]);
    let result = dispatch_test(&adapter, "get_relation", &[Value::from(kwargs)])
        .expect("get_relation must accept database=relation.database for ClickHouse");
    assert!(!result.is_undefined() && !result.is_none());
}

/// Same end-to-end via the `api.Relation.create(database=None, ...)`
/// factory used by upstream macros (e.g. `clickhouse__get_or_create_relation`).
#[test]
fn test_get_relation_with_clickhouse_try_new_database_none_round_trips() {
    use crate::relation::{RelationObject, RelationStatic, StaticBaseRelation};

    let relation_type = RelationStatic {
        adapter_type: AdapterType::ClickHouse,
        quoting: DEFAULT_RESOLVED_QUOTING,
    };
    let relation_value = relation_type
        .try_new(
            None,
            Some("default".to_string()),
            Some("stg_customers".to_string()),
            None,
            Some(DEFAULT_RESOLVED_QUOTING),
            None,
        )
        .unwrap();

    let inner = relation_value
        .downcast_object::<RelationObject>()
        .unwrap();
    assert_eq!(inner.inner().database(), Some(""));

    let database = relation_value.get_attr("database").unwrap();
    let schema = relation_value.get_attr("schema").unwrap();
    let identifier = relation_value.get_attr("identifier").unwrap();
    assert_eq!(database.as_str(), Some(""));

    let adapter = make_clickhouse_parse_adapter();
    let kwargs = Kwargs::from_iter([
        ("database", database),
        ("schema", schema),
        ("identifier", identifier),
    ]);
    let result = dispatch_test(&adapter, "get_relation", &[Value::from(kwargs)])
        .expect("get_relation must accept database=relation.database after api.Relation.create(database=None, ...)");
    assert!(!result.is_undefined() && !result.is_none());
}
