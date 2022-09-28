use crate::Pattern;

use super::{Block, NameID};
use serde::{Deserialize, Serialize};
use spade_common::{
    location_info::{Loc, WithLocation},
    name::{Identifier, Path},
};

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum BinaryOperator {
    Add,
    Sub,
    Mul,
    Eq,
    Gt,
    Lt,
    Ge,
    Le,
    LeftShift,
    RightShift,
    LogicalAnd,
    LogicalOr,
    LogicalXor,
    BitwiseOr,
    BitwiseAnd,
    BitwiseXor,
}
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum UnaryOperator {
    Sub,
    Not,
    BitwiseNot,
    Dereference,
    Reference,
}

#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub enum NamedArgument {
    /// Binds the arguent named LHS in the outer scope to the expression
    Full(Loc<Identifier>, Loc<Expression>),
    /// Binds a local variable to an argument with the same name
    Short(Loc<Identifier>, Loc<Expression>),
}
impl WithLocation for NamedArgument {}

/// Specifies how an argument is bound. Mainly used for error reporting without
/// code duplication
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub enum ArgumentKind {
    Positional,
    Named,
    ShortNamed,
}

#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub enum ArgumentList {
    Named(Vec<NamedArgument>),
    Positional(Vec<Loc<Expression>>),
}

#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct Argument {
    pub target: Loc<Identifier>,
    pub value: Loc<Expression>,
    pub kind: ArgumentKind,
}
impl WithLocation for ArgumentList {}

#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub enum ExprKind {
    Identifier(NameID),
    IntLiteral(u128),
    BoolLiteral(bool),
    TupleLiteral(Vec<Loc<Expression>>),
    ArrayLiteral(Vec<Loc<Expression>>),
    Index(Box<Loc<Expression>>, Box<Loc<Expression>>),
    TupleIndex(Box<Loc<Expression>>, Loc<u128>),
    FieldAccess(Box<Loc<Expression>>, Loc<Identifier>),
    BinaryOperator(Box<Loc<Expression>>, BinaryOperator, Box<Loc<Expression>>),
    UnaryOperator(UnaryOperator, Box<Loc<Expression>>),
    Match(Box<Loc<Expression>>, Vec<(Loc<Pattern>, Loc<Expression>)>),
    Block(Box<Block>),
    FnCall(Loc<NameID>, Loc<ArgumentList>),
    EntityInstance(Loc<NameID>, Loc<ArgumentList>),
    PipelineInstance {
        depth: Loc<u128>,
        name: Loc<NameID>,
        args: Loc<ArgumentList>,
    },
    If(
        Box<Loc<Expression>>,
        Box<Loc<Expression>>,
        Box<Loc<Expression>>,
    ),
    PipelineRef {
        stage: Loc<usize>,
        name: Loc<NameID>,
        declares_name: bool,
    },
}
impl WithLocation for ExprKind {}

impl ExprKind {
    pub fn with_id(self, id: u64) -> Expression {
        Expression { kind: self, id }
    }

    pub fn idless(self) -> Expression {
        Expression { kind: self, id: 0 }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ExprKind::Identifier(_) => "Identifier",
            ExprKind::IntLiteral(_) => "IntLiteral",
            ExprKind::BoolLiteral(_) => "BoolLiteral",
            ExprKind::TupleLiteral(_) => "TupleLiteral",
            ExprKind::ArrayLiteral(_) => "ArrayLiteral",
            ExprKind::Index(_, _) => "Index",
            ExprKind::TupleIndex(_, _) => "TupleIndex",
            ExprKind::FieldAccess(_, _) => "FieldAccess",
            ExprKind::BinaryOperator(_, _, _) => "BinaryOperator",
            ExprKind::UnaryOperator(_, _) => "UnaryOperator",
            ExprKind::Match(_, _) => "Match",
            ExprKind::Block(_) => "Block",
            ExprKind::FnCall(_, _) => "FnCall",
            ExprKind::EntityInstance(_, _) => "EntityInstance",
            ExprKind::PipelineInstance { .. } => "PipelineInstance",
            ExprKind::If(_, _, _) => "If",
            ExprKind::PipelineRef { .. } => "PipelineRef",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expression {
    pub kind: ExprKind,
    // This ID is used to associate types with the expression
    pub id: u64,
}
impl WithLocation for Expression {}

impl Expression {
    /// Create a new expression referencing an identifier with the specified
    /// id and name
    pub fn ident(expr_id: u64, name_id: u64, name: &str) -> Expression {
        ExprKind::Identifier(NameID(name_id, Path::from_strs(&[name]))).with_id(expr_id)
    }

    /// Returns the block that is this expression. Panics if the expression is not a block
    pub fn assume_block(&self) -> &Block {
        if let ExprKind::Block(ref block) = self.kind {
            block
        } else {
            panic!("Expression is not a block")
        }
    }
}

impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}
