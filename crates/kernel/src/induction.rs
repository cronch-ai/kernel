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
        Expr::Let { name, be, body } => {
            let body = project(body, field);
            // A binding the projection reads only through one field is bound to that field alone, so what computes
            // its other fields goes too.
            match only_field(&body, name) {
                Some(f) => Expr::Let {
                    name: name.clone(),
                    be: Box::new(project(be, &f)),
                    body: Box::new(unwrap_field(&body, name, &f)),
                },
                None => Expr::Let {
                    name: name.clone(),
                    be: be.clone(),
                    body: Box::new(body),
                },
            }
        }
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

/// Visits every subexpression of `expr` in which `name` still means the binding being looked at, none under a
/// binder that shadows it. `visit` says whether to look inside the one it is given.
fn each_use(expr: &Expr, name: &str, visit: &mut impl FnMut(&Expr) -> bool) {
    if !visit(expr) {
        return;
    }
    let mut go = |e: &Expr| each_use(e, name, visit);
    match expr {
        Expr::Lit { .. } | Expr::Var { .. } => {}
        Expr::Op { args, .. } => args.iter().for_each(go),
        Expr::Let { name: n, be, body } => {
            go(be);
            if n != name {
                go(body);
            }
        }
        Expr::If {
            cond,
            then,
            otherwise,
        } => {
            go(cond);
            go(then);
            go(otherwise);
        }
        Expr::Record { fields } => fields.values().for_each(go),
        Expr::List { items } => items.iter().for_each(go),
        Expr::Get { of, .. } | Expr::Label { of } => go(of),
        Expr::Match {
            of,
            cases,
            otherwise,
            ..
        } => {
            go(of);
            for c in cases.values().filter(|c| c.bind != name) {
                go(&c.body);
            }
            if let Some(o) = otherwise {
                go(o);
            }
        }
        Expr::Fold {
            over,
            init,
            acc,
            item,
            body,
        } => {
            go(over);
            go(init);
            if acc != name && item != name {
                go(body);
            }
        }
    }
}

/// The one field through which `body` reads `name`, if it reads it at all and only ever through that field.
fn only_field(body: &Expr, name: &str) -> Option<String> {
    let mut field: Option<String> = None;
    let mut bare = false;
    each_use(body, name, &mut |e| match e {
        Expr::Get { of, field: f } if matches!(&**of, Expr::Var { name: n } if n == name) => {
            match &field {
                Some(seen) if seen != f => bare = true,
                _ => field = Some(f.clone()),
            }
            // The variable under this `Get` is accounted for: don't count it again as read whole.
            false
        }
        Expr::Var { name: n } if n == name => {
            bare = true;
            true
        }
        _ => true,
    });
    if bare { None } else { field }
}

