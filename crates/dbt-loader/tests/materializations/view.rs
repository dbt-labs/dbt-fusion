use std::collections::BTreeMap;
use std::sync::Arc;

use dbt_adapter::relation::RelationObject;
use dbt_adapter_core::AdapterType;
use dbt_jinja_utils::mock_object::MockJinjaObject;
use dbt_schemas::dbt_types::RelationType;
use minijinja::Value;

use crate::macro_test_harness::{
    MacroTestHarness, assert_executed_contains, default_mock_config, executed_sql,
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn view_macro_name(adapter_type: AdapterType) -> &'static str {
    match adapter_type {
        AdapterType::Snowflake => "materialization_view_snowflake",
        AdapterType::Databricks => "materialization_view_databricks",
        AdapterType::Bigquery => "materialization_view_bigquery",
        AdapterType::Redshift => "materialization_view_redshift",
        AdapterType::Postgres => "materialization_view_default",
        other => panic!("unsupported adapter for view materialization test: {other:?}"),
    }
}

fn build_view_harness(adapter_type: AdapterType) -> MacroTestHarness {
    let harness = MacroTestHarness::for_adapter(adapter_type)
        .load_all_macros()
        .with_stub_functions()
        .with_behavior_flag("use_materialization_v2", false)
        .build()
        .expect("harness should build");

    let mock = harness.mock();
    mock.on("rename_relation", |_| Ok(Value::UNDEFINED));
    mock.on("drop_relation", |_| Ok(Value::UNDEFINED));
    mock.on("commit", |_| Ok(Value::UNDEFINED));
    mock.on("clean_sql", |args| {
        Ok(args.first().cloned().unwrap_or(Value::UNDEFINED))
    });
    mock.on("get_column_tags_from_model", |_| Ok(Value::UNDEFINED));
    mock.on("get_view_options", |_| {
        Ok(Value::from(BTreeMap::<String, Value>::new()))
    });
    mock.on("resolve_file_format", |_| Ok(Value::from("delta")));
    mock.on("is_uniform", |_| Ok(Value::from(false)));
    mock.on("has_dbr_capability", |_| Ok(Value::from(false)));

    harness
}

fn render_view(
    harness: &MacroTestHarness,
    adapter_type: AdapterType,
    ctx: BTreeMap<String, Value>,
) -> dbt_common::FsResult<String> {
    let call = format!("{{{{ {}() }}}}", view_macro_name(adapter_type));
    harness.render(&call, ctx)
}

fn full_refresh_mock_config() -> Arc<MockJinjaObject> {
    let mock = default_mock_config();
    mock.on("get", |args| {
        let key = args.first().and_then(|v| v.as_str());
        let default = args.get(1).cloned().unwrap_or(Value::UNDEFINED);
        match key {
            Some("contract") => Ok(Value::from_serialize(BTreeMap::from([(
                "enforced".to_string(),
                Value::from(false),
            )]))),
            Some("full_refresh") => Ok(Value::from(true)),
            _ => Ok(default),
        }
    });
    mock
}

// Scenarios:
// 1. Nothing already exists under the target namespace
// 2. A view already exists under the target namespace
// 3. A table already exists under the target namespace

/// Shorthand: build harness, mock get_relation → None, render, assert success.
fn run_no_existing(adapter_type: AdapterType) -> MacroTestHarness {
    let harness = build_view_harness(adapter_type);
    harness.mock().on("get_relation", |_| Ok(Value::from(())));
    let ctx = harness
        .materialization_context("my_view", "SELECT id, name FROM source_table")
        .build();
    render_view(&harness, adapter_type, ctx)
        .unwrap_or_else(|e| panic!("{adapter_type:?} view materialization failed: {e:?}"));
    harness
}

