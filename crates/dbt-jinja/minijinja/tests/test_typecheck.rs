#![cfg(feature = "unstable_machinery")]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use minijinja::compiler::codegen::CodeGenerationProfile;
use minijinja::funcsign_parser;
use minijinja::load_builtins_with_namespace;
use minijinja::machinery::Span;
use minijinja::value::Value;
use minijinja::{
    Argument, DynTypeObject, Environment, TypecheckingEventListener, UserDefinedFunctionType,
};

/// Simple listener that collects warnings into a Vec.
/// Uses RefCell since TypecheckingEventListener is used via Rc (single-threaded).
struct WarningCollector {
    warnings: RefCell<Vec<String>>,
}

impl WarningCollector {
    fn new() -> Self {
        Self {
            warnings: RefCell::new(Vec::new()),
        }
    }

    fn warnings(&self) -> Vec<String> {
        self.warnings.borrow().clone()
    }
}

impl TypecheckingEventListener for WarningCollector {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn warn(&self, message: &str) {
        self.warnings.borrow_mut().push(message.to_string());
    }

    fn set_span(&self, _span: &Span) {}
    fn new_block(&self, _block_id: usize) {}
    fn flush(&self) {}
    fn on_lookup(
        &self,
        _span: &Span,
        _simple_name: &str,
        _full_name: &str,
        _def_spans: Vec<Span>,
    ) {
    }
}

/// Build the minimal typecheck context required by the type checker.
/// The type checker expects TARGET_PACKAGE_NAME, ROOT_PACKAGE_NAME,
/// and DBT_AND_ADAPTERS_NAMESPACE to be present.
fn minimal_typecheck_context() -> BTreeMap<String, Value> {
    let mut ctx = BTreeMap::new();
    ctx.insert("TARGET_PACKAGE_NAME".to_string(), Value::from("test"));
    ctx.insert("ROOT_PACKAGE_NAME".to_string(), Value::from("test"));
    // DBT_AND_ADAPTERS_NAMESPACE must be an object (ValueMap = IndexMap<Value, Value>).
    let empty_map = indexmap::IndexMap::<Value, Value>::new();
    ctx.insert(
        "DBT_AND_ADAPTERS_NAMESPACE".to_string(),
        Value::from_object(empty_map),
    );
    ctx
}

/// Helper to run typecheck on a template source with a given function registry.
fn typecheck_template(
    source: &str,
    funcsigns: BTreeMap<String, DynTypeObject>,
) -> Vec<String> {
    let context = minimal_typecheck_context();
    let funcsigns = Arc::new(funcsigns);
    let builtins = load_builtins_with_namespace(None).unwrap();

    let mut env = Environment::new();
    env.profile = CodeGenerationProfile::TypeCheck(funcsigns.clone(), context.clone());

    let tmpl = env
        .template_from_named_str("test_template", source)
        .unwrap();

    let listener = Rc::new(WarningCollector::new());
    tmpl.typecheck(funcsigns, builtins, listener.clone(), context)
        .unwrap();

    listener.warnings()
}

/// Build a UserDefinedFunctionType from a funcsign string and parameter names.
fn build_udf_from_funcsign(name: &str, funcsign: &str, param_names: &[&str]) -> DynTypeObject {
    let builtins = load_builtins_with_namespace(None).unwrap();
    let (arg_types, ret_type) = funcsign_parser::parse(funcsign, builtins).unwrap();

    let args: Vec<Argument> = param_names
        .iter()
        .zip(arg_types.iter())
        .map(|(pname, ty)| Argument {
            name: pname.to_string(),
            type_: ty.clone(),
            is_optional: false,
        })
        .collect();

    DynTypeObject::new(Arc::new(UserDefinedFunctionType::new(
        name,
        args,
        ret_type,
        &PathBuf::from("test.sql"),
        &Span::default(),
        &format!("test.{name}"),
    )))
}

/// Register a UDF in the function registry under the qualified name
/// ("test.{name}"), which is what CodeGenerationProfile::TypeCheck uses
/// during compilation to resolve funcsign parameters.
fn register_udf(
    funcsigns: &mut BTreeMap<String, DynTypeObject>,
    name: &str,
    funcsign: &str,
    param_names: &[&str],
) {
    let udf = build_udf_from_funcsign(name, funcsign, param_names);
    funcsigns.insert(format!("test.{name}"), udf);
}

