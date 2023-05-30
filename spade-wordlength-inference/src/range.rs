use crate::inferer::{Equation, Var};
use num::BigInt;
use num::Signed;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Range {
    lo: BigInt,
    hi: BigInt,
}
impl Range {
    pub fn new(a: BigInt, b: BigInt) -> Self {
        Range {
            lo: a.clone().min(b.clone()),
            hi: a.max(b),
        }
    }

    pub fn lo(&self) -> &BigInt {
        &self.lo
    }

    pub fn hi(&self) -> &BigInt {
        &self.hi
    }

    pub fn add(&self, b: &Self) -> Self {
        let a = self;
        Self::new(a.lo.clone() + b.lo.clone(), a.hi.clone() + b.hi.clone())
    }

    pub fn sub(&self, b: &Self) -> Self {
        let a = self;
        Self::new(
            (a.lo.clone() - b.hi.clone())
                .min(a.lo.clone() - b.lo.clone())
                .min(a.lo.clone() - b.hi.clone())
                .min(a.lo.clone() - b.lo.clone()),
            (a.hi.clone() - b.hi.clone())
                .max(a.hi.clone() - b.lo.clone())
                .max(a.lo.clone() - b.hi.clone())
                .max(a.lo.clone() - b.lo.clone()),
        )
    }

    pub fn neg(&self) -> Self {
        Self::new(-self.hi.clone(), -self.lo.clone())
    }

    pub fn mul(&self, b: &Self) -> Self {
        let a = self;
        Self::new(
            (a.lo.clone() * b.hi.clone())
                .min(a.lo.clone() * b.lo.clone())
                .min(a.hi.clone() * b.hi.clone())
                .min(a.hi.clone() * b.lo.clone()),
            (a.lo.clone() * b.hi.clone())
                .max(a.lo.clone() * b.lo.clone())
                .max(a.hi.clone() * b.hi.clone())
                .max(a.hi.clone() * b.lo.clone()),
        )
    }

    pub fn union(&self, b: &Self) -> Self {
        let a = self;
        Self::new(
            a.lo.clone().min(b.lo.clone()),
            a.hi.clone().max(b.hi.clone()),
        )
    }

    pub fn subset(&self, b: &Range) -> Range {
        let a = self;
        Self::new(
            a.lo.clone().max(b.lo.clone()),
            a.hi.clone().min(b.hi.clone()),
        )
    }

    pub fn bit_manip(&self) -> Option<Self> {
        // This signed integers
        self.to_wordlength().map(|wl| {
            let a = -BigInt::from(2).pow(wl - 1);
            let b = BigInt::from(2).pow(wl - 1) - BigInt::from(1);

            Self::new(a.clone().min(b.clone()), a.max(b))
        })
    }

    pub fn to_wordlength(&self) -> Option<u32> {
        // NOTE: This can be considerably more fancy, taking into account the range and working
        // from there - but I'm keeping things simple for now.
        for i in 1..2048 {
            let n = BigInt::from(2).pow(i);
            if self.hi.abs() < n && self.lo.abs() < n + BigInt::from(1) {
                return Some(i + 1);
            }
        }
        None
    }

    pub fn zero() -> Range {
        Self::new(BigInt::from(0), BigInt::from(0))
    }
}

pub fn evaluate_ia(body: &Equation, known: &BTreeMap<Var, Range>) -> Option<Range> {
    match &body {
        Equation::V(var) => known.get(var).cloned(),
        Equation::Constant(range) => Some(range.clone()),
        Equation::Add(a, b) => match (evaluate_ia(a, known), evaluate_ia(b, known)) {
            (Some(a), Some(b)) => Some(a.add(&b)),
            _ => None,
        },
        Equation::Sub(a, b) => match (evaluate_ia(a, known), evaluate_ia(b, known)) {
            (Some(a), Some(b)) => Some(a.sub(&b)),
            _ => None,
        },
        Equation::Mul(a, b) => match (evaluate_ia(a, known), evaluate_ia(b, known)) {
            (Some(a), Some(b)) => Some(a.mul(&b)),
            _ => None,
        },
        Equation::BitManpi(a) => evaluate_ia(a, known).and_then(|x| x.bit_manip()),
        Equation::Neg(a) => evaluate_ia(a, known).map(|x| x.neg()),
        Equation::BitManipMax(a, b) => match (
            evaluate_ia(a, known).and_then(|x| x.bit_manip()),
            evaluate_ia(b, known).and_then(|x| x.bit_manip()),
        ) {
            (Some(a), Some(b)) => Some(a.union(&b)),
            _ => None,
        },
        Equation::Union(a, b) => match (evaluate_ia(a, known), evaluate_ia(b, known)) {
            (Some(a), Some(b)) => Some(a.union(&b)),
            _ => None,
        },
    }
}
