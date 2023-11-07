use num::BigUint;
use serde::{Deserialize, Serialize};
use spade_common::{
    name::{Identifier, NameID},
    num_ext::InfallibleToBigUint,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PrimitiveType {
    Int,
    Uint,
    Clock,
    Bool,
    Bit,
    Memory,
    Void,
}

impl std::fmt::Display for PrimitiveType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrimitiveType::Int => write!(f, "int"),
            PrimitiveType::Uint => write!(f, "uint"),
            PrimitiveType::Clock => write!(f, "clk"),
            PrimitiveType::Bool => write!(f, "bool"),
            PrimitiveType::Bit => write!(f, "bit"),
            PrimitiveType::Memory => write!(f, "Memory"),
            PrimitiveType::Void => write!(f, "()"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConcreteType {
    Tuple(Vec<ConcreteType>),
    Struct {
        name: NameID,
        members: Vec<(Identifier, ConcreteType)>,
    },
    Array {
        inner: Box<ConcreteType>,
        size: BigUint,
    },
    Enum {
        options: Vec<(NameID, Vec<(Identifier, ConcreteType)>)>,
    },
    Single {
        base: PrimitiveType,
        params: Vec<ConcreteType>,
    },
    Integer(BigUint),
    Backward(Box<ConcreteType>),
    Wire(Box<ConcreteType>),
}

impl ConcreteType {
    pub fn assume_struct(&self) -> (&NameID, &Vec<(Identifier, ConcreteType)>) {
        match self {
            ConcreteType::Struct { name, members } => (name, members),
            t => unreachable!("Assumed {} was a struct", t),
        }
    }

    pub fn is_port(&self) -> bool {
        match self {
            ConcreteType::Tuple(inner) => inner.iter().any(Self::is_port),
            ConcreteType::Struct { name: _, members } => members.iter().any(|(_, t)| t.is_port()),
            ConcreteType::Array { inner, size: _ } => inner.is_port(),
            // Enums cannot be ports
            ConcreteType::Enum { .. } => false,
            ConcreteType::Single {
                base: PrimitiveType::Memory,
                ..
            } => true,
            ConcreteType::Single {
                base: PrimitiveType::Clock,
                ..
            } => true,
            ConcreteType::Single { .. } => false,
            ConcreteType::Integer(_) => false,
            ConcreteType::Backward(_) => true,
            ConcreteType::Wire(_) => true,
        }
    }
}

impl std::fmt::Display for ConcreteType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConcreteType::Tuple(inner) => {
                write!(
                    f,
                    "({})",
                    inner
                        .iter()
                        .map(|p| format!("{}", p))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            ConcreteType::Struct { name, members } => {
                write!(
                    f,
                    "struct {name} {{{}}}",
                    members
                        .iter()
                        .map(|(name, t)| format!("{}: {}", name, t))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            ConcreteType::Array { inner, size } => {
                write!(f, "[{}; {}]", inner, size)
            }
            ConcreteType::Enum { options } => {
                write!(f, "enum {{",)?;
                let inner = options
                    .iter()
                    .map(|o| {
                        let param_list =
                            o.1.iter()
                                .map(|t| format!("{}", t.1))
                                .collect::<Vec<_>>()
                                .join(",");
                        format!("{} ( {} )", o.0 .0, param_list)
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                write!(f, "{}", inner)?;
                write!(f, "}}")
            }
            ConcreteType::Single { base, params } => {
                let params_str = if params.is_empty() {
                    format!("")
                } else {
                    format!(
                        "{}",
                        params
                            .iter()
                            .map(|p| format!("{}", p))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };

                write!(f, "{}{}", base, params_str)
            }
            ConcreteType::Integer(size) => {
                write!(f, "#{}", size)
            }
            ConcreteType::Backward(inner) => {
                write!(f, "&mut {}", inner)
            }
            ConcreteType::Wire(inner) => {
                write!(f, "&{}", inner)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum KnownType {
    Named(NameID),
    Integer(BigUint),
    Tuple,
    Array,
    Backward,
    Wire,
    Inverted,
    Traits(Vec<NameID>),
}

impl KnownType {
    pub fn integer(val: u64) -> Self {
        Self::Integer(val.to_biguint())
    }
}
