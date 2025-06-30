use crate::table::TableRepr;
use crate::Tuple;
use crate::{MappedSequence, TupleRepr};
use core::fmt;
use minijinja::listener::RenderingEventListener;
use minijinja::value::{Enumerator, Object, ObjectRepr};
use minijinja::{Error as MinijinjaError, State, Value};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug)]
pub struct ColumnAsTuple {
    /// Always-valid index into the table's columns.
    index: usize,
    of_table: TableRepr,
}

impl ColumnAsTuple {
    fn from_single_column_table(table: TableRepr) -> Tuple {
        debug_assert_eq!(table.num_columns(), 1);
        let repr = ColumnAsTuple {
            index: 0,
            of_table: table,
        };
        Tuple(Box::new(repr))
    }
}

impl TupleRepr for ColumnAsTuple {
    fn get_item_by_index(&self, idx: isize) -> Option<Value> {
        self.of_table.cell(idx, self.index as isize)
    }

    fn len(&self) -> usize {
        self.of_table.num_rows()
    }

    fn count_occurrences_of(&self, value: &Value) -> usize {
        self.of_table
            .count_occurrences_of_value_in_column(value, self.index as isize)
    }

    fn index_of(&self, value: &Value) -> Option<usize> {
        self.of_table
            .index_of_value_in_column(value, self.index as isize)
    }

    fn clone_repr(&self) -> Box<dyn TupleRepr> {
        Box::new(ColumnAsTuple {
            index: self.index,
            of_table: self.of_table.clone(),
        })
    }
}

/// A column from an Agate table.
///
/// https://agate.readthedocs.io/en/latest/api/columns_and_rows.html#agate.Column
#[derive(Debug)]
pub struct Column {
    /// Always-valid index into the table's columns.
    index: usize,
    /// Internal representation of the table that contains this column.
    of_table: TableRepr,
}

impl Column {
    pub(crate) fn new(index: usize, of_table: TableRepr) -> Self {
        debug_assert!(index < of_table.num_columns());
        Self { index, of_table }
    }

    /// Get the distinct values in this column, as a tuple.
    ///
    /// Equivalent to `tuple(set(self.values()))`.
    pub fn values_distinct(&self) -> Tuple {
        let table = self.of_table.column_distinct(self.index as isize);
        ColumnAsTuple::from_single_column_table(table)
    }

    /// Get the values in this column with any null values removed.
    ///
    /// Equivalent to `tuple(d for d in self.values() if d is not None)`
    pub fn values_without_nulls(&self) -> Tuple {
        let table = self.of_table.column_without_nulls(self.index as isize);
        ColumnAsTuple::from_single_column_table(table)
    }

    /// Get the values in this column sorted.
    ///
    /// Equivalent to `sorted(self.values(), key=null_handler)`
    pub fn values_sorted(&self) -> Tuple {
        let table = self.of_table.column_sorted(self.index as isize);
        ColumnAsTuple::from_single_column_table(table)
    }

    /// Get the values in this column with any null values removed and sorted.
    ///
    /// Equivalent to `sorted(self.values_without_nulls(), key=null_handler)`
    pub fn values_without_nulls_sorted(&self) -> Tuple {
        let table = self
            .of_table
            .column_without_nulls_sorted(self.index as isize);
        ColumnAsTuple::from_single_column_table(table)
    }
}

impl MappedSequence for Column {
    fn type_name(&self) -> &str {
        "Column"
    }

    fn values(&self) -> Tuple {
        let column = ColumnAsTuple {
            index: self.index,
            of_table: self.of_table.clone(),
        };
        let repr = Box::new(column);
        Tuple(repr)
    }

    fn keys(&self) -> Option<Tuple> {
        None
    }
}

impl Object for Column {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        MappedSequence::repr(self)
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        if let Some(name) = key.as_str() {
            match name {
                // The column's index.
                "index" => Some(Value::from(self.index)),
                // The name of this column.
                "name" => self
                    .of_table
                    .column_name(self.index as isize)
                    .map(Value::from),
                // An instance of `AgateDataType`.
                "data_type" => todo!(),
                _ => MappedSequence::get_value(self, key),
            }
        } else {
            MappedSequence::get_value(self, key)
        }
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        MappedSequence::enumerate(self)
    }

    fn call_method(
        self: &Arc<Self>,
        state: &State<'_, '_>,
        method: &str,
        args: &[Value],
        listeners: &[Rc<dyn RenderingEventListener>],
    ) -> Result<Value, MinijinjaError> {
        match method {
            // Column methods
            "values_distinct" => {
                let distinct = self.values_distinct();
                Ok(Value::from_object(distinct))
            }
            "values_without_nulls" => {
                let without_nulls = self.values_without_nulls();
                Ok(Value::from_object(without_nulls))
            }
            "values_sorted" => {
                let sorted = self.values_sorted();
                Ok(Value::from_object(sorted))
            }
            "values_without_nulls_sorted" => {
                let without_nulls_sorted = self.values_without_nulls_sorted();
                Ok(Value::from_object(without_nulls_sorted))
            }
            // MappedSequence methods
            _ => MappedSequence::call_method(self, state, method, args, listeners),
        }
    }

    fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        MappedSequence::render(self, f)
    }
}
