use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dbt_adapter::relation::RelationObject;
use dbt_adapter_core::AdapterType;
use dbt_jinja_utils::mock_object::MockJinjaObject;
use dbt_schemas::dbt_types::RelationType;
use minijinja::Value;

use crate::macro_test_harness::{
    MacroTestHarness, assert_executed_contains, default_mock_config, executed_sql,
};

fn mv_macro_name(adapter_type: AdapterType) -> &'static str {
    match adapter_type {
        AdapterType::Databricks => "materialization_materialized_view_databricks",
        AdapterType::ClickHouse => "materialization_materialized_view_clickhouse",
        other => panic!("unsupported adapter for materialized view test: {other:?}"),
    }
}

fn alter_mv_macro_name(adapter_type: AdapterType) -> &'static str {
    match adapter_type {
        AdapterType::Databricks => "databricks__get_alter_materialized_view_as_sql",
        other => panic!("unsupported adapter for MV alter test: {other:?}"),
    }
}

fn render_mv(
    harness: &MacroTestHarness,
    adapter_type: AdapterType,
    ctx: BTreeMap<String, Value>,
) -> dbt_common::FsResult<String> {
    let call = format!("{{{{ {}() }}}}", mv_macro_name(adapter_type));
    harness.render(&call, ctx)
}

fn mv_config() -> Arc<MockJinjaObject> {
    let mock = default_mock_config();
    mock.on("get", |args| {
        let key = args.first().and_then(|v| v.as_str());
        let default = args.get(1).cloned().unwrap_or(Value::UNDEFINED);
        match key {
            Some("contract") => Ok(Value::from_serialize(BTreeMap::from([(
                "enforced".to_string(),
                Value::from(false),
            )]))),
            Some("full_refresh") => Ok(Value::from(false)),
            Some("on_configuration_change") => Ok(Value::from("apply")),
            _ => Ok(default),
        }
    });
    mock
}

/// Build a mock `configuration_changes` value
fn config_changes_mock(requires_full_refresh: bool, changes: BTreeMap<&str, Value>) -> Value {
    let mock = Arc::new(MockJinjaObject::new());
    mock.set_attr("requires_full_refresh", Value::from(requires_full_refresh));
    mock.set_attr("changes", Value::from_serialize(changes));
    Value::from_dyn_object(mock)
}

mod databricks {
    use super::*;

    const ADAPTER: AdapterType = AdapterType::Databricks;

    fn build_harness() -> MacroTestHarness {
        let harness = MacroTestHarness::for_adapter(ADAPTER)
            .load_all_macros()
            .with_stub_functions()
            .with_behavior_flag("use_materialization_v2", false)
            .build()
            .expect("harness should build");

        let mock = harness.mock();
        mock.on("clean_sql", |args| {
            Ok(args.first().cloned().unwrap_or(Value::UNDEFINED))
        });
        mock.on("get_column_tags_from_model", |_| Ok(Value::UNDEFINED));
        mock.on("drop_relation", |_| Ok(Value::UNDEFINED));
        mock.on("rename_relation", |_| Ok(Value::UNDEFINED));
        mock.on("commit", |_| Ok(Value::UNDEFINED));
        mock.on("resolve_file_format", |_| Ok(Value::from("delta")));
        mock.on("is_uniform", |_| Ok(Value::from(false)));
        mock.on("has_dbr_capability", |_| Ok(Value::from(false)));
        mock.on("get_relation_config", |_| Ok(Value::UNDEFINED));
        mock.on("get_columns_in_relation", |_| {
            Ok(Value::from(Vec::<Value>::new()))
        });
        mock.on("parse_columns_and_constraints", |_| {
            Ok(Value::from(vec![
                Value::from(Vec::<Value>::new()),
                Value::UNDEFINED,
            ]))
        });

        let refresh = Value::from_serialize(BTreeMap::from([
            ("cron", Value::UNDEFINED),
            ("time_zone_value", Value::UNDEFINED),
        ]));
        let model_config = Arc::new(MockJinjaObject::new());
        model_config.set_attr(
            "partitioned_by",
            Value::from_serialize(BTreeMap::from([("partition_by", Value::UNDEFINED)])),
        );
        model_config.set_attr(
            "tblproperties",
            Value::from_serialize(BTreeMap::from([("tblproperties", Value::UNDEFINED)])),
        );
        model_config.set_attr(
            "comment",
            Value::from_serialize(BTreeMap::from([("comment", Value::UNDEFINED)])),
        );
        model_config.set_attr("refresh", refresh);
        let model_config_val = Value::from_dyn_object(model_config);
        mock.on("get_config_from_model", move |_| {
            Ok(model_config_val.clone())
        });

        harness
    }

