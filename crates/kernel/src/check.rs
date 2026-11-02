//! Static checking of expressions: what type an expression has, given the types of its variables, and what is wrong
//! with it if it has none.
//!
//! Checking is structural and gradual: `Any` conforms both ways, so shapes the interpreter checks when they arrive
//! pass here. Lists and the branches of an `if` take the join of their parts' types, so a list of records of different
//! shapes has the fields they share.
//!
//! Checking is also bidirectional. Where a type is expected (a harness's state, a contract's condition, an annotated
//! `let` or `fold`), an expression is checked against it rather than inferred and compared afterwards. That is what
//! lets an expression build a value at a variant type: a record literal whose tag field is a string literal naming a
//! case is checked against that case, and has the variant's type. An empty list literal checked against `List(T)` is a
//! `List(T)`, and so is an `append` whose list and item check against it. Everything else is inferred, and must
//! conform to what was expected.

use crate::ir::{Expr, Type};
use crate::ops::{self, Op, OpTypeError};
use crate::types::{conforms, join, of_value, record_view};
use serde::Serialize;
use serde_json::Value;
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

/// The type of an expression checked against `expected`, with these variables in scope, and what is wrong with it.
pub fn check(
    expr: &Expr,
    expected: &Type,
    scope: Vec<(String, Type)>,
) -> (Option<Type>, Vec<Problem>) {
    let mut checker = Checker {
        scope,
        problems: Vec::new(),
    };
    let ty = checker.check(expr, expected, "");
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
        self.synth(expr, at, None)
    }

    /// The expression's type, checked against `expected`.
    fn check(&mut self, expr: &Expr, expected: &Type, at: &str) -> Option<Type> {
        self.typed(expr, at, Some(expected))
    }

    fn typed(&mut self, expr: &Expr, at: &str, expected: Option<&Type>) -> Option<Type> {
        let Some(want) = expected.filter(|t| **t != Type::Any) else {
            return self.infer(expr, at);
        };
        match (expr, want) {
            // Variant introduction: the case the tag names, with the other fields checked against it.
            (Expr::Record { fields }, Type::Variant { tag, cases }) => {
                let case = match fields.get(tag) {
                    Some(Expr::Lit {
                        v: Value::String(case),
                    }) => cases.get(case),
                    _ => None,
                };
                if let Some(case_ty) = case {
                    let mut rest = fields.clone();
                    rest.remove(tag);
                    self.check(&Expr::Record { fields: rest }, case_ty, at)?;
                    return Some(want.clone());
                }
            }
            (Expr::Record { fields }, Type::Record(wanted)) => {
                let mut types = BTreeMap::new();
                let mut ok = true;
                for (name, e) in fields {
                    let field_at = child(at, &format!("fields/{name}"));
                    match self.typed(e, &field_at, wanted.get(name)) {
                        Some(t) => {
                            types.insert(name.clone(), t);
                        }
                        None => ok = false,
                    }
                }
                let got = Type::Record(types);
                if !ok {
                    return None;
                }
                return if conforms(&got, want) {
                    Some(got)
                } else {
                    self.problem(Problem::Mismatch {
                        at: at.to_string(),
                        expected: want.clone(),
                        got,
                    })
                };
            }
            (Expr::List { items }, Type::List(item)) => {
                let mut ok = true;
                for (i, e) in items.iter().enumerate() {
                    ok &= self
                        .check(e, item, &child(at, &format!("items/{i}")))
                        .is_some();
                }
                return ok.then(|| want.clone());
            }
            // `append` builds a list from a list and an item, so both are checked against what it must build.
            (Expr::Op { op, args }, Type::List(item)) if op == "append" && args.len() == 2 => {
                let list = self.check(&args[0], want, &child(at, "args/0"));
                let added = self.check(&args[1], item, &child(at, "args/1"));
                return (list.is_some() && added.is_some()).then(|| want.clone());
            }
            (Expr::Let { .. } | Expr::If { .. } | Expr::Match { .. } | Expr::Fold { .. }, _) => {
                return self.synth(expr, at, Some(want));
            }
            _ => {}
        }
        let got = self.infer(expr, at)?;
        if conforms(&got, want) {
            Some(got)
        } else {
            self.problem(Problem::Mismatch {
                at: at.to_string(),
                expected: want.clone(),
                got,
            })
        }
    }

    /// The expression's type, inferred, or, for the forms that pass it on, checked against `expected`.
    fn synth(&mut self, expr: &Expr, at: &str, expected: Option<&Type>) -> Option<Type> {
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
            Expr::Let { name, be, body, ty } => {
                let bound = match ty {
                    // An annotated binding has its annotation, whatever is wrong with what it binds.
                    Some(t) => {
                        self.check(be, t, &child(at, "be"));
                        Some(t.clone())
                    }
                    None => self.infer(be, &child(at, "be")),
                };
                self.scope.push((name.clone(), bound.unwrap_or(Type::Any)));
                let t = self.typed(body, &child(at, "in"), expected);
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
                let t = self.typed(then, &child(at, "then"), expected);
                let e = self.typed(otherwise, &child(at, "else"), expected);
                if let Some(want) = expected {
                    return (t.is_some() && e.is_some()).then(|| want.clone());
                }
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
                    let t = self.typed(&case.body, &body_at, expected);
                    self.scope.pop();
                    merge(self, t, body_at);
                }
                if let Some(o) = otherwise {
                    let body_at = child(at, "otherwise");
                    let t = self.typed(o, &body_at, expected);
                    merge(self, t, body_at);
                }
                match expected {
                    Some(want) => result.flatten().map(|_| want.clone()),
                    None => result.flatten(),
                }
            }
            Expr::Fold {
                over,
                init,
                acc,
                item,
                body,
                ty,
            } => {
                let over_at = child(at, "over");
                let item_ty = match self.infer(over, &over_at) {
                    Some(Type::List(t)) => Some(*t),
                    Some(Type::Any) => Some(Type::Any),
                    Some(got) => self.problem(Problem::NotAList { at: over_at, got }),
                    None => None,
                };
                // The accumulator's type: its annotation, or what is expected of the fold, or `init`'s.
                let declared = ty.clone().or(expected.cloned());
                let acc_ty = match &declared {
                    Some(t) => self.check(init, t, &child(at, "init")).map(|_| t.clone()),
                    None => self.infer(init, &child(at, "init")),
                };
                self.scope
                    .push((acc.clone(), acc_ty.clone().unwrap_or(Type::Any)));
                self.scope
                    .push((item.clone(), item_ty.unwrap_or(Type::Any)));
                let body_ty = match &declared {
                    Some(t) => self.check(body, t, &child(at, "body")),
                    None => self.infer(body, &child(at, "body")),
                };
                self.scope.truncate(self.scope.len() - 2);
                let (acc_ty, body_ty) = (acc_ty?, body_ty?);
                if !conforms(&body_ty, &acc_ty) {
                    return self.problem(Problem::Mismatch {
                        at: child(at, "body"),
                        expected: acc_ty,
                        got: body_ty,
                    });
                }
                if let Some(want) = expected
                    && !conforms(&acc_ty, want)
                {
                    return self.problem(Problem::Mismatch {
                        at: at.to_string(),
                        expected: want.clone(),
                        got: acc_ty,
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

    fn check_json(
        e: serde_json::Value,
        expected: &Type,
        scope: Vec<(&str, Type)>,
    ) -> (Option<Type>, Vec<Problem>) {
        let scope = scope.into_iter().map(|(n, t)| (n.to_string(), t)).collect();
        check(&serde_json::from_value(e).unwrap(), expected, scope)
    }

    fn slot() -> Type {
        serde_json::from_value(json!({ "Variant": { "tag": "kind", "cases": {
            "waiting": { "Record": {} },
            "ok": { "Record": { "value": "String" } },
        }}}))
        .unwrap()
    }

    #[test]
    fn a_record_literal_naming_a_case_has_the_variants_type() {
        let ok = json!({ "e": "record", "fields": { "kind": { "e": "lit", "v": "ok" }, "value": { "e": "var", "name": "s" } } });
        assert_eq!(
            check_json(ok.clone(), &slot(), vec![("s", Type::String)]),
            (Some(slot()), vec![])
        );
        // Inferred, it is only a record, which no variant accepts.
        assert!(!crate::types::conforms(
            &infer_json(ok.clone(), vec![("s", Type::String)]).0.unwrap(),
            &slot()
        ));

        // The case's fields are checked against it.
        let (_, problems) = check_json(ok, &slot(), vec![("s", Type::Int)]);
        assert_eq!(
            problems,
            [Problem::Mismatch {
                at: "/fields/value".into(),
                expected: Type::String,
                got: Type::Int
            }]
        );

        // A tag that names no case, or isn't a literal, is a record like any other.
        let unknown = json!({ "e": "record", "fields": { "kind": { "e": "lit", "v": "gone" } } });
        let (ty, problems) = check_json(unknown, &slot(), vec![]);
        assert_eq!(ty, None);
        assert!(matches!(problems[0], Problem::Mismatch { .. }));
    }

    #[test]
    fn expected_types_reach_through_lets_branches_and_lists() {
        let waiting =
            json!({ "e": "record", "fields": { "kind": { "e": "lit", "v": "waiting" } } });
        let ok = json!({ "e": "record", "fields": { "kind": { "e": "lit", "v": "ok" }, "value": { "e": "lit", "v": "x" } } });
        let branch =
            json!({ "e": "if", "cond": { "e": "var", "name": "c" }, "then": waiting, "else": ok });
        let slots = Type::List(Box::new(slot()));
        let both = json!({ "e": "list", "items": [branch.clone()] });
        assert_eq!(
            check_json(both, &slots, vec![("c", Type::Bool)]),
            (Some(slots.clone()), vec![])
        );
        assert_eq!(
            check_json(json!({ "e": "list", "items": [] }), &slots, vec![]),
            (Some(slots.clone()), vec![])
        );

        // An annotated binding has its annotation, so its uses are typed by it, even under `infer`.
        let annotated = json!({ "e": "let", "name": "s", "type": slot(), "be": branch, "in": { "e": "list", "items": [{ "e": "var", "name": "s" }] } });
        assert_eq!(
            infer_json(annotated, vec![("c", Type::Bool)]),
            (Some(slots.clone()), vec![])
        );

        // So does an annotated fold's accumulator, from an empty start.
        let collect = json!({ "e": "fold", "type": slots, "over": { "e": "var", "name": "xs" },
            "init": { "e": "list", "items": [] }, "acc": "a", "item": "x",
            "body": { "e": "op", "op": "append", "args": [{ "e": "var", "name": "a" },
                { "e": "record", "fields": { "kind": { "e": "lit", "v": "ok" }, "value": { "e": "var", "name": "x" } } }] } });
        let (ty, problems) = infer_json(collect, vec![("xs", Type::List(Box::new(Type::String)))]);
        assert_eq!(problems, []);
        assert_eq!(ty, Some(Type::List(Box::new(slot()))));
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
