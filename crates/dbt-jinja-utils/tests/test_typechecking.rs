//! Tests for type checking functionality in dbt-jinja-utils
//!
//! These tests verify the integration between MiniJinja's type checker
//! and dbt-jinja-utils's WarningPrinter listener.
//!
//! ## Testing Philosophy
//!
//! We test the **integration points** between dbt-jinja-utils and MiniJinja's
//! type checker, NOT the type checker's internal logic.
//!
//! ### What We Test ✅
//!
//! 1. **Integration works**: WarningPrinter receives warnings from type checker
//! 2. **WarningPrinter logic**: Formatting, sorting, deduplication
//! 3. **NoQA suppression**: Suppressing warnings via comments
//! 4. **Function registry**: dbt functions (ref, source, var, config) are recognized
//! 5. **Representative scenarios**: A few example warnings to ensure integration works
//!
//! ### What We DON'T Test ❌
//!
//! 1. **All MiniJinja error types**: We don't exhaustively test every warning type
//!    from MiniJinja's type checker. From dbt-jinja-utils's perspective, all warnings
//!    look the same (just strings with locations).
//!
//! 2. **Type inference logic**: We don't test how MiniJinja determines types or
//!    infers type mismatches - that's MiniJinja's responsibility.
//!
//! 3. **Every scenario**: We test representative examples (undefined variable,
//!    type mismatch, undefined filter) but not every combination.
//!
//! ### Why This Approach?
//!
//! All warnings flow through the same code path in WarningPrinter:
//! ```
//! template → typecheck() → listener.warn(location, message) → WarningPrinter
//! ```
//!
//! Testing every warning type doesn't increase coverage of OUR code. Instead,
//! we focus on testing our integration logic and dbt-specific features.

mod common;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use dashmap::DashMap;
use minijinja::compiler::typecheck::FunctionRegistry;
use minijinja::{CodeLocation, Environment, TypecheckingEventListener};
use serde::Serialize;

use dbt_common::io_args::IoArgs;
use dbt_jinja_utils::listener::WarningPrinter;

/// Serializable version of warnings for snapshot testing
#[derive(Debug, Serialize)]
struct WarningSnapshot {
    warnings: Vec<WarningEntry>,
}

#[derive(Debug, Serialize)]
struct WarningEntry {
    line: u32,
    col: u32,
    file: String,
    message: String,
}

impl From<(CodeLocation, String)> for WarningEntry {
    fn from((loc, msg): (CodeLocation, String)) -> Self {
        WarningEntry {
            line: loc.line,
            col: loc.col,
            file: loc.file.to_string_lossy().to_string(),
            message: msg,
        }
    }
}

/// Helper function to run typechecking on a template and return the listener
///
/// This is a convenience wrapper around `typecheck_template_with_registry`
/// that uses an empty function registry.
fn typecheck_template(
    template_source: &str,
    noqa_comments: Option<HashSet<u32>>,
) -> Result<Rc<WarningPrinter>, minijinja::Error> {
    typecheck_template_with_registry(
        template_source,
        common::create_empty_function_registry(),
        noqa_comments,
    )
}

/// Helper to create snapshot from listener
fn create_snapshot(listener: &WarningPrinter) -> WarningSnapshot {
    WarningSnapshot {
        warnings: listener
            .get_warnings()
            .into_iter()
            .map(WarningEntry::from)
            .collect(),
    }
}

#[test]
fn test_typecheck_simple() {
    let listener = typecheck_template("Hello {{ name }}!", None).expect("Typecheck should succeed");

    let snapshot = create_snapshot(&listener);
    insta::assert_yaml_snapshot!(snapshot);
}

#[test]
fn test_typecheck_with_two_lines() {
    let listener = typecheck_template("Hello {{ name }}!\nGoodbye {{ name }}.", None)
        .expect("Typecheck should succeed");

    let snapshot = create_snapshot(&listener);
    insta::assert_yaml_snapshot!(snapshot);
}

#[test]
fn test_typecheck_with_filter() {
    let listener =
        typecheck_template("{{ name | upper }}", None).expect("Typecheck should succeed");

    let snapshot = create_snapshot(&listener);
    insta::assert_yaml_snapshot!(snapshot);
}

#[test]
fn test_typecheck_with_noqa() {
    // Create noqa comments for line 1
    let noqa_comments = HashSet::from([1]);

    let listener =
        typecheck_template("{{ name }}", Some(noqa_comments)).expect("Typecheck should succeed");

    let snapshot = create_snapshot(&listener);
    insta::assert_yaml_snapshot!(snapshot);
}

#[test]
fn test_typecheck_type_mismatch() {
    // Test type mismatch for binary operator
    // This is a representative example of type checking warnings
    let listener = typecheck_template("{{ 'abc' + 123 }}", None).expect("Typecheck should succeed");

    let snapshot = create_snapshot(&listener);
    // Should contain a type mismatch warning for + operator
    insta::assert_yaml_snapshot!(snapshot);
}

#[test]
fn test_typecheck_multiple_warnings_sorted() {
    // Test that multiple warnings are sorted by line and column
    let template = r#"
{{ z }}
{{ a }}
{{ m }}
"#;

    let listener = typecheck_template(template, None).expect("Typecheck should succeed");

    let snapshot = create_snapshot(&listener);
    // Warnings should be in order: line 2 col 1, line 3 col 1, line 4 col 1
    // Variable names (z, a, m) shouldn't affect order - only position matters
    insta::assert_yaml_snapshot!(snapshot);
}

