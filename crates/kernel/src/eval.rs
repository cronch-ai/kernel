//! Partial evaluation: run a Procedure as far as its holes allow.
//!
//! Evaluation is strict and goes on past a hole wherever the hole's value isn't needed, so one call reports every hole
//! that blocks the result at once, and independent holes can be resolved in parallel. A nested Ask whose arguments are
//! known becomes a closed Ask, identified by its content key; `resolved` supplies values for the keys the runtime has
//! closed. A resolution must inhabit its Ask's output type: the kernel checks it rather than trust whoever supplied it.

use crate::check::Path;
use crate::ir::{Answer, Ask, Expr, Type, World};
use crate::key::ask_key;
use crate::ops::{self, Op, OpEvalError};
use crate::types::inhabits;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// A hole evaluation is blocked on.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Open {
    /// A computational hole: a closed Ask the runtime can try to resolve.
    Ask { at: Path, key: String, ask: Ask },
    /// An epistemic hole with no value. No resolution exists; the Procedure must be replaced.
    Opaque {
        at: Path,
        #[serde(rename = "type")]
        ty: Type,
    },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EvalError {
    Operator {
        at: Path,
        op: String,
        error: OpEvalError,
    },
    /// The value supplied for a hole doesn't inhabit the hole's output type.
    Resolution {
        at: Path,
        key: String,
        expected: Type,
        got: Value,
    },
    UnknownVariable {
        at: Path,
        name: String,
    },
    UnknownFact {
        at: Path,
        name: String,
    },
    UnknownOperator {
        at: Path,
        op: String,
    },
    NotABool {
        at: Path,
        got: Value,
    },
    NoSuchField {
        at: Path,
        field: String,
        of: Value,
    },
    NotAList {
        at: Path,
        got: Value,
    },
    /// A label read outside a harness's labelled evaluation, which is the only one that knows labels.
    Unlabeled {
        at: Path,
    },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Evaluation {
    Done { value: Value },
    Blocked { open: Vec<Open> },
    Failed { error: EvalError },
}

/// A partial value: known, or unknown until these holes are resolved.
enum Ev {
    Known(Value),
    Unknown(Vec<Open>),
}

fn merge(into: &mut Vec<Open>, from: Vec<Open>) {
    for o in from {
        let duplicate = into.iter().any(|x| match (x, &o) {
            (Open::Ask { key: a, .. }, Open::Ask { key: b, .. }) => a == b,
            (Open::Opaque { at: a, .. }, Open::Opaque { at: b, .. }) => a == b,
            _ => false,
        });
        if !duplicate {
            into.push(o);
        }
    }
}

/// Every operand known, or the union of the holes blocking any of them.
fn all(evs: Vec<Ev>) -> Result<Vec<Value>, Vec<Open>> {
    let mut known = Vec::new();
    let mut open: Option<Vec<Open>> = None;
    for ev in evs {
        match ev {
            Ev::Known(v) => known.push(v),
            Ev::Unknown(o) => merge(open.get_or_insert_with(Vec::new), o),
        }
    }
    match open {
        None => Ok(known),
        Some(o) => Err(o),
    }
}

fn child(at: &str, segment: &str) -> Path {
    format!("{at}/{segment}")
}

pub fn evaluate(
    ask: &Ask,
    world: &World,
    answer: &Answer,
    resolved: &BTreeMap<String, Value>,
) -> Evaluation {
    let body = match answer {
        Answer::Value(value) => {
            return Evaluation::Done {
                value: value.clone(),
            };
        }
        Answer::Procedure(body) => body,
    };
    let scope = ask
        .spec
        .params
        .iter()
        .map(|p| (p.name.clone(), Some(ask.args.get(&p.name).cloned())))
        .collect();
    let mut evaluator = Evaluator {
        world,
        resolved,
        scope,
    };
    match evaluator.eval(body, "") {
        Ok(Ev::Known(value)) => Evaluation::Done { value },
        Ok(Ev::Unknown(open)) => Evaluation::Blocked { open },
        Err(error) => Evaluation::Failed { error },
    }
}

struct Evaluator<'a> {
    world: &'a World,
    resolved: &'a BTreeMap<String, Value>,
    /// Variables in scope, innermost last. `Some(None)` is bound but unknown: its holes are reported where it was bound.
    scope: Vec<(String, Option<Option<Value>>)>,
}