/// `body` with every `name.field` read as `name`.
fn unwrap_field(body: &Expr, name: &str, field: &str) -> Expr {
    let again = |e: &Expr| Box::new(unwrap_field(e, name, field));
    match body {
        Expr::Get { of, field: f }
            if f == field && matches!(&**of, Expr::Var { name: n } if n == name) =>
        {
            Expr::Var {
                name: name.to_string(),
            }
        }
        Expr::Lit { .. } | Expr::Var { .. } => body.clone(),
        Expr::Op { op, args } => Expr::Op {
            op: op.clone(),
            args: args.iter().map(|a| unwrap_field(a, name, field)).collect(),
        },
        Expr::Let {
            name: n,
            be,
            body: inner,
        } => Expr::Let {
            name: n.clone(),
            be: again(be),
            body: if n == name {
                inner.clone()
            } else {
                again(inner)
            },
        },
        Expr::If {
            cond,
            then,
            otherwise,
        } => Expr::If {
            cond: again(cond),
            then: again(then),
            otherwise: again(otherwise),
        },
        Expr::Record { fields } => Expr::Record {
            fields: fields
                .iter()
                .map(|(k, v)| (k.clone(), unwrap_field(v, name, field)))
                .collect(),
        },
        Expr::List { items } => Expr::List {
            items: items.iter().map(|i| unwrap_field(i, name, field)).collect(),
        },
        Expr::Get { of, field: f } => Expr::Get {
            of: again(of),
            field: f.clone(),
        },
        Expr::Label { of } => Expr::Label { of: again(of) },
        Expr::Match {
            of,
            on,
            cases,
            otherwise,
        } => Expr::Match {
            of: again(of),
            on: on.clone(),
            cases: cases
                .iter()
                .map(|(k, c)| {
                    let body = if c.bind == name {
                        c.body.clone()
                    } else {
                        unwrap_field(&c.body, name, field)
                    };
                    (
                        k.clone(),
                        Case {
                            bind: c.bind.clone(),
                            body,
                        },
                    )
                })
                .collect(),
            otherwise: otherwise.as_ref().map(|o| again(o)),
        },
        Expr::Fold {
            over,
            init,
            acc,
            item,
            body: inner,
        } => Expr::Fold {
            over: again(over),
            init: again(init),
            acc: acc.clone(),
            item: item.clone(),
            body: if acc == name || item == name {
                inner.clone()
            } else {
                again(inner)
            },
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn e(v: serde_json::Value) -> Expr {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_binding_read_through_one_field_is_projected_too() {
        // let acc = {state: s, actions: [junk]} in {state: acc.state, actions: acc.actions}
        let step = e(json!({ "e": "let", "name": "acc",
            "be": { "e": "record", "fields": { "state": { "e": "var", "name": "s" }, "actions": { "e": "list", "items": [{ "e": "var", "name": "junk" }] } } },
            "in": { "e": "record", "fields": {
                "state": { "e": "get", "of": { "e": "var", "name": "acc" }, "field": "state" },
                "actions": { "e": "get", "of": { "e": "var", "name": "acc" }, "field": "actions" },
            } },
        }));
        let projected = project(&step, "state");
        assert_eq!(
            projected,
            e(
                json!({ "e": "let", "name": "acc", "be": { "e": "var", "name": "s" }, "in": { "e": "var", "name": "acc" } })
            )
        );

        // Read whole, or through two fields, a binding stays as it is.
        let whole = e(
            json!({ "e": "let", "name": "acc", "be": { "e": "var", "name": "x" },
            "in": { "e": "record", "fields": { "state": { "e": "var", "name": "acc" } } } }),
        );
        assert_eq!(project(&whole, "state"), whole_projected());
        fn whole_projected() -> Expr {
            serde_json::from_value(json!({ "e": "let", "name": "acc", "be": { "e": "var", "name": "x" }, "in": { "e": "var", "name": "acc" } })).unwrap()
        }

        // Read through a field first and whole later, it stays whole too.
        let mixed = e(
            json!({ "e": "let", "name": "acc", "be": { "e": "var", "name": "x" },
            "in": { "e": "if", "cond": { "e": "get", "of": { "e": "var", "name": "acc" }, "field": "ok" },
                "then": { "e": "record", "fields": { "state": { "e": "var", "name": "acc" } } },
                "else": { "e": "record", "fields": { "state": { "e": "lit", "v": 0 } } } } }),
        );
        let projected = project(&mixed, "state");
        assert!(
            matches!(&projected, Expr::Let { be, .. } if **be == e(json!({ "e": "var", "name": "x" })))
        );

        // A shadowing binding hides the inner uses: only the outer `be` reads the outer `acc`.
        let shadowed = e(
            json!({ "e": "let", "name": "acc", "be": { "e": "record", "fields": { "state": { "e": "lit", "v": 1 }, "actions": { "e": "var", "name": "junk" } } },
            "in": { "e": "let", "name": "acc",
                "be": { "e": "record", "fields": { "state": { "e": "get", "of": { "e": "var", "name": "acc" }, "field": "state" } } },
                "in": { "e": "record", "fields": { "state": { "e": "var", "name": "acc" } } } } }),
        );
        let projected = project(&shadowed, "state");
        assert_eq!(
            projected,
            e(
                json!({ "e": "let", "name": "acc", "be": { "e": "lit", "v": 1 },
            "in": { "e": "let", "name": "acc",
                "be": { "e": "record", "fields": { "state": { "e": "var", "name": "acc" } } },
                "in": { "e": "var", "name": "acc" } } })
            )
        );
    }
}
