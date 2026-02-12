//! Evaluates [SelectExpression] against [Nodes] to produce a set of selected node unique_ids.
//!
//! This module exists to reproduce and fix issue #1279: intersection selectors with an empty
//! exclude set should return the included nodes, not an empty selection.

use std::collections::BTreeSet;

use dbt_common::node_selector::{MethodName, SelectionCriteria, SelectExpression};
use dbt_common::FsResult;
use dbt_schemas::schemas::Nodes;
use regex::Regex;

/// Converts a glob pattern (`*` = any chars) to a regex and checks if `value` matches.
/// Escapes regex special chars and replaces `*` with `.*`.
fn pattern_matches(value: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return value.is_empty();
    }
    let mut re_str = String::with_capacity(pattern.len() * 2);
    for c in pattern.chars() {
        match c {
            '*' => re_str.push_str(".*"),
            '.' | '[' | ']' | '(' | ')' | '{' | '}' | '^' | '$' | '|' | '\\' | '+' | '?' => {
                re_str.push('\\');
                re_str.push(c);
            }
            _ => re_str.push(c),
        }
    }
    re_str = format!("^{re_str}$");
    if let Ok(re) = Regex::new(&re_str) {
        re.is_match(value)
    } else {
        false
    }
}

/// Normalizes a path string to use forward slashes for consistent matching.
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// Filters nodes by selection criteria.
fn filter_nodes_by_criteria(
    criteria: &SelectionCriteria,
    nodes: &Nodes,
    evaluator: &dyn Fn(&SelectExpression, &Nodes) -> FsResult<BTreeSet<String>>,
) -> FsResult<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    let pattern = normalize_path(&criteria.value);

    for unique_id in nodes.materializable_keys() {
        let Some(node) = nodes.get_node(unique_id) else {
            continue;
        };

        let matches = match criteria.method {
            MethodName::Path => {
                let path_str = normalize_path(&node.original_file_path().to_string_lossy());
                pattern_matches(&path_str, &pattern)
            }
            MethodName::Fqn => {
                let fqn = node.selector_string();
                pattern_matches(&fqn, &pattern)
            }
            MethodName::Tag => node
                .tags()
                .iter()
                .any(|t| pattern_matches(t, &pattern)),
            MethodName::Package => pattern_matches(&node.package_name(), &pattern),
            MethodName::ResourceType => {
                let rt = node.resource_type().as_static_ref();
                pattern_matches(rt, &pattern)
            }
            MethodName::File => {
                let path_str = normalize_path(&node.original_file_path().to_string_lossy());
                let file_name = path_str
                    .rsplit('/')
                    .next()
                    .unwrap_or(&path_str);
                pattern_matches(file_name, &pattern)
            }
            _ => false,
        };

        if matches {
            result.insert(unique_id.clone());
        }
    }

    // Handle nested exclude in Atom
    if let Some(ref exclude_expr) = criteria.exclude {
        let excluded = evaluator(exclude_expr, nodes)?;
        result = result.difference(&excluded).cloned().collect();
    }

    Ok(result)
}