    fn render_alter(harness: &MacroTestHarness, changes: Value) -> String {
        let relation = harness.relation(
            "TEST_DB",
            "TEST_SCHEMA",
            "my_mv",
            Some(RelationType::MaterializedView),
        );
        let existing = harness.relation(
            "TEST_DB",
            "TEST_SCHEMA",
            "my_mv",
            Some(RelationType::MaterializedView),
        );
        let mut ctx = harness
            .materialization_context("my_mv", "SELECT 1")
            .relation_type(RelationType::MaterializedView)
            .config(Value::from_dyn_object(mv_config()))
            .with("relation", RelationObject::new(relation).into_value())
            .with("changes", changes)
            .with("sql_val", Value::from("SELECT 1"))
            .with("existing", RelationObject::new(existing).into_value())
            .build();
        ctx.insert(
            "model".to_string(),
            Value::from_serialize(BTreeMap::from([
                ("alias", Value::from("my_mv")),
                ("unique_id", Value::from("model.test_project.my_mv")),
                ("columns", Value::from(BTreeMap::<String, Value>::new())),
                ("constraints", Value::from(Vec::<Value>::new())),
            ])),
        );
        let macro_name = alter_mv_macro_name(ADAPTER);
        let call = format!(
            "{{% set r = {macro_name}(relation, changes, sql_val, existing, none, none) %}}{{{{ r }}}}"
        );
        harness
            .render(&call, ctx)
            .unwrap_or_else(|e| panic!("alter MV macro failed: {e:?}"))
    }

    #[test]
    fn no_existing_relation_creates_mv() {
        let harness = build_harness();
        harness.mock().on("get_relation", |_| Ok(Value::from(())));

        let ctx = harness
            .materialization_context("my_mv", "SELECT id, name FROM source")
            .relation_type(RelationType::MaterializedView)
            .config(Value::from_dyn_object(mv_config()))
            .build();

        render_mv(&harness, ADAPTER, ctx)
            .unwrap_or_else(|e| panic!("MV materialization failed: {e:?}"));

        assert_executed_contains(harness.mock(), "create or replace materialized view");
    }

    #[test]
    fn alter_full_refresh_with_partition_by_uses_drop_and_create() {
        let h = build_harness();
        let changes = config_changes_mock(
            true,
            BTreeMap::from([
                ("partition_by", Value::from(true)),
                ("tags", Value::UNDEFINED),
            ]),
        );
        let result = render_alter(&h, changes);
        let lower = result.to_lowercase();

        assert!(
            lower.contains("drop"),
            "Expected DROP SQL from drop_and_create path, got: {result}",
        );
        assert!(
            lower.contains("create"),
            "Expected CREATE SQL from drop_and_create path, got: {result}",
        );
    }

    #[test]
    fn alter_full_refresh_without_partition_by_uses_replace() {
        let h = build_harness();
        let changes =
            config_changes_mock(true, BTreeMap::from([("tags", Value::from("some_tag"))]));
        let result = render_alter(&h, changes);

        h.mock().observed_calls().assert_not_called("drop_relation");
        assert!(
            result.to_lowercase().contains("create or replace"),
            "Expected CREATE OR REPLACE when no partition_by change, got: {result}",
        );
    }

    #[test]
    fn alter_refresh_schedule_without_full_refresh() {
        let h = build_harness();
        let changes = config_changes_mock(
            false,
            BTreeMap::from([
                (
                    "refresh",
                    Value::from_serialize(BTreeMap::from([
                        ("cron", Value::from("0 0 * * *")),
                        ("time_zone_value", Value::from("UTC")),
                        ("is_altered", Value::from(true)),
                    ])),
                ),
                ("tags", Value::UNDEFINED),
            ]),
        );
        let result = render_alter(&h, changes);

        assert!(
            result.to_uppercase().contains("ALTER MATERIALIZED VIEW"),
            "Expected ALTER statement, got: {result}",
        );
    }

    #[test]
    fn alter_no_changes_returns_empty_list() {
        let h = build_harness();
        let changes = config_changes_mock(
            false,
            BTreeMap::from([("refresh", Value::UNDEFINED), ("tags", Value::UNDEFINED)]),
        );
        let result = render_alter(&h, changes);

        assert!(
            !result.contains("ALTER MATERIALIZED VIEW"),
            "Should not ALTER without changes, got: {result}",
        );
        assert!(
            !result.to_lowercase().contains("drop"),
            "Should not DROP without changes, got: {result}",
        );
    }
}

