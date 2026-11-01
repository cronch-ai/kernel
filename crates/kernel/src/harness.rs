//! Harnesses and the interfaces they run against.
//!
//! A harness is a state machine `⟨A, S, s₀, δ⟩`: what a run starts with, the type of its state, the initial state,
//! and the step function. `δ` is an expression over `state` and `input` that gives the next state and a list of
//! actions. It is data: the term that is hashed is the term that is checked, proved and run, and it is never compiled.
//! All nondeterminism is in the inputs, which the environment chooses.
//!
//! An interface is what a harness can do and be reached by: operations it may call and channels the world may send on.
//! None are built in: every one comes from an installed capability.

use crate::check::{Problem, infer};
use crate::ir::{Expr, Type};
use crate::types::{conforms, inhabits};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Harness {
    /// `A`: the type of the value a run is started with.
    pub arg: Type,
    /// `S`: the type of the state.
    pub state: Type,
    /// `s₀`.
    pub init: Value,
    /// `δ : S × Inp → S × List(Act)`, over the variables `state` and `input`, giving `{state, actions}`.
    pub step: Expr,
}

/// Whether a call delivers one result, or any number of items and then an end.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    Single,
    Stream,
}

impl Mode {
    fn is_single(&self) -> bool {
        *self == Mode::Single
    }
}

/// An operation a harness may call.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Operation {
    pub input: Type,
    pub output: Type,
    #[serde(default, skip_serializing_if = "Mode::is_single")]
    pub mode: Mode,
}

/// A channel the world may reach a harness on, unprompted.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Channel {
    #[serde(rename = "type")]
    pub ty: Type,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Interface {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operations: BTreeMap<String, Operation>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, Channel>,
}

/// Which expression of a harness or interface a problem is in.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "in", rename_all = "camelCase")]
pub enum Place {
    Step,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HarnessProblem {
    /// A problem inside one of the expressions.
    Expr { place: Place, problem: Problem },
    /// `s₀` doesn't inhabit `S`.
    Init { expected: Type, got: Value },
    /// `δ` doesn't give `{state: S, actions: List}`.
    Step { expected: Type, got: Type },
}

/// What `δ` must give: the next state and the actions.
pub fn step_type(state: &Type) -> Type {
    Type::Record(BTreeMap::from([
        ("state".to_string(), state.clone()),
        ("actions".to_string(), Type::List(Box::new(Type::Any))),
    ]))
}

/// Problems with a harness. The input is typed `Any`: which input arrives, and its shape, is checked by the
/// interpreter when it arrives, against the interface.
pub fn check_harness(harness: &Harness) -> Vec<HarnessProblem> {
    let mut problems = Vec::new();
    if !inhabits(&harness.init, &harness.state) {
        problems.push(HarnessProblem::Init {
            expected: harness.state.clone(),
            got: harness.init.clone(),
        });
    }
    let scope = vec![
        ("state".to_string(), harness.state.clone()),
        ("input".to_string(), Type::Any),
    ];
    let (ty, found) = infer(&harness.step, scope);
    problems.extend(found.into_iter().map(|problem| HarnessProblem::Expr {
        place: Place::Step,
        problem,
    }));
    let expected = step_type(&harness.state);
    if let Some(got) = ty
        && !conforms(&got, &expected)
    {
        problems.push(HarnessProblem::Step { expected, got });
    }
    problems
}
