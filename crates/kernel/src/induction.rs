//! Inductive invariants of a harness's state, over every state and every well-typed input.
//!
//! An invariant here is an expression over `state`. It is inductive when it holds of `s₀` (initiation) and, whenever it
//! holds of a state, it holds of the state `δ` gives on any input (consecution). Then it holds of every state of every
//! run, whatever the environment does, without bounding depth or listing configurations.
//!
//! The kernel decides initiation by evaluating. Consecution quantifies over infinitely many states and inputs, so the
//! kernel writes it as an SMT-LIB query (`obligation`) whose models are counterexamples to induction: a state where the
//! invariant holds and an input after which it doesn't. A solver searches; the kernel checks what it finds by
//! evaluating `δ` on it (`check_step`). A solver's `unsat` is not a verdict here, as nowhere in the kernel: it comes
//! with no certificate the kernel can check.
//!
//! The query is over `δ`'s next state only: other parts of the step are projected away, which can only add steps (one
//! whose actions would fail to evaluate), so the obligation is at least as strong as consecution. Inputs whose type
//! has no sort (a spawned child's result, a failure's error, both `Any`) are left out, and the query says which.

use crate::check::{Problem, infer};
use crate::harness::{Harness, Interface, input_type};
use crate::ir::{Case, Expr, Type};
use crate::labeled::{self, LValue};
use crate::smtlib::{self, Floats, Refusal};
use crate::types::inhabits;
use serde::Serialize;
use serde_json::Value;

/// `expr`'s field `field`, pushed through records, branches, bindings and matches, so that only what computes that
/// field remains.
pub fn project(expr: &Expr, field: &str) -> Expr {
    match expr {
        Expr::Record { fields } if fields.contains_key(field) => fields[field].clone(),
        Expr::If {
            cond,
            then,
            otherwise,
        } => Expr::If {
            cond: cond.clone(),
            then: Box::new(project(then, field)),
            otherwise: Box::new(project(otherwise, field)),
        },
        Expr::Let { name, be, body } => Expr::Let {
            name: name.clone(),
            be: be.clone(),
            body: Box::new(project(body, field)),
        },
        Expr::Match {
            of,
            on,
            cases,
            otherwise,
        } => Expr::Match {
            of: of.clone(),
            on: on.clone(),
            cases: cases
                .iter()
                .map(|(name, c)| {
                    let case = Case {
                        bind: c.bind.clone(),
                        body: project(&c.body, field),
                    };
                    (name.clone(), case)
                })
                .collect(),
            otherwise: otherwise.as_ref().map(|o| Box::new(project(o, field))),
        },
        other => Expr::Get {
            of: Box::new(other.clone()),
            field: field.to_string(),
        },
    }
}

