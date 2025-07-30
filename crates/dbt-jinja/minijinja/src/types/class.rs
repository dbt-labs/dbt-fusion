use crate::types::builtin::Type;
use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
};

// Import the type_erase macro
use super::type_erase::type_erase;

pub trait ClassType: Send + Sync + std::fmt::Debug {
    fn get_attribute(&self, key: &str) -> Result<Type, crate::Error> {
        Err(crate::Error::new(
            crate::error::ErrorKind::InvalidOperation,
            format!("{self:?}.{key} is not supported"),
        ))
    }

    fn constructor(
        &self,
        _args: &[Type],
        _kwargs: &BTreeMap<String, Type>,
    ) -> Result<Type, crate::Error> {
        Err(crate::Error::new(
            crate::error::ErrorKind::InvalidOperation,
            format!("{self:?} does not support constructor"),
        ))
    }

    fn subscript(&self, _index: &Type) -> Result<Type, crate::Error> {
        Err(crate::Error::new(
            crate::error::ErrorKind::InvalidOperation,
            format!("{self:?} does not support subscript"),
        ))
    }
}

// Type-erased version of ClassType
type_erase! {
    pub trait ClassType => DynClassType {
        fn get_attribute(&self, key: &str) -> Result<Type, crate::Error>;
        fn constructor(&self, args: &[Type], kwargs: &BTreeMap<String, Type>) -> Result<Type, crate::Error>;
        fn subscript(&self, index: &Type) -> Result<Type, crate::Error>;
    }
}

impl std::fmt::Debug for DynClassType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.type_debug())
    }
}

impl PartialEq for DynClassType {
    fn eq(&self, other: &Self) -> bool {
        // Compare by pointer equality for type-erased objects
        self.ptr == other.ptr && self.vtable == other.vtable
    }
}

impl Eq for DynClassType {}

impl Hash for DynClassType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash the pointer and vtable
        self.ptr.hash(state);
        self.vtable.hash(state);
    }
}

impl PartialOrd for DynClassType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DynClassType {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare by pointer values
        match self.ptr.cmp(&other.ptr) {
            std::cmp::Ordering::Equal => self.vtable.cmp(&other.vtable),
            other => other,
        }
    }
}

unsafe impl Send for DynClassType {}
unsafe impl Sync for DynClassType {}
