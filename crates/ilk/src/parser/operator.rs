use std::collections::HashMap;

use thiserror::Error;

use super::ident::scan_ident;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorClass {
    Xfx,
    Xfy,
    Yfx,
    Fy,
}

impl OperatorClass {
    pub(crate) fn is_prefix(self) -> bool {
        matches!(self, Self::Fy)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperatorDefinition<'a> {
    pub name: &'a str,
    pub precedence: u16,
    pub class: OperatorClass,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum OperatorError {
    #[error("duplicate operator definition")]
    Duplicate,
    #[error("invalid operator name")]
    InvalidName,
    #[error("operator precedence must be between 1 and 1200")]
    InvalidPrecedence,
}

#[derive(Clone, Default)]
pub struct OperatorConfig<'a> {
    map: HashMap<(&'a str, bool), OperatorDefinition<'a>>,
}

impl<'a> OperatorConfig<'a> {
    pub fn new(defs: &[OperatorDefinition<'a>]) -> Result<Self, OperatorError> {
        let mut map: HashMap<(&'a str, bool), OperatorDefinition<'a>> = HashMap::new();

        for operator in defs.into_iter() {
            if !(1..=1200).contains(&operator.precedence) {
                return Err(OperatorError::InvalidPrecedence);
            }

            let name = operator.name;
            if scan_ident(name, 0) != Some(name.len()) || name.starts_with("/*") {
                return Err(OperatorError::InvalidName);
            }

            let key = (name, operator.class.is_prefix());
            if map.contains_key(&key) {
                return Err(OperatorError::Duplicate);
            }

            map.insert(key, *operator);
        }

        Ok(Self { map })
    }

    pub fn get(&self, name: &'a str, prefix: bool) -> Option<&OperatorDefinition<'a>> {
        self.map.get(&(name, prefix))
    }
}
