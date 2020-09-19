pub mod expression;
pub use expression::{ExprKind, Expression};

/**
  Representation of the language with most language constructs still present, with
  more correctness guaranatees than the AST, such as types actually existing.
*/
use crate::location_info::{Loc, WithLocation};
use crate::types::Type;

#[derive(PartialEq, Debug, Clone)]
pub enum Identifier {
    Named(String),
    Anonymous(u64),
}
impl WithLocation for Identifier {}

#[derive(PartialEq, Debug, Clone)]
pub struct Path(pub Vec<Loc<Identifier>>);
impl WithLocation for Path {}
impl Path {
    pub fn from_strs(names: &[&str]) -> Self {
        Self(
            names
                .iter()
                .map(|s| s.to_string())
                .map(Identifier::Named)
                .map(Loc::nowhere)
                .collect(),
        )
    }
}

#[derive(PartialEq, Debug, Clone)]
pub struct Block {
    pub statements: Vec<Loc<Statement>>,
    pub result: Loc<Expression>,
}
impl WithLocation for Block {}

#[derive(PartialEq, Debug, Clone)]
pub enum Statement {
    Binding(Loc<Identifier>, Option<Loc<Type>>, Loc<Expression>),
    Register(Loc<Register>),
}
impl WithLocation for Statement {}

#[derive(PartialEq, Debug, Clone)]
pub struct Register {
    pub name: Loc<Identifier>,
    pub clock: Loc<Path>,
    pub reset: Option<(Loc<Expression>, Loc<Expression>)>,
    pub value: Loc<Expression>,
    pub value_type: Option<Loc<Type>>,
}
impl WithLocation for Register {}

#[derive(PartialEq, Debug, Clone)]
pub struct Entity {
    pub name: Loc<Identifier>,
    pub inputs: Vec<(Loc<Identifier>, Loc<Type>)>,
    pub output_type: Loc<Type>,
    pub block: Loc<Block>,
}
impl WithLocation for Entity {}
