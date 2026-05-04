use crate::record_batch_utils::array_first_value_as_i64;
use crate::AdapterType;

use arrow::array::RecordBatch;
use dbt_agate::AgateTable;
use dbt_yaml::Value as YmlValue;
use minijinja::listener::RenderingEventListener;
use minijinja::value::{Enumerator, Object};
use minijinja::{State, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
            let values: Vec<_> = Self::SNOWFLAKE_DML_COLUMNS
                .iter()
                .map(|col| Self::first_i64(batch, col))
                .collect();
            if values.iter().any(Option::is_some) {
                return values.into_iter().map(|v| v.unwrap_or(0)).sum();
            }
        }
        batch.num_rows() as i64
    }

    /// Read the first row of a named numeric column as i64, or None if
    /// missing/null. Delegates to [`array_first_value_as_i64`] which handles
    /// Int8–64, UInt8–64, and Decimal128(_, 0) to account for Snowflake ADBC
    /// returning Decimal128(38, 0) when USE_HIGH_PRECISION is enabled.
    fn first_i64(batch: &RecordBatch, column_name: &str) -> Option<i64> {
        let schema = batch.schema();
        let idx = schema.index_of(column_name).ok()?;
        array_first_value_as_i64(batch.column(idx).as_ref(), schema.field(idx).data_type())
    }

    /// Column names that Snowflake returns for DML result metadata.
    const SNOWFLAKE_DML_COLUMNS: &[&str] = &[
        "number of rows inserted",
        "number of rows updated",
        "number of rows deleted",
    ];

    /// Returns true if the schema contains DML metadata columns that should
    /// be drained even when `fetch=false`. Keeps detection logic co-located
    /// with the parsing in `rows_affected` — only the columns that are
    /// actually summed are matched here.
    pub fn schema_has_dml_metadata(
        schema: &arrow::datatypes::Schema,
        adapter_type: AdapterType,
    ) -> bool {
        match adapter_type {
            AdapterType::Snowflake => schema
                .fields()
                .iter()
                .any(|f| Self::SNOWFLAKE_DML_COLUMNS.contains(&f.name().as_str())),
            _ => false,
        }
    }

    /// Convert to the `BTreeMap<String, YmlValue>` format expected by
    /// `ContextRunResult.adapter_response` / `RunResultOutput.adapter_response`.
    ///
    /// Keys match dbt-core's serialisation: `_message` (not `message`),
    /// `code`, `rows_affected`, and optionally `query_id`.
    pub fn to_adapter_response_map(&self) -> BTreeMap<String, YmlValue> {
        let mut map = BTreeMap::new();
        map.insert(
            "_message".to_string(),
            YmlValue::string(self.message.clone()),
        );
        map.insert("code".to_string(), YmlValue::string(self.code.clone()));
        map.insert(
            "rows_affected".to_string(),
            dbt_yaml::to_value(self.rows_affected).expect("i64 serialises to YAML"),
        );
        if let Some(qid) = &self.query_id {
            map.insert("query_id".to_string(), YmlValue::string(qid.clone()));
        }
        map
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
    use arrow::array::{Decimal128Array, Int64Array};
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
    fn test_snowflake_merge_decimal128_high_precision() {
        // Snowflake ADBC returns Decimal128(38, 0) when USE_HIGH_PRECISION is enabled
        let schema = Schema::new(vec![
            Field::new(
                "number of rows inserted",
                DataType::Decimal128(38, 0),
                false,
            ),
            Field::new("number of rows updated", DataType::Decimal128(38, 0), false),
            Field::new("number of rows deleted", DataType::Decimal128(38, 0), false),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(
                    Decimal128Array::from(vec![200])
                        .with_precision_and_scale(38, 0)
                        .unwrap(),
                ),
                Arc::new(
                    Decimal128Array::from(vec![75])
                        .with_precision_and_scale(38, 0)
                        .unwrap(),
                ),
                Arc::new(
                    Decimal128Array::from(vec![25])
                        .with_precision_and_scale(38, 0)
                        .unwrap(),
                ),
            ],
        )
        .unwrap();
        let resp = AdapterResponse::new(&batch, AdapterType::Snowflake);
        assert_eq!(resp.rows_affected, 300);
        assert_eq!(resp.message, "SUCCESS 300");
    }

    #[test]
    fn test_snowflake_insert_only_partial_dml_columns() {
        // INSERT...SELECT returns only "number of rows inserted"
        let schema = Schema::new(vec![Field::new(
            "number of rows inserted",
            DataType::Int64,
            false,
        )]);
        let batch =
            RecordBatch::try_new(Arc::new(schema), vec![Arc::new(Int64Array::from(vec![42]))])
                .unwrap();
        let resp = AdapterResponse::new(&batch, AdapterType::Snowflake);
        assert_eq!(resp.rows_affected, 42);
        assert_eq!(resp.message, "SUCCESS 42");
    }

    #[test]
    fn test_snowflake_empty_batch_returns_zero() {
        let schema = Schema::new(vec![
            Field::new("number of rows inserted", DataType::Int64, false),
            Field::new("number of rows updated", DataType::Int64, false),
            Field::new("number of rows deleted", DataType::Int64, false),
        ]);
        let batch = RecordBatch::new_empty(Arc::new(schema));
        let resp = AdapterResponse::new(&batch, AdapterType::Snowflake);
        assert_eq!(resp.rows_affected, 0);
        assert_eq!(resp.message, "SUCCESS 0");
    }

    #[test]
    fn test_snowflake_null_dml_values_treated_as_zero() {
        let schema = Schema::new(vec![
            Field::new("number of rows inserted", DataType::Int64, true),
            Field::new("number of rows updated", DataType::Int64, true),
            Field::new("number of rows deleted", DataType::Int64, true),
        ]);
        // inserted = 50, updated = NULL, deleted = NULL
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(Int64Array::from(vec![Some(50)])),
                Arc::new(Int64Array::from(vec![None])),
                Arc::new(Int64Array::from(vec![None])),
            ],
        )
        .unwrap();
        let resp = AdapterResponse::new(&batch, AdapterType::Snowflake);
        assert_eq!(resp.rows_affected, 50);
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
        assert!(!AdapterResponse::schema_has_dml_metadata(
            &schema,
            AdapterType::Snowflake
        ));
    }

    #[test]
    fn test_schema_has_dml_metadata() {
        let dml_schema = Schema::new(vec![Field::new(
            "number of rows inserted",
            DataType::Int64,
            false,
        )]);
        assert!(AdapterResponse::schema_has_dml_metadata(
            &dml_schema,
            AdapterType::Snowflake
        ));
        assert!(!AdapterResponse::schema_has_dml_metadata(
            &dml_schema,
            AdapterType::Bigquery
        ));
    }

    #[test]
    fn test_first_i64_missing_column_returns_none() {
        // Schema has no DML columns — first_i64 should return None for each
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(Int64Array::from(vec![99]))],
        )
        .unwrap();
        assert!(AdapterResponse::first_i64(&batch, "number of rows inserted").is_none());
        assert!(AdapterResponse::first_i64(&batch, "nonexistent").is_none());
    }

    #[test]
    fn test_to_adapter_response_map_matches_core_format() {
        let resp = AdapterResponse {
            message: "SUCCESS 42".to_string(),
            code: "SUCCESS".to_string(),
            rows_affected: 42,
            query_id: Some("01c2f954-abc".to_string()),
        };
        let map = resp.to_adapter_response_map();

        // Core uses `_message`, not `message`
        assert_eq!(
            map.get("_message").and_then(|v| v.as_str()),
            Some("SUCCESS 42")
        );
        assert_eq!(
            map.get("code").and_then(|v| v.as_str()),
            Some("SUCCESS")
        );
        assert_eq!(
            map.get("rows_affected").and_then(|v| v.as_i64()),
            Some(42)
        );
        assert_eq!(
            map.get("query_id").and_then(|v| v.as_str()),
            Some("01c2f954-abc")
        );
        // `message` key should NOT be present (Core uses `_message`)
        assert!(map.get("message").is_none());
    }

    #[test]
    fn test_to_adapter_response_map_omits_null_query_id() {
        let resp = AdapterResponse {
            message: "SUCCESS 0".to_string(),
            code: "SUCCESS".to_string(),
            rows_affected: 0,
            query_id: None,
        };
        let map = resp.to_adapter_response_map();
        assert!(map.get("query_id").is_none());
        assert_eq!(map.len(), 3); // _message, code, rows_affected
    }
}