/// The type without the parts that have no sort: a variant's cases that hold `Any` are dropped, and named in
/// `excluded`. `None` if what remains still holds `Any` outside a variant.
fn prune(ty: &Type, at: &str, excluded: &mut Vec<String>) -> Option<Type> {
    match ty {
        Type::Any => None,
        Type::List(t) => prune(t, at, excluded).map(|t| Type::List(Box::new(t))),
        Type::Record(fields) => fields
            .iter()
            .map(|(n, t)| prune(t, at, excluded).map(|t| (n.clone(), t)))
            .collect::<Option<_>>()
            .map(Type::Record),
        Type::Variant { tag, cases } => {
            let mut kept = std::collections::BTreeMap::new();
            for (name, t) in cases {
                let path = format!("{at}{name}");
                match prune(t, &format!("{path}."), excluded) {
                    // A case that is itself a variant with no cases has no values: nothing is left out.
                    Some(Type::Variant { cases, .. }) if cases.is_empty() => {}
                    Some(t) => {
                        kept.insert(name.clone(), t);
                    }
                    None => excluded.push(path),
                }
            }
            Some(Type::Variant {
                tag: tag.clone(),
                cases: kept,
            })
        }
        scalar => Some(scalar.clone()),
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Obligation {
    /// Whether the invariant holds of `s₀`, decided by the kernel.
    pub initiation: bool,
    /// Consecution as an SMT-LIB query: `sat` exactly where there is a counterexample to induction.
    pub text: String,
    /// The input cases the query leaves out, as paths of case names (`ret.spawn`).
    pub excluded: Vec<String>,
    /// The types to read a model's state and input at (`readValues`), which are the query's own.
    pub reads: [Type; 2],
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ObligationError {
    /// The invariant isn't a condition on the state.
    Invariant { problems: Vec<Problem> },
    /// The state type has no sort.
    State,
    /// Part of the query has no faithful reading.
    Refused { refusal: Refusal },
}

fn invariant_problems(harness: &Harness, invariant: &Expr) -> Vec<Problem> {
    let (ty, mut problems) = infer(
        invariant,
        vec![("state".to_string(), harness.state.clone())],
    );
    if let Some(got) = ty
        && !crate::types::conforms(&got, &Type::Bool)
    {
        problems.push(Problem::Mismatch {
            at: String::new(),
            expected: Type::Bool,
            got,
        });
    }
    problems
}

fn holds(invariant: &Expr, state: &Value) -> bool {
    labeled::eval(
        invariant,
        vec![("state".to_string(), LValue::trusted(state))],
    )
    .is_ok_and(|v| v.value() == Value::Bool(true))
}

/// Initiation, decided, and consecution, as a query for a solver.
pub fn obligation(
    harness: &Harness,
    interface: &Interface,
    invariant: &Expr,
    floats: Floats,
) -> Result<Obligation, ObligationError> {
    let problems = invariant_problems(harness, invariant);
    if !problems.is_empty() {
        return Err(ObligationError::Invariant { problems });
    }
    let mut excluded = Vec::new();
    let input =
        prune(&input_type(harness, interface), "", &mut excluded).ok_or(ObligationError::State)?;
    let state = prune(&harness.state, "", &mut Vec::new()).ok_or(ObligationError::State)?;
    let next = project(&harness.step, "state");
    let text = smtlib::induction(&state, &input, invariant, &next, &excluded, floats)
        .map_err(|refusal| ObligationError::Refused { refusal })?;
    Ok(Obligation {
        initiation: holds(invariant, &harness.init),
        text,
        excluded,
        reads: [state, input],
    })
}

/// What `δ` does to the invariant on one state and one input.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Induction {
    /// The invariant holds of the next state too.
    Preserved { next: Value },
    /// A counterexample to induction: the invariant holds here, and not after this input.
    NotInductive { next: Value },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InductionError {
    Invariant {
        problems: Vec<Problem>,
    },
    /// The state doesn't have the harness's state type.
    State,
    /// The input isn't one the interface allows.
    Input,
    /// The invariant doesn't hold of the state, so nothing follows from the step.
    NotHeld,
    /// `δ` fails on this state and input, or gives no state of its type: the run would end, not continue.
    Step,
}

/// Evaluate one step from a state where the invariant holds: the kernel's check of a solver's counterexample.
pub fn check_step(
    harness: &Harness,
    interface: &Interface,
    invariant: &Expr,
    state: &Value,
    input: &Value,
) -> Result<Induction, InductionError> {
    let problems = invariant_problems(harness, invariant);
    if !problems.is_empty() {
        return Err(InductionError::Invariant { problems });
    }
    if !inhabits(state, &harness.state) {
        return Err(InductionError::State);
    }
    if !inhabits(input, &input_type(harness, interface)) {
        return Err(InductionError::Input);
    }
    if !holds(invariant, state) {
        return Err(InductionError::NotHeld);
    }
    let scope = vec![
        ("state".to_string(), LValue::trusted(state)),
        ("input".to_string(), LValue::trusted(input)),
    ];
    let next = labeled::eval(&harness.step, scope)
        .ok()
        .and_then(|r| r.field("state"))
        .map(|s| s.value())
        .filter(|s| inhabits(s, &harness.state))
        .ok_or(InductionError::Step)?;
    Ok(if holds(invariant, &next) {
        Induction::Preserved { next }
    } else {
        Induction::NotInductive { next }
    })
}
