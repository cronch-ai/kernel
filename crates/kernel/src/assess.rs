//! The kernel's verdict on an Answer: which of Constructed, Evaluable and Satisfied it has reached, and on what basis.
//!
//! The report keeps three things apart. Conformance is `checks`: what Cronch verified about the value. Trust is
//! `assumptions`: what the result rests on that nobody checked. Holes are `open`: what is still missing. Authority is
//! not the kernel's: effect holes are listed like any other, and the runtime gates them.

use crate::check::{Assumption, Hole, Problem, check, check_ask};
use crate::eval::{EvalError, Evaluation, Open, evaluate};
use crate::ir::{Answer, Ask, World};
use crate::key::ask_key;
use crate::validate::{Check, Outcome, Verdict, validate};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Not a valid construction: it doesn't type-check against the Ask.
    IllFormed,
    /// A valid construction, blocked on open holes. Not an Answer yet.
    Constructed,
    /// Evaluated, with no failed check, but waiting on a Checker's Evidence.
    Evaluable,
    /// Evaluated, and every check the contract asks for has passed (prose criteria are reported, never passed).
    Satisfied,
    /// Evaluation failed, or a check failed.
    Failed,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Report {
    pub key: String,
    pub state: State,
    pub problems: Vec<Problem>,
    pub holes: Vec<Hole>,
    pub assumptions: Vec<Assumption>,
    pub open: Vec<Open>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    pub checks: Vec<Check>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<EvalError>,
}

pub fn assess(
    ask: &Ask,
    world: &World,
    answer: &Answer,
    resolved: &BTreeMap<String, Value>,
    evidence: &[Verdict],
) -> Report {
    let construction = check(ask, world, answer);
    let mut report = Report {
        key: ask_key(ask, &world.facts),
        state: State::IllFormed,
        problems: check_ask(ask)
            .into_iter()
            .chain(construction.problems)
            .collect(),
        holes: construction.holes,
        assumptions: construction.assumptions,
        open: Vec::new(),
        value: None,
        checks: Vec::new(),
        error: None,
    };
    if !report.problems.is_empty() {
        return report;
    }
    match evaluate(ask, world, answer, resolved) {
        Evaluation::Blocked { open } => {
            report.state = State::Constructed;
            report.open = open;
        }
        Evaluation::Failed { error } => {
            report.state = State::Failed;
            report.error = Some(error);
        }
        Evaluation::Done { value } => {
            report.checks = validate(&ask.spec, &value, evidence);
            let outcomes = || report.checks.iter().map(|c| &c.outcome);
            report.state = if outcomes().any(|o| matches!(o, Outcome::Fail { .. })) {
                State::Failed
            } else if outcomes().any(|o| matches!(o, Outcome::Delegated { .. })) {
                State::Evaluable
            } else {
                State::Satisfied
            };
            report.value = Some(value);
        }
    }
    report
}