mod clickhouse {
    use super::*;

    const ADAPTER: AdapterType = AdapterType::ClickHouse;

    fn build_harness() -> MacroTestHarness {
        let harness = MacroTestHarness::for_adapter(ADAPTER)
            .load_all_macros()
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

    fn build_harness_with_target_table_lookup_guard() -> MacroTestHarness {
        let harness = MacroTestHarness::for_adapter(ADAPTER)
            .load_all_macros()
            .with_macro(
                "dbt",
                "load_cached_relation",
                r#"
                    {% macro load_cached_relation(relation) %}
                      {% if relation.identifier == 'events' and not relation.is_table %}
                        {{ exceptions.raise_compiler_error("Expected ClickHouse materialized_view existence lookup to use the table-typed target relation") }}
                      {% endif %}
                      {% do return(adapter.get_relation(
                        database=relation.database,
                        schema=relation.schema,
                        identifier=relation.identifier
                      )) -%}
                    {% endmacro %}
                "#,
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

    fn build_harness_with_noop_change_detection() -> MacroTestHarness {
        let harness = MacroTestHarness::for_adapter(ADAPTER)
            .load_all_macros()
            .with_stub_functions()
            .with_global(
                "target",
                Value::from_serialize(BTreeMap::<String, Value>::new()),
            )
            .build()
            .expect("harness should build");

        harness.mock().on("commit", |_| Ok(Value::UNDEFINED));
        harness.mock().on("drop_relation", |_| Ok(Value::UNDEFINED));
        let current_state =
            clickhouse_current_state("select id, count() as total from raw group by id");
        harness
            .mock()
            .on("describe_clickhouse_materialized_view", move |_| {
                Ok(current_state.clone())
            });
        harness.mock().on("get_columns_in_relation", |_| {
            Ok(clickhouse_columns(vec![
                ("id", "UInt64"),
                ("total", "UInt64"),
            ]))
        });
        harness.mock().on("get_column_schema_from_query", |_| {
            Ok(clickhouse_columns(vec![
                ("id", "UInt64"),
                ("total", "UInt64"),
            ]))
        });

        harness
    }

    fn build_harness_with_schema_rebuild_detection() -> MacroTestHarness {
        let harness = MacroTestHarness::for_adapter(ADAPTER)
            .load_all_macros()
            .with_stub_functions()
            .with_global(
                "target",
                Value::from_serialize(BTreeMap::<String, Value>::new()),
            )
            .build()
            .expect("harness should build");

        harness.mock().on("commit", |_| Ok(Value::UNDEFINED));
        harness.mock().on("drop_relation", |_| Ok(Value::UNDEFINED));
        let current_state =
            clickhouse_current_state("select id, sum(amount) as total from raw group by id");
        harness
            .mock()
            .on("describe_clickhouse_materialized_view", move |_| {
                Ok(current_state.clone())
            });
        harness.mock().on("get_columns_in_relation", |_| {
            Ok(clickhouse_columns(vec![
                ("id", "UInt64"),
                ("total", "UInt64"),
            ]))
        });
        harness.mock().on("get_column_schema_from_query", |_| {
            Ok(clickhouse_columns(vec![
                ("id", "UInt64"),
                ("total", "UInt64"),
                ("bucket", "Date"),
            ]))
        });
        harness
            .mock()
            .on("rename_relation", |_| Ok(Value::UNDEFINED));

        harness
    }

    fn build_harness_with_schema_query_guard() -> MacroTestHarness {
        let harness = MacroTestHarness::for_adapter(ADAPTER)
            .load_all_macros()
            .with_stub_functions()
            .with_global(
                "target",
                Value::from_serialize(BTreeMap::<String, Value>::new()),
            )
            .build()
            .expect("harness should build");

        harness.mock().on("commit", |_| Ok(Value::UNDEFINED));
        harness.mock().on("drop_relation", |_| Ok(Value::UNDEFINED));
        let current_state =
            clickhouse_current_state("select id, count() as total from raw group by id");
        harness
            .mock()
            .on("describe_clickhouse_materialized_view", move |_| {
                Ok(current_state.clone())
            });
        harness.mock().on("get_columns_in_relation", |_| {
            Ok(clickhouse_columns(vec![
                ("id", "UInt64"),
                ("total", "UInt64"),
            ]))
        });
        harness.mock().on("get_column_schema_from_query", |args| {
            let schema_query = args
                .first()
                .and_then(|arg| arg.as_str())
                .unwrap_or_default();
            let normalized = schema_query.to_lowercase();
            assert!(
                normalized.contains("select * from")
                    && normalized.contains("where false")
                    && normalized.contains("limit 0")
                    && normalized.contains("select id, count() as total from raw group by id"),
                "Expected desired schema lookup to use an empty subquery, got: {schema_query}",
            );
            Ok(clickhouse_columns(vec![
                ("id", "UInt64"),
                ("total", "UInt64"),
            ]))
        });

        harness
    }

    fn config_with_target_table(
        catchup: bool,
        target_table: Option<&'static str>,
    ) -> Arc<MockJinjaObject> {
        config_with_target_table_and_refreshable(catchup, target_table, Value::from(()))
    }

    fn config_with_target_table_and_refreshable(
        catchup: bool,
        target_table: Option<&'static str>,
        refreshable: Value,
    ) -> Arc<MockJinjaObject> {
        let mock = default_mock_config();
        mock.on("get", move |args| {
            let key = args.first().and_then(|v| v.as_str());
            let default = args.get(1).cloned().unwrap_or(Value::UNDEFINED);
            match key {
                Some("contract") => Ok(Value::from_serialize(BTreeMap::from([(
                    "enforced".to_string(),
                    Value::from(false),
                )]))),
                Some("catchup") => Ok(Value::from(catchup)),
                Some("engine") => Ok(Value::from("MergeTree()")),
                Some("order_by") => Ok(Value::from("id")),
                Some("refreshable") => Ok(refreshable.clone()),
                Some("full_refresh") => Ok(Value::from(false)),
                Some("on_configuration_change") => Ok(Value::from("apply")),
                Some("target_table") => Ok(target_table.map(Value::from).unwrap_or(default)),
                _ => Ok(default),
            }
        });
        mock
    }

    fn config(catchup: bool) -> Arc<MockJinjaObject> {
        config_with_target_table(catchup, None)
    }

    fn clickhouse_current_state(query: &str) -> Value {
        Value::from_serialize(BTreeMap::from([
            (
                "target_table",
                Value::from_serialize(BTreeMap::from([
                    ("engine", Value::from("MergeTree")),
                    ("order_by", Value::from("id")),
                    ("primary_key", Value::UNDEFINED),
                    ("partition_by", Value::UNDEFINED),
                    (
                        "create_table_query",
                        Value::from(
                            "CREATE TABLE `TEST_SCHEMA`.`events` ENGINE = MergeTree() ORDER BY id",
                        ),
                    ),
                ])),
            ),
            (
                "materialized_views",
                Value::from_serialize(BTreeMap::from([(
                    "events_mv",
                    Value::from_serialize(BTreeMap::from([
                        ("query", Value::from(query)),
                        (
                            "create_table_query",
                            Value::from(format!(
                                "CREATE MATERIALIZED VIEW `TEST_SCHEMA`.`events_mv` TO `TEST_SCHEMA`.`events` AS {query}",
                            )),
                        ),
                    ])),
                )])),
            ),
        ]))
    }

    fn clickhouse_current_state_without_generated_mv() -> Value {
        Value::from_serialize(BTreeMap::from([
            (
                "target_table",
                Value::from_serialize(BTreeMap::from([
                    ("engine", Value::from("MergeTree")),
                    ("order_by", Value::from("id")),
                    ("primary_key", Value::UNDEFINED),
                    ("partition_by", Value::UNDEFINED),
                    (
                        "create_table_query",
                        Value::from(
                            "CREATE TABLE `TEST_SCHEMA`.`events` ENGINE = MergeTree() ORDER BY id",
                        ),
                    ),
                ])),
            ),
            (
                "materialized_views",
                Value::from_serialize(BTreeMap::<String, Value>::new()),
            ),
        ]))
    }

    fn clickhouse_columns(columns: Vec<(&str, &str)>) -> Value {
        Value::from_serialize(
            columns
                .into_iter()
                .map(|(name, data_type)| BTreeMap::from([("name", name), ("data_type", data_type)]))
                .collect::<Vec<_>>(),
        )
    }

    fn render_apply_query_changes_result(
        harness: &MacroTestHarness,
        model_name: &str,
        query_changes: BTreeMap<&str, Value>,
    ) -> dbt_common::FsResult<String> {
        let target_relation = harness.relation(
            "TEST_DB",
            "TEST_SCHEMA",
            model_name,
            Some(RelationType::Table),
        );
        let views = BTreeMap::from([(
            "mv",
            Value::from("select id, sum(amount) as total from raw group by id"),
        )]);
        let ctx = harness
            .materialization_context(model_name, "select 1")
            .with("views", Value::from_serialize(views))
            .with("query_changes", Value::from_serialize(query_changes))
            .with(
                "target_relation",
                RelationObject::new(target_relation).into_value(),
            )
            .build();
        let call = "\
            {{ clickhouse__update_changed_mvs(target_relation, '', '', views, query_changes) }}\
        ";
        harness.render(call, ctx)
    }

    fn render_apply_query_changes(
        harness: &MacroTestHarness,
        model_name: &str,
        query_changes: BTreeMap<&str, Value>,
    ) {
        render_apply_query_changes_result(harness, model_name, query_changes)
            .unwrap_or_else(|e| panic!("ClickHouse MV query update macro failed: {e:?}"));
    }

    #[test]
    fn initial_create_uses_model_relation_as_destination_table() {
        let harness = build_harness();
        harness.mock().on("get_relation", |_| Ok(Value::from(())));

        let ctx = harness
            .materialization_context("events", "select id, count() as total from raw group by id")
            .config(Value::from_dyn_object(config(false)))
            .build();
        render_mv(&harness, ADAPTER, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse MV materialization failed: {e:?}"));
        let sqls = executed_sql(harness.mock()).join("\n").to_lowercase();

        assert!(
            sqls.contains("create table") && sqls.contains("`test_schema`.`events`"),
            "Expected target table creation for the model relation, got: {sqls}",
        );
        assert!(
            sqls.contains("create materialized view")
                && sqls.contains("`test_schema`.`events_mv`")
                && sqls.contains("to `test_schema`.`events`"),
            "Expected MV creation pointing to the model relation, got: {sqls}",
        );
        assert!(
            !sqls.contains("create materialized view if not exists"),
            "Generated MV creation should fail on name collisions instead of silently reusing them, got: {sqls}",
        );
    }

    #[test]
    fn existence_lookup_uses_model_relation_as_target_table() {
        let harness = build_harness_with_target_table_lookup_guard();
        harness.mock().on("get_relation", |_| Ok(Value::from(())));

        let ctx = harness
            .materialization_context("events", "select id, count() as total from raw group by id")
            .config(Value::from_dyn_object(config(false)))
            .build();
        render_mv(&harness, ADAPTER, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse MV materialization failed: {e:?}"));
    }

    #[test]
    fn initial_create_rejects_existing_generated_mv_name_collision() {
        let harness = build_harness();
        let generated_mv = harness.relation(
            "TEST_DB",
            "TEST_SCHEMA",
            "events_mv",
            Some(RelationType::MaterializedView),
        );
        let get_relation_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&get_relation_calls);
        harness.mock().on("get_relation", move |_| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(Value::from(()))
            } else {
                Ok(RelationObject::new(Arc::clone(&generated_mv)).into_value())
            }
        });

        let ctx = harness
            .materialization_context("events", "select id, count() as total from raw group by id")
            .config(Value::from_dyn_object(config(false)))
            .build();
        let err = render_mv(&harness, ADAPTER, ctx)
            .expect_err("Generated MV name collision should fail before creating target table");
        let err = err.to_string();

        assert!(
            err.contains("needs to create generated materialized view")
                && err.contains("already exists"),
            "Expected generated-MV collision error, got: {err}",
        );
        assert!(
            executed_sql(harness.mock()).is_empty(),
            "Name collision should fail before executing DDL, got: {:?}",
            executed_sql(harness.mock()),
        );
    }

    #[test]
    fn existing_non_table_relation_is_replaced_with_target_table_and_generated_mv() {
        let harness = build_harness();
        let existing_mv = harness.relation(
            "TEST_DB",
            "TEST_SCHEMA",
            "events",
            Some(RelationType::MaterializedView),
        );
        let get_relation_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&get_relation_calls);
        harness.mock().on("get_relation", move |_| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(RelationObject::new(Arc::clone(&existing_mv)).into_value())
            } else {
                Ok(Value::from(()))
            }
        });
        harness.mock().on("drop_relation", |_| Ok(Value::UNDEFINED));

