use crate::AdapterType;

use arrow::array::{Array, Int64Array, RecordBatch};
use dbt_agate::AgateTable;
use minijinja::listener::RenderingEventListener;
use minijinja::value::{Enumerator, Object};
use minijinja::{State, Value};
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use std::sync::Arc;

/// Response from adapter statement execution
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterResponse {
    /// Mantle compare_results historically emits `_message`
    #[serde(default, alias = "_message")]
    pub message: String,
    /// Status code from adapter
    #[serde(default)]
    pub code: String,
    /// Rows affected by statement
    #[serde(default)]
    pub rows_affected: i64,
    /// Query ID of executed statement, if available
    #[serde(default)]
    pub query_id: Option<String>,
}

impl AdapterResponse {
    pub fn new(batch: &RecordBatch, adapter_type: AdapterType) -> Self {
        let rows_affected = Self::rows_affected(batch, adapter_type);

        Self {
            message: format!("SUCCESS {}", rows_affected),
            code: Self::code(batch, adapter_type),
            rows_affected,
            query_id: Self::query_id(batch, adapter_type),
        }
    }

    /// Compute rows affected. For Snowflake DML statements (MERGE, INSERT,
    /// UPDATE, DELETE), the result batch contains columns like
    /// "number of rows inserted" — sum those instead of using batch.num_rows().
    fn rows_affected(batch: &RecordBatch, adapter_type: AdapterType) -> i64 {
        if batch.num_rows() == 0 {
            return 0;
        }
        if adapter_type == AdapterType::Snowflake {
            let inserted = Self::first_i64(batch, "number of rows inserted");
            let updated = Self::first_i64(batch, "number of rows updated");
            let deleted = Self::first_i64(batch, "number of rows deleted");
            if inserted.is_some() || updated.is_some() || deleted.is_some() {
                return inserted.unwrap_or(0) + updated.unwrap_or(0) + deleted.unwrap_or(0);
            }
        }
        batch.num_rows() as i64
    }

    /// Read the first row of a named Int64 column, or None if missing/null.
    fn first_i64(batch: &RecordBatch, column_name: &str) -> Option<i64> {
        let col = batch.column_by_name(column_name)?;
        if col.is_empty() || col.is_null(0) {
            return None;
        }
        col.as_any().downcast_ref::<Int64Array>().map(|a| a.value(0))
    }

    /// Returns true if the schema contains DML metadata columns that should
    /// be drained even when `fetch=false`. Keeps detection logic co-located
    /// with the parsing in `rows_affected`.
    pub fn schema_has_dml_metadata(schema: &arrow::datatypes::Schema, adapter_type: AdapterType) -> bool {
        match adapter_type {
            AdapterType::Snowflake => schema.fields().iter().any(|f| {
                f.name().to_lowercase().starts_with("number of rows ")
            }),
            _ => false,
        }
    }

    /// Get the code for the response from the batch.
    fn code(_batch: &RecordBatch, _adapter_type: AdapterType) -> String {
        "SUCCESS".to_string()
    }

    /// Get the query ID for the response from the batch.
    pub(crate) fn query_id(batch: &RecordBatch, adapter_type: AdapterType) -> Option<String> {
        match adapter_type {
            AdapterType::Snowflake => batch
                .schema()
                .metadata()
                .get("SNOWFLAKE_QUERY_ID")
                .map(|query_id: &String| query_id.to_string()),
            AdapterType::Bigquery => batch
                .schema()
                .metadata()
                .get("BIGQUERY:query_id")
                .map(|query_id: &String| query_id.to_string()),
            AdapterType::Databricks => batch
                .schema()
                .metadata()
                .get("DATABRICKS_QUERY_ID")
                .map(|query_id: &String| query_id.to_string()),
            _ => None,
        }
    }
}

impl Object for AdapterResponse {
    fn call(
        self: &Arc<Self>,
        _state: &State,
        _args: &[Value],
        _listeners: &[Rc<dyn RenderingEventListener>],
    ) -> Result<Value, minijinja::Error> {
        unimplemented!("Is response from 'execute' callable?")
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        match key.as_str()? {
            "message" => Some(Value::from(self.message.clone())),
            "code" => Some(Value::from(self.code.clone())),
            "rows_affected" => Some(Value::from(self.rows_affected)),
            "query_id" => Some(Value::from(self.query_id.clone())),
            _ => None,
        }
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Str(&["message", "code", "rows_affected", "query_id"])
    }
}

