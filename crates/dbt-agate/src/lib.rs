use core::fmt;
use std::rc::Rc;
use std::sync::Arc;

use minijinja::arg_utils::ArgsIter;
use minijinja::listener::RenderingEventListener;
use minijinja::value::{Enumerator, Object, ObjectRepr};
use minijinja::{Error as MinijinjaError, ErrorKind, State, Value, assert_nullary_args};

mod column;
mod columns;
mod converters;
mod decimal;
mod print_table;
mod row;
mod rows;
mod table;

pub(crate) mod flat_record_batch;
mod vec_of_rows;

pub use column::Column;
pub use columns::Columns;
pub use print_table::print_table;
pub use row::Row;
pub use rows::Rows;
pub use table::AgateTable;

/// Agate uses Python tuples to represent sequences of values.
///
/// Unlike Python lists, tuples are immutable and have a smaller interface.
trait TupleRepr: fmt::Debug + Send + Sync {
    /// Get a value from the tuple by index.
    fn get_item_by_index(&self, idx: isize) -> Option<Value>;

    /// Get the length of the tuple.
    fn len(&self) -> usize;

    /// Implement the `count` method for tuples.
    fn count_occurrences_of(&self, value: &Value) -> usize;

    /// Implement the `index` method for tuples.
    fn index_of(&self, value: &Value) -> Option<usize>;

    /// Clone this tuple representation (virtually-dispatched).
    fn clone_repr(&self) -> Box<dyn TupleRepr>;
}

/// A tuple object that behaves like a Python tuple.
///
/// We need a custom implementation to avoid materializing the tuple in memory
/// by relying on sharing references to the underlying data from a table.
#[derive(Debug)]
pub struct Tuple(Box<dyn TupleRepr>);

impl Tuple {
    /// Get a value from the tuple by index.
    pub fn get(&self, idx: isize) -> Option<Value> {
        self.0.get_item_by_index(idx)
    }

    /// Get the length of the tuple.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if the tuple is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Count the number of occurrences of a value in the tuple.
    pub fn count(&self, value: &Value) -> usize {
        self.0.count_occurrences_of(value)
    }

    /// Find the index of a value in the tuple.
    pub fn index(&self, value: &Value) -> Option<usize> {
        self.0.index_of(value)
    }
}

impl fmt::Display for Tuple {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(")?;
        for i in 0..self.len() {
            let value = self.get(i as isize).unwrap();
            write!(f, "{value}, ")?;
        }
        write!(f, ")")
    }
}

impl Object for Tuple {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Seq
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        if let Some(idx) = key.as_i64() {
            self.get(idx as isize)
        } else {
            None
        }
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Seq(self.len())
    }

    fn enumerator_len(self: &Arc<Self>) -> Option<usize> {
        Some(self.len())
    }

    #[allow(clippy::only_used_in_recursion)]
    fn call_method(
        self: &Arc<Self>,
        state: &State,
        name: &str,
        args: &[Value],
        listeners: &[Rc<dyn RenderingEventListener>],
    ) -> Result<Value, MinijinjaError> {
        match name {
            "count" => {
                let iter = ArgsIter::for_unnamed_pos_args("tuple.count", 0, args);
                let value = iter.next_arg::<&Value>()?;
                iter.finish()?;
                let count = self.0.count_occurrences_of(value);
                Ok(Value::from(count))
            }
            "index" => {
                let iter = ArgsIter::for_unnamed_pos_args("tuple.index", 0, args);
                let value = iter.next_arg::<&Value>()?;
                iter.finish()?;
                let idx = self.0.index_of(value);
                Ok(Value::from(idx))
            }
            _ => Object::call_method(self, state, name, args, listeners),
        }
    }

    fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The equivalent of tuple(zip(first, second)) in Python.
#[derive(Debug)]
struct ZippedTupleRepr {
    /// The first tuple in the zipped tuple.
    first: Box<dyn TupleRepr>,
    /// The second tuple in the zipped tuple.
    second: Box<dyn TupleRepr>,
}

impl ZippedTupleRepr {
    /// Create a new zipped tuple from two tuples.
    pub fn new(first: Box<dyn TupleRepr>, second: Box<dyn TupleRepr>) -> Self {
        debug_assert!(first.len() == second.len());
        Self { first, second }
    }
}

