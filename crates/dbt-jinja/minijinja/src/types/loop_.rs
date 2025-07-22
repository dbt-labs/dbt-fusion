use crate::types::{builtin::Type, class::ClassType};

#[derive(Default, Clone, Eq, PartialEq)]
pub struct LoopType;

impl std::fmt::Debug for LoopType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("loop")
    }
}

impl ClassType for LoopType {
    fn get_attribute(&self, key: &str) -> Result<Type, crate::Error> {
        match key {
            "first" => Ok(Type::Bool),
            "last" => Ok(Type::Bool),
            _ => Err(crate::Error::new(
                crate::error::ErrorKind::InvalidOperation,
                format!("Unknown attribute: {self:?}.{key}"),
            )),
        }
    }
}