        let ctx = harness
            .materialization_context("events", "select id, count() as total from raw group by id")
            .config(Value::from_dyn_object(config(false)))
            .build();
        render_mv(&harness, ADAPTER, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse MV materialization failed: {e:?}"));
        let sqls = executed_sql(harness.mock()).join("\n").to_lowercase();

        harness
            .mock()
            .observed_calls()
            .assert_called("drop_relation");
        harness
            .mock()
            .observed_calls()
            .assert_not_called("describe_clickhouse_materialized_view");
        assert!(
            sqls.contains("create table") && sqls.contains("`test_schema`.`events`"),
            "Expected replacement to create the model target table, got: {sqls}",
        );
        assert!(
            sqls.contains("create materialized view")
                && sqls.contains("`test_schema`.`events_mv`")
                && sqls.contains("to `test_schema`.`events`"),
            "Expected replacement to create generated MV pointing to target table, got: {sqls}",
        );
    }

    #[test]
    fn named_mv_sections_are_rejected_for_generated_mv_ux() {
        let harness = build_harness();
        harness.mock().on("get_relation", |_| Ok(Value::from(())));
        let ctx = harness
            .materialization_context(
                "events",
                "--daily:begin\nselect id, count() as total from raw group by id\n--daily:end",
            )
            .config(Value::from_dyn_object(config(false)))
            .build();

        let err = render_mv(&harness, ADAPTER, ctx).expect_err(
            "ClickHouse MV materialization should reject user-named MV sections in Fusion",
        );
        let err = err.to_string();

        assert!(
            err.contains("ClickHouse materialized_view models in Fusion create one generated materialized view"),
            "Expected generated-MV UX error, got: {err}",
        );
    }

