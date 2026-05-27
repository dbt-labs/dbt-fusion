use crate::column::Column;
use crate::value::none_value;

use minijinja::value::{Enumerator, Object};
use minijinja::{Error, ErrorKind, Value};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

static BLOCK_COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/").expect("valid block comment regex"));
static LINE_COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"--.*?(\n|$)").expect("valid line comment regex"));
static WHITESPACE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+").expect("valid whitespace regex"));
static ENGINE_CLAUSE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\bengine\s*=\s*(.*?)(?:\s+order\s+by|\s+primary\s+key|\s+partition\s+by|\s+ttl\s+|\s+settings\s+|\s+empty\s+as|\s+as\s+|$)",
    )
    .expect("valid ClickHouse engine clause regex")
});
static ENGINE_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(.*\)$").expect("valid engine name regex"));
static EMPTY_PARENS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(\)$").expect("valid empty parens regex"));
static TTL_CLAUSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bttl\s+").expect("valid ClickHouse TTL clause regex"));
static REFRESH_CLAUSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\brefresh\b").expect("valid ClickHouse REFRESH regex"));
static RANDOMIZE_FOR_CLAUSE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\brandomize\s+for\b").expect("valid ClickHouse RANDOMIZE FOR regex")
});
static DEPENDS_ON_CLAUSE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bdepends\s+on\b").expect("valid ClickHouse DEPENDS ON regex")
});
static APPEND_CLAUSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bappend\b").expect("valid ClickHouse APPEND regex"));
static MATERIALIZED_VIEW_TO_TARGET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)\bto\s+((?:`[^`]+`|"[^"]+"|[^\s.()]+)(?:\s*\.\s*(?:`[^`]+`|"[^"]+"|[^\s.()]+))*)"#,
    )
    .expect("valid ClickHouse materialized view target regex")
});
static MATERIALIZED_VIEW_PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)^\s*create\s+materialized\s+view\s+(?:if\s+not\s+exists\s+)?(?:`[^`]+`|"[^"]+"|[^\s.()]+)(?:\s*\.\s*(?:`[^`]+`|"[^"]+"|[^\s.()]+))*\s*(?P<clauses>.*)$"#,
    )
    .expect("valid ClickHouse materialized view prefix regex")
});

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClickHouseMaterializedViewState {
    #[serde(default)]
    pub target_table: ClickHouseTargetTableState,
    #[serde(default)]
    pub target_table_columns: Vec<ClickHouseColumnState>,
    #[serde(default)]
    pub materialized_views: BTreeMap<String, ClickHouseMaterializedViewObjectState>,
}

impl ClickHouseMaterializedViewState {
    pub fn single_mv_name(target_identifier: &str) -> String {
        format!("{target_identifier}_mv")
    }

    pub fn try_from_value(value: &Value) -> Result<Self, Error> {
        let target_table = get_value(value, "target_table")
            .map(|v| ClickHouseTargetTableState::try_from_value(&v))
            .transpose()?
            .unwrap_or_default();
        let target_table_columns = get_value(value, "target_table_columns")
            .map(|v| column_signatures_from_value(&v))
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .map(|signature| ClickHouseColumnState { signature })
            .collect();
        let materialized_views = get_value(value, "materialized_views")
            .map(|v| materialized_views_from_value(&v))
            .transpose()?
            .unwrap_or_default();

        Ok(Self {
            target_table,
            target_table_columns,
            materialized_views,
        })
    }

