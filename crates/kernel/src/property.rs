//! Properties: what a harness must never do, stated over the events of its runs.
//!
//! A property is `□φ`: `φ` holds at every event of every run. `φ` is first-order past-time temporal logic, so every
//! property is a safety property: a run violates it exactly when some finite prefix does, and that prefix is a
//! counterexample anyone can replay. Past-time operators look back over what already happened (`prev`, `since`,
//! `once`, `historically`), which is what lets a property be checked as the trace grows, and lets it be monitored at
//! runtime before it is proved.
//!
//! Atoms are expressions over `event` (the event at the current position, as data) and the variables quantifiers
//! bind. An atom that fails to evaluate (a field the event doesn't have) is false, so field access is guarded by a
//! separate atom under `implies`, or by an `if`. Quantifiers are bounded by the trace: `exists x of e` ranges over the
//! values `e` gave at the events so far, which is how a property connects a result to the call it answers.

use crate::check::{Problem, check, infer};
use crate::ir::{Expr, Type};
use crate::labeled::{self, LValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "f", rename_all = "camelCase")]
pub enum Formula {
    Atom {
        holds: Expr,
    },
    Not {
        of: Box<Formula>,
    },
    And {
        of: Vec<Formula>,
    },
    Or {
        of: Vec<Formula>,
    },
    Implies {
        #[serde(rename = "if")]
        condition: Box<Formula>,
        then: Box<Formula>,
    },
    /// `x` ranges over the values `of` gave at the events so far.
    Exists {
        var: String,
        of: Expr,
        body: Box<Formula>,
    },
    Forall {
        var: String,
        of: Expr,
        body: Box<Formula>,
    },
    /// At the previous event. False at the first.
    Prev {
        of: Box<Formula>,
    },
    /// `right` held at some event so far, and `left` at every event after it.
    Since {
        left: Box<Formula>,
        right: Box<Formula>,
    },
    /// At some event so far.
    Once {
        of: Box<Formula>,
    },
    /// At every event so far.
    Historically {
        of: Box<Formula>,
    },
}

/// `□always`: at every event of every run.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub always: Formula,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PropertyProblem {
    /// Where in the formula: a `/`-separated path of field names and indices.
    pub at: String,
    pub problem: Problem,
}

/// Problems with a property's atoms and quantifier bounds. Events are typed `Any`, as `δ`'s inputs are.
pub fn check_property(property: &Property) -> Vec<PropertyProblem> {
    let mut out = Vec::new();
    check_formula(&property.always, "", &mut vec![], &mut out);
    out
}

fn check_formula(f: &Formula, at: &str, bound: &mut Vec<String>, out: &mut Vec<PropertyProblem>) {
    let child = |s: &str| format!("{at}/{s}");
    let mut expr = |e: &Expr, at: String, bound: &Vec<String>, want: Option<Type>| {
        let scope = std::iter::once("event".to_string())
            .chain(bound.iter().cloned())
            .map(|n| (n, Type::Any))
            .collect();
        let (_, problems) = match &want {
            Some(expected) => check(e, expected, scope),
            None => infer(e, scope),
        };
        out.extend(problems.into_iter().map(|problem| PropertyProblem {
            at: at.clone(),
            problem,
        }));
    };
    match f {
        Formula::Atom { holds } => expr(holds, child("holds"), bound, Some(Type::Bool)),
        Formula::Not { of }
        | Formula::Prev { of }
        | Formula::Once { of }
        | Formula::Historically { of } => check_formula(of, &child("of"), bound, out),
        Formula::And { of } | Formula::Or { of } => of
            .iter()
            .enumerate()
            .for_each(|(i, g)| check_formula(g, &child(&format!("of/{i}")), bound, out)),
        Formula::Implies { condition, then } => {
            check_formula(condition, &child("if"), bound, out);
            check_formula(then, &child("then"), bound, out);
        }
        Formula::Since { left, right } => {
            check_formula(left, &child("left"), bound, out);
            check_formula(right, &child("right"), bound, out);
        }
        Formula::Exists { var, of, body } | Formula::Forall { var, of, body } => {
            expr(of, child("of"), bound, None);
            bound.push(var.clone());
            check_formula(body, &child("body"), bound, out);
            bound.pop();
        }
    }
}