/// Shorthand: build harness, mock get_relation → existing view, render, assert success.
fn run_existing_view(adapter_type: AdapterType) -> MacroTestHarness {
    let harness = build_view_harness(adapter_type);
    let existing = harness.relation(
        "TEST_DB",
        "TEST_SCHEMA",
        "my_view",
        Some(RelationType::View),
    );
    harness.mock().on("get_relation", move |_| {
        Ok(RelationObject::new(Arc::clone(&existing)).into_value())
    });
    let ctx = harness
        .materialization_context("my_view", "SELECT id, name FROM source_table")
        .build();
    render_view(&harness, adapter_type, ctx)
        .unwrap_or_else(|e| panic!("{adapter_type:?} view materialization failed: {e:?}"));
    harness
}

/// Shorthand: build harness, mock get_relation → existing table, render with
/// full_refresh config, assert success.
fn run_existing_table(adapter_type: AdapterType) -> MacroTestHarness {
    let harness = build_view_harness(adapter_type);
    let existing = harness.relation(
        "TEST_DB",
        "TEST_SCHEMA",
        "my_view",
        Some(RelationType::Table),
    );
    harness.mock().on("get_relation", move |_| {
        Ok(RelationObject::new(Arc::clone(&existing)).into_value())
    });
    harness.mock().on("drop_relation", |_| Ok(Value::from(())));
    let ctx = harness
        .materialization_context("my_view", "SELECT id, name FROM source_table")
        .config(Value::from_dyn_object(full_refresh_mock_config()))
        .build();
    render_view(&harness, adapter_type, ctx)
        .unwrap_or_else(|e| panic!("{adapter_type:?} view materialization failed: {e:?}"));
    harness
}

mod snowflake {
    use super::*;
    const ADAPTER: AdapterType = AdapterType::Snowflake;

    #[test]
    fn no_existing_relation() {
        let h = run_no_existing(ADAPTER);
        h.mock().observed_calls().assert_not_called("drop_relation");
        assert_executed_contains(h.mock(), "create or replace");
    }

    #[test]
    fn existing_view_replaced_without_drop() {
        let h = run_existing_view(ADAPTER);
        h.mock().observed_calls().assert_not_called("drop_relation");
        assert_executed_contains(h.mock(), "create or replace");
    }

    #[test]
    fn existing_table_dropped_before_create() {
        let h = run_existing_table(ADAPTER);
        h.mock().observed_calls().assert_called("drop_relation");
        assert_executed_contains(h.mock(), "create or replace");
    }
}

mod databricks {
    use super::*;
    const ADAPTER: AdapterType = AdapterType::Databricks;

    // -- use_materialization_v2 = false, default --------------------

    #[test]
    fn no_existing_relation() {
        let h = run_no_existing(ADAPTER);
        h.mock().observed_calls().assert_not_called("drop_relation");
        assert_executed_contains(h.mock(), "create or replace");
    }

    #[test]
    fn existing_view_replaced_without_drop() {
        let h = run_existing_view(ADAPTER);
        h.mock().observed_calls().assert_not_called("drop_relation");
        assert_executed_contains(h.mock(), "create or replace");
    }

    #[test]
    fn existing_table_dropped_before_create() {
        let h = run_existing_table(ADAPTER);
        h.mock().observed_calls().assert_called("drop_relation");
        assert_executed_contains(h.mock(), "create or replace");
    }

    // -- use_materialization_v2 = true ----------------------------------

    #[test]
    fn v2_no_existing_relation() {
        let harness = build_view_harness(ADAPTER);
        harness.set_behavior_flags([("use_materialization_v2", true)]);
        harness.mock().on("get_relation", |_| Ok(Value::from(())));

        let ctx = harness
            .materialization_context("my_view", "SELECT id, name FROM source_table")
            .build();
        render_view(&harness, ADAPTER, ctx).expect("v2 materialization should succeed");

        harness
            .mock()
            .observed_calls()
            .assert_not_called("drop_relation");
        assert_executed_contains(harness.mock(), "create or replace");
    }

