use std::collections::BTreeMap;

use dbt_schemas::schemas::macros::DbtDocsMacro;
use minijinja::{
    constants::{MACRO_DISPATCH_ORDER, TARGET_PACKAGE_NAME},
    value::Value as MinijinjaValue,
};

use crate::functions::DocMacro;
use crate::phases::compile_and_run_context::DbtNamespace;

/// Builds a context for resolving `macros:` entries in YAML property files and for patching
/// macro catalog fields such as `description` in `schema.yml`.
///
/// dbt-core only exposes a narrow Jinja scope there (e.g. `doc()`), not callable project
/// macros. Omitting macro namespace keys matches that behavior so `{{ my_macro(...) }}` in a
/// macro description fails instead of being rendered into SQL at parse time.
pub fn build_macro_properties_resolve_context(
    root_project_name: &str,
    local_project_name: &str,
    docs_macros: &BTreeMap<String, DbtDocsMacro>,
    macro_dispatch_order: BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, MinijinjaValue> {
    build_resolve_context(
        root_project_name,
        local_project_name,
        docs_macros,
        macro_dispatch_order,
        vec![],
    )
}

/// Builds a context for resolving models
pub fn build_resolve_context(
    root_project_name: &str,
    local_project_name: &str,
    docs_macros: &BTreeMap<String, DbtDocsMacro>,
    macro_dispatch_order: BTreeMap<String, Vec<String>>,
    namespace_keys: Vec<String>,
) -> BTreeMap<String, MinijinjaValue> {
    let mut ctx = BTreeMap::new();
    let docs_map: BTreeMap<(String, String), String> = docs_macros
        .values()
        .map(|v| {
            (
                (v.package_name.clone(), v.name.clone()),
                v.block_contents.clone(),
            )
        })
        .collect();

    ctx.insert(
        "doc".to_string(),
        MinijinjaValue::from_object(DocMacro::new(root_project_name.to_string(), docs_map)),
    );

    ctx.insert(
        MACRO_DISPATCH_ORDER.to_string(),
        MinijinjaValue::from_object(
            macro_dispatch_order
                .into_iter()
                .map(|(k, v)| (MinijinjaValue::from(k), MinijinjaValue::from(v)))
                .collect::<BTreeMap<_, _>>(),
        ),
    );

    ctx.insert(
        TARGET_PACKAGE_NAME.to_string(),
        MinijinjaValue::from(local_project_name),
    );

    ctx.insert("execute".to_string(), MinijinjaValue::from(false));
    ctx.insert("node".to_string(), MinijinjaValue::NONE);

    ctx.insert("connection_name".to_string(), MinijinjaValue::from(""));

    // Insert DbtNamespace objects for each namespace key
    for key in namespace_keys {
        ctx.insert(
            key.clone(),
            MinijinjaValue::from_object(DbtNamespace::new(&key)),
        );
    }

    ctx
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::environment_builder::{JinjaEnvBuilder, MacroUnitsWrapper};
    use crate::serde::into_typed_with_jinja;
    use dbt_adapter::sql_types::SATypeOpsImpl;
    use dbt_adapter::Adapter;
    use dbt_adapter_core::AdapterType;
    use dbt_common::io_args::IoArgs;
    use dbt_schemas::schemas::macros::DbtDocsMacro;
    use dbt_schemas::schemas::properties::MacrosProperties;
    use dbt_schemas::schemas::relations::DEFAULT_DBT_QUOTING;
    use minijinja::dispatch_object::THREAD_LOCAL_DEPENDENCIES;
    use minijinja::macro_unit::{MacroInfo, MacroUnit};
    use minijinja::machinery::Span;
    use minijinja::UndefinedBehavior;

    static DEPS_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn set_thread_local_dependencies(pkgs: impl IntoIterator<Item = String>) {
        let _guard = DEPS_TEST_LOCK.lock().unwrap();
        let deps = THREAD_LOCAL_DEPENDENCIES.get_or_init(|| Mutex::new(BTreeSet::new()));
        let mut deps = deps.lock().unwrap();
        deps.clear();
        deps.extend(pkgs);
    }

    fn create_macro_unit(name: &str, sql: &str) -> MacroUnit {
        MacroUnit {
            info: MacroInfo {
                name: name.to_string(),
                path: PathBuf::from("macros/test.sql"),
                span: Span {
                    start_line: 0,
                    start_col: 0,
                    start_offset: 0,
                    end_line: 0,
                    end_col: 0,
                    end_offset: 0,
                },
                funcsign: None,
                args: vec![],
                unique_id: "test".to_string(),
                name_span: Span::default(),
            },
            sql: sql.to_string(),
        }
    }

    fn parse_jinja_env_with_my_pkg_macro() -> crate::jinja_environment::JinjaEnv {
        set_thread_local_dependencies(std::iter::once("my_pkg".to_string()));
        let mut macro_units = MacroUnitsWrapper::new(BTreeMap::new());
        macro_units.macros.insert(
            "my_pkg".to_string(),
            vec![create_macro_unit(
                "cents_to_dollars",
                "{% macro cents_to_dollars(column_name) %}{{ column_name }} / 100.0{% endmacro %}",
            )],
        );
        let adapter = Adapter::new_parse_phase_adapter(
            AdapterType::Postgres,
            dbt_yaml::Mapping::default(),
            DEFAULT_DBT_QUOTING,
            Box::new(SATypeOpsImpl::new(AdapterType::Postgres)),
            None,
        );
        JinjaEnvBuilder::new()
            .with_undefined_behavior(UndefinedBehavior::Strict)
            .with_adapter(std::sync::Arc::new(adapter))
            .with_root_package("my_pkg".to_string())
            .try_with_macros(macro_units)
            .expect("Failed to register macros")
            .build()
    }

    /// One macro-properties YAML blob; description uses the package namespace (matches Fusion
    /// before the fix when namespaces were injected).
    fn macro_yaml_value_with_desc(description: &str) -> dbt_yaml::Value {
        let mut mapping = dbt_yaml::Mapping::new();
        mapping.insert(
            dbt_yaml::Value::String("name".to_string(), dbt_yaml::Span::default()),
            dbt_yaml::Value::String("cents_to_dollars".to_string(), dbt_yaml::Span::default()),
        );
        mapping.insert(
            dbt_yaml::Value::String("description".to_string(), dbt_yaml::Span::default()),
            dbt_yaml::Value::String(description.to_string(), dbt_yaml::Span::default()),
        );
        dbt_yaml::Value::Mapping(mapping, dbt_yaml::Span::default())
    }

    #[test]
    fn macro_properties_context_omits_macro_namespace_keys() {
        let docs = BTreeMap::new();
        let dispatch = BTreeMap::new();
        let full = build_resolve_context(
            "root_pkg",
            "local_pkg",
            &docs,
            dispatch.clone(),
            vec!["dbt".to_string(), "local_pkg".to_string()],
        );
        let narrow = build_macro_properties_resolve_context(
            "root_pkg",
            "local_pkg",
            &docs,
            dispatch,
        );
        assert!(full.contains_key("dbt"));
        assert!(full.contains_key("local_pkg"));
        assert!(!narrow.contains_key("dbt"));
        assert!(!narrow.contains_key("local_pkg"));
        assert!(narrow.get("doc").is_some());
        assert_eq!(
            narrow.get(TARGET_PACKAGE_NAME).and_then(|v| v.as_str()),
            Some("local_pkg")
        );
    }

    #[test]
    fn narrow_context_errors_on_macro_call_in_description() {
        let env = parse_jinja_env_with_my_pkg_macro();
        let dispatch = BTreeMap::new();
        let full_ctx = build_resolve_context(
            "my_pkg",
            "my_pkg",
            &BTreeMap::new(),
            dispatch.clone(),
            vec!["my_pkg".to_string()],
        );
        let narrow_ctx = build_macro_properties_resolve_context("my_pkg", "my_pkg", &BTreeMap::new(), dispatch);

        let yml = macro_yaml_value_with_desc("{{ my_pkg.cents_to_dollars('price_cents') }}");
        let io = IoArgs::default();
        let parsed: MacrosProperties = into_typed_with_jinja(
            &io,
            yml.clone(),
            false,
            &env,
            &full_ctx,
            &[],
            None,
            false,
        )
        .expect("full resolve context should render macro in description");
        assert!(
            parsed.description.unwrap().contains("price_cents / 100.0"),
            "expected macro body in description"
        );

        let err = into_typed_with_jinja::<MacrosProperties, _>(
            &io,
            yml,
            false,
            &env,
            &narrow_ctx,
            &[],
            None,
            false,
        )
        .expect_err("narrow context should not resolve package macros in description");
        let msg = err.to_string();
        assert!(
            msg.contains("undefined") || msg.contains("Undefined"),
            "expected undefined error, got: {msg}"
        );
    }

    #[test]
    fn narrow_context_allows_doc_in_macro_description() {
        let env = parse_jinja_env_with_my_pkg_macro();
        let mut docs = BTreeMap::new();
        docs.insert(
            "doc.my_pkg.my_doc".to_string(),
            DbtDocsMacro {
                name: "my_doc".to_string(),
                package_name: "my_pkg".to_string(),
                path: PathBuf::from("models/docs.md"),
                original_file_path: PathBuf::from("models/docs.md"),
                unique_id: "doc.my_pkg.my_doc".to_string(),
                block_contents: "hello from doc".to_string(),
            },
        );
        let narrow_ctx = build_macro_properties_resolve_context("my_pkg", "my_pkg", &docs, BTreeMap::new());
        let yml = macro_yaml_value_with_desc("{{ doc('my_doc') }}");
        let io = IoArgs::default();
        let parsed: MacrosProperties = into_typed_with_jinja(
            &io,
            yml,
            false,
            &env,
            &narrow_ctx,
            &[],
            None,
            false,
        )
        .expect("doc() should work in macro description with narrow context");
        assert_eq!(parsed.description.as_deref(), Some("hello from doc"));
    }
}