    #[test]
    fn legacy_target_table_macro_is_rejected_for_generated_mv_ux() {
        let harness = build_harness();
        let ctx = harness
            .materialization_context("events", "select id from raw")
            .config(Value::from_dyn_object(config(false)))
            .build();

        let err = harness
            .render("{{ materialization_target_table(this) }}", ctx)
            .expect_err("ClickHouse MV materialization should reject legacy target-table macro");
        let err = err.to_string();

        assert!(
            err.contains("materialization_target_table() is not supported"),
            "Expected legacy target-table macro UX error, got: {err}",
        );
        assert!(
            executed_sql(harness.mock()).is_empty(),
            "Legacy target-table macro should fail before executing DDL, got: {:?}",
            executed_sql(harness.mock()),
        );
    }

    #[test]
    fn target_table_comment_markers_are_rejected_for_generated_mv_ux() {
        let harness = build_harness();
        harness.mock().on("get_relation", |_| Ok(Value::from(())));
        let ctx = harness
            .materialization_context(
                "events",
                "-- materialization_target_table: TEST_SCHEMA.other_events\nselect id from raw",
            )
            .config(Value::from_dyn_object(config(false)))
            .build();

        let err = render_mv(&harness, ADAPTER, ctx)
            .expect_err("ClickHouse MV materialization should reject stale target-table markers");
        let err = err.to_string();

        assert!(
            err.contains("materialization_target_table markers are not supported"),
            "Expected target-table marker UX error, got: {err}",
        );
        assert!(
            executed_sql(harness.mock()).is_empty(),
            "Target-table marker should fail before executing DDL, got: {:?}",
            executed_sql(harness.mock()),
        );
    }

