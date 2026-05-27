use std::collections::BTreeMap;
use std::sync::Arc;

use dbt_adapter::relation::RelationObject;
use dbt_adapter_core::AdapterType;
use minijinja::Value;

use dbt_schemas::dbt_types::RelationType;

use crate::macro_test_harness::{MacroTestHarness, default_mock_config, executed_sql};

const ADAPTER: AdapterType = AdapterType::ClickHouse;

fn render_table(
    harness: &MacroTestHarness,
    ctx: BTreeMap<String, Value>,
) -> dbt_common::FsResult<String> {
    harness.render("{{ materialization_table_clickhouse() }}", ctx)
}

fn build_clickhouse_table_harness_with_associated_mv_search(
    found_associated_mvs: &[&str],
) -> MacroTestHarness {
    let found_associated_mvs = found_associated_mvs
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let search_noop = r#"
        {% macro clickhouse__search_associated_mvs_to_target(relation_schema, relation_name, mv_suffixes) %}
          {{ return(([__FOUND_ASSOCIATED_MVS__], [relation_name ~ '_mv'])) }}
        {% endmacro %}
    "#
    .replace("__FOUND_ASSOCIATED_MVS__", &found_associated_mvs);

    let harness = MacroTestHarness::for_adapter(ADAPTER)
        .load_all_macros()
        .with_macro(
            "dbt_clickhouse",
            "clickhouse__search_associated_mvs_to_target",
            &search_noop,
        )
        .with_stub_functions()
        .with_global(
            "target",
            Value::from_serialize(BTreeMap::<String, Value>::new()),
        )
        .build()
        .expect("harness should build");

    harness.mock().on("commit", |_| Ok(Value::UNDEFINED));

    harness
}

#[test]
fn existing_table_drops_generated_mv_before_create_or_replace() {
    let harness = build_clickhouse_table_harness_with_associated_mv_search(&["events_mv"]);
    let existing = harness.relation(
        "TEST_DB",
        "TEST_SCHEMA",
        "events",
        Some(RelationType::Table),
    );
    harness.mock().on("get_relation", move |_| {
        Ok(RelationObject::new(Arc::clone(&existing)).into_value())
    });

    let ctx = harness
        .materialization_context("events", "select id, count() as total from raw group by id")
        .config(Value::from_dyn_object(default_mock_config()))
        .build();
    render_table(&harness, ctx)
        .unwrap_or_else(|e| panic!("ClickHouse table materialization failed: {e:?}"));

    let sqls = executed_sql(harness.mock());
    let drop_index = sqls
        .iter()
        .position(|sql| {
            let sql = sql.to_lowercase();
            sql.contains("drop view if exists") && sql.contains("`test_schema`.`events_mv`")
        })
        .expect("expected generated MV drop");
    let create_index = sqls
        .iter()
        .position(|sql| sql.to_lowercase().contains("create or replace table"))
        .expect("expected create or replace table statement");

    assert!(
        drop_index < create_index,
        "Expected generated MV drop before table replacement, got: {sqls:?}",
    );
}

#[test]
fn orphan_generated_mv_is_dropped_before_new_table_create() {
    let harness = build_clickhouse_table_harness_with_associated_mv_search(&["events_mv"]);
    harness.mock().on("get_relation", |_| Ok(Value::from(())));

    let ctx = harness
        .materialization_context("events", "select id, count() as total from raw group by id")
        .config(Value::from_dyn_object(default_mock_config()))
        .build();
    render_table(&harness, ctx)
        .unwrap_or_else(|e| panic!("ClickHouse table materialization failed: {e:?}"));

    let sqls = executed_sql(harness.mock());
    let drop_index = sqls
        .iter()
        .position(|sql| {
            let sql = sql.to_lowercase();
            sql.contains("drop view if exists") && sql.contains("`test_schema`.`events_mv`")
        })
        .expect("expected orphan generated MV drop");
    let create_index = sqls
        .iter()
        .position(|sql| sql.to_lowercase().contains("create or replace table"))
        .expect("expected create or replace table statement");

    assert!(
        drop_index < create_index,
        "Expected orphan generated MV drop before table replacement, got: {sqls:?}",
    );
}

#[test]
fn new_table_does_not_drop_unassociated_same_name_mv() {
    let harness = build_clickhouse_table_harness_with_associated_mv_search(&[]);
    harness.mock().on("get_relation", |_| Ok(Value::from(())));

    let ctx = harness
        .materialization_context("events", "select id, count() as total from raw group by id")
        .config(Value::from_dyn_object(default_mock_config()))
        .build();
    render_table(&harness, ctx)
        .unwrap_or_else(|e| panic!("ClickHouse table materialization failed: {e:?}"));

    let sqls = executed_sql(harness.mock());

    assert!(
        sqls.iter()
            .all(|sql| !sql.to_lowercase().contains("drop view if exists")),
        "Expected no generated MV drop when associated-MV search does not prove ownership, got: {sqls:?}",
    );
}