fn value_as_pair(value: &Value) -> Option<(Value, Value)> {
    // value must be a tuple of length 2 to have a chance
    // of matching the pairs in the zipped tuple
    if value.len().unwrap_or(0) != 2 {
        return None;
    }
    let fst_value = value.get_item_by_index(0);
    let snd_value = value.get_item_by_index(1);
    if fst_value.is_err() || snd_value.is_err() {
        return None;
    }
    Some((fst_value.unwrap(), snd_value.unwrap()))
}

impl TupleRepr for ZippedTupleRepr {
    fn get_item_by_index(&self, idx: isize) -> Option<Value> {
        let fst = self.first.get_item_by_index(idx)?;
        let snd = self.second.get_item_by_index(idx)?;
        Some(Value::from_iter([fst, snd]))
    }

    fn len(&self) -> usize {
        // first and second have the same length
        self.first.len()
    }

    fn count_occurrences_of(&self, value: &Value) -> usize {
        match value_as_pair(value) {
            Some((fst_value, snd_value)) => {
                let mut count = 0;
                for i in 0..self.len() {
                    let fst = self.first.get_item_by_index(i as isize).unwrap();
                    let snd = self.second.get_item_by_index(i as isize).unwrap();
                    if fst == fst_value && snd == snd_value {
                        count += 1;
                    }
                }
                count
            }
            None => 0,
        }
    }

    fn index_of(&self, value: &Value) -> Option<usize> {
        match value_as_pair(value) {
            Some((fst_value, snd_value)) => {
                for i in 0..self.len() {
                    let fst = self.first.get_item_by_index(i as isize).unwrap();
                    let snd = self.second.get_item_by_index(i as isize).unwrap();
                    if fst == fst_value && snd == snd_value {
                        return Some(i);
                    }
                }
                None
            }
            None => None,
        }
    }

    fn clone_repr(&self) -> Box<dyn TupleRepr> {
        let first = self.first.clone_repr();
        let second = self.second.clone_repr();
        let repr = ZippedTupleRepr::new(first, second);
        Box::new(repr)
    }
}

/// An object that behaves like a Python `OrderedDict`.
///
/// We need a custom implementation to avoid materializing the map in memory.
/// All operations are lazy and delegate to the underlying `TupleRepr` objects
/// produced by `MappedSequence`.
#[derive(Debug)]
pub struct OrderedDict {
    /// The keys in the ordered dictionary.
    keys: Tuple,
    /// The values in the ordered dictionary.
    values: Tuple,
}

impl OrderedDict {
    fn new(keys: Tuple, values: Tuple) -> Self {
        debug_assert!(keys.len() == values.len());
        Self { keys, values }
    }

    /// Retrieve the value for a given key.
    pub fn get(&self, key: &Value) -> Option<Value> {
        // These ordered maps are small, so a linear search is not only acceptable
        // but probably faster than building hash maps on value creation.
        for i in 0..self.keys.len() {
            if self
                .keys
                .get(i as isize)
                .map(|k| k == *key)
                .unwrap_or(false)
            {
                return self.values.get(i as isize);
            }
        }
        None
    }

    /// The equivalent of tuple(zip(self.keys(), self.values())).
    pub fn items(&self) -> Option<Tuple> {
        let keys = self.keys.0.clone_repr();
        let values = self.values.0.clone_repr();
        let zipped = ZippedTupleRepr::new(keys, values);
        let repr = Box::new(zipped);
        Some(Tuple(repr))
    }
}

impl fmt::Display for OrderedDict {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let len = self.keys.len();
        write!(f, "OrderedDict({{")?;
        for i in 0..len {
            let key = self.keys.get(i as isize).unwrap();
            let value = self.values.get(i as isize).unwrap();
            write!(f, "{key}: {value}")?;
            if i < len - 1 {
                write!(f, ", ")?;
            }
        }
        write!(f, "}})")
    }
}

impl Object for OrderedDict {
    // TODO(felipecrv): Implement Object for OrderedDict
}

/// A generic container for immutable data that can be accessed either by
/// numeric index or by key. This is similar to a Python `OrderedDict` except
/// that the keys are optional and iteration over it returns the values instead
/// of keys.
///
/// Implementors should delegate Object and fmt::Display methods to this trait.
pub trait MappedSequence {
    // https://github.com/wireservice/agate/blob/master/agate/mapped_sequence.py

    /// The equivalent of Python's type(self).__name__.
    fn type_name(&self) -> &str {
        "MappedSequence"
    }

