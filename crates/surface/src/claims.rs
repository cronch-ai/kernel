//! What is claimed about a harness, written in source: properties, elaborated into the kernel's formulas, and
//! invariants, into expressions over the generated state.
//!
//! A property is `always(phi)`. `phi` is built from `and`, `or`, `not`, `implies`, `since`, `once`, `prev`, `historically`,
//! `exists` and `forall`, whose leaves are atoms: an event helper (`called`, `returned`, `failed`, `refused`, `effect`,
//! `msg`), a node helper (`ok`, `failed`, `refused`, `skipped`, `running`, `waiting`), or any boolean expression over
//! `event`. Each helper is an atom of its own, a `match` down to the case of the event it is about, and false at any
//! other.
//!
//! In a property or an invariant, a call node's name is its slot, `{kind, call, value, error}`, which the language
//! defines (decision 6 of the plan), and a carried value's name is its value in the current round. A property reads slots from the state each event carries, and only at the root
//! run's own steps: at any other event, an atom that reads a slot is false.

use crate::What;
use crate::elaborate::{Bound, Cx, Read, eq, get, if_, let_, lit, op, var};
use crate::lex::Span;
use crate::syntax::{Binary, Expr, ExprKind, Name, Unary};
use kernel::harness::{Harness, Interface};
use kernel::ir::{Case, Expr as K, Type};
use kernel::property::{Formula, Property, check_property};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// An invariant of the generated state, over the variable `state`.
#[derive(Serialize, Clone, Debug)]
pub struct Invariant {
    pub name: String,
    pub expr: K,
}

const TEMPORAL: [&str; 4] = ["always", "once", "prev", "historically"];
const EVENT_HELPERS: [&str; 6] = ["called", "returned", "failed", "refused", "effect", "msg"];
const NODE_HELPERS: [&str; 6] = ["ok", "failed", "refused", "skipped", "running", "waiting"];

fn atom(holds: K) -> Formula {
    Formula::Atom { holds }
}

fn ev(fields: &[&str]) -> K {
    fields.iter().fold(var("event"), |of, f| get(of, f))
}

/// `then`, read only once every condition holds: false otherwise.
fn guarded(conditions: Vec<K>, then: K) -> K {
    conditions
        .into_iter()
        .rev()
        .fold(then, |inner, c| if_(c, inner, lit(json!(false))))
}

/// `then`, where `of` is the case `case` of its variant over `tag`, bound as `bind`; false anywhere else.
fn when_case(of: K, tag: &str, case: &str, bind: &str, then: K) -> K {
    K::Match {
        of: Box::new(of),
        on: tag.to_string(),
        cases: BTreeMap::from([(
            case.to_string(),
            Case {
                bind: bind.to_string(),
                body: then,
            },
        )]),
        otherwise: Some(Box::new(lit(json!(false)))),
    }
}

/// `body`, at the root run's own steps, which carry its state, with the event as `@s`; false at any other event.
fn at_root_step(body: K) -> K {
    let cases = ["in", "out", "refused"]
        .into_iter()
        .map(|kind| {
            let case = Case {
                bind: "@s".to_string(),
                body: body.clone(),
            };
            (kind.to_string(), case)
        })
        .collect();
    let step = K::Match {
        of: Box::new(var("event")),
        on: "kind".to_string(),
        cases,
        otherwise: Some(Box::new(lit(json!(false)))),
    };
    let at_root = eq(op("len", vec![ev(&["origin"])]), lit(json!(0)));
    guarded(vec![at_root], step)
}

/// The root run's state, inside `at_root_step`.
fn root_state() -> K {
    get(var("@s"), "state")
}

/// `then`, at a performed call of any operation, its case bound as `@a`, and the event as `@e`.
fn at_any_call(then: K) -> K {
    when_case(
        var("event"),
        "kind",
        "out",
        "@e",
        when_case(get(var("@e"), "action"), "kind", "call", "@a", then),
    )
}

/// `then`, at an input of the kind `kind`, its case bound as `@i`.
fn at_input(kind: &str, then: K) -> K {
    when_case(
        var("event"),
        "kind",
        "in",
        "@e",
        when_case(get(var("@e"), "input"), "kind", kind, "@i", then),
    )
}