    #[test]
    fn associated_mv_search_normalizes_quoted_to_targets() {
        let harness = build_harness();
        let ctx = harness
            .materialization_context("events", "select 1")
            .with("relation_schema", Value::from("TEST_SCHEMA"))
            .with("relation_name", Value::from("events"))
            .build();

        let sql = harness
            .render(
                "{{ clickhouse__associated_mvs_to_target_sql(relation_schema, relation_name) }}",
                ctx,
            )
            .unwrap_or_else(|e| panic!("associated MV search macro failed: {e:?}"));
        let sql = sql.to_lowercase();

        assert!(
            sql.contains("database = 'test_schema'"),
            "Expected associated MV search to stay scoped to the target schema, got: {sql}",
        );
        assert!(
            sql.contains("replaceregexpall")
                && sql.contains("extract(")
                && sql.contains("'test_schema.events'")
                && sql.contains("'events'"),
            "Expected associated MV search to normalize quoted and unqualified TO targets, got: {sql}",
        );
        assert!(
            !sql.contains('?'),
            "Expected associated MV search to avoid ADBC parameter-marker false positives, got: {sql}",
        );
    }

    #[test]
    fn explicit_target_table_config_is_rejected_for_generated_mv_ux() {
        let harness = build_harness();
        let ctx = harness
            .materialization_context("events", "select id, count() as total from raw group by id")
            .config(Value::from_dyn_object(config_with_target_table(
                false,
                Some("other_events"),
            )))
            .build();

        let err = render_mv(&harness, ADAPTER, ctx)
            .expect_err("ClickHouse MV materialization should reject explicit target_table config");
        let err = err.to_string();

        assert!(
            err.contains("target_table config is not supported"),
            "Expected explicit target-table UX error, got: {err}",
        );
        assert!(
            executed_sql(harness.mock()).is_empty(),
            "Explicit target-table config should fail before executing DDL, got: {:?}",
            executed_sql(harness.mock()),
        );
    }

