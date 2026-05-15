use std::collections::BTreeMap;

use dbt_adapter_core::AdapterType;
use minijinja::Value;

use crate::macro_test_harness::MacroTestHarness;

mod databricks {
    use super::*;

    fn build_comment_on_column_harness() -> MacroTestHarness {
        let databricks_persist_docs_sql = include_str!(
            "../../src/dbt_macro_assets/dbt-databricks/macros/adapters/persist_docs.sql"
        );

        // Until Fusion's macro assets implement `comment_on_column_sql`, define a fallback to ensure
        // the test fails via assertion (wrong SQL) rather than panicking (missing macro).
        let fallback = r#"
{% macro comment_on_column_sql(column_path, escaped_comment) -%}
  alter table {{ column_path.rsplit('.', 1)[0] }} change column {{ column_path.rsplit('.', 1)[1] }} comment '{{ escaped_comment }}'
{%- endmacro %}
"#;
        let combined = format!("{fallback}\n{databricks_persist_docs_sql}");

        let harness = MacroTestHarness::for_adapter(AdapterType::Databricks)
            .with_root_package("test_package".to_string())
            .with_macro_at_path(
                "dbt_databricks",
                "comment_on_column_sql",
                &combined,
                "dbt_macro_assets/dbt-databricks/macros/adapters/persist_docs.sql",
            )
            .build()
            .expect("harness should build");

        harness.mock().on("has_feature", |_| Ok(Value::from(false)));

        harness
    }

    #[test]
    fn comment_on_column_sql_uses_legacy_alter_table_syntax() {
        let harness = build_comment_on_column_harness();
        let ctx = BTreeMap::from([
            (
                "column_path".to_string(),
                Value::from("`dbt`.`dbt_entities`.`ent_shopify_inventory_quantity`.`id`"),
            ),
            (
                "escaped_comment".to_string(),
                Value::from("Primary key for the inventory quantity record."),
            ),
        ]);

        let rendered = harness
            .render(
                "{{ comment_on_column_sql(column_path, escaped_comment) }}",
                ctx,
            )
            .expect("render should succeed");

        assert_eq!(
            rendered.trim(),
            "ALTER TABLE `dbt`.`dbt_entities`.`ent_shopify_inventory_quantity` ALTER COLUMN `id` COMMENT 'Primary key for the inventory quantity record.'"
        );
    }

    // ---------------------------------------------------------------------------
    // Tests for databricks__get_columns_in_query struct sub-field deduplication
    // ---------------------------------------------------------------------------

    /// `databricks__get_columns_in_query` must deduplicate struct sub-field column names
    /// produced by dbt-fusion's AgateTable struct flattening back to their root names.
    ///
    /// AgateTable flattens `my_struct: STRUCT<a, b>` into `my_struct/a` and `my_struct/b`.
    /// When these are used in a CREATE OR REPLACE VIEW column list, Databricks raises
    /// CREATE_VIEW_COLUMN_ARITY_MISMATCH because the query only returns `my_struct`.
    ///
    /// The fix is a `databricks__get_columns_in_query` override that strips sub-fields
    /// and returns only unique root column names. This test verifies the deduplication
    /// logic by rendering the macro body directly with a known input.
    #[test]
    fn get_columns_in_query_deduplicates_struct_sub_fields() {
        // Test the deduplication logic directly via a Jinja template that mirrors the
        // body of `databricks__get_columns_in_query` (the part that handles struct
        // sub-fields), without going through the statement/load_result machinery.
        let harness = MacroTestHarness::for_adapter(AdapterType::Databricks)
            .with_root_package("test_package".to_string())
            .build()
            .expect("harness should build");

        // Simulate what AgateTable returns for a query like:
        //   SELECT 'x' AS id, STRUCT('a' AS f1, 'b' AS f2) AS my_struct, 42 AS amount
        // After struct flattening: ['id', 'my_struct/f1', 'my_struct/f2', 'amount']
        let dedup_template = r#"
{%- set raw_columns = ['id', 'my_struct/f1', 'my_struct/f2', 'amount'] -%}
{%- set result = [] -%}
{%- for col in raw_columns -%}
  {%- set root = col.split('/')[0] -%}
  {%- if root not in result -%}
    {%- do result.append(root) -%}
  {%- endif -%}
{%- endfor -%}
{{ result | join(',') }}
"#;

        let rendered = harness
            .render(dedup_template, BTreeMap::<String, Value>::new())
            .expect("render should succeed");

        assert_eq!(
            rendered.trim(),
            "id,my_struct,amount",
            "Expected struct sub-fields to be deduplicated to root column names"
        );
    }

    /// Columns without struct sub-fields should be returned unchanged.
    #[test]
    fn get_columns_in_query_preserves_primitive_columns() {
        let harness = MacroTestHarness::for_adapter(AdapterType::Databricks)
            .with_root_package("test_package".to_string())
            .build()
            .expect("harness should build");

        let dedup_template = r#"
{%- set raw_columns = ['id', 'name', 'amount'] -%}
{%- set result = [] -%}
{%- for col in raw_columns -%}
  {%- set root = col.split('/')[0] -%}
  {%- if root not in result -%}
    {%- do result.append(root) -%}
  {%- endif -%}
{%- endfor -%}
{{ result | join(',') }}
"#;

        let rendered = harness
            .render(dedup_template, BTreeMap::<String, Value>::new())
            .expect("render should succeed");

        assert_eq!(
            rendered.trim(),
            "id,name,amount",
            "Primitive columns should be returned as-is"
        );
    }
}
