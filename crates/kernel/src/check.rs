//! Static checking of an Answer against its Ask: is it a valid construction?
//!
//! A construction is valid when it type-checks against the Ask's signature. Checking also lists its holes:
//! the computational ones (nested Asks, which the runtime can try to close) and the epistemic ones (assumptions, opaque
//! inputs and world facts, whose justification is outside what Cronch checks). Being valid says nothing about whether
//! the holes can be filled, or whether filling them satisfies the Ask: construction is not satisfaction.

use crate::ir::{Answer, Ask, AskSpec, Expr, Form, Type, World};
use crate::ops::{self, Op, OpTypeError};
use crate::types::{conforms, inhabits, of_value};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Where in a Procedure something is: a `/`-separated path of fields from the body's root (`""`).
pub type Path = String;

fn child(at: &str, segment: &str) -> Path {
    format!("{at}/{segment}")
}

/// Why an epistemic hole is not allowed where a derivation is required.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Underived {
    RawValue,
    Assumption,
    Opaque,
}

/// What makes an Ask or a construction ill-formed.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Problem {
    /// A closed Ask lacks a value for one of its parameters.
    MissingArgument {
        ask: String,
        param: String,
    },
    /// A closed Ask, or an Ask inside a Procedure, has an argument its spec doesn't declare.
    UnexpectedArgument {
        at: Path,
        ask: String,
        arg: String,
    },
    /// A closed Ask's argument doesn't inhabit its parameter's type.
    ArgumentMismatch {
        ask: String,
        param: String,
        expected: Type,
        got: Value,
    },
    /// A spec declares two parameters with one name.
    DuplicateParameter {
        at: Path,
        ask: String,
        param: String,
    },
    /// An Ask inside a Procedure lacks an argument expression for a parameter.
    MissingHoleArgument {
        at: Path,
        ask: String,
        param: String,
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
    NotAList {
        at: Path,
        got: Type,
    },
    NoSuchField {
        at: Path,
        field: String,
        of: Type,
    },
    /// An assumed value doesn't inhabit the type it is assumed at.
    AssumptionMismatch {
        at: Path,
        expected: Type,
        got: Value,
    },
    /// The Ask requires a derivation, and this part of the Answer is opaque.
    NotDerived {
        at: Path,
        reason: Underived,
    },
}

/// A computational hole: an Ask inside a Procedure.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Hole {
    pub at: Path,
    pub spec: AskSpec,
}

/// An epistemic hole: something the result rests on that Cronch has not checked.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Assumption {
    /// The Answer is a raw value: the responder's word, with no derivation (the degenerate Procedure `assume(v)`).
    RawValue { value: Value },
    /// The responder assumes a value inside its Procedure.
    Assumed {
        at: Path,
        #[serde(rename = "type")]
        ty: Type,
        value: Value,
        #[serde(skip_serializing_if = "String::is_empty")]
        note: String,
        #[serde(skip_serializing_if = "String::is_empty")]
        source: String,
    },
    /// An input with no value at all, like `sorry`: the Procedure can't be evaluated past it.
    Opaque {
        at: Path,
        #[serde(rename = "type")]
        ty: Type,
        #[serde(skip_serializing_if = "String::is_empty")]
        note: String,
    },
    /// An environmental input from the requester's declared world.
    Fact {
        name: String,
        value: Value,
        #[serde(skip_serializing_if = "String::is_empty")]
        source: String,
    },
}

/// The outcome of static checking.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Construction {
    pub problems: Vec<Problem>,
    pub holes: Vec<Hole>,
    pub assumptions: Vec<Assumption>,
}

/// Problems with a closed Ask itself: arguments that are missing, unexpected or of the wrong type.
pub fn check_ask(ask: &Ask) -> Vec<Problem> {
    let mut problems = check_spec(&ask.spec);
    let name = &ask.spec.name;
    for p in &ask.spec.params {
        match ask.args.get(&p.name) {
            None => problems.push(Problem::MissingArgument {
                ask: name.clone(),
                param: p.name.clone(),
            }),
            Some(v) if !inhabits(v, &p.ty) => problems.push(Problem::ArgumentMismatch {
                ask: name.clone(),
                param: p.name.clone(),
                expected: p.ty.clone(),
                got: v.clone(),
            }),
            Some(_) => {}
        }
    }
    for arg in ask.args.keys() {
        if !ask.spec.params.iter().any(|p| &p.name == arg) {
            problems.push(Problem::UnexpectedArgument {
                at: String::new(),
                ask: name.clone(),
                arg: arg.clone(),
            });
        }
    }
    problems
}

/// Problems with a spec alone: parameters declared twice.
pub fn check_spec(spec: &AskSpec) -> Vec<Problem> {
    duplicate_params("", spec)
}

fn duplicate_params(at: &str, spec: &AskSpec) -> Vec<Problem> {
    let mut seen = Vec::new();
    let mut problems = Vec::new();
    for p in &spec.params {
        if seen.contains(&&p.name) {
            problems.push(Problem::DuplicateParameter {
                at: at.to_string(),
                ask: spec.name.clone(),
                param: p.name.clone(),
            });
        }
        seen.push(&p.name);
    }
    problems
}