/// Evaluates a selector expression against the given nodes and returns the set of matching node unique_ids.
pub fn evaluate_selector(expr: &SelectExpression, nodes: &Nodes) -> FsResult<BTreeSet<String>> {
    match expr {
        SelectExpression::Atom(criteria) => {
            filter_nodes_by_criteria(criteria, nodes, &evaluate_selector)
        }
        SelectExpression::And(expressions) => {
            // Special-case: And([includes, Exclude(pattern)]) → set difference (fix for #1279)
            if expressions.len() == 2
                && let SelectExpression::Exclude(exclude_pattern) = &expressions[1]
            {
                let included = evaluate_selector(&expressions[0], nodes)?;
                let excluded = evaluate_selector(exclude_pattern, nodes)?;
                if excluded.is_empty() {
                    return Ok(included);
                }
                return Ok(included.difference(&excluded).cloned().collect());
            }
            // General And: intersect all results
            let mut acc: Option<BTreeSet<String>> = None;
            for e in expressions {
                let set = evaluate_selector(e, nodes)?;
                acc = Some(match acc {
                    None => set,
                    Some(a) => a.intersection(&set).cloned().collect(),
                });
            }
            Ok(acc.unwrap_or_default())
        }
        SelectExpression::Or(expressions) => {
            let mut result = BTreeSet::new();
            for e in expressions {
                result.extend(evaluate_selector(e, nodes)?);
            }
            Ok(result)
        }
        SelectExpression::Exclude(inner) => {
            // Exclude returns the set to subtract; caller (And special-case) does the subtraction
            evaluate_selector(inner, nodes)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use dbt_common::node_selector::{MethodName, SelectionCriteria, SelectExpression};
    use dbt_schemas::schemas::{
        AdapterAttr, CommonAttributes, DbtModel, DbtModelAttr, NodeBaseAttributes, Nodes,
    };

    use super::evaluate_selector;

    /// Builds minimal [Nodes] with models bronze_1, bronze_2, bronze_3 whose paths match
    /// `models/test_exclude/bronze/bronze_*` for path-based selector tests.
    fn make_test_nodes_with_bronze_models() -> Nodes {
        let mut models = BTreeMap::new();
        for (name, unique_id) in [
            ("bronze_1", "model.test.bronze_1"),
            ("bronze_2", "model.test.bronze_2"),
            ("bronze_3", "model.test.bronze_3"),
        ] {
            let path = PathBuf::from(format!("models/test_exclude/bronze/{name}.sql"));
            let model = DbtModel {
                __common_attr__: CommonAttributes {
                    unique_id: unique_id.to_string(),
                    name: name.to_string(),
                    package_name: "test".to_string(),
                    fqn: vec!["test".to_string(), name.to_string()],
                    path: path.clone(),
                    original_file_path: path,
                    ..Default::default()
                },
                __base_attr__: NodeBaseAttributes::default(),
                __model_attr__: DbtModelAttr::default(),
                __adapter_attr__: AdapterAttr::default(),
                ..Default::default()
            };
            models.insert(unique_id.to_string(), Arc::new(model));
        }
        Nodes {
            models,
            ..Default::default()
        }
    }

    /// Builds [Nodes] with tagged models for tag-based tests.
    fn make_test_nodes_with_tags() -> Nodes {
        let mut models = BTreeMap::new();
        for (name, unique_id, tags) in [
            (
                "model_a",
                "model.test.model_a",
                vec!["tag1".to_string(), "tag2".to_string()],
            ),
            ("model_b", "model.test.model_b", vec!["tag1".to_string()]),
            ("model_c", "model.test.model_c", vec!["tag2".to_string()]),
        ] {
            let path = PathBuf::from(format!("models/{name}.sql"));
            let model = DbtModel {
                __common_attr__: CommonAttributes {
                    unique_id: unique_id.to_string(),
                    name: name.to_string(),
                    package_name: "test".to_string(),
                    fqn: vec!["test".to_string(), name.to_string()],
                    path: path.clone(),
                    original_file_path: path,
                    tags,
                    ..Default::default()
                },
                __base_attr__: NodeBaseAttributes::default(),
                __model_attr__: DbtModelAttr::default(),
                __adapter_attr__: AdapterAttr::default(),
                ..Default::default()
            };
            models.insert(unique_id.to_string(), Arc::new(model));
        }
        Nodes {
            models,
            ..Default::default()
        }
    }

    #[test]
    fn test_issue_1279_repro() {
        let nodes = make_test_nodes_with_bronze_models();
        let path_include = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronze/bronze_*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let path_exclude_no_match = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronse/no_such_model_*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::And(vec![
            SelectExpression::Atom(path_include),
            SelectExpression::Exclude(Box::new(SelectExpression::Atom(path_exclude_no_match))),
        ]);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert_eq!(
            result.len(),
            3,
            "Expected 3 nodes (intersection with empty exclude), got {}",
            result.len()
        );
        assert!(result.contains("model.test.bronze_1"));
        assert!(result.contains("model.test.bronze_2"));
        assert!(result.contains("model.test.bronze_3"));
    }

    #[test]
    fn test_and_empty_exclude_returns_included() {
        let nodes = make_test_nodes_with_bronze_models();
        let path_include = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronze/bronze_*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let path_exclude_no_match = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "path/that/does/not/exist/*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::And(vec![
            SelectExpression::Atom(path_include),
            SelectExpression::Exclude(Box::new(SelectExpression::Atom(path_exclude_no_match))),
        ]);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_and_exclude_with_matches_subtracts() {
        let nodes = make_test_nodes_with_bronze_models();
        let path_include = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronze/bronze_*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let path_exclude_bronze_1 = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronze/bronze_1.sql".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::And(vec![
            SelectExpression::Atom(path_include),
            SelectExpression::Exclude(Box::new(SelectExpression::Atom(path_exclude_bronze_1))),
        ]);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert_eq!(result.len(), 2);
        assert!(!result.contains("model.test.bronze_1"));
        assert!(result.contains("model.test.bronze_2"));
        assert!(result.contains("model.test.bronze_3"));
    }

    #[test]
    fn test_atom_path_matching_bronze_returns_nodes() {
        let nodes = make_test_nodes_with_bronze_models();
        let path_criteria = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronze/bronze_*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::Atom(path_criteria);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert!(
            !result.is_empty(),
            "Expected at least one node for path match bronze_*, got 0"
        );
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_atom_path_no_match_returns_empty() {
        let nodes = make_test_nodes_with_bronze_models();
        let path_criteria = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/nonexistent/*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::Atom(path_criteria);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert!(result.is_empty());
    }

    #[test]
    fn test_or_unions_results() {
        let nodes = make_test_nodes_with_bronze_models();
        let path_a = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronze/bronze_1.sql".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let path_b = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronze/bronze_2.sql".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::Or(vec![
            SelectExpression::Atom(path_a),
            SelectExpression::Atom(path_b),
        ]);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert_eq!(result.len(), 2);
        assert!(result.contains("model.test.bronze_1"));
        assert!(result.contains("model.test.bronze_2"));
    }

    #[test]
    fn test_and_intersects_without_exclude() {
        let nodes = make_test_nodes_with_bronze_models();
        let path_bronze = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronze/*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let fqn_1 = SelectionCriteria::new(
            MethodName::Fqn,
            vec![],
            "*.bronze_1".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::And(vec![
            SelectExpression::Atom(path_bronze),
            SelectExpression::Atom(fqn_1),
        ]);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert_eq!(result.len(), 1);
        assert!(result.contains("model.test.bronze_1"));
    }

    #[test]
    fn test_exclude_alone_returns_excluded_set() {
        let nodes = make_test_nodes_with_bronze_models();
        let path_bronze_1 = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronze/bronze_1.sql".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::Exclude(Box::new(SelectExpression::Atom(path_bronze_1)));

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert_eq!(result.len(), 1);
        assert!(result.contains("model.test.bronze_1"));
    }

    #[test]
    fn test_tag_exclude_empty() {
        let nodes = make_test_nodes_with_tags();
        let tag_include = SelectionCriteria::new(
            MethodName::Tag,
            vec![],
            "tag1".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let tag_exclude_no_match = SelectionCriteria::new(
            MethodName::Tag,
            vec![],
            "nonexistent_tag".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::And(vec![
            SelectExpression::Atom(tag_include),
            SelectExpression::Exclude(Box::new(SelectExpression::Atom(tag_exclude_no_match))),
        ]);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert_eq!(result.len(), 2);
        assert!(result.contains("model.test.model_a"));
        assert!(result.contains("model.test.model_b"));
    }

    #[test]
    fn test_fqn_exclude_empty() {
        let nodes = make_test_nodes_with_bronze_models();
        let fqn_include = SelectionCriteria::new(
            MethodName::Fqn,
            vec![],
            "test.bronze_*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let fqn_exclude_no_match = SelectionCriteria::new(
            MethodName::Fqn,
            vec![],
            "other.nonexistent_*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::And(vec![
            SelectExpression::Atom(fqn_include),
            SelectExpression::Exclude(Box::new(SelectExpression::Atom(fqn_exclude_no_match))),
        ]);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_empty_include_with_exclude() {
        let nodes = make_test_nodes_with_bronze_models();
        let path_no_match = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/empty/*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let path_exclude_no_match = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/also_empty/*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let expr = SelectExpression::And(vec![
            SelectExpression::Atom(path_no_match),
            SelectExpression::Exclude(Box::new(SelectExpression::Atom(path_exclude_no_match))),
        ]);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert!(result.is_empty());
    }

    #[test]
    fn test_nested_and_with_exclude() {
        let nodes = make_test_nodes_with_bronze_models();
        let path_include = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "models/test_exclude/bronze/bronze_*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let path_exclude_no_match = SelectionCriteria::new(
            MethodName::Path,
            vec![],
            "no_match/*".to_string(),
            false,
            None,
            None,
            None,
            None,
        );
        let inner_and = SelectExpression::And(vec![
            SelectExpression::Atom(path_include),
            SelectExpression::Exclude(Box::new(SelectExpression::Atom(path_exclude_no_match))),
        ]);
        let expr = SelectExpression::And(vec![inner_and]);

        let result = evaluate_selector(&expr, &nodes).expect("evaluate_selector should not error");
        assert_eq!(result.len(), 3);
    }
}
