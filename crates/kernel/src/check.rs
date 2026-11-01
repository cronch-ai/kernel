//! Static checking of expressions: what type an expression has, given the types of its variables, and what is wrong
//! with it if it has none.
//!
//! Checking is structural and gradual: `Any` conforms both ways, so shapes the interpreter checks when they arrive
//! pass here. Lists and the branches of an `if` take the join of their parts' types, so a list of records of different
//! shapes has the fields they share.

use crate::ir::{Expr, Type};
use crate::ops::{self, Op, OpTypeError};
use crate::types::{conforms, join, of_value, record_view};
use serde::Serialize;
use std::collections::BTreeMap;

/// Where in an expression something is: a `/`-separated path of fields from the root (`""`).
pub type Path = String;

fn child(at: &str, segment: &str) -> Path {
    format!("{at}/{segment}")
}

/// What makes an expression ill-typed.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Problem {
    UnknownVariable {
        at: Path,
        name: String,
    },
    UnknownOperator {
        at: Path,
        op: String,
    },
    Operator {
        at: Path,
        op: String,
        error: OpTypeError,
    },
    Mismatch {
        at: Path,
        expected: Type,
        got: Type,
    },
    NotARecord {
        at: Path,
        got: Type,
    },
    NoSuchField {
        at: Path,
        field: String,
        of: Type,
    },
    NotAList {
        at: Path,
        got: Type,
    },
    /// A `match` on something that isn't a variant tagged by its `on` field.
    NotAVariant {
        at: Path,
        on: String,
        got: Type,
    },
    NoSuchCase {
        at: Path,
        case: String,
    },
    /// A `match` that covers neither every case nor has an `otherwise`.
    NotExhaustive {
        at: Path,
        missing: Vec<String>,
    },
}

/// The type of an expression with these variables in scope, and what is wrong with it.
pub fn infer(expr: &Expr, scope: Vec<(String, Type)>) -> (Option<Type>, Vec<Problem>) {
    let mut checker = Checker {
        scope,
        problems: Vec::new(),
    };
    let ty = checker.infer(expr, "");
    (ty, checker.problems)
}

struct Checker {
    /// Variables in scope, innermost last.
    scope: Vec<(String, Type)>,
    problems: Vec<Problem>,
}

impl Checker {
    fn problem(&mut self, p: Problem) -> Option<Type> {
        self.problems.push(p);
        None
    }