    fn target_column_signatures(&self) -> Vec<String> {
        self.target_table_columns
            .iter()
            .map(|column| column.signature.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClickHouseColumnState {
    pub signature: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClickHouseTargetTableState {
    pub engine: Option<String>,
    pub order_by: Option<String>,
    pub primary_key: Option<String>,
    pub partition_by: Option<String>,
    pub ttl: Option<String>,
    pub create_table_query: Option<String>,
}

impl ClickHouseTargetTableState {
    fn try_from_value(value: &Value) -> Result<Self, Error> {
        Ok(Self {
            engine: option_string(get_value(value, "engine"))?,
            order_by: option_string(get_value(value, "order_by"))?,
            primary_key: option_string(get_value(value, "primary_key"))?,
            partition_by: option_string(get_value(value, "partition_by"))?,
            ttl: option_string(get_value(value, "ttl"))?,
            create_table_query: option_string(get_value(value, "create_table_query"))?,
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClickHouseMaterializedViewObjectState {
    pub query: Option<String>,
    pub refreshable_clause: Option<String>,
    pub create_table_query: Option<String>,
}

impl ClickHouseMaterializedViewObjectState {
    fn try_from_value(value: &Value) -> Result<Self, Error> {
        Ok(Self {
            query: option_string(get_value(value, "query").or_else(|| get_value(value, "sql")))?,
            refreshable_clause: option_string(get_value(value, "refreshable_clause"))?,
            create_table_query: option_string(get_value(value, "create_table_query"))?,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ClickHouseMaterializedViewQueryChanges {
    pub changed: Vec<String>,
    pub missing: Vec<String>,
    pub recreate: Vec<String>,
}

impl ClickHouseMaterializedViewQueryChanges {
    pub fn has_changes(&self) -> bool {
        !self.changed.is_empty() || !self.missing.is_empty() || !self.recreate.is_empty()
    }

    pub fn requires_full_refresh(&self) -> bool {
        !self.recreate.is_empty()
    }
}

impl Object for ClickHouseMaterializedViewQueryChanges {
    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        match key.as_str() {
            Some("has_changes") => Some(Value::from(self.has_changes())),
            Some("requires_full_refresh") => Some(Value::from(self.requires_full_refresh())),
            Some("changed") => Some(Value::from(self.changed.clone())),
            Some("missing") => Some(Value::from(self.missing.clone())),
            Some("recreate") => Some(Value::from(self.recreate.clone())),
            _ => None,
        }
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Str(&[
            "has_changes",
            "requires_full_refresh",
            "changed",
            "missing",
            "recreate",
        ])
    }
}

#[derive(Clone, Debug, Default)]
pub struct ClickHouseMaterializedViewChangeList {
    pub changes: Vec<String>,
    pub requires_full_refresh: bool,
    pub current_column_signatures: Vec<String>,
    pub expected_column_signatures: Vec<String>,
}

impl ClickHouseMaterializedViewChangeList {
    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }
}

impl Object for ClickHouseMaterializedViewChangeList {
    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        match key.as_str() {
            Some("has_changes") => Some(Value::from(self.has_changes())),
            Some("requires_full_refresh") => Some(Value::from(self.requires_full_refresh)),
            Some("changes") => Some(Value::from(self.changes.clone())),
            Some("current_column_signatures") => {
                Some(Value::from(self.current_column_signatures.clone()))
            }
            Some("expected_column_signatures") => {
                Some(Value::from(self.expected_column_signatures.clone()))
            }
            _ => None,
        }
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Str(&[
            "has_changes",
            "requires_full_refresh",
            "changes",
            "current_column_signatures",
            "expected_column_signatures",
        ])
    }
}

#[derive(Clone, Debug)]
pub struct ClickHouseMaterializedViewConfigChangeset {
    pub query_changes: ClickHouseMaterializedViewQueryChanges,
    pub target_table_config_changes: ClickHouseMaterializedViewChangeList,
    pub target_table_schema_changes: ClickHouseMaterializedViewChangeList,
}

impl ClickHouseMaterializedViewConfigChangeset {
    pub fn new(
        current: ClickHouseMaterializedViewState,
        desired: ClickHouseMaterializedViewState,
    ) -> Self {
        Self {
            query_changes: diff_materialized_views(&current, &desired),
            target_table_config_changes: diff_target_table_config(&current, &desired),
            target_table_schema_changes: diff_target_table_schema(&current, &desired),
        }
    }

    pub fn has_changes(&self) -> bool {
        self.query_changes.has_changes()
            || self.target_table_config_changes.has_changes()
            || self.target_table_schema_changes.has_changes()
    }

    pub fn requires_full_refresh(&self) -> bool {
        self.target_table_config_changes.requires_full_refresh
            || self.target_table_schema_changes.requires_full_refresh
    }
}

impl Object for ClickHouseMaterializedViewConfigChangeset {
    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        match key.as_str() {
            Some("has_changes") => Some(Value::from(self.has_changes())),
            Some("requires_full_refresh") => Some(Value::from(self.requires_full_refresh())),
            Some("query_changes") => Some(Value::from_object(self.query_changes.clone())),
            Some("target_table_config_changes") => {
                Some(Value::from_object(self.target_table_config_changes.clone()))
            }
            Some("target_table_schema_changes") => {
                Some(Value::from_object(self.target_table_schema_changes.clone()))
            }
            _ => None,
        }
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Str(&[
            "has_changes",
            "requires_full_refresh",
            "query_changes",
            "target_table_config_changes",
            "target_table_schema_changes",
        ])
    }
}

pub fn clickhouse_materialized_view_config_changeset(
    current_value: &Value,
    desired_value: &Value,
) -> Result<Value, Error> {
    let current = ClickHouseMaterializedViewState::try_from_value(current_value)?;
    let desired = ClickHouseMaterializedViewState::try_from_value(desired_value)?;
    let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

    if changeset.has_changes() {
        Ok(Value::from_object(changeset))
    } else {
        Ok(none_value())
    }
}

pub fn materialized_view_points_to_target(
    create_table_query: Option<&str>,
    target_schema: &str,
    target_identifier: &str,
) -> bool {
    let Some(create_table_query) = create_table_query else {
        return false;
    };
    let Some(captures) = MATERIALIZED_VIEW_TO_TARGET_RE.captures(create_table_query) else {
        return false;
    };
    let Some(raw_target) = captures.get(1).map(|target| target.as_str()) else {
        return false;
    };
    let target_parts = normalize_clickhouse_relation_parts(raw_target);
    let expected_schema = normalize_clickhouse_identifier(target_schema);
    let expected_identifier = normalize_clickhouse_identifier(target_identifier);

    match target_parts.as_slice() {
        [identifier] => identifier == &expected_identifier,
        [.., schema, identifier] => {
            schema == &expected_schema && identifier == &expected_identifier
        }
        _ => false,
    }
}

fn diff_materialized_views(
    current: &ClickHouseMaterializedViewState,
    desired: &ClickHouseMaterializedViewState,
) -> ClickHouseMaterializedViewQueryChanges {
    let mut changes = ClickHouseMaterializedViewQueryChanges::default();

    for (name, desired_view) in &desired.materialized_views {
        let Some(current_view) = current.materialized_views.get(name) else {
            changes.missing.push(name.clone());
            continue;
        };

        if materialized_view_ddl_config_changed(desired_view, current_view) {
            changes.recreate.push(name.clone());
            continue;
        }

        let current_query = current_view.query.as_deref().unwrap_or_default();
        let desired_query = desired_view.query.as_deref().unwrap_or_default();
        if canonicalize_sql(current_query) != canonicalize_sql(desired_query) {
            changes.changed.push(name.clone());
        }
    }

    changes
}

fn diff_target_table_config(
    current: &ClickHouseMaterializedViewState,
    desired: &ClickHouseMaterializedViewState,
) -> ClickHouseMaterializedViewChangeList {
    let current_config = &current.target_table;
    let desired_config = &desired.target_table;
    let mut changes = Vec::new();

    let current_create_engine =
        engine_config_from_create_table_query(current_config.create_table_query.as_deref());
    let desired_engine = normalize_engine_config(desired_config.engine.as_deref());

    if let Some(current_create_engine) = current_create_engine {
        if current_create_engine != desired_engine {
            changes.push("engine".to_string());
        }
    } else if normalize_engine_name(current_config.engine.as_deref())
        != normalize_engine_name(desired_config.engine.as_deref())
    {
        changes.push("engine".to_string());
    }

    let desired_order_by = desired_config.order_by.as_deref().or_else(|| {
        normalize_engine_name(desired_config.engine.as_deref())
            .contains("mergetree")
            .then_some("tuple()")
    });
    if normalize_order_by_value(current_config.order_by.as_deref())
        != normalize_order_by_value(desired_order_by)
    {
        changes.push("order_by".to_string());
    }

    let current_primary_key = normalize_order_by_value(current_config.primary_key.as_deref());
    let desired_primary_key =
        if desired_config.primary_key.is_some() || !current_primary_key.is_empty() {
            normalize_order_by_value(desired_config.primary_key.as_deref().or(desired_order_by))
        } else {
            String::new()
        };
    if current_primary_key != desired_primary_key {
        changes.push("primary_key".to_string());
    }

    if normalize_config_value(current_config.partition_by.as_deref())
        != normalize_config_value(desired_config.partition_by.as_deref())
    {
        changes.push("partition_by".to_string());
    }

    let current_ttl = normalize_config_value(current_config.ttl.as_deref());
    let desired_ttl = normalize_config_value(desired_config.ttl.as_deref());
    if desired_ttl.is_empty() {
        if !current_ttl.is_empty()
            || create_table_query_has_ttl(current_config.create_table_query.as_deref())
        {
            changes.push("ttl".to_string());
        }
    } else if !current_ttl.is_empty() {
        if current_ttl != desired_ttl {
            changes.push("ttl".to_string());
        }
    } else {
        let current_create = normalize_config_value(current_config.create_table_query.as_deref());
        if !current_create.contains(&desired_ttl) {
            changes.push("ttl".to_string());
        }
    }

    ClickHouseMaterializedViewChangeList {
        requires_full_refresh: !changes.is_empty(),
        changes,
        ..Default::default()
    }
}

fn diff_target_table_schema(
    current: &ClickHouseMaterializedViewState,
    desired: &ClickHouseMaterializedViewState,
) -> ClickHouseMaterializedViewChangeList {
    let current_column_signatures = current.target_column_signatures();
    let expected_column_signatures = desired.target_column_signatures();
    let mut changes = Vec::new();

    if current_column_signatures != expected_column_signatures {
        changes.push("columns".to_string());
    }

    ClickHouseMaterializedViewChangeList {
        requires_full_refresh: !changes.is_empty(),
        changes,
        current_column_signatures,
        expected_column_signatures,
    }
}

fn materialized_view_ddl_config_changed(
    desired: &ClickHouseMaterializedViewObjectState,
    current: &ClickHouseMaterializedViewObjectState,
) -> bool {
    let expected_refreshable = normalize_config_value(desired.refreshable_clause.as_deref());
    let current_create =
        canonicalize_sql(current.create_table_query.as_deref().unwrap_or_default());
    let current_prefix_sql = current_create.split(" to ").next().unwrap_or_default();
    let current_clause_sql = materialized_view_ddl_clause_sql(current_prefix_sql);
    let current_clause = normalize_config_value(Some(current_clause_sql));

    if expected_refreshable.is_empty() {
        return materialized_view_ddl_has_refreshable_clause(current_clause_sql);
    }

    if !current_clause.contains(&expected_refreshable) {
        return true;
    }

    for (clause_name, clause_re) in [
        ("randomizefor", &*RANDOMIZE_FOR_CLAUSE_RE),
        ("dependson", &*DEPENDS_ON_CLAUSE_RE),
        ("append", &*APPEND_CLAUSE_RE),
    ] {
        if clause_re.is_match(current_clause_sql) && !expected_refreshable.contains(clause_name) {
            return true;
        }
    }

    false
}

fn materialized_view_ddl_clause_sql(current_prefix_sql: &str) -> &str {
    let Some(captures) = MATERIALIZED_VIEW_PREFIX_RE.captures(current_prefix_sql) else {
        return current_prefix_sql;
    };
    captures
        .name("clauses")
        .map(|clauses| clauses.as_str())
        .unwrap_or(current_prefix_sql)
}

fn materialized_view_ddl_has_refreshable_clause(current_prefix_sql: &str) -> bool {
    [
        &*REFRESH_CLAUSE_RE,
        &*RANDOMIZE_FOR_CLAUSE_RE,
        &*DEPENDS_ON_CLAUSE_RE,
        &*APPEND_CLAUSE_RE,
    ]
    .iter()
    .any(|clause_re| clause_re.is_match(current_prefix_sql))
}

fn canonicalize_sql(sql: &str) -> String {
    let sql = BLOCK_COMMENT_RE.replace_all(sql, " ");
    let sql = LINE_COMMENT_RE.replace_all(&sql, " ");
    let sql = WHITESPACE_RE.replace_all(&sql, " ");
    sql.trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !matches!(c, '`' | '"'))
        .collect()
}

fn normalize_config_value(value: Option<&str>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    let value = value.trim();
    let value = value
        .strip_prefix('(')
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or(value);
    let normalized = value
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    normalized
        .chars()
        .filter(|c| !matches!(c, '`' | '"'))
        .collect()
}

fn normalize_column_data_type(value: Option<&str>) -> String {
    let mut normalized = normalize_config_value(value);
    loop {
        let Some(inner) = strip_type_wrapper(&normalized, "nullable")
            .or_else(|| strip_type_wrapper(&normalized, "lowcardinality"))
        else {
            break;
        };
        normalized = inner.to_string();
    }

    if normalized == "fixedstring(256)" {
        "string".to_string()
    } else {
        normalized
    }
}

fn strip_type_wrapper<'a>(value: &'a str, wrapper: &str) -> Option<&'a str> {
    let rest = value.strip_prefix(wrapper)?;
    rest.strip_prefix('(')?.strip_suffix(')')
}

fn normalize_order_by_value(value: Option<&str>) -> String {
    match normalize_config_value(value).as_str() {
        "" | "tuple" | "tuple()" => String::new(),
        normalized => normalized.to_string(),
    }
}

fn normalize_engine_name(value: Option<&str>) -> String {
    let normalized = normalize_config_value(value);
    ENGINE_NAME_RE.replace(&normalized, "").to_string()
}

fn normalize_engine_config(value: Option<&str>) -> String {
    let normalized = normalize_config_value(value);
    EMPTY_PARENS_RE.replace(&normalized, "").to_string()
}

fn engine_config_from_create_table_query(create_table_query: Option<&str>) -> Option<String> {
    let normalized = create_table_query.map(|value| {
        WHITESPACE_RE
            .replace_all(value, " ")
            .trim()
            .to_ascii_lowercase()
    })?;
    let captures = ENGINE_CLAUSE_RE.captures(&normalized)?;
    captures
        .get(1)
        .map(|engine| normalize_engine_config(Some(engine.as_str())))
        .filter(|engine| !engine.is_empty())
}

fn create_table_query_has_ttl(create_table_query: Option<&str>) -> bool {
    create_table_query.is_some_and(|query| TTL_CLAUSE_RE.is_match(query))
}

fn normalize_clickhouse_relation_parts(raw_relation: &str) -> Vec<String> {
    raw_relation
        .split('.')
        .map(normalize_clickhouse_identifier)
        .filter(|part| !part.is_empty())
        .collect()
}

fn normalize_clickhouse_identifier(identifier: &str) -> String {
    let identifier = identifier.trim();
    let identifier = identifier
        .strip_prefix('`')
        .and_then(|value| value.strip_suffix('`'))
        .or_else(|| {
            identifier
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
        })
        .unwrap_or(identifier);
    identifier.to_ascii_lowercase()
}

fn get_value(value: &Value, key: &str) -> Option<Value> {
    value
        .get_item(&Value::from(key))
        .ok()
        .filter(|v| !v.is_undefined())
}

fn option_string(value: Option<Value>) -> Result<Option<String>, Error> {
    let Some(value) = value else {
        return Ok(None);
    };
    value_to_string(&value)
}

fn value_to_string(value: &Value) -> Result<Option<String>, Error> {
    if value.is_none() || value.is_undefined() {
        return Ok(None);
    }
    if let Some(raw) = value.as_str() {
        return Ok(Some(raw.to_string()));
    }
    if let Ok(iter) = value.try_iter() {
        let mut items = Vec::new();
        for item in iter {
            if let Some(item) = value_to_string(&item)? {
                items.push(item);
            }
        }
        return Ok(Some(items.join(",")));
    }
    Ok(Some(value.to_string()))
}

fn materialized_views_from_value(
    value: &Value,
) -> Result<BTreeMap<String, ClickHouseMaterializedViewObjectState>, Error> {
    let mut views = BTreeMap::new();
    for key in value.try_iter().map_err(|e| {
        Error::new(
            ErrorKind::InvalidOperation,
            format!("materialized_views must be iterable: {e}"),
        )
    })? {
        let name = value_to_string(&key)?.ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidOperation,
                "materialized view names must be strings",
            )
        })?;
        let view_value = value.get_item(&key)?;
        views.insert(
            name,
            ClickHouseMaterializedViewObjectState::try_from_value(&view_value)?,
        );
    }
    Ok(views)
}

fn column_signatures_from_value(value: &Value) -> Result<Vec<String>, Error> {
    let mut signatures = Vec::new();
    for column_value in value.try_iter().map_err(|e| {
        Error::new(
            ErrorKind::InvalidOperation,
            format!("target_table_columns must be iterable: {e}"),
        )
    })? {
        if let Some(signature) = column_signature_from_value(&column_value)? {
            signatures.push(signature);
        }
    }
    Ok(signatures)
}

fn column_signature_from_value(value: &Value) -> Result<Option<String>, Error> {
    if let Some(column) = value.downcast_object_ref::<Column>() {
        return Ok(Some(format_column_signature(
            column.name(),
            Some(column.data_type().as_str()),
        )));
    }

    if let Some(name) = value.as_str() {
        return Ok(Some(name.to_string()));
    }

    let name = option_string(get_value(value, "name").or_else(|| get_value(value, "column")))?;
    let data_type =
        option_string(get_value(value, "data_type").or_else(|| get_value(value, "dtype")))?;
    Ok(name.map(|name| format_column_signature(&name, data_type.as_deref())))
}

fn format_column_signature(name: &str, data_type: Option<&str>) -> String {
    let data_type = normalize_column_data_type(data_type);
    if data_type.is_empty() {
        name.to_string()
    } else {
        format!("{name}:{data_type}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(
        engine: &str,
        order_by: &str,
        query: &str,
        columns: Vec<(&str, &str)>,
    ) -> ClickHouseMaterializedViewState {
        ClickHouseMaterializedViewState {
            target_table: ClickHouseTargetTableState {
                engine: Some(engine.to_string()),
                order_by: Some(order_by.to_string()),
                create_table_query: Some(format!(
                    "CREATE TABLE `test`.`events` ENGINE = {engine} ORDER BY {order_by}"
                )),
                ..Default::default()
            },
            target_table_columns: columns
                .into_iter()
                .map(|(name, data_type)| ClickHouseColumnState {
                    signature: format_column_signature(name, Some(data_type)),
                })
                .collect(),
            materialized_views: BTreeMap::from([(
                "events_mv".to_string(),
                ClickHouseMaterializedViewObjectState {
                    query: Some(query.to_string()),
                    create_table_query: Some(format!(
                        "CREATE MATERIALIZED VIEW `test`.`events_mv` TO `test`.`events` AS {query}"
                    )),
                    ..Default::default()
                },
            )]),
        }
    }

    #[test]
    fn materialized_view_target_parser_accepts_quoted_target() {
        assert!(materialized_view_points_to_target(
            Some(
                "CREATE MATERIALIZED VIEW `TEST_SCHEMA`.`events_mv` TO `TEST_SCHEMA`.`events` AS select 1"
            ),
            "TEST_SCHEMA",
            "events",
        ));
    }

    #[test]
    fn materialized_view_target_parser_accepts_unquoted_target() {
        assert!(materialized_view_points_to_target(
            Some("CREATE MATERIALIZED VIEW events_mv TO TEST_SCHEMA.events AS select 1"),
            "test_schema",
            "events",
        ));
    }

    #[test]
    fn materialized_view_target_parser_rejects_other_target() {
        assert!(!materialized_view_points_to_target(
            Some(
                "CREATE MATERIALIZED VIEW `TEST_SCHEMA`.`events_mv` TO `TEST_SCHEMA`.`other_events` AS select 1"
            ),
            "TEST_SCHEMA",
            "events",
        ));
    }

    #[test]
    fn query_only_change_can_modify_existing_mv_without_target_rebuild() {
        let current = state(
            "MergeTree()",
            "id",
            "select id, count() as total from raw group by id",
            vec![("id", "UInt64"), ("total", "UInt64")],
        );
        let desired = state(
            "MergeTree()",
            "id",
            "select id, sum(amount) as total from raw group by id",
            vec![("id", "UInt64"), ("total", "UInt64")],
        );

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(changeset.has_changes());
        assert!(!changeset.requires_full_refresh());
        assert_eq!(changeset.query_changes.changed, vec!["events_mv"]);
    }

    #[test]
    fn refreshable_config_change_recreates_mv_without_target_rebuild() {
        let query = "select id, count() as total from raw group by id";
        let mut current = state(
            "MergeTree()",
            "id",
            query,
            vec![("id", "UInt64"), ("total", "UInt64")],
        );
        let mut desired = state(
            "MergeTree()",
            "id",
            query,
            vec![("id", "UInt64"), ("total", "UInt64")],
        );
        let current_view = current
            .materialized_views
            .get_mut("events_mv")
            .expect("events_mv should exist");
        let current_create_table_query = format!(
            "CREATE MATERIALIZED VIEW `test`.`events_mv` REFRESH EVERY 2 HOUR TO `test`.`events` AS {query}"
        );
        current_view.create_table_query = Some(current_create_table_query);
        let desired_view = desired
            .materialized_views
            .get_mut("events_mv")
            .expect("events_mv should exist");
        desired_view.refreshable_clause = Some("REFRESH EVERY 1 HOUR".to_string());

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(changeset.has_changes());
        assert!(changeset.query_changes.requires_full_refresh());
        assert!(!changeset.requires_full_refresh());
        assert_eq!(changeset.query_changes.recreate, vec!["events_mv"]);
    }

    #[test]
    fn removing_refreshable_append_recreates_mv_without_target_rebuild() {
        let query = "select id, count() as total from raw group by id";
        let mut current = state(
            "MergeTree()",
            "id",
            query,
            vec![("id", "UInt64"), ("total", "UInt64")],
        );
        let mut desired = state(
            "MergeTree()",
            "id",
            query,
            vec![("id", "UInt64"), ("total", "UInt64")],
        );
        let current_view = current
            .materialized_views
            .get_mut("events_mv")
            .expect("events_mv should exist");
        current_view.create_table_query = Some(format!(
            "CREATE MATERIALIZED VIEW `test`.`events_mv` REFRESH EVERY 1 HOUR APPEND TO `test`.`events` AS {query}"
        ));
        let desired_view = desired
            .materialized_views
            .get_mut("events_mv")
            .expect("events_mv should exist");
        desired_view.refreshable_clause = Some("REFRESH EVERY 1 HOUR".to_string());

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(changeset.has_changes());
        assert!(!changeset.requires_full_refresh());
        assert_eq!(changeset.query_changes.recreate, vec!["events_mv"]);
    }

    #[test]
    fn refresh_word_in_generated_mv_name_does_not_recreate_non_refreshable_mv() {
        let query = "select id, count() as total from raw group by id";
        let current = ClickHouseMaterializedViewState {
            target_table: ClickHouseTargetTableState {
                engine: Some("MergeTree()".to_string()),
                order_by: Some("id".to_string()),
                create_table_query: Some(
                    "CREATE TABLE `test`.`refresh-events` ENGINE = MergeTree() ORDER BY id"
                        .to_string(),
                ),
                ..Default::default()
            },
            target_table_columns: vec![
                ClickHouseColumnState {
                    signature: format_column_signature("id", Some("UInt64")),
                },
                ClickHouseColumnState {
                    signature: format_column_signature("total", Some("UInt64")),
                },
            ],
            materialized_views: BTreeMap::from([(
                "refresh-events_mv".to_string(),
                ClickHouseMaterializedViewObjectState {
                    query: Some(query.to_string()),
                    create_table_query: Some(format!(
                        "CREATE MATERIALIZED VIEW `test`.`refresh-events_mv` TO `test`.`refresh-events` AS {query}"
                    )),
                    ..Default::default()
                },
            )]),
        };
        let desired = ClickHouseMaterializedViewState {
            materialized_views: BTreeMap::from([(
                "refresh-events_mv".to_string(),
                ClickHouseMaterializedViewObjectState {
                    query: Some(query.to_string()),
                    ..Default::default()
                },
            )]),
            ..current.clone()
        };

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(
            !changeset.has_changes(),
            "unexpected changeset for non-refreshable MV: {changeset:?}"
        );
    }

    #[test]
    fn quoted_target_table_order_by_does_not_force_target_rebuild() {
        let query = "select id, event_type, count() as total from raw group by id, event_type";
        let mut current = state(
            "MergeTree()",
            "`id`, `event_type`",
            query,
            vec![
                ("id", "UInt64"),
                ("event_type", "String"),
                ("total", "UInt64"),
            ],
        );
        current.target_table.create_table_query = Some(
            "CREATE TABLE `test`.`events` ENGINE = MergeTree() ORDER BY (`id`, `event_type`)"
                .to_string(),
        );
        let desired = state(
            "MergeTree()",
            "id,event_type",
            query,
            vec![
                ("id", "UInt64"),
                ("event_type", "String"),
                ("total", "UInt64"),
            ],
        );

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(!changeset.has_changes());
    }

    #[test]
    fn clickhouse_default_primary_key_matches_desired_order_by() {
        let query = "select status, count() as total from raw group by status";
        let mut current = state(
            "SummingMergeTree()",
            "status",
            query,
            vec![("status", "String"), ("total", "UInt64")],
        );
        current.target_table.primary_key = Some("status".to_string());
        let desired = state(
            "SummingMergeTree()",
            "status",
            query,
            vec![("status", "String"), ("total", "UInt64")],
        );

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(
            !changeset.has_changes(),
            "ClickHouse defaults PRIMARY KEY to ORDER BY; unexpected changeset: {changeset:?}"
        );
    }

    #[test]
    fn nullable_query_schema_matches_clickhouse_materialized_target_columns() {
        let query = "select status, sum(amount) as total_amount, count() as order_count from raw group by status";
        let current = state(
            "SummingMergeTree()",
            "status",
            query,
            vec![
                ("status", "FixedString(256)"),
                ("total_amount", "Int64"),
                ("order_count", "UInt64"),
            ],
        );
        let desired = state(
            "SummingMergeTree()",
            "status",
            query,
            vec![
                ("status", "Nullable(String)"),
                ("total_amount", "Nullable(Int64)"),
                ("order_count", "Nullable(UInt64)"),
            ],
        );

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(
            !changeset.has_changes(),
            "ClickHouse query schema nullability should not force target rebuild: {changeset:?}"
        );
    }

    #[test]
    fn quoted_source_relation_query_matches_clickhouse_as_select() {
        let current = state(
            "SummingMergeTree()",
            "status",
            "SELECT status, sum(toInt64(amount)) AS total_amount, count() AS order_count FROM test.raw_orders GROUP BY status",
            vec![
                ("status", "String"),
                ("total_amount", "Int64"),
                ("order_count", "UInt64"),
            ],
        );
        let desired = state(
            "SummingMergeTree()",
            "status",
            "select status, sum(toInt64(amount)) as total_amount, count() as order_count from `test`.`raw_orders` group by status",
            vec![
                ("status", "String"),
                ("total_amount", "Int64"),
                ("order_count", "UInt64"),
            ],
        );

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(
            !changeset.has_changes(),
            "ClickHouse as_select should match Fusion quoted relation SQL: {changeset:?}"
        );
    }

    #[test]
    fn default_mergetree_order_by_tuple_matches_empty_sorting_key() {
        let query = "select id, count() as total from raw group by id";
        let mut current = state(
            "MergeTree()",
            "",
            query,
            vec![("id", "UInt64"), ("total", "UInt64")],
        );
        current.target_table.create_table_query =
            Some("CREATE TABLE `test`.`events` ENGINE = MergeTree() ORDER BY tuple()".to_string());
        let mut desired = state(
            "MergeTree()",
            "tuple()",
            query,
            vec![("id", "UInt64"), ("total", "UInt64")],
        );
        desired.target_table.order_by = None;

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(!changeset.has_changes());
    }

    #[test]
    fn target_table_engine_change_requires_target_rebuild() {
        let current = state(
            "MergeTree()",
            "id",
            "select id, count() as total from raw group by id",
            vec![("id", "UInt64"), ("total", "UInt64")],
        );
        let desired = state(
            "ReplacingMergeTree(version)",
            "id",
            "select id, count() as total from raw group by id",
            vec![("id", "UInt64"), ("total", "UInt64")],
        );

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(changeset.requires_full_refresh());
        assert_eq!(
            changeset.target_table_config_changes.changes,
            vec!["engine"]
        );
    }

    #[test]
    fn removing_target_table_ttl_requires_target_rebuild() {
        let query = "select id, count() as total from raw group by id";
        let mut current = state(
            "MergeTree()",
            "id",
            query,
            vec![("id", "UInt64"), ("total", "UInt64")],
        );
        current.target_table.create_table_query = Some(
            "CREATE TABLE `test`.`events` ENGINE = MergeTree() ORDER BY id TTL event_time + INTERVAL 7 DAY"
                .to_string(),
        );
        let desired = state(
            "MergeTree()",
            "id",
            query,
            vec![("id", "UInt64"), ("total", "UInt64")],
        );

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(changeset.requires_full_refresh());
        assert_eq!(changeset.target_table_config_changes.changes, vec!["ttl"]);
    }

    #[test]
    fn target_table_column_type_change_requires_target_rebuild() {
        let current = state(
            "MergeTree()",
            "id",
            "select id, count() as total from raw group by id",
            vec![("id", "UInt64"), ("total", "UInt64")],
        );
        let desired = state(
            "MergeTree()",
            "id",
            "select id, count() as total from raw group by id",
            vec![("id", "UInt64"), ("total", "Float64")],
        );

        let changeset = ClickHouseMaterializedViewConfigChangeset::new(current, desired);

        assert!(changeset.requires_full_refresh());
        assert_eq!(
            changeset.target_table_schema_changes.changes,
            vec!["columns"]
        );
    }
}
