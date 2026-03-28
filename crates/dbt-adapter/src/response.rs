use crate::record_batch_utils::get_column_values;
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
    /// Number of rows inserted by a DML statement, if available
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_inserted: Option<i64>,
    /// Number of rows updated by a DML statement, if available
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_updated: Option<i64>,
    /// Number of rows deleted by a DML statement, if available
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_deleted: Option<i64>,
}

impl AdapterResponse {
    pub fn new(batch: &RecordBatch, adapter_type: AdapterType) -> Self {
        let (rows_inserted, rows_updated, rows_deleted) =
            Self::parse_dml_counts(batch, adapter_type);

        let rows_affected = match (&rows_inserted, &rows_updated, &rows_deleted) {
            (Some(i), Some(u), Some(d)) => i + u + d,
            _ => batch.num_rows() as i64,
        };

        Self {
            message: Self::message(batch, adapter_type),
            code: Self::code(batch, adapter_type),
            rows_affected,
            query_id: Self::query_id(batch, adapter_type),
            rows_inserted,
            rows_updated,
            rows_deleted,
        }
    }

    /// Extract DML row counts from the RecordBatch for supported adapters.
    fn parse_dml_counts(
        batch: &RecordBatch,
        adapter_type: AdapterType,
    ) -> (Option<i64>, Option<i64>, Option<i64>) {
        if batch.num_rows() == 0 {
            return (None, None, None);
        }
        match adapter_type {
            AdapterType::Snowflake => {
                let inserted = Self::first_i64(batch, "number of rows inserted");
                let updated = Self::first_i64(batch, "number of rows updated");
                let deleted = Self::first_i64(batch, "number of rows deleted");
                (inserted, updated, deleted)
            }
            _ => (None, None, None),
        }
    }

    /// Get the first row's value from a named Int64 column, or None if
    /// the column is missing or the value is null.
    fn first_i64(batch: &RecordBatch, column_name: &str) -> Option<i64> {
        let arr = get_column_values::<Int64Array>(batch, column_name).ok()?;
        if !arr.is_empty() && !arr.is_null(0) {
            Some(arr.value(0))
        } else {
            None
        }
    }

    /// Get the message for the response from the batch.
    fn message(batch: &RecordBatch, _adapter_type: AdapterType) -> String {
        format!("{} {}", "SUCCESS", batch.num_rows())
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
            "rows_inserted" => self.rows_inserted.map(Value::from),
            "rows_updated" => self.rows_updated.map(Value::from),
            "rows_deleted" => self.rows_deleted.map(Value::from),
            _ => None,
        }
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Str(&[
            "message",
            "code",
            "rows_affected",
            "query_id",
            "rows_inserted",
            "rows_updated",
            "rows_deleted",
        ])
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
                ..Default::default()
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

    fn snowflake_merge_batch(inserted: i64, updated: i64, deleted: i64) -> RecordBatch {
        let schema = Schema::new(vec![
            Field::new("number of rows inserted", DataType::Int64, false),
            Field::new("number of rows updated", DataType::Int64, false),
            Field::new("number of rows deleted", DataType::Int64, false),
        ]);
        RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(Int64Array::from(vec![inserted])),
                Arc::new(Int64Array::from(vec![updated])),
                Arc::new(Int64Array::from(vec![deleted])),
            ],
        )
        .unwrap()
    }

    #[test]
    fn test_snowflake_merge_parses_dml_counts() {
        let batch = snowflake_merge_batch(100, 50, 10);
        let resp = AdapterResponse::new(&batch, AdapterType::Snowflake);
        assert_eq!(resp.rows_inserted, Some(100));
        assert_eq!(resp.rows_updated, Some(50));
        assert_eq!(resp.rows_deleted, Some(10));
        assert_eq!(resp.rows_affected, 160);
    }

    #[test]
    fn test_empty_batch_has_no_dml_counts() {
        let schema = Schema::new(vec![
            Field::new("number of rows inserted", DataType::Int64, false),
        ]);
        let batch = RecordBatch::new_empty(Arc::new(schema));
        let resp = AdapterResponse::new(&batch, AdapterType::Snowflake);
        assert_eq!(resp.rows_inserted, None);
        assert_eq!(resp.rows_affected, 0);
    }

    #[test]
    fn test_non_snowflake_has_no_dml_counts() {
        let batch = snowflake_merge_batch(100, 50, 10);
        let resp = AdapterResponse::new(&batch, AdapterType::Bigquery);
        assert_eq!(resp.rows_inserted, None);
        assert_eq!(resp.rows_updated, None);
        assert_eq!(resp.rows_deleted, None);
        assert_eq!(resp.rows_affected, 1); // batch.num_rows()
    }

    #[test]
    fn test_jinja_get_value_exposes_dml_fields() {
        let response = Arc::new(AdapterResponse {
            rows_inserted: Some(100),
            rows_updated: Some(50),
            rows_deleted: None,
            ..Default::default()
        });
        assert_eq!(response.get_value(&Value::from("rows_inserted")), Some(Value::from(100_i64)));
        assert_eq!(response.get_value(&Value::from("rows_updated")), Some(Value::from(50_i64)));
        assert_eq!(response.get_value(&Value::from("rows_deleted")), None);
        assert_eq!(response.get_value(&Value::from("nonexistent")), None);
    }

    #[test]
    fn test_serde_backward_compat() {
        let json = r#"{"message":"SUCCESS 42","code":"SUCCESS","rows_affected":42,"query_id":"q-1"}"#;
        let resp: AdapterResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.rows_affected, 42);
        assert_eq!(resp.rows_inserted, None);
    }

    #[test]
    fn test_serde_omits_none_dml_fields() {
        let resp = AdapterResponse {
            message: "SUCCESS 0".to_string(),
            code: "SUCCESS".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("rows_inserted"));
        assert!(!json.contains("rows_updated"));
        assert!(!json.contains("rows_deleted"));
    }
}