    /// Values as a Python tuple.
    ///
    /// __iter__ should iterate over the values in this sequence.
    /// enumerate(self) should enumerate the values in this sequence.
    /// __len__ should return the number of values in this sequence.
    /// __eq__, __ne__, and __contains__ should use these values.
    fn values(&self) -> Tuple;

    /// A Python list of the keys in the sequence (optional).
    fn keys(&self) -> Option<Tuple>;

    /// A tuple of (key, value) pairs in this [`MappedSequence`] (optional).
    fn items(&self) -> Option<Tuple> {
        let dict = self.dict()?;
        dict.items()
    }

    /// Retrieve the value for a given key, or a default value if the key is not
    /// present.
    fn get(self: &Arc<Self>, key: &Value, default: Option<&Value>) -> Value {
        let value = self.get_value(key);
        match value {
            Some(value) => value,
            None => default.cloned().unwrap_or(Value::from(())),
        }
    }

    /// Retrieve the contents of this sequence as an ordered dict.
    ///
    /// If keys() are not defined, this is also not defined.
    fn dict(&self) -> Option<OrderedDict> {
        self.keys()
            .map(|keys| OrderedDict::new(keys, self.values()))
    }

    // impl of the Object trait for MappedSequence objects ---------------------

    /// See [`minijinja::Value::repr`].
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Seq
    }

    /// Retrieve values from this array by index, slice or key.
    ///
    /// Based on MappedSequence.__getitem__.
    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        if let Some(idx) = key.as_i64() {
            self.values().get(idx as isize)
        } else {
            let dict = self.dict()?;
            dict.get(key)
        }
    }

    /// See [`minijinja::Object::enumerate`].
    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Seq(self.values().len())
    }

    /// See [`minijinja::Object::call_method`].
    fn call_method(
        self: &Arc<Self>,
        state: &State,
        name: &str,
        args: &[Value],
        listeners: &[Rc<dyn RenderingEventListener>],
    ) -> Result<Value, MinijinjaError> {
        match name {
            // MappedSequence methods
            "values" => {
                assert_nullary_args!("MappedSequence.values", args)?;
                let values = self.values();
                Ok(Value::from_object(values))
            }
            "keys" => {
                assert_nullary_args!("MappedSequence.keys", args)?;
                let keys = self
                    .keys()
                    .map(Value::from_object)
                    .unwrap_or(Value::from(()));
                Ok(keys)
            }
            "items" => {
                assert_nullary_args!("MappedSequence.items", args)?;
                if let Some(items) = self.items() {
                    Ok(Value::from_object(items))
                } else {
                    // trying to approximate a `raise KeyError`
                    Err(MinijinjaError::new(
                        ErrorKind::NonKey,
                        format!("{} type does not define keys()", self.type_name()),
                    ))
                }
            }
            "get" => {
                // def get(self, key, default=None)
                let iter = ArgsIter::new("MappedSequence.get", &["key"], args);
                let key = iter.next_arg::<&Value>()?;
                let default = iter.next_kwarg::<Option<&Value>>("default")?;
                iter.finish()?;
                let value = self.get(key, default);
                Ok(value)
            }
            "dict" => {
                assert_nullary_args!("MappedSequence.items", args)?;
                if let Some(dict) = self.dict() {
                    Ok(Value::from_object(dict))
                } else {
                    // trying to approximate a `raise KeyError`
                    Err(MinijinjaError::new(
                        ErrorKind::NonKey,
                        format!("{} type does not define keys()", self.type_name()),
                    ))
                }
            }
            _ => {
                if let Some(value) = self.get_value(&Value::from(name)) {
                    return value.call(state, args, listeners);
                }
                Err(MinijinjaError::from(ErrorKind::UnknownMethod(
                    "MappedSequence".to_string(),
                    name.to_string(),
                )))
            }
        }
    }

    /// See [`minijinja::Object::render`].
    fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt(f)
    }

    // impl of the fmt::Display trait for MappedSequence objects -------------------

    /// Used to implement the equivalent of __unicode__, __str__, and __repr__ in Python.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let values = self.values();
        let len = values.len();
        write!(f, "<agate.{}: (", self.type_name())?;
        for i in 0..len.min(5) {
            if let Some(value) = values.get(i as isize) {
                write!(f, "{value}")?;
                if i < len - 1 {
                    write!(f, ", ")?;
                }
            }
        }
        if len > 5 {
            write!(f, ", ...")?;
        }
        write!(f, ")>")?;
        Ok(())
    }
}