    #[test]
    fn v2_existing_view_replaced() {
        let harness = build_view_harness(ADAPTER);
        harness.set_behavior_flags([("use_materialization_v2", true)]);
        let existing = harness.relation(
            "TEST_DB",
            "TEST_SCHEMA",
            "my_view",
            Some(RelationType::View),
        );
        harness.mock().on("get_relation", move |_| {
            Ok(RelationObject::new(Arc::clone(&existing)).into_value())
        });

        let ctx = harness
            .materialization_context("my_view", "SELECT id, name FROM source_table")
            .build();
        render_view(&harness, ADAPTER, ctx).expect("v2 materialization should succeed");

        harness
            .mock()
            .observed_calls()
            .assert_not_called("drop_relation");
        assert_executed_contains(harness.mock(), "create");
    }
}

mod bigquery {
    use super::*;
    const ADAPTER: AdapterType = AdapterType::Bigquery;

    #[test]
    fn no_existing_relation() {
        let h = run_no_existing(ADAPTER);
        h.mock().observed_calls().assert_not_called("drop_relation");
        assert_executed_contains(h.mock(), "create or replace");
    }

    #[test]
    fn existing_view_replaced_without_drop() {
        let h = run_existing_view(ADAPTER);
        h.mock().observed_calls().assert_not_called("drop_relation");
        assert_executed_contains(h.mock(), "create or replace");
    }

    #[test]
    fn existing_table_dropped_before_create() {
        let h = run_existing_table(ADAPTER);
        h.mock().observed_calls().assert_called("drop_relation");
        assert_executed_contains(h.mock(), "create or replace");
    }
}

mod redshift {
    use super::*;
    const ADAPTER: AdapterType = AdapterType::Redshift;

    #[test]
    fn no_existing_relation() {
        let h = run_no_existing(ADAPTER);
        h.mock().observed_calls().assert_called("rename_relation");
        assert_executed_contains(h.mock(), "create");
    }

    #[test]
    fn existing_view_swapped_via_rename() {
        let h = run_existing_view(ADAPTER);
        h.mock().observed_calls().assert_called("rename_relation");
        assert_executed_contains(h.mock(), "create");
    }

    #[test]
    fn existing_table_swapped_via_rename() {
        let h = run_existing_table(ADAPTER);
        h.mock().observed_calls().assert_called("rename_relation");
        assert_executed_contains(h.mock(), "create");
    }
}

mod postgres {
    use super::*;
    const ADAPTER: AdapterType = AdapterType::Postgres;

    #[test]
    fn no_existing_relation() {
        let h = run_no_existing(ADAPTER);
        h.mock().observed_calls().assert_called("rename_relation");
        assert_executed_contains(h.mock(), "create");
    }

    #[test]
    fn existing_view_swapped_via_rename() {
        let h = run_existing_view(ADAPTER);
        h.mock().observed_calls().assert_called("rename_relation");
        assert_executed_contains(h.mock(), "create");
    }

    #[test]
    fn existing_table_swapped_via_rename() {
        let h = run_existing_table(ADAPTER);
        h.mock().observed_calls().assert_called("rename_relation");
        assert_executed_contains(h.mock(), "create");
    }
}

mod clickhouse {
    use super::*;

    const ADAPTER: AdapterType = AdapterType::ClickHouse;

    fn render_clickhouse_view(
        harness: &MacroTestHarness,
        ctx: BTreeMap<String, Value>,
    ) -> dbt_common::FsResult<String> {
        harness.render("{{ materialization_view_clickhouse() }}", ctx)
    }