    #[test]
    fn explicit_target_table_config_is_rejected_before_refreshable_validation() {
        let harness = build_harness();
        let ctx = harness
            .materialization_context("events", "select id, count() as total from raw group by id")
            .config(Value::from_dyn_object(
                config_with_target_table_and_refreshable(
                    false,
                    Some("other_events"),
                    Value::from("bad_refreshable_config"),
                ),
            ))
            .build();

        let err = render_mv(&harness, ADAPTER, ctx)
            .expect_err("ClickHouse MV materialization should reject explicit target_table config");
        let err = err.to_string();

        assert!(
            err.contains("target_table config is not supported"),
            "Expected target-table UX error before refreshable validation, got: {err}",
        );
        assert!(
            !err.contains("refreshable"),
            "Explicit target-table config should be reported before refreshable validation, got: {err}",
        );
    }

    #[test]
    fn unchanged_materialization_records_skip_without_running_sql() {
        let harness = build_harness_with_noop_change_detection();
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
            .config(Value::from_dyn_object(config(false)))
            .build();
        render_mv(&harness, ADAPTER, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse MV materialization failed: {e:?}"));

        assert!(
            executed_sql(harness.mock()).is_empty(),
            "No-op materialization should not execute SQL, got: {:?}",
            executed_sql(harness.mock()),
        );
        harness
            .mock()
            .observed_calls()
            .assert_not_called("drop_relation");
    }

    #[test]
    fn existing_materialization_reads_clickhouse_state_through_adapter() {
        let harness = build_harness_with_noop_change_detection();
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
            .config(Value::from_dyn_object(config(false)))
            .build();
        render_mv(&harness, ADAPTER, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse MV materialization failed: {e:?}"));

        harness
            .mock()
            .observed_calls()
            .assert_called("describe_clickhouse_materialized_view");
    }

    #[test]
    fn desired_schema_lookup_uses_empty_subquery() {
        let harness = build_harness_with_schema_query_guard();
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
            .config(Value::from_dyn_object(config(false)))
            .build();
        render_mv(&harness, ADAPTER, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse MV materialization failed: {e:?}"));

        harness
            .mock()
            .observed_calls()
            .assert_called("get_column_schema_from_query");
    }

    #[test]
    fn existing_target_table_rejects_unowned_generated_mv_name_collision() {
        let harness = build_harness();
        let existing_target = harness.relation(
            "TEST_DB",
            "TEST_SCHEMA",
            "events",
            Some(RelationType::Table),
        );
        let generated_mv = harness.relation(
            "TEST_DB",
            "TEST_SCHEMA",
            "events_mv",
            Some(RelationType::MaterializedView),
        );
        let get_relation_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&get_relation_calls);
        harness.mock().on("get_relation", move |_| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(RelationObject::new(Arc::clone(&existing_target)).into_value())
            } else {
                Ok(RelationObject::new(Arc::clone(&generated_mv)).into_value())
            }
        });
        let current_state = clickhouse_current_state_without_generated_mv();
        harness
            .mock()
            .on("describe_clickhouse_materialized_view", move |_| {
                Ok(current_state.clone())
            });

        let ctx = harness
            .materialization_context("events", "select id, count() as total from raw group by id")
            .config(Value::from_dyn_object(config(false)))
            .build();
        let err = render_mv(&harness, ADAPTER, ctx)
            .expect_err("Unowned generated MV name collision should fail before applying changes");
        let err = err.to_string();

        assert!(
            err.contains("needs generated materialized view")
                && err.contains("does not point to the model target table"),
            "Expected unowned generated-MV collision error, got: {err}",
        );
        assert!(
            executed_sql(harness.mock()).is_empty(),
            "Unowned generated MV should fail before executing DDL, got: {:?}",
            executed_sql(harness.mock()),
        );
        harness
            .mock()
            .observed_calls()
            .assert_not_called("get_columns_in_relation");
    }

