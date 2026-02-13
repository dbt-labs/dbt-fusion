//! Common test utilities for dbt-jinja-utils tests
//!
//! This module provides shared helpers used across multiple test files,
//! including function registry setup, test environment creation, and
//! test data builders.

use minijinja::compiler::typecheck::FunctionRegistry;
use minijinja::machinery::Span;
use minijinja::{Argument, DynTypeObject, Type, UserDefinedFunctionType};
use std::path::PathBuf;
use std::sync::Arc;

/// Create a function registry with common dbt functions
///
/// This registry is populated with dbt functions (ref, source, var, etc.)
/// with appropriate type signatures for type checking tests.
///
/// # Returns
/// An Arc<FunctionRegistry> with dbt functions registered
///
/// # Example
/// ```
/// let registry = create_dbt_function_registry();
/// // Use in typecheck: tmpl.typecheck(registry, builtins, listener, context)
/// ```
pub fn create_dbt_function_registry() -> Arc<FunctionRegistry> {
    let mut registry = FunctionRegistry::new();

    // Register ref() function
    // Signature: ref(model_name: str) -> str
    // or: ref(package: str, model_name: str) -> str
    let ref_func = UserDefinedFunctionType::new(
        "ref",
        vec![Argument {
            name: "model_name".to_string(),
            type_: Type::String(None),
            is_optional: false,
        }],
        Type::String(None), // Returns a string (relation name)
        &PathBuf::from("test.jinja"),
        &Span::default(),
        "test.ref",
    );
    registry.insert("ref".to_string(), DynTypeObject::new(Arc::new(ref_func)));

    // Register source() function
    // Signature: source(source_name: str, table_name: str) -> str
    let source_func = UserDefinedFunctionType::new(
        "source",
        vec![
            Argument {
                name: "source_name".to_string(),
                type_: Type::String(None),
                is_optional: false,
            },
            Argument {
                name: "table_name".to_string(),
                type_: Type::String(None),
                is_optional: false,
            },
        ],
        Type::String(None), // Returns a string (relation name)
        &PathBuf::from("test.jinja"),
        &Span::default(),
        "test.source",
    );
    registry.insert(
        "source".to_string(),
        DynTypeObject::new(Arc::new(source_func)),
    );

    // Register var() function
    // Signature: var(var_name: str, default: Any = None) -> Any
    let var_func = UserDefinedFunctionType::new(
        "var",
        vec![
            Argument {
                name: "var_name".to_string(),
                type_: Type::String(None),
                is_optional: false,
            },
            Argument {
                name: "default".to_string(),
                type_: Type::Any { hard: true }, // Can be any type
                is_optional: true,
            },
        ],
        Type::Any { hard: true }, // Returns any type
        &PathBuf::from("test.jinja"),
        &Span::default(),
        "test.var",
    );
    registry.insert("var".to_string(), DynTypeObject::new(Arc::new(var_func)));

    // Register config() function
    // Signature: config(**kwargs) -> None
    let config_func = UserDefinedFunctionType::new(
        "config",
        vec![],     // Accepts arbitrary kwargs
        Type::None, // Returns None
        &PathBuf::from("test.jinja"),
        &Span::default(),
        "test.config",
    );
    registry.insert(
        "config".to_string(),
        DynTypeObject::new(Arc::new(config_func)),
    );

    Arc::new(registry)
}

/// Create an empty function registry (for testing unregistered functions)
///
/// # Returns
/// An Arc<FunctionRegistry> with no functions registered
///
/// # Example
/// ```
/// let empty_registry = create_empty_function_registry();
/// // Type checking will report "function not defined" for all functions
/// ```
pub fn create_empty_function_registry() -> Arc<FunctionRegistry> {
    Arc::new(FunctionRegistry::new())
}

/// Create a function registry with only specific functions
///
/// Useful for testing partial registrations or specific scenarios
///
/// # Arguments
/// * `functions` - List of function names to register ("ref", "source", "var", "config")
///
/// # Returns
/// An Arc<FunctionRegistry> with only the specified functions
///
/// # Example
/// ```
/// let registry = create_partial_function_registry(&["ref", "source"]);
/// // Only ref and source are registered, var and config will trigger warnings
/// ```
pub fn create_partial_function_registry(functions: &[&str]) -> Arc<FunctionRegistry> {
    let full_registry = create_dbt_function_registry();
    let mut partial_registry = FunctionRegistry::new();

    for func_name in functions {
        if let Some(func_obj) = full_registry.get(*func_name) {
            partial_registry.insert(func_name.to_string(), func_obj.clone());
        }
    }

    Arc::new(partial_registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_dbt_function_registry() {
        let registry = create_dbt_function_registry();

        // Verify expected functions are registered
        assert!(registry.contains_key("ref"));
        assert!(registry.contains_key("source"));
        assert!(registry.contains_key("var"));
        assert!(registry.contains_key("config"));

        // Verify count
        assert_eq!(registry.len(), 4);
    }

    #[test]
    fn test_create_empty_function_registry() {
        let registry = create_empty_function_registry();
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_create_partial_function_registry() {
        let registry = create_partial_function_registry(&["ref", "source"]);

        assert!(registry.contains_key("ref"));
        assert!(registry.contains_key("source"));
        assert!(!registry.contains_key("var"));
        assert!(!registry.contains_key("config"));
        assert_eq!(registry.len(), 2);
    }
}
