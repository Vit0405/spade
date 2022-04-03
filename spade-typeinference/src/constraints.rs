use spade_common::location_info::{Loc, WithLocation};

use crate::equation::InnerTypeVar;

#[derive(Clone)]
pub enum ConstraintExpr {
    Integer(i128),
    Var(InnerTypeVar),
    Sum(Box<ConstraintExpr>, Box<ConstraintExpr>),
    Sub(Box<ConstraintExpr>),
}

impl WithLocation for ConstraintExpr {}

impl ConstraintExpr {
    /// Evaluates the ConstraintExpr returning a new simplified form
    fn evaluate(&self) -> ConstraintExpr {
        match self {
            ConstraintExpr::Integer(_) => self.clone(),
            ConstraintExpr::Var(_) => self.clone(),
            ConstraintExpr::Sum(lhs, rhs) => match (lhs.as_ref(), rhs.as_ref()) {
                (ConstraintExpr::Integer(l), ConstraintExpr::Integer(r)) => {
                    ConstraintExpr::Integer(l + r)
                }
                _ => self.clone(),
            },
            ConstraintExpr::Sub(inner) => match **inner {
                ConstraintExpr::Integer(val) => ConstraintExpr::Integer(-val),
                _ => self.clone(),
            },
        }
    }
}

impl std::fmt::Display for ConstraintExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstraintExpr::Integer(val) => write!(f, "{val}"),
            ConstraintExpr::Var(var) => write!(f, "{var}"),
            ConstraintExpr::Sum(rhs, lhs) => write!(f, "({rhs} + {lhs})"),
            ConstraintExpr::Sub(val) => write!(f, "(-{val})"),
        }
    }
}

pub struct TypeConstraints {
    pub inner: Vec<(InnerTypeVar, Loc<ConstraintExpr>)>,
}

impl TypeConstraints {
    pub fn new() -> Self {
        Self { inner: vec![] }
    }

    pub fn add_constraint(&mut self, lhs: InnerTypeVar, rhs: Loc<ConstraintExpr>) {
        self.inner.push((lhs, rhs));
    }

    /// Calls `evaluate` on all constraints. If any constraints are now `T = Integer(val)`,
    /// those updated values are returned. Such constraints are then removed
    pub fn update_constraints(&mut self) -> Vec<Loc<(InnerTypeVar, i128)>> {
        let mut new_known = vec![];
        self.inner = self
            .inner
            .iter_mut()
            .filter_map(|(expr, constraint)| {
                let result = constraint.map_ref(ConstraintExpr::evaluate);

                match result.inner {
                    ConstraintExpr::Integer(val) => {
                        // ().at_loc(..).map is a somewhat ugly way to wrap an arbitrary type
                        // in a known Loc. This is done to avoid having to impl WithLocation for
                        // the the unusual tuple used here
                        new_known.push(().at_loc(&constraint).map(|_| (expr.clone(), val)));
                        None
                    }
                    ConstraintExpr::Var(_) => Some((expr.clone(), result)),
                    ConstraintExpr::Sum(_, _) => Some((expr.clone(), result)),
                    ConstraintExpr::Sub(_) => Some((expr.clone(), result)),
                }
            })
            .collect();

        new_known
    }
}

impl std::fmt::Display for TypeConstraints {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (lhs, rhs) in &self.inner {
            write!(f, "{lhs}: {rhs}",)?;
        }
        Ok(())
    }
}
