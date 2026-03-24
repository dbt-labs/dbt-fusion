use std::collections::BTreeMap;

use dbt_common::adapter::AdapterType;
use dbt_schemas::dbt_types::RelationType;
use minijinja::Value;

use crate::macro_test_harness::MacroTestHarness;

mod redshift {
    use super::*;

    fn build_grant_harness() -> MacroTestHarness {
        MacroTestHarness::for_adapter(AdapterType::Redshift)
            .load_all_macros()
            .with_stub_functions()
            .build()
            .expect("harness should build")
    }

    fn grant_ctx(
        harness: &MacroTestHarness,
        grantees: Vec<&str>,
    ) -> BTreeMap<String, Value> {
        let relation = harness.relation(
            "test_db",
            "test_schema",
            "test_table",
            Some(RelationType::Table),
        );
        BTreeMap::from([
            ("relation".to_string(), relation.as_value()),
            ("privilege".to_string(), Value::from("select")),
            (
                "grantees".to_string(),
                Value::from(
                    grantees
                        .into_iter()
                        .map(Value::from)
                        .collect::<Vec<_>>(),
                ),
            ),
        ])
    }

    #[test]
    fn grant_sql_quotes_iam_user_with_special_characters() {
        let harness = build_grant_harness();
        let ctx = grant_ctx(&harness, vec!["IAM:firstname.lastname"]);

        let rendered = harness
            .render("{{ get_grant_sql(relation, privilege, grantees) }}", ctx)
            .expect("render should succeed");

        assert!(
            rendered.contains(r#""IAM:firstname.lastname""#),
            "Grantee with special characters must be double-quoted, got: {rendered:?}"
        );
    }

    #[test]
    fn revoke_sql_quotes_iam_user_with_special_characters() {
        let harness = build_grant_harness();
        let ctx = grant_ctx(&harness, vec!["IAM:firstname.lastname"]);

        let rendered = harness
            .render(
                "{{ get_revoke_sql(relation, privilege, grantees) }}",
                ctx,
            )
            .expect("render should succeed");

        assert!(
            rendered.contains(r#""IAM:firstname.lastname""#),
            "Grantee with special characters must be double-quoted, got: {rendered:?}"
        );
    }

    #[test]
    fn grant_sql_does_not_quote_plain_username() {
        let harness = build_grant_harness();
        let ctx = grant_ctx(&harness, vec!["analyst_role"]);

        let rendered = harness
            .render("{{ get_grant_sql(relation, privilege, grantees) }}", ctx)
            .expect("render should succeed");

        assert!(
            rendered.contains("analyst_role"),
            "Plain grantee should appear in output, got: {rendered:?}"
        );
        assert!(
            !rendered.contains(r#""analyst_role""#),
            "Plain grantee should NOT be double-quoted, got: {rendered:?}"
        );
    }

    #[test]
    fn grant_sql_quotes_multiple_grantees() {
        let harness = build_grant_harness();
        let ctx = grant_ctx(&harness, vec!["IAM:alice.smith", "IAM:bob.jones"]);

        let rendered = harness
            .render("{{ get_grant_sql(relation, privilege, grantees) }}", ctx)
            .expect("render should succeed");

        assert!(
            rendered.contains(r#""IAM:alice.smith""#),
            "First grantee must be quoted, got: {rendered:?}"
        );
        assert!(
            rendered.contains(r#""IAM:bob.jones""#),
            "Second grantee must be quoted, got: {rendered:?}"
        );
    }
}