/// The value of an expression at an event, with these variables bound; `None` if it fails to evaluate.
fn value_at(e: &Expr, event: &Value, env: &[(String, Value)]) -> Option<Value> {
    let scope = std::iter::once(("event".to_string(), LValue::trusted(event)))
        .chain(env.iter().map(|(n, v)| (n.clone(), LValue::trusted(v))))
        .collect();
    labeled::eval(e, scope).ok().map(|v| v.value())
}

/// Whether `f` holds at position `i` of `trace`.
fn holds_at(f: &Formula, trace: &[Value], i: usize, env: &mut Vec<(String, Value)>) -> bool {
    match f {
        Formula::Atom { holds } => value_at(holds, &trace[i], env) == Some(Value::Bool(true)),
        Formula::Not { of } => !holds_at(of, trace, i, env),
        Formula::And { of } => of.iter().all(|g| holds_at(g, trace, i, env)),
        Formula::Or { of } => of.iter().any(|g| holds_at(g, trace, i, env)),
        Formula::Implies { condition, then } => {
            !holds_at(condition, trace, i, env) || holds_at(then, trace, i, env)
        }
        Formula::Exists { var, of, body } | Formula::Forall { var, of, body } => {
            let mut domain: Vec<Value> = Vec::new();
            for event in &trace[..=i] {
                if let Some(v) = value_at(of, event, env)
                    && !v.is_null()
                    && !domain.contains(&v)
                {
                    domain.push(v);
                }
            }
            let exists = matches!(f, Formula::Exists { .. });
            let mut check = |v: Value| {
                env.push((var.clone(), v));
                let r = holds_at(body, trace, i, env);
                env.pop();
                r
            };
            if exists {
                domain.into_iter().any(&mut check)
            } else {
                domain.into_iter().all(&mut check)
            }
        }
        Formula::Prev { of } => i > 0 && holds_at(of, trace, i - 1, env),
        Formula::Since { left, right } => (0..=i).rev().any(|j| {
            holds_at(right, trace, j, env) && (j + 1..=i).all(|k| holds_at(left, trace, k, env))
        }),
        Formula::Once { of } => (0..=i).any(|j| holds_at(of, trace, j, env)),
        Formula::Historically { of } => (0..=i).all(|j| holds_at(of, trace, j, env)),
    }
}

/// Whether a trace satisfies `□φ`, and if not, the first event where `φ` fails.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "verdict", rename_all = "camelCase")]
pub enum Monitored {
    Holds,
    Violated { at: usize },
}

pub fn monitor(property: &Property, trace: &[Value]) -> Monitored {
    match first_violation(property, trace, 0) {
        None => Monitored::Holds,
        Some(at) => Monitored::Violated { at },
    }
}

/// The first event at or after `from` where `φ` fails, given everything before it.
pub fn first_violation(property: &Property, trace: &[Value], from: usize) -> Option<usize> {
    (from..trace.len()).find(|&i| !holds_at(&property.always, trace, i, &mut Vec::new()))
}

/// Whether the formula has no quantifiers, so the incremental monitor can decide it.
pub fn quantifier_free(f: &Formula) -> bool {
    match f {
        Formula::Atom { .. } => true,
        Formula::Exists { .. } | Formula::Forall { .. } => false,
        Formula::Not { of }
        | Formula::Prev { of }
        | Formula::Once { of }
        | Formula::Historically { of } => quantifier_free(of),
        Formula::And { of } | Formula::Or { of } => of.iter().all(quantifier_free),
        Formula::Implies { condition, then } => quantifier_free(condition) && quantifier_free(then),
        Formula::Since { left, right } => quantifier_free(left) && quantifier_free(right),
    }
}