/// Check an Answer against the Ask it answers, in the requester's declared world.
pub fn check(ask: &Ask, world: &World, answer: &Answer) -> Construction {
    let derivation = ask.spec.form == Form::Derivation;
    match answer {
        Answer::Value(value) => {
            let mut c = Construction::default();
            if derivation {
                c.problems.push(Problem::NotDerived {
                    at: String::new(),
                    reason: Underived::RawValue,
                });
            }
            if !inhabits(value, &ask.spec.output) {
                c.problems.push(Problem::Mismatch {
                    at: String::new(),
                    expected: ask.spec.output.clone(),
                    got: of_value(value),
                });
            }
            c.assumptions.push(Assumption::RawValue {
                value: value.clone(),
            });
            c
        }
        Answer::Procedure(body) => {
            let scope = ask
                .spec
                .params
                .iter()
                .map(|p| (p.name.clone(), p.ty.clone()))
                .collect();
            let mut checker = Checker {
                world,
                derivation,
                scope,
                out: Construction::default(),
            };
            if let Some(got) = checker.infer(body, "")
                && !conforms(&got, &ask.spec.output)
            {
                checker.out.problems.push(Problem::Mismatch {
                    at: String::new(),
                    expected: ask.spec.output.clone(),
                    got,
                });
            }
            checker.out
        }
    }
}

struct Checker<'w> {
    world: &'w World,
    derivation: bool,
    /// Variables in scope, innermost last: the Ask's parameters, then enclosing `let`s.
    scope: Vec<(String, Type)>,
    out: Construction,
}

impl Checker<'_> {
    fn problem(&mut self, p: Problem) -> Option<Type> {
        self.out.problems.push(p);
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
            Expr::Fact { name } => match self.world.facts.get(name) {
                Some(fact) => {
                    if !self
                        .out
                        .assumptions
                        .iter()
                        .any(|a| matches!(a, Assumption::Fact { name: n, .. } if n == name))
                    {
                        self.out.assumptions.push(Assumption::Fact {
                            name: name.clone(),
                            value: fact.value.clone(),
                            source: fact.source.clone(),
                        });
                    }
                    Some(fact.ty.clone().unwrap_or_else(|| of_value(&fact.value)))
                }
                None => self.problem(Problem::UnknownFact {
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
            Expr::Ask { spec, args } => {
                self.out.problems.extend(duplicate_params(at, spec));
                for p in &spec.params {
                    let arg_at = child(at, &format!("args/{}", p.name));
                    match args.get(&p.name) {
                        None => {
                            self.problem(Problem::MissingHoleArgument {
                                at: at.to_string(),
                                ask: spec.name.clone(),
                                param: p.name.clone(),
                            });
                        }
                        Some(arg) => {
                            if let Some(got) = self.infer(arg, &arg_at)
                                && !conforms(&got, &p.ty)
                            {
                                self.problem(Problem::Mismatch {
                                    at: arg_at,
                                    expected: p.ty.clone(),
                                    got,
                                });
                            }
                        }
                    }
                }
                for arg in args.keys() {
                    if !spec.params.iter().any(|p| &p.name == arg) {
                        self.problem(Problem::UnexpectedArgument {
                            at: at.to_string(),
                            ask: spec.name.clone(),
                            arg: arg.clone(),
                        });
                    }
                }
                self.out.holes.push(Hole {
                    at: at.to_string(),
                    spec: spec.clone(),
                });
                Some(spec.output.clone())
            }
            Expr::Assume {
                v,
                ty,
                note,
                source,
            } => {
                if self.derivation {
                    self.problem(Problem::NotDerived {
                        at: at.to_string(),
                        reason: Underived::Assumption,
                    });
                }
                if !inhabits(v, ty) {
                    self.problem(Problem::AssumptionMismatch {
                        at: at.to_string(),
                        expected: ty.clone(),
                        got: v.clone(),
                    });
                }
                self.out.assumptions.push(Assumption::Assumed {
                    at: at.to_string(),
                    ty: ty.clone(),
                    value: v.clone(),
                    note: note.clone(),
                    source: source.clone(),
                });
                Some(ty.clone())
            }
            Expr::Opaque { ty, note } => {
                if self.derivation {
                    self.problem(Problem::NotDerived {
                        at: at.to_string(),
                        reason: Underived::Opaque,
                    });
                }
                self.out.assumptions.push(Assumption::Opaque {
                    at: at.to_string(),
                    ty: ty.clone(),
                    note: note.clone(),
                });
                Some(ty.clone())
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
                    (Some(t), Some(e)) if conforms(&e, &t) => Some(t),
                    (Some(t), Some(e)) if conforms(&t, &e) => Some(e),
                    (Some(t), Some(e)) => self.problem(Problem::Mismatch {
                        at: child(at, "else"),
                        expected: t,
                        got: e,
                    }),
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
                let first = types.first().cloned().unwrap_or(Type::Any);
                for (i, t) in types.iter().enumerate().skip(1) {
                    if !conforms(t, &first) {
                        return self.problem(Problem::Mismatch {
                            at: child(at, &format!("items/{i}")),
                            expected: first,
                            got: t.clone(),
                        });
                    }
                }
                Some(Type::List(Box::new(first)))
            }
            Expr::Get { of, field } => match self.infer(of, &child(at, "of"))? {
                Type::Any => Some(Type::Any),
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
