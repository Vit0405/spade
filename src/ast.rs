use crate::lexer::TokenKind;
use crate::location_info::{Loc, WithLocation};

#[derive(PartialEq, Debug, Clone)]
pub struct Identifier(pub String);

impl WithLocation for Identifier {}

#[derive(PartialEq, Debug, Clone)]
pub struct Path(pub Vec<Loc<Identifier>>);
impl WithLocation for Path {}

impl Path {
    pub fn as_strs(&self) -> Vec<&str> {
        self.0.iter().map(|id| id.inner.0.as_ref()).collect()
    }
    pub fn as_strings(&self) -> Vec<String> {
        self.0.iter().map(|id| id.inner.0.clone()).collect()
    }
}

impl std::fmt::Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_strs().join("::"))
    }
}

#[derive(PartialEq, Debug, Clone)]
pub enum Type {
    Named(Path),
    WithSize(Box<Loc<Type>>, Loc<Expression>),
    UnitType,
}
impl WithLocation for Type {}

#[derive(PartialEq, Debug, Clone)]
pub enum Expression {
    Identifier(Loc<Path>),
    IntLiteral(u128),
    If(
        Box<Loc<Expression>>,
        Box<Loc<Expression>>,
        Box<Loc<Expression>>,
    ),
    BinaryOperator(Box<Loc<Expression>>, TokenKind, Box<Loc<Expression>>),
    Block(Box<Block>),
}
impl WithLocation for Expression {}

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
pub struct Entity {
    pub name: Loc<Identifier>,
    pub inputs: Vec<(Loc<Identifier>, Loc<Type>)>,
    pub output_type: Loc<Type>,
    /// The body is an expression for ID assignment purposes, but semantic analysis
    /// ensures that it is always a block.
    pub body: Loc<Expression>,
}
impl WithLocation for Entity {}

#[derive(PartialEq, Debug, Clone)]
pub struct Register {
    pub name: Loc<Identifier>,
    pub clock: Loc<Path>,
    pub reset: Option<(Loc<Expression>, Loc<Expression>)>,
    pub value: Loc<Expression>,
    pub value_type: Option<Loc<Type>>,
}
impl WithLocation for Register {}

/// Items are things typically present at the top level of a module such as
/// entities, pipelines, submodules etc.
#[derive(PartialEq, Debug, Clone)]
pub enum Item {
    Entity(Loc<Entity>),
}
impl WithLocation for Item {}

#[derive(PartialEq, Debug, Clone)]
pub struct ModuleBody {
    pub members: Vec<Item>,
}