#[test]
fn test_noqa_only_suppresses_target_line() {
    // Test that NoQA only suppresses the line it's on, not other lines
    let template = r#"{{ undefined1 }}
{{ undefined2 }}"#;

    // NoQA comment on line 1 only
    let noqa_comments = HashSet::from([1]);

    let listener =
        typecheck_template(template, Some(noqa_comments)).expect("Typecheck should succeed");

    let snapshot = create_snapshot(&listener);
    // Should only have warning for line 2 (undefined2)
    // Line 1 (undefined1) should be suppressed by NoQA
    insta::assert_yaml_snapshot!(snapshot);
}

#[test]
fn test_noqa_multiple_lines() {
    // Test multiple NoQA comments on different lines
    let template = r#"{{ a }}
{{ b }}
{{ c }}"#;

    // NoQA on lines 1 and 3
    let noqa_comments = HashSet::from([1, 3]);

    let listener =
        typecheck_template(template, Some(noqa_comments)).expect("Typecheck should succeed");

    let snapshot = create_snapshot(&listener);
    // Should only have warning for line 2 (b)
    // Lines 1 and 3 should be suppressed
    insta::assert_yaml_snapshot!(snapshot);
}

/// Helper to run typechecking with a custom function registry
fn typecheck_template_with_registry(
    template_source: &str,
    function_registry: Arc<FunctionRegistry>,
    noqa_comments: Option<HashSet<u32>>,
) -> Result<Rc<WarningPrinter>, minijinja::Error> {
    typecheck_template_with_registry_and_context(
        template_source,
        function_registry,
        noqa_comments,
        BTreeMap::new(),
    )
}

/// Helper to run typechecking with custom function registry and context
fn typecheck_template_with_registry_and_context(
    template_source: &str,
    function_registry: Arc<FunctionRegistry>,
    noqa_comments: Option<HashSet<u32>>,
    context: BTreeMap<String, minijinja::Value>,
) -> Result<Rc<WarningPrinter>, minijinja::Error> {
    use minijinja::Type;

    // Create environment and add template
    let mut env = Environment::new();
    env.add_template("test", template_source)?;

    // Get the template
    let tmpl = env.get_template("test")?;

    // Populate builtins with function types from registry
    // The type checker looks for ref/source in builtins, not just registry
    let builtins = Arc::new(DashMap::new());
    for (name, func_obj) in function_registry.iter() {
        builtins.insert(name.clone(), Type::Object(func_obj.clone()));
    }

    // Create WarningPrinter listener
    let listener = Rc::new(WarningPrinter::new(
        IoArgs::default(),
        PathBuf::from("test.jinja"),
        noqa_comments,
    ));
    let listener_clone = Rc::clone(&listener);

    // Run typecheck with custom function registry and context
    tmpl.typecheck(
        function_registry,
        builtins,
        listener_clone as Rc<dyn TypecheckingEventListener>,
        context,
    )?;

    Ok(listener)
}

#[test]
fn test_typecheck_with_dbt_functions() {
    // Test function registry integration with type checker
    // This test covers all major dbt functions: ref, source, var, config
    // Note: MiniJinja has special handling for ref() and source() in typemeta.rs
    // but from dbt-jinja-utils's perspective, all registered functions work the same way
    let registry = common::create_dbt_function_registry();

    // Template using multiple dbt functions
    let template = r#"
{{ ref('model1') }}
{{ source('src', 'table') }}
{{ var('my_var') }}
{{ config(materialized='table') }}
"#;

    let listener = typecheck_template_with_registry(template, registry, None)
        .expect("Typecheck should succeed");

    let snapshot = create_snapshot(&listener);
    // Should have no "function not defined" warnings
    insta::assert_yaml_snapshot!(snapshot);
}

#[test]
fn test_typecheck_with_context_variables() {
    // NOTE: This test documents current behavior where context variables
    // still trigger "unknown variable" warnings during type checking.
    // In real dbt usage, some variables are provided in the typecheck context
    // but MiniJinja's type checker doesn't recognize context variables as "defined"
    // unless they're explicitly in the builtins or through other mechanisms.
    //
    // This is expected behavior - the type checker is conservative and warns about
    // variables that aren't explicitly declared, even if they're in the context.
    use minijinja::Value;

    let registry = common::create_empty_function_registry();

    // Create a context with some variables
    let mut context = BTreeMap::new();
    context.insert("project_name".to_string(), Value::from("my_project"));
    context.insert("schema_name".to_string(), Value::from("public"));

    // Template using context variables
    let template = r#"{{ project_name }}_{{ schema_name }}"#;

    let listener = typecheck_template_with_registry_and_context(template, registry, None, context)
        .expect("Typecheck should succeed");

    let snapshot = create_snapshot(&listener);
    // Currently: context variables still trigger warnings (expected behavior)
    insta::assert_yaml_snapshot!(snapshot);
}

// NOTE: Tests for unregistered functions and partial registries are skipped
// because the type checker has special handling for ref/source/var that expects
// them to be present in builtins. Testing the absence of functions would require
// deeper integration with the full dbt environment builder.
