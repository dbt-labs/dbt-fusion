use crate::table::TableRepr;
use crate::{MappedSequence, Tuple, TupleRepr};
use minijinja::listener::RenderingEventListener;
use minijinja::value::{Enumerator, Object, ObjectRepr};
use minijinja::{State, Value};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug)]
pub struct RowAsTuple {
    /// Always-valid index into the table's rows.
    index: usize,
    of_table: Arc<TableRepr>,
}

impl TupleRepr for RowAsTuple {
    fn get_item_by_index(&self, col_idx: isize) -> Option<Value> {
        let ncols = self.of_table.row_num_columns();
        let col_idx = if col_idx < 0 {
            let adjusted = col_idx + ncols as isize;
            if adjusted < 0 {
                return None;
            }
            adjusted as usize
        } else {
            col_idx as usize
        };
        self.of_table.row_cell(self.index, col_idx)
    }

    fn len(&self) -> usize {
        self.of_table.row_num_columns()
    }

    fn count_occurrences_of(&self, value: &Value) -> usize {
        let ncols = self.of_table.row_num_columns();
        let mut count = 0;
        for col_idx in 0..ncols {
            if let Some(cell) = self.of_table.row_cell(self.index, col_idx) {
                if cell == *value {
                    count += 1;
                }
            }
        }
        count
    }

    fn index_of(&self, value: &Value) -> Option<usize> {
        let ncols = self.of_table.row_num_columns();
        for col_idx in 0..ncols {
            if let Some(cell) = self.of_table.row_cell(self.index, col_idx) {
                if cell == *value {
                    return Some(col_idx);
                }
            }
        }
        None
    }

    fn clone_repr(&self) -> Box<dyn TupleRepr> {
        Box::new(RowAsTuple {
            index: self.index,
            of_table: Arc::clone(&self.of_table),
        })
    }
}

/// A row from an Agate table.
///
/// https://agate.readthedocs.io/en/latest/api/columns_and_rows.html#agate.Row
#[derive(Debug)]
pub struct Row {
    /// Always-valid index into the table's rows.
    index: usize,
    /// Internal representation of the table that contains this row.
    of_table: Arc<TableRepr>,
}

impl Row {
    /// Create a new row from an index and a table.
    pub(crate) fn new(index: usize, of_table: Arc<TableRepr>) -> Self {
        Self { index, of_table }
    }
}

/// A tuple of column names from the original (unflattened) schema.
///
/// Used by `Row::keys()` so that row key iteration matches the
/// original column count rather than the flattened column count.
#[derive(Debug)]
struct OriginalColumnNamesAsTuple {
    of_table: Arc<TableRepr>,
}

impl TupleRepr for OriginalColumnNamesAsTuple {
    fn get_item_by_index(&self, idx: isize) -> Option<Value> {
        let ncols = self.of_table.row_num_columns();
        let idx = if idx < 0 {
            let adjusted = idx + ncols as isize;
            if adjusted < 0 {
                return None;
            }
            adjusted as usize
        } else {
            idx as usize
        };
        if idx >= ncols {
            return None;
        }
        self.of_table
            .original_column_name(idx)
            .map(Value::from)
    }

    fn len(&self) -> usize {
        self.of_table.row_num_columns()
    }

    fn count_occurrences_of(&self, needle: &Value) -> usize {
        if let Some(name) = needle.as_str() {
            let ncols = self.of_table.row_num_columns();
            (0..ncols)
                .filter(|&i| self.of_table.original_column_name(i) == Some(name))
                .count()
        } else {
            0
        }
    }

    fn index_of(&self, needle: &Value) -> Option<usize> {
        if let Some(name) = needle.as_str() {
            let ncols = self.of_table.row_num_columns();
            (0..ncols).find(|&i| self.of_table.original_column_name(i) == Some(name))
        } else {
            None
        }
    }

    fn clone_repr(&self) -> Box<dyn TupleRepr> {
        Box::new(OriginalColumnNamesAsTuple {
            of_table: Arc::clone(&self.of_table),
        })
    }
}

impl MappedSequence for Row {
    fn type_name(&self) -> &str {
        "Row"
    }

    fn values(&self) -> Tuple {
        let row = RowAsTuple {
            index: self.index,
            of_table: Arc::clone(&self.of_table),
        };
        let repr = Box::new(row);
        Tuple(repr)
    }

    fn keys(&self) -> Option<Tuple> {
        let repr = OriginalColumnNamesAsTuple {
            of_table: Arc::clone(&self.of_table),
        };
        Some(Tuple(Box::new(repr)))
    }
}

impl Object for Row {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        MappedSequence::repr(self)
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        MappedSequence::get_value(self, key)
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
    ) -> Result<Value, minijinja::Error> {
        MappedSequence::call_method(self, state, method, args, listeners)
    }

    fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        MappedSequence::render(self, f)
    }
}