impl Evaluator<'_> {
    fn eval(&mut self, expr: &Expr, at: &str) -> Result<Ev, EvalError> {
        match expr {
            Expr::Lit { v } => Ok(Ev::Known(v.clone())),
            Expr::Var { name } => match self.scope.iter().rev().find(|(n, _)| n == name) {
                Some((_, Some(Some(v)))) => Ok(Ev::Known(v.clone())),
                Some((_, _)) => Ok(Ev::Unknown(Vec::new())),
                None => Err(EvalError::UnknownVariable {
                    at: at.to_string(),
                    name: name.clone(),
                }),
            },
            Expr::Fact { name } => self
                .world
                .facts
                .get(name)
                .map(|f| Ev::Known(f.value.clone()))
                .ok_or_else(|| EvalError::UnknownFact {
                    at: at.to_string(),
                    name: name.clone(),
                }),
            Expr::Op { op, args } => {
                let parsed = Op::parse(op).ok_or_else(|| EvalError::UnknownOperator {
                    at: at.to_string(),
                    op: op.clone(),
                })?;
                let evs = self.eval_all(
                    args.iter()
                        .enumerate()
                        .map(|(i, a)| (a, child(at, &format!("args/{i}")))),
                )?;
                match all(evs) {
                    Ok(values) => ops::apply(parsed, &values).map(Ev::Known).map_err(|error| {
                        EvalError::Operator {
                            at: at.to_string(),
                            op: op.clone(),
                            error,
                        }
                    }),
                    Err(open) => Ok(Ev::Unknown(open)),
                }
            }
            Expr::Ask { spec, args } => {
                let names: Vec<&String> = args.keys().collect();
                let evs = self.eval_all(
                    args.iter()
                        .map(|(n, a)| (a, child(at, &format!("args/{n}")))),
                )?;
                let values = match all(evs) {
                    Ok(values) => values,
                    Err(open) => return Ok(Ev::Unknown(open)),
                };
                let closed = Ask {
                    spec: spec.clone(),
                    args: names.into_iter().cloned().zip(values).collect(),
                };
                let key = ask_key(&closed, &BTreeMap::new());
                match self.resolved.get(&key) {
                    Some(v) if inhabits(v, &spec.output) => Ok(Ev::Known(v.clone())),
                    Some(v) => Err(EvalError::Resolution {
                        at: at.to_string(),
                        key,
                        expected: spec.output.clone(),
                        got: v.clone(),
                    }),
                    None => Ok(Ev::Unknown(vec![Open::Ask {
                        at: at.to_string(),
                        key,
                        ask: closed,
                    }])),
                }
            }
            Expr::Assume { v, .. } => Ok(Ev::Known(v.clone())),
            Expr::Opaque { ty, .. } => Ok(Ev::Unknown(vec![Open::Opaque {
                at: at.to_string(),
                ty: ty.clone(),
            }])),
            Expr::Let { name, be, body } => {
                let bound = self.eval(be, &child(at, "be"))?;
                let (value, mut open) = match bound {
                    Ev::Known(v) => (Some(v), None),
                    Ev::Unknown(o) => (None, Some(o)),
                };
                self.scope.push((name.clone(), Some(value)));
                let result = self.eval(body, &child(at, "in"));
                self.scope.pop();
                match (result?, open.take()) {
                    (ev, None) => Ok(ev),
                    (Ev::Known(_), Some(o)) => Ok(Ev::Unknown(o)),
                    (Ev::Unknown(more), Some(mut o)) => {
                        merge(&mut o, more);
                        Ok(Ev::Unknown(o))
                    }
                }
            }
            Expr::If {
                cond,
                then,
                otherwise,
            } => {
                let cond_at = child(at, "cond");
                match self.eval(cond, &cond_at)? {
                    Ev::Unknown(open) => Ok(Ev::Unknown(open)),
                    Ev::Known(Value::Bool(true)) => self.eval(then, &child(at, "then")),
                    Ev::Known(Value::Bool(false)) => self.eval(otherwise, &child(at, "else")),
                    Ev::Known(got) => Err(EvalError::NotABool { at: cond_at, got }),
                }
            }
            Expr::Record { fields } => {
                let names: Vec<&String> = fields.keys().collect();
                let evs = self.eval_all(
                    fields
                        .iter()
                        .map(|(n, e)| (e, child(at, &format!("fields/{n}")))),
                )?;
                match all(evs) {
                    Ok(values) => Ok(Ev::Known(Value::Object(
                        names
                            .into_iter()
                            .cloned()
                            .zip(values)
                            .collect::<Map<_, _>>(),
                    ))),
                    Err(open) => Ok(Ev::Unknown(open)),
                }
            }
            Expr::List { items } => {
                let evs = self.eval_all(
                    items
                        .iter()
                        .enumerate()
                        .map(|(i, e)| (e, child(at, &format!("items/{i}")))),
                )?;
                match all(evs) {
                    Ok(values) => Ok(Ev::Known(Value::Array(values))),
                    Err(open) => Ok(Ev::Unknown(open)),
                }
            }
            Expr::Get { of, field } => match self.eval(of, &child(at, "of"))? {
                Ev::Unknown(open) => Ok(Ev::Unknown(open)),
                Ev::Known(v) => {
                    v.get(field)
                        .cloned()
                        .map(Ev::Known)
                        .ok_or_else(|| EvalError::NoSuchField {
                            at: at.to_string(),
                            field: field.clone(),
                            of: v,
                        })
                }
            },
            Expr::Label { .. } => Err(EvalError::Unlabeled { at: at.to_string() }),
            Expr::Fold {
                over,
                init,
                acc,
                item,
                body,
            } => {
                let over_at = child(at, "over");
                let items = match self.eval(over, &over_at)? {
                    Ev::Unknown(open) => return Ok(Ev::Unknown(open)),
                    Ev::Known(Value::Array(items)) => items,
                    Ev::Known(got) => return Err(EvalError::NotAList { at: over_at, got }),
                };
                let (mut value, mut open) = match self.eval(init, &child(at, "init"))? {
                    Ev::Known(v) => (Some(v), None),
                    Ev::Unknown(o) => (None, Some(o)),
                };
                // Strict, like `let`: once the accumulator is unknown the result is, but every item is still visited,
                // so the holes of all of them are reported at once.
                let body_at = child(at, "body");
                for x in items {
                    self.scope.push((acc.clone(), Some(value.take())));
                    self.scope.push((item.clone(), Some(Some(x))));
                    let result = self.eval(body, &body_at);
                    self.scope.truncate(self.scope.len() - 2);
                    match result? {
                        Ev::Known(v) => value = Some(v),
                        Ev::Unknown(o) => merge(open.get_or_insert_with(Vec::new), o),
                    }
                }
                match (open, value) {
                    (None, Some(v)) => Ok(Ev::Known(v)),
                    (open, _) => Ok(Ev::Unknown(open.unwrap_or_default())),
                }
            }
        }
    }

    fn eval_all<'e>(
        &mut self,
        exprs: impl Iterator<Item = (&'e Expr, Path)>,
    ) -> Result<Vec<Ev>, EvalError> {
        exprs.map(|(e, at)| self.eval(e, &at)).collect()
    }
}