/// Whether `k` reads the variable `name` anywhere, shadowed or not: enough to know whether to bind it.
pub(crate) fn mentions(k: &K, name: &str) -> bool {
    match k {
        K::Lit { .. } => false,
        K::Var { name: n } => n == name,
        K::Op { args, .. } => args.iter().any(|a| mentions(a, name)),
        K::Let { be, body, .. } => mentions(be, name) || mentions(body, name),
        K::If {
            cond,
            then,
            otherwise,
        } => mentions(cond, name) || mentions(then, name) || mentions(otherwise, name),
        K::Record { fields } => fields.values().any(|f| mentions(f, name)),
        K::List { items } => items.iter().any(|i| mentions(i, name)),
        K::Get { of, .. } | K::Label { of } => mentions(of, name),
        K::Match {
            of,
            cases,
            otherwise,
            ..
        } => {
            mentions(of, name)
                || cases.values().any(|c| mentions(&c.body, name))
                || otherwise.as_ref().is_some_and(|o| mentions(o, name))
        }
        K::Fold {
            over, init, body, ..
        } => mentions(over, name) || mentions(init, name) || mentions(body, name),
    }
}

impl Cx<'_> {
    fn helper_problem(&mut self, span: Span, name: &str, expected: &str) {
        self.report(
            span,
            What::Helper {
                name: name.to_string(),
                expected: expected.to_string(),
            },
        );
    }

    /// An operation or channel named by a string or a path from a capability, if the interface has it.
    fn named(
        &mut self,
        e: &Expr,
        locals: &[String],
        interface: &Interface,
        channel: bool,
    ) -> Option<String> {
        let name = match &e.kind {
            ExprKind::Lit(Value::String(s)) => s.clone(),
            _ => self.operation(e, locals)?,
        };
        let known = if channel {
            interface.channels.contains_key(&name)
        } else {
            interface.operations.contains_key(&name)
                || name
                    .strip_prefix("spawn.")
                    .is_some_and(|child| self.names.get(child) == Some(&Bound::Harness))
        };
        if !known {
            let what = if channel {
                What::UnknownChannel { channel: name }
            } else {
                What::UnknownOperation { op: name }
            };
            self.report(e.span, what);
            return None;
        }
        Some(name)
    }

    /// An expression over `event`, the quantified variables in `locals`, and `binds`, lowered. Call nodes it reads are
    /// their slots, read from the event's state, so it is false at events that aren't the root run's steps.
    fn over_event(&mut self, e: &Expr, locals: &[String], binds: Vec<(&str, K)>) -> K {
        let mut scope: Vec<String> = locals.to_vec();
        scope.push("event".to_string());
        scope.extend(binds.iter().map(|(n, _)| n.to_string()));
        let mut reads = Vec::new();
        self.walk(e, &mut scope.clone(), &mut reads);
        let mut slots = BTreeSet::new();
        for (i, read) in reads {
            if self.calls.contains_key(&i) {
                if read == Read::Value {
                    slots.insert(i);
                }
            } else {
                let node = self.node_name(i).to_string();
                self.report(e.span, What::NotASlot { node });
            }
        }
        let mut body = self.lower(e, &mut scope);
        let param = self.file.param.text.clone();
        let carried = self.carries.iter().any(|(c, _)| mentions(&body, c));
        let reads_state =
            !slots.is_empty() || carried || mentions(&body, "@nodes") || mentions(&body, &param);
        for &n in slots.iter().rev() {
            body = let_(
                self.node_name(n),
                get(get(root_state(), "nodes"), self.node_name(n)),
                body,
            );
        }
        if reads_state {
            body = self.carried_from(&root_state(), body);
            body = let_(&param, get(root_state(), "arg"), body);
            body = let_("@nodes", get(root_state(), "nodes"), body);
        }
        for (name, value) in binds.into_iter().rev() {
            body = let_(name, value, body);
        }
        if reads_state {
            body = at_root_step(body);
        }
        body
    }

    /// The optional filter of an event helper, with `binder` bound to `value`: `true` if there is none.
    fn filter(
        &mut self,
        args: &[crate::syntax::Arg],
        locals: &[String],
        binder: &str,
        value: K,
    ) -> K {
        match args.get(1) {
            Some(f) => self.over_event(&f.value, locals, vec![(binder, value)]),
            None => lit(json!(true)),
        }
    }

    fn helper(
        &mut self,
        e: &Expr,
        name: &str,
        args: &[crate::syntax::Arg],
        locals: &mut Vec<String>,
        interface: &Interface,
    ) -> Option<Formula> {
        let arg = |i: usize| args.get(i).map(|a| &a.value);
        if TEMPORAL.contains(&name) {
            let (Some(of), 1) = (arg(0), args.len()) else {
                self.helper_problem(e.span, name, "one formula");
                return None;
            };
            let of = Box::new(self.formula(of, locals, interface, false)?);
            return Some(match name {
                "once" => Formula::Once { of },
                "prev" => Formula::Prev { of },
                "historically" => Formula::Historically { of },
                _ => {
                    self.report(e.span, What::NestedAlways);
                    return None;
                }
            });
        }
        // `failed(node)` and `refused(node)` are about a node's slot; `failed(op)` and `refused(op)` about events.
        let node = match arg(0).map(|a| &a.kind) {
            Some(ExprKind::Name(n)) if !locals.contains(n) => {
                self.calls.keys().copied().find(|&i| self.node_name(i) == n)
            }
            _ => None,
        };
        if NODE_HELPERS.contains(&name) && (node.is_some() || !EVENT_HELPERS.contains(&name)) {
            let (Some(n), 1) = (node, args.len()) else {
                self.helper_problem(e.span, name, "a call or receive node");
                return None;
            };
            let slot = get(get(root_state(), "nodes"), self.node_name(n));
            return Some(atom(at_root_step(eq(get(slot, "kind"), lit(json!(name))))));
        }
        let channel = name == "msg";
        if name == "effect" {
            if !args.is_empty() {
                self.helper_problem(e.span, name, "no arguments");
                return None;
            }
            return Some(atom(at_any_call(get(var("@e"), "effect"))));
        }
        let (Some(target), 1..=2) = (arg(0), args.len()) else {
            let what = if channel { "a channel" } else { "an operation" };
            self.helper_problem(
                e.span,
                name,
                &format!("{what}, then optionally a condition"),
            );
            return None;
        };
        let target = self.named(target, locals, interface, channel)?;
        let holds = match name {
            "called" => {
                let filter = self.filter(args, locals, "args", get(var("@c"), "args"));
                at_any_call(when_case(var("@a"), "op", &target, "@c", filter))
            }
            "msg" => {
                let filter = self.filter(args, locals, "value", get(var("@c"), "value"));
                at_input(
                    "msg",
                    when_case(var("@i"), "channel", &target, "@c", filter),
                )
            }
            "returned" => {
                let filter = self.filter(args, locals, "value", get(var("@c"), "value"));
                at_input("ret", when_case(var("@i"), "op", &target, "@c", filter))
            }
            "failed" => {
                let filter = self.filter(args, locals, "error", get(var("@i"), "error"));
                let of_it = eq(get(var("@i"), "op"), lit(json!(target)));
                at_input("failed", if_(of_it, filter, lit(json!(false))))
            }
            _ => {
                if args.len() > 1 {
                    self.helper_problem(e.span, name, "an operation, and no condition");
                    return None;
                }
                at_input("refused", eq(get(var("@i"), "op"), lit(json!(target))))
            }
        };
        Some(atom(holds))
    }

    fn formula(
        &mut self,
        e: &Expr,
        locals: &mut Vec<String>,
        interface: &Interface,
        top: bool,
    ) -> Option<Formula> {
        let both = |cx: &mut Self, a: &Expr, b: &Expr, locals: &mut Vec<String>| {
            let a = cx.formula(a, locals, interface, false);
            let b = cx.formula(b, locals, interface, false);
            Some((a?, b?))
        };
        match &e.kind {
            ExprKind::Binary(Binary::And, a, b) => {
                let (a, b) = both(self, a, b, locals)?;
                Some(Formula::And { of: vec![a, b] })
            }
            ExprKind::Binary(Binary::Or, a, b) => {
                let (a, b) = both(self, a, b, locals)?;
                Some(Formula::Or { of: vec![a, b] })
            }
            ExprKind::Binary(Binary::Implies, a, b) => {
                let (a, b) = both(self, a, b, locals)?;
                Some(Formula::Implies {
                    condition: Box::new(a),
                    then: Box::new(b),
                })
            }
            ExprKind::Binary(Binary::Since, a, b) => {
                let (a, b) = both(self, a, b, locals)?;
                Some(Formula::Since {
                    left: Box::new(a),
                    right: Box::new(b),
                })
            }
            ExprKind::Unary(Unary::Not, a) => Some(Formula::Not {
                of: Box::new(self.formula(a, locals, interface, false)?),
            }),
            ExprKind::Call(callee, args) => match &callee.kind {
                ExprKind::Name(f) if f == "always" && top && args.len() == 1 => {
                    self.formula(&args[0].value, locals, interface, false)
                }
                ExprKind::Name(f)
                    if !locals.contains(f)
                        && (TEMPORAL.contains(&f.as_str())
                            || EVENT_HELPERS.contains(&f.as_str())
                            || NODE_HELPERS.contains(&f.as_str())) =>
                {
                    self.helper(e, f, args, locals, interface)
                }
                _ => Some(atom(self.over_event(e, locals, vec![]))),
            },
            ExprKind::Quantified {
                exists,
                var: bound,
                over,
                body,
            } => {
                let of = self.over_event(over, locals, vec![]);
                locals.push(bound.text.clone());
                let body = self.formula(body, locals, interface, false);
                locals.pop();
                let (var, body) = (bound.text.clone(), Box::new(body?));
                Some(if *exists {
                    Formula::Exists { var, of, body }
                } else {
                    Formula::Forall { var, of, body }
                })
            }
            _ => Some(atom(self.over_event(e, locals, vec![]))),
        }
    }

    /// A property, checked by the kernel, or `None` after reporting why it isn't one.
    pub(crate) fn property(
        &mut self,
        name: &Name,
        body: &Expr,
        harness: &Harness,
        interface: &Interface,
    ) -> Option<Property> {
        let before = self.problems.len();
        let always = self.formula(body, &mut Vec::new(), interface, true)?;
        if self.problems.len() > before {
            return None;
        }
        let property = Property {
            name: name.text.clone(),
            always,
        };
        let problems = check_property(&property, harness, interface);
        for p in &problems {
            self.report(
                body.span,
                What::Type {
                    problem: Box::new(p.problem.clone()),
                },
            );
        }
        problems.is_empty().then_some(property)
    }

    /// An invariant over the state, whose slots and start value its names read.
    pub(crate) fn invariant(
        &mut self,
        name: &Name,
        body: &Expr,
        state: &Type,
    ) -> Option<Invariant> {
        let mut reads = Vec::new();
        self.walk(body, &mut Vec::new(), &mut reads);
        let mut slots = BTreeSet::new();
        for (i, read) in reads {
            if !self.calls.contains_key(&i) {
                let node = self.node_name(i).to_string();
                self.report(body.span, What::NotASlot { node });
            } else if read == Read::Value {
                slots.insert(i);
            }
        }
        let mut expr = self.lower(body, &mut Vec::new());
        for &n in slots.iter().rev() {
            expr = let_(
                self.node_name(n),
                get(get(var("state"), "nodes"), self.node_name(n)),
                expr,
            );
        }
        let param = self.file.param.text.clone();
        expr = self.carried_from(&var("state"), expr);
        expr = let_(&param, get(var("state"), "arg"), expr);
        expr = let_("@nodes", get(var("state"), "nodes"), expr);
        let t = self.type_of(&expr, body.span, &[("state".to_string(), state.clone())]);
        self.expect(t, &Type::Bool, body.span);
        Some(Invariant {
            name: name.text.clone(),
            expr,
        })
    }
}