    #[test]
    fn applying_query_change_uses_modify_query_without_recreating_mv() {
        let harness = build_harness();
        render_apply_query_changes(
            &harness,
            "events",
            BTreeMap::from([
                ("changed", Value::from_serialize(vec!["events_mv"])),
                ("missing", Value::from_serialize(Vec::<String>::new())),
                ("recreate", Value::from_serialize(Vec::<String>::new())),
            ]),
        );
        let sqls = executed_sql(harness.mock()).join("\n").to_lowercase();

        assert!(
            sqls.contains("alter table") && sqls.contains("modify query"),
            "Expected ALTER TABLE MODIFY QUERY, got: {sqls}",
        );
        assert!(
            !sqls.contains("create materialized view"),
            "Query-only changes should not recreate the MV, got: {sqls}",
        );
    }

    #[test]
    fn applying_refreshable_change_recreates_mv_without_rebuilding_target() {
        let harness = build_harness();
        render_apply_query_changes(
            &harness,
            "events",
            BTreeMap::from([
                ("changed", Value::from_serialize(Vec::<String>::new())),
                ("missing", Value::from_serialize(Vec::<String>::new())),
                ("recreate", Value::from_serialize(vec!["events_mv"])),
            ]),
        );
        let sqls = executed_sql(harness.mock()).join("\n").to_lowercase();

        assert!(
            sqls.contains("drop view if exists") && sqls.contains("create materialized view"),
            "Expected MV drop and recreate, got: {sqls}",
        );
        assert!(
            !sqls.contains("create table"),
            "MV-level config changes should not rebuild the target table, got: {sqls}",
        );
    }

    #[test]
    fn applying_missing_generated_mv_creates_it_when_name_is_available() {
        let harness = build_harness();
        harness.mock().on("get_relation", |_| Ok(Value::from(())));
        render_apply_query_changes(
            &harness,
            "events",
            BTreeMap::from([
                ("changed", Value::from_serialize(Vec::<String>::new())),
                ("missing", Value::from_serialize(vec!["events_mv"])),
                ("recreate", Value::from_serialize(Vec::<String>::new())),
            ]),
        );
        let sqls = executed_sql(harness.mock()).join("\n").to_lowercase();

        assert!(
            sqls.contains("create materialized view")
                && sqls.contains("`test_schema`.`events_mv`")
                && sqls.contains("to `test_schema`.`events`"),
            "Expected missing generated MV to be created, got: {sqls}",
        );
    }

    #[test]
    fn applying_missing_generated_mv_rejects_name_collision() {
        let harness = build_harness();
        let generated_mv = harness.relation(
            "TEST_DB",
            "TEST_SCHEMA",
            "events_mv",
            Some(RelationType::MaterializedView),
        );
        harness.mock().on("get_relation", move |_| {
            Ok(RelationObject::new(Arc::clone(&generated_mv)).into_value())
        });
        let err = render_apply_query_changes_result(
            &harness,
            "events",
            BTreeMap::from([
                ("changed", Value::from_serialize(Vec::<String>::new())),
                ("missing", Value::from_serialize(vec!["events_mv"])),
                ("recreate", Value::from_serialize(Vec::<String>::new())),
            ]),
        )
        .expect_err("Generated MV name collision should fail before creating a missing MV");
        let err = err.to_string();

        assert!(
            err.contains("needs to create generated materialized view")
                && err.contains("already exists"),
            "Expected generated-MV collision error, got: {err}",
        );
        assert!(
            executed_sql(harness.mock()).is_empty(),
            "Name collision should fail before executing DDL, got: {:?}",
            executed_sql(harness.mock()),
        );
    }

    #[test]
    fn applying_query_shape_change_rebuilds_target_table() {
        let harness = build_harness_with_schema_rebuild_detection();
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
            .materialization_context(
                "events",
                "select id, sum(amount) as total, toDate(ts) as bucket from raw group by id, bucket",
            )
            .config(Value::from_dyn_object(config(false)))
            .build();
        render_mv(&harness, ADAPTER, ctx)
            .unwrap_or_else(|e| panic!("ClickHouse MV materialization failed: {e:?}"));
        let sqls = executed_sql(harness.mock()).join("\n").to_lowercase();

        assert!(
            sqls.contains("create table") && sqls.contains("events__dbt_tmp"),
            "Expected target table replacement for query shape changes, got: {sqls}",
        );
        assert!(
            !sqls.contains("modify query"),
            "Query shape changes should not use MODIFY QUERY, got: {sqls}",
        );
    }
}
