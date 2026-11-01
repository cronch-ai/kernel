//! A claim about an Answer as SMT-LIB: a query a solver answers with a counterexample, and a validation script that
//! confirms one.
//!
//! The query declares the claim's variables as constants, defines the Answer as the export does (its nested Asks
//! uninterpreted), asserts that the claim is false, and asks for the values the kernel needs to re-check what the
//! solver found. A claim's evaluation can reach nested Asks only once its variables are known, so a check takes rounds:
//! the first query reads the variables, each later one fixes what is known and reads the holes the kernel's evaluation
//! reached next. The solver's answers are proposals; only the kernel's re-check (`claim::assess_claim`) is a verdict.

use super::ExportError;
use super::encode::Encoder;
pub use super::encode::Floats;
use crate::claim::{Claim, Witness};
use crate::eval::Open;
use crate::ir::{Answer, Ask, Param, Type, World};
use serde::{Deserialize, Serialize};

/// A value a query asks the solver for, in the order of its `get-value`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "of", rename_all = "camelCase")]
pub enum Read {
    Var {
        name: String,
        #[serde(rename = "type")]
        ty: Type,
    },
    Hole {
        key: String,
        ask: Ask,
        #[serde(rename = "type")]
        ty: Type,
    },
}

impl Read {
    pub fn ty(&self) -> &Type {
        match self {
            Read::Var { ty, .. } | Read::Hole { ty, .. } => ty,
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Query {
    pub script: String,
    pub reads: Vec<Read>,
}

/// A script that is `sat` where the claim fails with `witness` fixed, asking for the missing `vars` and `holes`. Read
/// with `Floats::Real`, it searches far faster wherever floats are divided or multiplied, but what it finds may not be
/// a counterexample under rounding, and its `unsat` doesn't mean there is none.
pub fn query(
    claim: &Claim,
    answer: &Answer,
    witness: &Witness,
    vars: &[Param],
    holes: &[Open],
    floats: Floats,
) -> Result<Query, ExportError> {
    let mut reads: Vec<Read> = vars
        .iter()
        .map(|p| Read::Var {
            name: p.name.clone(),
            ty: p.ty.clone(),
        })
        .collect();
    reads.extend(holes.iter().filter_map(|o| match o {
        Open::Ask { key, ask, .. } => Some(Read::Hole {
            key: key.clone(),
            ask: ask.clone(),
            ty: ask.spec.output.clone(),
        }),
        Open::Opaque { .. } => None,
    }));
    let world = World::default();
    let script = Encoder::new(&world)
        .reading(floats)
        .claim(claim, answer, witness, &reads)?
        .to_string();
    Ok(Query { script, reads })
}

/// A self-contained script, with every value of the counterexample fixed, that any solver confirms with `sat`: it
/// takes evaluation, not search.
pub fn validation(
    claim: &Claim,
    answer: &Answer,
    witness: &Witness,
) -> Result<String, ExportError> {
    let world = World::default();
    Ok(Encoder::new(&world)
        .claim(claim, answer, witness, &[])?
        .to_string())
}