/// An incremental monitor for the quantifier-free fragment: the truth of every subformula at the last event, which
/// is all a past-time formula needs to be decided at the next one. Its state is small and part of a configuration, so
/// invariants can talk about it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct MonitorState {
    /// Each subformula's value at the last event, in post-order. Empty before the first.
    #[serde(default)]
    pub last: Vec<bool>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MonitorError {
    /// Quantifiers need the values seen so far, not a fixed set of truths: they are checked on whole traces.
    Quantified,
    /// The state isn't one this property's monitor could be in: it holds a value per subformula.
    State { expected: usize, got: usize },
}

/// The incremental monitor over more events: its state after them, and whether `φ` held at each. On a violation it
/// stops there, with `at` counted from the first of `events`. A host keeps the state between steps and hands it back
/// unchanged, so it needs neither the trace so far nor the time to re-check it.
pub fn monitor_step(
    property: &Property,
    state: &MonitorState,
    events: &[Value],
) -> Result<(MonitorState, Monitored), MonitorError> {
    let formula = &property.always;
    if !quantifier_free(formula) {
        return Err(MonitorError::Quantified);
    }
    let expected = size(formula);
    if !state.last.is_empty() && state.last.len() != expected {
        return Err(MonitorError::State {
            expected,
            got: state.last.len(),
        });
    }
    let mut state = state.clone();
    for (at, event) in events.iter().enumerate() {
        let (next, holds) = advance(formula, &state, event)?;
        state = next;
        if !holds {
            return Ok((state, Monitored::Violated { at }));
        }
    }
    Ok((state, Monitored::Holds))
}

/// How many subformulas `f` has, counting itself: the length of its monitor's state after the first event.
fn size(f: &Formula) -> usize {
    1 + match f {
        Formula::Atom { .. } => 0,
        Formula::Exists { body, .. } | Formula::Forall { body, .. } => size(body),
        Formula::Not { of }
        | Formula::Prev { of }
        | Formula::Once { of }
        | Formula::Historically { of } => size(of),
        Formula::And { of } | Formula::Or { of } => of.iter().map(size).sum(),
        Formula::Implies { condition, then } => size(condition) + size(then),
        Formula::Since { left, right } => size(left) + size(right),
    }
}

/// The monitor after one more event, and whether `φ` holds there.
pub fn advance(
    formula: &Formula,
    state: &MonitorState,
    event: &Value,
) -> Result<(MonitorState, bool), MonitorError> {
    let mut now = Vec::new();
    let first = state.last.is_empty();
    let root = step(formula, &state.last, first, event, &mut now)?;
    Ok((MonitorState { last: now }, root))
}