/// Register a UDF under both the qualified and bare name.
/// The CFG's current_macro uses bare names (from MacroName instruction),
/// so the per-block return type check (typemeta.rs:550) and last-block
/// implicit return check (typemeta.rs:592) require the bare name key.
fn register_udf_with_bare_name(
    funcsigns: &mut BTreeMap<String, DynTypeObject>,
    name: &str,
    funcsign: &str,
    param_names: &[&str],
) {
    let udf = build_udf_from_funcsign(name, funcsign, param_names);
    funcsigns.insert(format!("test.{name}"), udf.clone());
    funcsigns.insert(name.to_string(), udf);
}

#[test]
fn test_typecheck_funcsign_with_is_not_none_narrowing() {
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (optional[integer]) -> string --#}
{% if a is not none %}
  {{ a + 1 }}
{% else %}
  {{ a }}
{% endif %}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(optional[integer]) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Unknown local variable
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_unknown_local_variable() {
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{{ unknown_var }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(integer) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Arithmetic type mismatch (string + integer)
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_arithmetic_type_mismatch() {
    let source = r#"{%- macro my_macro(a, b) -%}
{#-- funcsign: (string, integer) -> string --#}
{{ a + b }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(string, integer) -> string", &["a", "b"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Undefined function
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_undefined_function() {
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{{ not_a_function() }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(integer) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Undefined filter
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_undefined_filter() {
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{{ a | not_a_filter }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(integer) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Single-branch variable definition
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_single_branch_variable_definition() {
    // Variable `x` is only set in the truthy branch; using it after the if
    // should warn that it's not defined in one predecessor block.
    // We use an if/else to ensure the merge creates two distinct paths.
    let source = r#"{%- macro my_macro(cond) -%}
{#-- funcsign: (boolean) -> string --#}
{% if cond %}{% set x = 1 %}{% else %}nope{% endif %}
{{ x }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(boolean) -> string", &["cond"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Both-branch variable definition (control for single-branch test)
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_both_branch_variable_definition() {
    // Variable `x` is set in both branches; no warning expected.
    let source = r#"{%- macro my_macro(cond) -%}
{#-- funcsign: (boolean) -> string --#}
{% if cond %}{% set x = 1 %}{% else %}{% set x = 2 %}{% endif %}
{{ x }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(boolean) -> string", &["cond"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Nested if/else with variable defined in all leaf branches (control)
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_nested_if_all_branches_define_var() {
    // Variable `x` is set in every leaf branch of a nested if/else;
    // no single-branch warning expected.
    let source = r#"{%- macro my_macro(a, b) -%}
{#-- funcsign: (boolean, boolean) -> string --#}
{% if a %}
  {% if b %}{% set x = 1 %}{% else %}{% set x = 2 %}{% endif %}
{% else %}
  {% set x = 3 %}
{% endif %}
{{ x }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(boolean, boolean) -> string", &["a", "b"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Comparison type mismatch
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_comparison_type_mismatch() {
    let source = r#"{%- macro my_macro(a, b) -%}
{#-- funcsign: (string, integer) -> string --#}
{% if a > b %}yes{% endif %}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(string, integer) -> string", &["a", "b"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Explicit return type mismatch
// The per-block return type check uses the bare macro name from the CFG to
// look up the function registry. We register under the bare name to exercise
// this code path.
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_explicit_return_type_mismatch() {
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> integer --#}
{{ return("hello") }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf_with_bare_name(&mut funcsigns, "my_macro", "(integer) -> integer", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Implicit return type mismatch
// The last-block return type check fires when a macro's declared return type
// is not compatible with String (the implicit return).
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_implicit_return_type_mismatch() {
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> integer --#}
hello world
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf_with_bare_name(&mut funcsigns, "my_macro", "(integer) -> integer", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Correct return type (control for return type mismatch tests)
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_correct_return_type() {
    // Explicit return matches declared type; no warning expected.
    // The macro declares -> string, and both the explicit return("hello")
    // and the implicit return (rendered text) are strings.
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{{ return("hello") }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf_with_bare_name(&mut funcsigns, "my_macro", "(integer) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// `is string` type narrowing
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_is_string_narrowing() {
    // `a` has union type string|integer. After `is string`, the truthy branch
    // should narrow to string, allowing `a.upper()` without warnings.
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (string | integer) -> string --#}
{% if a is string %}{{ a.upper() }}{% endif %}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(string | integer) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// `is integer` negative narrowing / type exclusion
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_is_integer_else_narrowing() {
    // `a` has union type string|integer. `is integer` narrows truthy to integer;
    // the else branch should narrow to string, allowing `a.upper()`.
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (string | integer) -> string --#}
{% if a is integer %}{{ a + 1 }}{% else %}{{ a.upper() }}{% endif %}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(string | integer) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// For loop element type inference
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_for_loop_element_type_inference() {
    // Iterate over a list[integer] parameter; element should be inferred as
    // integer, so `item + 1` should produce no type errors.
    let source = r#"{%- macro my_macro(items) -%}
{#-- funcsign: (list[integer]) -> string --#}
{% for item in items %}{{ item + 1 }}{% endfor %}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(list[integer]) -> string", &["items"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Undefined macro call
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_undefined_macro_call() {
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{{ unknown_macro() }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(integer) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Unpack list length mismatch
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_unpack_list_length_mismatch() {
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{% set x, y = [1, 2, 3] %}
{{ x }}{{ y }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(integer) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Non-integer slice index
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_non_integer_slice_index() {
    let source = r#"{%- macro my_macro(items) -%}
{#-- funcsign: (list[integer]) -> string --#}
{{ items["a":"b"] }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(list[integer]) -> string", &["items"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Valid integer slice index (control for non-integer slice test)
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_valid_integer_slice_index() {
    // Slice indices are integers; no warning expected.
    let source = r#"{%- macro my_macro(items) -%}
{#-- funcsign: (list[integer]) -> string --#}
{{ items[0:2] }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(list[integer]) -> string", &["items"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Valid string method (no warning expected)
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_valid_string_method() {
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (string) -> string --#}
{{ a.upper() }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(string) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Invalid attribute on type
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_invalid_attribute_on_type() {
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (integer) -> string --#}
{{ a.nonexistent() }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(integer) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ===========================================================================
// Additional coverage: null dereference, nested control flow, loop edge cases
// ===========================================================================

// ---------------------------------------------------------------------------
// Attribute access on optional type without guard (null dereference)
// Paper's most common bug pattern (17/30 in evaluation).
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_optional_attribute_access_no_guard() {
    // `a` is optional[string], accessing `.upper()` without an `is not none`
    // guard should warn about potential None dereference.
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (optional[string]) -> string --#}
{{ a.upper() }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(optional[string]) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Attribute access on optional type WITH `is not none` guard (control)
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_optional_attribute_access_with_guard() {
    // After `is not none`, accessing `.upper()` is safe; no warning expected.
    let source = r#"{%- macro my_macro(a) -%}
{#-- funcsign: (optional[string]) -> string --#}
{% if a is not none %}{{ a.upper() }}{% endif %}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(optional[string]) -> string", &["a"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Nested loop: inner loop variable used inside inner body (no false positive)
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_nested_loop_variable() {
    // `row` and `cell` are loop variables; using them inside their
    // respective loop bodies should not produce single-branch warnings.
    let source = r#"{%- macro my_macro(matrix) -%}
{#-- funcsign: (list[list[integer]]) -> string --#}
{% for row in matrix %}{% for cell in row %}{{ cell + 1 }}{% endfor %}{% endfor %}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(list[list[integer]]) -> string", &["matrix"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Variable set inside loop body, used after loop (single-branch)
// The loop may not execute, so `x` might be undefined after the loop.
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_var_defined_in_loop_used_after() {
    let source = r#"{%- macro my_macro(items) -%}
{#-- funcsign: (list[integer]) -> string --#}
{% for item in items %}{% set x = item %}{% endfor %}
{{ x }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(list[integer]) -> string", &["items"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}

// ---------------------------------------------------------------------------
// Nested if/else where only SOME leaf branches define a variable
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_nested_if_partial_branch_definition() {
    // `x` is set in the inner-if truthy branch and the outer else,
    // but NOT in the inner-if else branch. Should warn.
    let source = r#"{%- macro my_macro(a, b) -%}
{#-- funcsign: (boolean, boolean) -> string --#}
{% if a %}
  {% if b %}{% set x = 1 %}{% else %}nope{% endif %}
{% else %}
  {% set x = 3 %}
{% endif %}
{{ x }}
{%- endmacro -%}"#;

    let mut funcsigns = BTreeMap::new();
    register_udf(&mut funcsigns, "my_macro", "(boolean, boolean) -> string", &["a", "b"]);

    let warnings = typecheck_template(source, funcsigns);
    insta::assert_yaml_snapshot!(warnings);
}
