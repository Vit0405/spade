use thiserror::Error;

use crate::equation::{FreeTypeVar, TypeVarRef};

use super::equation::{InnerTypeVar, TypedExpression};
use spade_common::{
    location_info::{Loc, WithLocation},
    name::{Identifier, NameID},
};

/// A trace of a unification error. The `failing` field indicates which exact type failed to unify,
/// while the `inside` is the "top level" type which failed to unify if it's not the same as
/// failing.
///
/// For example, if unifying `int<7>` with `int<8>`, this would be `failing: 8, inside: int<8>`
/// while if unifying `int<7>` with `bool`, inside would be `None`
#[derive(Debug, PartialEq, Clone)]
pub struct UnificationTrace {
    pub failing: FreeTypeVar,
    pub inside: Option<FreeTypeVar>,
}
impl WithLocation for UnificationTrace {}
impl std::fmt::Display for UnificationTrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.outer())
    }
}
impl UnificationTrace {
    pub fn new(failing: FreeTypeVar) -> Self {
        Self {
            failing,
            inside: None,
        }
    }

    pub fn outer(&self) -> &FreeTypeVar {
        self.inside.as_ref().unwrap_or(&self.failing)
    }
}
pub trait UnificationErrorExt<T> {
    fn add_context<'a, 'b>(
        self,
        lhs: TypeVarRef<'a>,
        rhs: TypeVarRef<'b>,
    ) -> std::result::Result<T, UnificationError>;
}
impl<T> UnificationErrorExt<T> for std::result::Result<T, UnificationError> {
    fn add_context<'a, 'b>(
        self,
        lhs: TypeVarRef<'a>,
        rhs: TypeVarRef<'b>,
    ) -> std::result::Result<T, UnificationError> {
        match self {
            Ok(val) => Ok(val),
            Err((mut old_lhs, mut old_rhs)) => {
                old_lhs.inside.replace(lhs.as_free());
                old_rhs.inside.replace(rhs.as_free());
                Err((old_lhs, old_rhs))
            }
        }
    }
}

pub type UnificationError = (UnificationTrace, UnificationTrace);

#[derive(Debug, Error, PartialEq, Clone)]
pub enum Error {
    #[error("The specified expression has no type information {}", 0.0)]
    UnknownType(TypedExpression),
    #[error("Type mismatch between {0:?} and {1:?}")]
    TypeMismatch(UnificationTrace, UnificationTrace),

    #[error("Entity output type mismatch")]
    EntityOutputTypeMismatch {
        expected: UnificationTrace,
        got: UnificationTrace,
        // The location of the type specification
        type_spec: Loc<()>,
        // The location of the output expression with the offending type
        output_expr: Loc<()>,
    },
    // An entity output mismatch where the output type was unspecified and defaulted
    // to unit
    #[error("Entity output type mismatch without spec")]
    UnspecedEntityOutputTypeMismatch {
        expected: UnificationTrace,
        got: UnificationTrace,
        // The location of the output expression with the offending type
        output_expr: Loc<()>,
    },
    #[error("Type error: expected {expected}, got: {got}")]
    UnspecifiedTypeError {
        expected: UnificationTrace,
        got: UnificationTrace,
        loc: Loc<()>,
    },
    #[error("Int literal not compatible")]
    IntLiteralIncompatible { got: UnificationTrace, loc: Loc<()> },
    #[error("If condition must be boolean")]
    NonBooleanCondition { got: UnificationTrace, loc: Loc<()> },
    #[error("If condition mismatch")]
    IfConditionMismatch {
        expected: UnificationTrace,
        got: UnificationTrace,
        first_branch: Loc<()>,
        incorrect_branch: Loc<()>,
    },
    #[error("Match branch mismatch")]
    MatchBranchMismatch {
        expected: UnificationTrace,
        got: UnificationTrace,
        first_branch: Loc<()>,
        incorrect_branch: Loc<()>,
    },
    #[error("Non clock used as register clock")]
    NonClockClock {
        expected: UnificationTrace,
        got: UnificationTrace,
        loc: Loc<()>,
    },
    #[error("Reset condition must be a bool")]
    NonBoolReset {
        expected: UnificationTrace,
        got: UnificationTrace,
        loc: Loc<()>,
    },
    #[error("Reset value must match register type")]
    RegisterResetMismatch {
        expected: UnificationTrace,
        got: UnificationTrace,
        loc: Loc<()>,
    },

    #[error("Named argument mismatch")]
    NamedArgumentMismatch {
        name: Loc<Identifier>,
        expr: Loc<()>,
        expected: UnificationTrace,
        got: UnificationTrace,
    },
    #[error("Named argument mismatch")]
    ShortNamedArgumentMismatch {
        name: Loc<Identifier>,
        expected: UnificationTrace,
        got: UnificationTrace,
    },
    #[error("Positional argument mismatch")]
    PositionalArgumentMismatch {
        index: usize,
        expr: Loc<()>,
        expected: UnificationTrace,
        got: UnificationTrace,
    },

    #[error("Tuple index of generic argument")]
    TupleIndexOfGeneric { loc: Loc<()> },
    #[error("Tuple index of non-tuple")]
    TupleIndexOfNonTuple { got: InnerTypeVar, loc: Loc<()> },
    #[error("Tuple index out of bounds")]
    TupleIndexOutOfBounds { index: Loc<u128>, actual_size: u128 },

    #[error("Field access on incomplete")]
    FieldAccessOnIncomplete { loc: Loc<()> },
    #[error("Field access on generic")]
    FieldAccessOnGeneric { loc: Loc<()>, name: NameID },
    #[error("Field access on non-struct")]
    FieldAccessOnNonStruct { loc: Loc<()>, got: InnerTypeVar },
    #[error("Field access on integer")]
    FieldAccessOnInteger { loc: Loc<()> },
    #[error("Field access on enum")]
    FieldAccessOnEnum { loc: Loc<()>, actual_type: NameID },
    #[error("Field access on primitive type")]
    FieldAccessOnPrimitive { loc: Loc<()>, actual_type: NameID },
    #[error("No such field")]
    NoSuchField {
        field: Loc<Identifier>,
        _struct: NameID,
    },

    #[error("Array element mismatch")]
    ArrayElementMismatch {
        expected: UnificationTrace,
        got: UnificationTrace,
        loc: Loc<()>,
        first_element: Loc<()>,
    },

    #[error("Index must be an integer")]
    IndexMustBeInteger { got: UnificationTrace, loc: Loc<()> },
    #[error("Indexee must be an array")]
    IndexeeMustBeArray { got: UnificationTrace, loc: Loc<()> },

    #[error("Pattern type mismatch")]
    PatternTypeMismatch {
        pattern: Loc<()>,
        expected: UnificationTrace,
        got: UnificationTrace,
    },

    #[error("The first argument of a pipeline must be a clock")]
    FirstPipelineArgNotClock {
        expected: UnificationTrace,
        spec: Loc<UnificationTrace>,
    },

    #[error("Attempting to instanciate generic type")]
    GenericTypeInstanciation,
}
pub type Result<T> = std::result::Result<T, Error>;