/// Evaluate `f` at the new event, appending every subformula's value in post-order; returns `f`'s.
fn step(
    f: &Formula,
    last: &[bool],
    first: bool,
    event: &Value,
    now: &mut Vec<bool>,
) -> Result<bool, MonitorError> {
    let sub = |g: &Formula, now: &mut Vec<bool>| step(g, last, first, event, now);
    let before = |idx: usize| !first && last[idx];
    let v = match f {
        Formula::Atom { holds } => value_at(holds, event, &[]) == Some(Value::Bool(true)),
        Formula::Not { of } => !sub(of, now)?,
        Formula::And { of } => {
            let mut all = true;
            for g in of {
                all &= sub(g, now)?;
            }
            all
        }
        Formula::Or { of } => {
            let mut any = false;
            for g in of {
                any |= sub(g, now)?;
            }
            any
        }
        Formula::Implies { condition, then } => {
            let c = sub(condition, now)?;
            let t = sub(then, now)?;
            !c || t
        }
        Formula::Exists { .. } | Formula::Forall { .. } => return Err(MonitorError::Quantified),
        Formula::Prev { of } => {
            // `prev` needs its operand's value at the last event: its index is the last one `sub` appends.
            sub(of, now)?;
            before(now.len() - 1)
        }
        Formula::Since { left, right } => {
            let l = sub(left, now)?;
            let r = sub(right, now)?;
            r || (l && before(now.len()))
        }
        Formula::Once { of } => sub(of, now)? || before(now.len()),
        Formula::Historically { of } => sub(of, now)? && (first || last[now.len()]),
    };
    now.push(v);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn atom(field: &str, value: Value) -> Formula {
        serde_json::from_value(json!({ "f": "atom", "holds": {
            "e": "op", "op": "eq", "args": [{ "e": "get", "of": { "e": "var", "name": "event" }, "field": field }, { "e": "lit", "v": value }]
        }}))
        .unwrap()
    }

    fn formulas() -> Vec<Formula> {
        let (a, b) = (atom("a", json!(true)), atom("b", json!(true)));
        let bx = |f: Formula| Box::new(f);
        vec![
            Formula::Prev { of: bx(a.clone()) },
            Formula::Since {
                left: bx(a.clone()),
                right: bx(b.clone()),
            },
            Formula::Once { of: bx(b.clone()) },
            Formula::Historically { of: bx(a.clone()) },
            Formula::Implies {
                condition: bx(b.clone()),
                then: bx(Formula::Prev {
                    of: bx(Formula::Historically { of: bx(a.clone()) }),
                }),
            },
            Formula::Not {
                of: bx(Formula::Since {
                    left: bx(Formula::Not { of: bx(a.clone()) }),
                    right: bx(b.clone()),
                }),
            },
        ]
    }

    #[test]
    fn stepping_agrees_with_the_whole_trace() {
        // Every trace of length up to 4, delivered in two steps split at every point.
        let letters = [(false, false), (false, true), (true, false), (true, true)];
        for f in formulas() {
            let property = Property {
                name: "p".to_string(),
                always: f.clone(),
            };
            for len in 1..=4usize {
                for code in 0..4usize.pow(len as u32) {
                    let trace: Vec<Value> = (0..len)
                        .map(|i| {
                            let (a, b) = letters[(code / 4usize.pow(i as u32)) % 4];
                            json!({ "a": a, "b": b })
                        })
                        .collect();
                    for split in 0..=len {
                        let start = MonitorState::default();
                        let (mid, first) =
                            monitor_step(&property, &start, &trace[..split]).unwrap();
                        let stepped = match first {
                            Monitored::Violated { at } => Monitored::Violated { at },
                            Monitored::Holds => {
                                match monitor_step(&property, &mid, &trace[split..]).unwrap().1 {
                                    Monitored::Violated { at } => {
                                        Monitored::Violated { at: split + at }
                                    }
                                    Monitored::Holds => Monitored::Holds,
                                }
                            }
                        };
                        assert_eq!(stepped, monitor(&property, &trace), "{f:?} on {trace:?}");
                        if !mid.last.is_empty() {
                            assert_eq!(mid.last.len(), size(&f));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_state_for_another_property_is_refused() {
        let property = Property {
            name: "p".to_string(),
            always: formulas()[0].clone(),
        };
        let state = MonitorState { last: vec![true] };
        assert_eq!(
            monitor_step(&property, &state, &[json!({})]),
            Err(MonitorError::State {
                expected: 2,
                got: 1
            })
        );
    }

    #[test]
    fn the_monitor_agrees_with_the_trace_semantics() {
        // Every trace of length up to 5 over the four combinations of `a` and `b`.
        let letters = [(false, false), (false, true), (true, false), (true, true)];
        for f in formulas() {
            for len in 1..=5usize {
                for code in 0..4usize.pow(len as u32) {
                    let trace: Vec<Value> = (0..len)
                        .map(|i| {
                            let (a, b) = letters[(code / 4usize.pow(i as u32)) % 4];
                            json!({ "a": a, "b": b })
                        })
                        .collect();
                    let mut state = MonitorState::default();
                    for i in 0..len {
                        let (next, now) = advance(&f, &state, &trace[i]).unwrap();
                        assert_eq!(
                            now,
                            holds_at(&f, &trace, i, &mut vec![]),
                            "{f:?} on {trace:?} at {i}"
                        );
                        state = next;
                    }
                }
            }
        }
    }
}