impl TryFrom<Value> for AdapterResponse {
    type Error = minijinja::Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Some(response) = value.downcast_object::<AdapterResponse>() {
            Ok((*response).clone())
        } else if let Some(message_str) = value.as_str() {
            Ok(AdapterResponse {
                message: message_str.to_string(),
                code: "".to_string(),
                rows_affected: 0,
                query_id: None,
            })
        } else {
            Err(minijinja::Error::new(
                minijinja::ErrorKind::CannotDeserialize,
                "Failed to downcast response",
            ))
        }
    }
}

/// load_result response object
#[derive(Debug)]
pub struct ResultObject {
    pub response: AdapterResponse,
    pub table: Option<AgateTable>,
    #[allow(unused)]
    pub data: Option<Value>,
}

impl ResultObject {
    pub fn new(response: AdapterResponse, table: Option<AgateTable>) -> Self {
        let data = if let Some(table) = &table {
            Some(Value::from_object(table.rows()))
        } else {
            Some(Value::UNDEFINED)
        };
        Self {
            response,
            table,
            data,
        }
    }
}

impl Object for ResultObject {
    fn call_method(
        self: &Arc<Self>,
        _state: &State<'_, '_>,
        method: &str,
        _args: &[Value],
        _listeners: &[Rc<dyn RenderingEventListener>],
    ) -> Result<Value, minijinja::Error> {
        // NOTE: the `keys` method is used by the `stage_external_sources` macro in
        // `dbt-external-table`. Don't delete this unless the external package is fixed.
        if method == "keys" {
            Ok(Value::from_iter(["response", "table", "data"]))
        } else {
            Err(minijinja::Error::new(
                minijinja::ErrorKind::UnknownMethod,
                format!("Unknown method on ResultObject: '{method}'"),
            ))
        }
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        match key.as_str()? {
            "table" => self
                .table
                .as_ref()
                .map(|t| Value::from_object((*t).clone())),
            "data" => self.data.clone(),
            "response" => Some(Value::from_object(self.response.clone())),
            _ => Some(Value::UNDEFINED), // Only return empty at Parsetime TODO fix later
        }
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Str(&["table", "data", "response"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Int64Array;
    use arrow::datatypes::{DataType, Field, Schema};

    #[test]
    fn test_snowflake_merge_sums_dml_counts() {
        let schema = Schema::new(vec![
            Field::new("number of rows inserted", DataType::Int64, false),
            Field::new("number of rows updated", DataType::Int64, false),
            Field::new("number of rows deleted", DataType::Int64, false),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(Int64Array::from(vec![100])),
                Arc::new(Int64Array::from(vec![50])),
                Arc::new(Int64Array::from(vec![10])),
            ],
        )
        .unwrap();
        let resp = AdapterResponse::new(&batch, AdapterType::Snowflake);
        assert_eq!(resp.rows_affected, 160);
        assert_eq!(resp.message, "SUCCESS 160");

        // Non-Snowflake ignores DML columns, falls back to batch.num_rows()
        let resp = AdapterResponse::new(&batch, AdapterType::Bigquery);
        assert_eq!(resp.rows_affected, 1);
    }

    #[test]
    fn test_snowflake_select_uses_num_rows() {
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema.clone()),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();
        let resp = AdapterResponse::new(&batch, AdapterType::Snowflake);
        assert_eq!(resp.rows_affected, 3);

        // DML metadata detection returns false for non-DML schemas
        assert!(!AdapterResponse::schema_has_dml_metadata(&schema, AdapterType::Snowflake));
    }

    #[test]
    fn test_schema_has_dml_metadata() {
        let dml_schema = Schema::new(vec![
            Field::new("number of rows inserted", DataType::Int64, false),
        ]);
        assert!(AdapterResponse::schema_has_dml_metadata(&dml_schema, AdapterType::Snowflake));
        assert!(!AdapterResponse::schema_has_dml_metadata(&dml_schema, AdapterType::Bigquery));
    }
}