    /// The expression's type, or `None` after recording why it has none.
    fn infer(&mut self, expr: &Expr, at: &str) -> Option<Type> {
        match expr {
            Expr::Lit { v } => Some(of_value(v)),
            Expr::Var { name } => match self.scope.iter().rev().find(|(n, _)| n == name) {
                Some((_, t)) => Some(t.clone()),
                None => self.problem(Problem::UnknownVariable {
                    at: at.to_string(),
                    name: name.clone(),
                }),
            },
            Expr::Op { op, args } => {
                let types: Vec<Option<Type>> = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| self.infer(a, &child(at, &format!("args/{i}"))))
                    .collect();
                let Some(parsed) = Op::parse(op) else {
                    return self.problem(Problem::UnknownOperator {
                        at: at.to_string(),
                        op: op.clone(),
                    });
                };
                let types: Vec<Type> = types.into_iter().collect::<Option<_>>()?;
                match ops::result_type(parsed, &types) {
                    Ok(t) => Some(t),
                    Err(error) => self.problem(Problem::Operator {
                        at: at.to_string(),
                        op: op.clone(),
                        error,
                    }),
                }
            }
            Expr::Let { name, be, body } => {
                let bound = self.infer(be, &child(at, "be"));
                self.scope.push((name.clone(), bound.unwrap_or(Type::Any)));
                let t = self.infer(body, &child(at, "in"));
                self.scope.pop();
                t
            }
            Expr::If {
                cond,
                then,
                otherwise,
            } => {
                let cond_at = child(at, "cond");
                if let Some(c) = self.infer(cond, &cond_at)
                    && !conforms(&c, &Type::Bool)
                {
                    self.problem(Problem::Mismatch {
                        at: cond_at,
                        expected: Type::Bool,
                        got: c,
                    });
                }
                let t = self.infer(then, &child(at, "then"));
                let e = self.infer(otherwise, &child(at, "else"));
                match (t, e) {
                    (Some(t), Some(e)) => match join(&t, &e) {
                        Some(j) => Some(j),
                        None => self.problem(Problem::Mismatch {
                            at: child(at, "else"),
                            expected: t,
                            got: e,
                        }),
                    },
                    _ => None,
                }
            }
            Expr::Record { fields } => {
                let mut types = BTreeMap::new();
                let mut ok = true;
                for (name, e) in fields {
                    match self.infer(e, &child(at, &format!("fields/{name}"))) {
                        Some(t) => {
                            types.insert(name.clone(), t);
                        }
                        None => ok = false,
                    }
                }
                ok.then_some(Type::Record(types))
            }
            Expr::List { items } => {
                let types: Vec<Option<Type>> = items
                    .iter()
                    .enumerate()
                    .map(|(i, e)| self.infer(e, &child(at, &format!("items/{i}"))))
                    .collect();
                let types: Vec<Type> = types.into_iter().collect::<Option<_>>()?;
                let mut element = types.first().cloned().unwrap_or(Type::Any);
                for (i, t) in types.iter().enumerate().skip(1) {
                    match join(&element, t) {
                        Some(j) => element = j,
                        None => {
                            return self.problem(Problem::Mismatch {
                                at: child(at, &format!("items/{i}")),
                                expected: element,
                                got: t.clone(),
                            });
                        }
                    }
                }
                Some(Type::List(Box::new(element)))
            }
            Expr::Get { of, field } => match self.infer(of, &child(at, "of"))? {
                Type::Any => Some(Type::Any),
                // A field every case has is typed; one only some cases have is read gradually, and checked when it is.
                variant @ Type::Variant { .. } => match record_view(&variant) {
                    Type::Record(fields) => Some(fields.get(field).cloned().unwrap_or(Type::Any)),
                    _ => Some(Type::Any),
                },
                Type::Record(fields) => match fields.get(field) {
                    Some(t) => Some(t.clone()),
                    None => self.problem(Problem::NoSuchField {
                        at: at.to_string(),
                        field: field.clone(),
                        of: Type::Record(fields),
                    }),
                },
                got => self.problem(Problem::NotARecord {
                    at: at.to_string(),
                    got,
                }),
            },
            Expr::Label { of } => {
                self.infer(of, &child(at, "of"))?;
                Some(Type::List(Box::new(Type::String)))
            }
            Expr::Match {
                of,
                on,
                cases,
                otherwise,
            } => {
                let of_at = child(at, "of");
                let of_ty = self.infer(of, &of_at);
                // Each case's binding type: the variant's case, or `Any` if what is matched is unchecked.
                let declared: Option<BTreeMap<String, Type>> = match &of_ty {
                    None | Some(Type::Any) => None,
                    Some(Type::Variant {
                        tag,
                        cases: declared,
                    }) if tag == on => Some(declared.clone()),
                    Some(got) => {
                        return self.problem(Problem::NotAVariant {
                            at: of_at,
                            on: on.clone(),
                            got: got.clone(),
                        });
                    }
                };
                if let Some(declared) = &declared {
                    for name in cases.keys().filter(|c| !declared.contains_key(*c)) {
                        self.problem(Problem::NoSuchCase {
                            at: child(at, &format!("cases/{name}")),
                            case: name.clone(),
                        });
                    }
                    let missing: Vec<String> = declared
                        .keys()
                        .filter(|c| !cases.contains_key(*c))
                        .cloned()
                        .collect();
                    if otherwise.is_none() && !missing.is_empty() {
                        self.problem(Problem::NotExhaustive {
                            at: at.to_string(),
                            missing,
                        });
                    }
                } else if otherwise.is_none() && of_ty.is_some() {
                    // Matching an unchecked value: only a default covers what it might be.
                    self.problem(Problem::NotExhaustive {
                        at: at.to_string(),
                        missing: vec![],
                    });
                }
                let mut result: Option<Option<Type>> = None;
                let mut merge = |this: &mut Self, t: Option<Type>, body_at: String| {
                    result = Some(match (result.take(), t) {
                        (None, t) => t,
                        (Some(Some(acc)), Some(t)) => match join(&acc, &t) {
                            Some(j) => Some(j),
                            None => this.problem(Problem::Mismatch {
                                at: body_at,
                                expected: acc,
                                got: t,
                            }),
                        },
                        _ => None,
                    });
                };
                for (name, case) in cases {
                    let bound = declared
                        .as_ref()
                        .and_then(|d| d.get(name).cloned())
                        .unwrap_or(Type::Any);
                    self.scope.push((case.bind.clone(), bound));
                    let body_at = child(at, &format!("cases/{name}/body"));
                    let t = self.infer(&case.body, &body_at);
                    self.scope.pop();
                    merge(self, t, body_at);
                }
                if let Some(o) = otherwise {
                    let body_at = child(at, "otherwise");
                    let t = self.infer(o, &body_at);
                    merge(self, t, body_at);
                }
                result.flatten()
            }
            Expr::Fold {
                over,
                init,
                acc,
                item,
                body,
            } => {
                let over_at = child(at, "over");
                let item_ty = match self.infer(over, &over_at) {
                    Some(Type::List(t)) => Some(*t),
                    Some(Type::Any) => Some(Type::Any),
                    Some(got) => self.problem(Problem::NotAList { at: over_at, got }),
                    None => None,
                };
                let acc_ty = self.infer(init, &child(at, "init"));
                self.scope
                    .push((acc.clone(), acc_ty.clone().unwrap_or(Type::Any)));
                self.scope
                    .push((item.clone(), item_ty.unwrap_or(Type::Any)));
                let body_ty = self.infer(body, &child(at, "body"));
                self.scope.truncate(self.scope.len() - 2);
                let (acc_ty, body_ty) = (acc_ty?, body_ty?);
                if !conforms(&body_ty, &acc_ty) {
                    return self.problem(Problem::Mismatch {
                        at: child(at, "body"),
                        expected: acc_ty,
                        got: body_ty,
                    });
                }
                Some(acc_ty)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn infer_json(e: serde_json::Value, scope: Vec<(&str, Type)>) -> (Option<Type>, Vec<Problem>) {
        let scope = scope.into_iter().map(|(n, t)| (n.to_string(), t)).collect();
        infer(&serde_json::from_value(e).unwrap(), scope)
    }

    #[test]
    fn folds_must_be_over_lists_and_keep_their_accumulators_type() {
        let sum = json!({ "e": "fold", "over": { "e": "var", "name": "xs" }, "init": { "e": "lit", "v": 0 },
                          "acc": "a", "item": "x", "body": { "e": "op", "op": "add", "args": [{ "e": "var", "name": "a" }, { "e": "var", "name": "x" }] } });
        let ints = Type::List(Box::new(Type::Int));
        assert_eq!(
            infer_json(sum, vec![("xs", ints.clone())]),
            (Some(Type::Int), vec![])
        );

        let over_int = json!({ "e": "fold", "over": { "e": "lit", "v": 3 }, "init": { "e": "lit", "v": 0 },
                               "acc": "a", "item": "x", "body": { "e": "var", "name": "a" } });
        assert_eq!(
            infer_json(over_int, vec![]).1,
            [Problem::NotAList {
                at: "/over".into(),
                got: Type::Int
            }]
        );

        let changes_type = json!({ "e": "fold", "over": { "e": "var", "name": "xs" }, "init": { "e": "lit", "v": 0 },
                                   "acc": "a", "item": "x", "body": { "e": "lit", "v": "text" } });
        assert_eq!(
            infer_json(changes_type, vec![("xs", ints)]).1,
            [Problem::Mismatch {
                at: "/body".into(),
                expected: Type::Int,
                got: Type::String
            }]
        );
    }

    #[test]
    fn lists_and_branches_join_their_parts() {
        let actions = json!({ "e": "list", "items": [
            { "e": "record", "fields": { "kind": { "e": "lit", "v": "call" }, "op": { "e": "lit", "v": "x" } } },
            { "e": "record", "fields": { "kind": { "e": "lit", "v": "halt" }, "value": { "e": "lit", "v": 1 } } },
        ]});
        let kind_only = Type::Record(BTreeMap::from([("kind".to_string(), Type::String)]));
        assert_eq!(
            infer_json(actions, vec![]).0,
            Some(Type::List(Box::new(kind_only)))
        );

        let mixed = json!({ "e": "if", "cond": { "e": "lit", "v": true }, "then": { "e": "lit", "v": 1 }, "else": { "e": "lit", "v": "one" } });
        assert_eq!(
            infer_json(mixed, vec![]).1[0],
            Problem::Mismatch {
                at: "/else".into(),
                expected: Type::Int,
                got: Type::String
            }
        );
    }
}
