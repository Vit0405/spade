use std::collections::HashMap;

use serde::{Serialize, Deserialize};
use spade_common::{location_info::Loc, name::NameID};
use spade_hir::{Expression, Pattern};
use spade_mir::ValueName;

#[derive(Clone, Serialize, Deserialize)]
pub enum NameSource {
    Name(Loc<NameID>),
    Expr(Loc<u64>),
}

impl std::fmt::Display for NameSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NameSource::Name(n) => write!(f, "{n}"),
            NameSource::Expr(id) => write!(f, "e{id}"),
        }
    }
}

impl From<&Loc<NameID>> for NameSource {
    fn from(n: &Loc<NameID>) -> Self {
        NameSource::Name(n.clone())
    }
}

impl From<&Loc<u64>> for NameSource {
    fn from(e: &Loc<u64>) -> Self {
        NameSource::Expr(e.clone())
    }
}

impl From<&Loc<Expression>> for NameSource {
    fn from(e: &Loc<Expression>) -> Self {
        Self::from(&e.map_ref(|e| e.id))
    }
}

impl From<&Loc<Pattern>> for NameSource {
    fn from(e: &Loc<Pattern>) -> Self {
        Self::from(&e.map_ref(|e| e.id))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum NamedValue {
    /// This name corresponds to the primary expression of the specified name. I.e. the location
    /// where the value is actually stored
    Primary(NameSource),
    /// This value is a secondary expression used in the calculation of the primary name. For
    /// example, a boolean value or a pipelined version of a value. The associated string
    /// is a description of the use
    Secondary(NameSource, String),
}

impl std::fmt::Display for NamedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NamedValue::Primary(n) => write!(f, "{n}"),
            NamedValue::Secondary(n, descr) => write!(f, "{n}({descr})"),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct NameSourceMap {
    pub inner: HashMap<ValueName, NamedValue>,
}

impl NameSourceMap {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    pub fn insert_primary(&mut self, name: &ValueName, unmangled: NameSource) {
        self.inner
            .insert(name.clone(), NamedValue::Primary(unmangled));
    }
    pub fn insert_secondary(
        &mut self,
        name: &ValueName,
        unmangled: NameSource,
        description: String,
    ) {
        self.inner
            .insert(name.clone(), NamedValue::Secondary(unmangled, description));
    }

    pub fn merge(&mut self, other: NameSourceMap) {
        for (k, v) in other.inner {
            if let Some(_) = self.inner.insert(k.clone(), v) {
                panic!("Duplicate name map for {k}")
            }
        }
    }
}
