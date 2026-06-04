use std::collections::BTreeMap;

use dbt_adapter_core::AdapterType;
use minijinja::Value;

use crate::macro_test_harness::MacroTestHarness;

mod clickhouse {
    use super::*;

    fn build_harness() -> MacroTestHarness {
        MacroTestHarness::for_adapter(AdapterType::ClickHouse)
            .load_all_macros()
            .build()
            .expect("harness should build")
    }

    fn relation(schema: &str, identifier: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("schema".to_string(), schema.to_string()),
            ("identifier".to_string(), identifier.to_string()),
        ])
    }

    #[test]
    fn clickhouse_catalog_relations_filters_by_schema_and_identifier() {
        let harness = build_harness();
        let ctx = BTreeMap::from([
            ("information_schema", Value::UNDEFINED),
            (
                "relations",
                Value::from_serialize(vec![
                    relation("clickstream", "bets"),
                    relation("analytics", "fct_market_odds"),
                ]),
            ),
        ]);

        let rendered = harness
            .render(
                "{{ clickhouse__get_catalog_relations_sql(information_schema, relations) }}",
                ctx,
            )
            .expect("catalog SQL should render");
        let normalized = rendered
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();

        assert!(
            normalized.contains("from system.columns as columns"),
            "Expected catalog SQL to read ClickHouse system.columns, got: {rendered}",
        );
        assert!(
            normalized.contains("join system.tables as tables"),
            "Expected catalog SQL to join ClickHouse system.tables, got: {rendered}",
        );
        assert!(
            normalized.contains("cast(null, 'nullable(string)') as table_owner"),
            "Expected table_owner to use a typed nullable string for Arrow conversion, got: {rendered}",
        );
        assert!(
            normalized.contains("cast(columns.position, 'decimal(38, 0)') as column_index"),
            "Expected column_index to use Decimal128-compatible output, got: {rendered}",
        );
        assert!(
            normalized.contains("columns.database = 'clickstream' and columns.table = 'bets'"),
            "Expected relation-specific filter for clickstream.bets, got: {rendered}",
        );
        assert!(
            normalized
                .contains("columns.database = 'analytics' and columns.table = 'fct_market_odds'"),
            "Expected relation-specific filter for analytics.fct_market_odds, got: {rendered}",
        );
    }

    #[test]
    fn clickhouse_catalog_relations_requires_schema() {
        let harness = build_harness();
        let ctx = BTreeMap::from([(
            "relations",
            Value::from_serialize(vec![BTreeMap::from([(
                "identifier".to_string(),
                "bets".to_string(),
            )])]),
        )]);

        let err = harness
            .render(
                "{{ clickhouse__get_catalog_relations_where_clause_sql(relations) }}",
                ctx,
            )
            .expect_err("relations without a schema should fail");
        let err = err.to_string();

        assert!(
            err.contains("requires a list of relations, each with a schema"),
            "Expected schema validation error, got: {err}",
        );
    }
}
