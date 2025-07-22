use std::any::Any;

use dbt_adapter_proc_macros::{BaseColumnObject, StaticBaseColumnObject};
use dbt_common::current_function_name;
use minijinja::arg_utils::ArgParser;
use minijinja::arg_utils::check_num_args;
use minijinja::value::Enumerator;
use minijinja::{Error as MinijinjaError, Value};
use serde::{Deserialize, Serialize};

use super::base::StaticBaseColumn;
use super::base::{BaseColumn, BaseColumnProperties};

/// A struct representing a column type for use with static methods
#[derive(Clone, Debug, StaticBaseColumnObject)]
pub struct BigqueryColumnType;

impl StaticBaseColumn for BigqueryColumnType {
    fn try_new(
        name: String,
        dtype: String,
        _char_size: Option<u32>,
        _numeric_precision: Option<u64>,
        _numeric_scale: Option<u64>,
    ) -> Result<Value, MinijinjaError> {
        Ok(Value::from_object(BigqueryColumn::basic(name, dtype)))
    }

    // Translate the column type to a Bigquery type
    // https://github.com/dbt-labs/dbt-adapters/blob/6f2aae13e39c5df1c93e5d514678914142d71768/dbt-bigquery/src/dbt/adapters/bigquery/column.py#L16
    fn translate_type(args: &[Value]) -> Result<Value, MinijinjaError> {
        let mut args = ArgParser::new(args, None);
        let column_type: String = args.get("dtype")?;
        let column_type = match column_type.to_uppercase().as_str() {
            "TEXT" => "STRING",
            "FLOAT" => "FLOAT64",
            "INTEGER" => "INT64",
            _ => &column_type,
        };
        Ok(Value::from(column_type))
    }

    // https://github.com/dbt-labs/dbt-adapters/blob/6f2aae13e39c5df1c93e5d514678914142d71768/dbt-bigquery/src/dbt/adapters/bigquery/column.py#L97
    fn numeric_type(args: &[Value]) -> Result<Value, MinijinjaError> {
        let mut args: ArgParser = ArgParser::new(args, None);
        let dtype: String = args.get("dtype")?;
        Ok(Value::from(dtype))
    }

    // https://cloud.google.com/bigquery/docs/reference/standard-sql/data-types#string_type
    fn string_type(args: &[Value]) -> Result<Value, MinijinjaError> {
        let mut args: ArgParser = ArgParser::new(args, None);
        let size = args.get::<usize>("size").ok();
        match size {
            Some(size) => Ok(Value::from(format!("STRING({size})"))),
            _ => Ok(Value::from("STRING".to_string())),
        }
    }
}

/// A struct representing a column
#[derive(Clone, Debug, Default, BaseColumnObject, Serialize, Deserialize)]
pub struct BigqueryColumn {
    pub name: String,
    pub dtype: String,
    #[serde(default = "BigqueryColumn::default_mode")]
    pub mode: String,
}

impl BigqueryColumn {
    pub fn default_mode() -> String {
        "NULLABLE".to_owned()
    }

    pub fn basic(name: String, dtype: String) -> Self {
        Self {
            name,
            dtype,
            mode: "NULLABLE".to_owned(),
        }
    }
}

impl BaseColumn for BigqueryColumn {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_value(&self) -> Value {
        Value::from_object(self.clone())
    }

    fn is_numeric(&self) -> bool {
        matches!(self.dtype.to_lowercase().as_str(), "numeric")
    }

    fn is_integer(&self) -> bool {
        matches!(self.dtype.to_lowercase().as_str(), "int64")
    }

    fn is_float(&self) -> bool {
        matches!(self.dtype.to_lowercase().as_str(), "float64")
    }

    fn is_string(&self) -> bool {
        matches!(self.dtype.to_lowercase().as_str(), "string")
    }

    fn quoted(&self) -> Value {
        Value::from(&format!("`{}`", self.name))
    }

    // TODO: impl data_type - need to handle nested types
    // https://github.com/dbt-labs/dbt-adapters/blob/6f2aae13e39c5df1c93e5d514678914142d71768/dbt-bigquery/src/dbt/adapters/bigquery/column.py#L80
    fn data_type(&self) -> Value {
        Value::from(self.dtype.to_lowercase())
    }
}

impl BaseColumnProperties for BigqueryColumn {
    fn name_prop(&self) -> &str {
        &self.name
    }

    fn dtype_prop(&self) -> &str {
        &self.dtype
    }

    fn char_size_prop(&self) -> Option<u32> {
        None
    }

    fn numeric_precision_prop(&self) -> Option<u64> {
        None
    }

    fn numeric_scale_prop(&self) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // todo: refactor [StaticBaseColumn] and delete this test. We are just a pass-through since we
    // should trust the formatter that creates the actual dtype on the column
    #[test]
    fn test_bigquery_column_type_translation() {
        let args = vec![
            ("string".to_string(), "string"),
            ("STRING".to_string(), "STRING"),
        ];

        for (input, expected) in args {
            let value = Value::from(input.clone());
            let translated =
                BigqueryColumnType::translate_type(&[value]).expect("Failed to translate type");
            let result = translated
                .as_str()
                .expect("Failed to convert type to string");
            assert_eq!(result, expected, "Failed to translate type: {input}");
        }
    }
}
