#![cfg(feature = "unstable_machinery")]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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
    fn on_lookup(&self, _span: &Span, _simple_name: &str, _full_name: &str, _def_spans: Vec<Span>) {
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
fn typecheck_template(source: &str, funcsigns: BTreeMap<String, DynTypeObject>) -> Vec<String> {
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

/// Parse macro name, parameter names, and funcsign from a SQL template.
///
/// Expects the template to contain:
///   `{% macro NAME(param1, param2, ...) %}`
///   `{#-- funcsign: SIGNATURE --#}`
fn parse_macro_metadata(source: &str) -> (&str, Vec<&str>, &str) {
    let macro_start = source
        .find("macro ")
        .expect("SQL file must contain a macro definition");
    let after_macro = &source[macro_start + 6..];
    let paren_start = after_macro.find('(').expect("macro must have parameters");
    let name = after_macro[..paren_start].trim();
    let paren_end = after_macro
        .find(')')
        .expect("macro must have closing paren");
    let params_str = &after_macro[paren_start + 1..paren_end];
    let params: Vec<&str> = params_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    let funcsign_marker = "funcsign:";
    let fs_start = source
        .find(funcsign_marker)
        .expect("SQL file must contain a funcsign comment");
    let after_fs = &source[fs_start + funcsign_marker.len()..];
    let fs_end = after_fs
        .find("--#")
        .expect("funcsign comment must end with --#");
    let funcsign = after_fs[..fs_end].trim();

    (name, params, funcsign)
}

/// Collect all .sql files from a directory, sorted by name for deterministic ordering.
fn collect_sql_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    files.sort();
    files
}

/// Run typecheck on each .sql file in a directory, using the given registration function,
/// and assert the warnings match the snapshot stored next to the .sql file.
fn run_typecheck_dir(
    dir: &Path,
    register_fn: fn(&mut BTreeMap<String, DynTypeObject>, &str, &str, &[&str]),
) {
    let files = collect_sql_files(dir);
    assert!(
        !files.is_empty(),
        "No .sql files found in {}",
        dir.display()
    );

    for sql_path in files {
        let source = std::fs::read_to_string(&sql_path).unwrap();
        let (name, params, funcsign) = parse_macro_metadata(&source);
        let param_refs: Vec<&str> = params.into_iter().collect();

        let mut funcsigns_map = BTreeMap::new();
        register_fn(&mut funcsigns_map, name, funcsign, &param_refs);

        let warnings = typecheck_template(&source, funcsigns_map);
        let snap_name = sql_path.file_stem().unwrap().to_str().unwrap();
        insta::with_settings!({
            snapshot_path => dir,
            prepend_module_to_snapshot => false,
            omit_expression => true,
        }, {
            insta::assert_yaml_snapshot!(snap_name, warnings);
        });
    }
}

// ---------------------------------------------------------------------------
// General typecheck tests (register UDF under qualified name only)
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/typecheck-inputs/udf");
    run_typecheck_dir(&dir, register_udf);
}

// ---------------------------------------------------------------------------
// Return-type typecheck tests (register UDF under both qualified and bare name)
// ---------------------------------------------------------------------------
#[test]
fn test_typecheck_return_type() {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/typecheck-inputs/udf_and_bare_name");
    run_typecheck_dir(&dir, register_udf_with_bare_name);
}