    fn build_harness_with_associated_mv_search(found_associated_mvs: &[&str]) -> MacroTestHarness {
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

    fn build_harness_with_failing_associated_mv_search() -> MacroTestHarness {
        let search_should_not_be_called = r#"
            {% macro clickhouse__search_associated_mvs_to_target(relation_schema, relation_name, mv_suffixes) %}
              {{ exceptions.raise_compiler_error('associated MV search should not run for a new ClickHouse view') }}
            {% endmacro %}
        "#;

        let harness = MacroTestHarness::for_adapter(ADAPTER)
            .load_all_macros()
            .with_macro(
                "dbt_clickhouse",
                "clickhouse__search_associated_mvs_to_target",
                search_should_not_be_called,
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

    fn clickhouse_view_context(harness: &MacroTestHarness) -> BTreeMap<String, Value> {
        let mut ctx = harness
            .materialization_context("events", "select id, count() as total from raw group by id")
            .config(Value::from_dyn_object(default_mock_config()))
            .build();
        ctx.insert(
            "model".to_string(),
            Value::from_serialize(BTreeMap::from([
                ("alias", Value::from("events")),
                ("unique_id", Value::from("model.test_project.events")),
                ("columns", Value::from(BTreeMap::<String, Value>::new())),
                (
                    "config",
                    Value::from_serialize(BTreeMap::from([("materialized", Value::from("view"))])),
                ),
            ])),
        );
        ctx
    }

    fn assert_generated_mv_drop_before_view_create(sqls: &[String]) {
        let drop_index = sqls
            .iter()
            .position(|sql| {
                let sql = sql.to_lowercase();
                sql.contains("drop view if exists") && sql.contains("`test_schema`.`events_mv`")
            })
            .expect("expected generated MV drop");
        let create_index = sqls
            .iter()
            .position(|sql| sql.to_lowercase().contains("create or replace view"))
            .expect("expected create or replace view statement");

        assert!(
            drop_index < create_index,
            "Expected generated MV drop before view replacement, got: {sqls:?}",
        );
    }

    #[test]
    fn existing_view_drops_generated_mv_before_create_or_replace() {
        let harness = build_harness_with_associated_mv_search(&["events_mv"]);
        let existing =
            harness.relation("TEST_DB", "TEST_SCHEMA", "events", Some(RelationType::View));
        harness.mock().on("get_relation", move |_| {
            Ok(RelationObject::new(Arc::clone(&existing)).into_value())
        });

        let ctx = clickhouse_view_context(&harness);
        render_clickhouse_view(&harness, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse view materialization failed: {e:?}"));

        let sqls = executed_sql(harness.mock());
        assert_generated_mv_drop_before_view_create(&sqls);
    }

    #[test]
    fn existing_table_drops_generated_mv_before_create_or_replace() {
        let harness = build_harness_with_associated_mv_search(&["events_mv"]);
        let existing = harness.relation(
            "TEST_DB",
            "TEST_SCHEMA",
            "events",
            Some(RelationType::Table),
        );
        harness.mock().on("get_relation", move |_| {
            Ok(RelationObject::new(Arc::clone(&existing)).into_value())
        });

        let ctx = clickhouse_view_context(&harness);
        render_clickhouse_view(&harness, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse view materialization failed: {e:?}"));

        let sqls = executed_sql(harness.mock());
        assert_generated_mv_drop_before_view_create(&sqls);
    }

    #[test]
    fn no_existing_relation_does_not_search_associated_mvs() {
        let harness = build_harness_with_failing_associated_mv_search();
        harness.mock().on("get_relation", |_| Ok(Value::from(())));

        let ctx = clickhouse_view_context(&harness);
        render_clickhouse_view(&harness, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse view materialization failed: {e:?}"));

        let sqls = executed_sql(harness.mock());

        assert!(
            sqls.iter()
                .all(|sql| !sql.to_lowercase().contains("drop view if exists")),
            "Expected no generated MV drop for first-run ClickHouse view, got: {sqls:?}",
        );
    }

    #[test]
    fn new_view_does_not_drop_unassociated_same_name_mv() {
        let harness = build_harness_with_associated_mv_search(&[]);
        harness.mock().on("get_relation", |_| Ok(Value::from(())));

        let ctx = clickhouse_view_context(&harness);
        render_clickhouse_view(&harness, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse view materialization failed: {e:?}"));

        let sqls = executed_sql(harness.mock());

        assert!(
            sqls.iter()
                .all(|sql| !sql.to_lowercase().contains("drop view if exists")),
            "Expected no generated MV drop when associated-MV search does not prove ownership, got: {sqls:?}",
        );
    }
}
