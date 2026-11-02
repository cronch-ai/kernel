//! What a property reads: each event as `event`, with every field it can have.
//!
//! A trace is kept as the interpreter records it, so what is hashed and audited never changes. A property reads a
//! view of each event instead, built from the recorded event alone, so a trace recorded by any kernel version has the
//! same view:
//!
//! * `in`, an input a run stepped on, with its `label` (empty if it carries none);
//! * `out`, an action that was performed, with its `effect`. A call or spawn carries its identifier as
//!   `action.call`, as a cancel already does, and a call to an operation with a footprint carries it as
//!   `action.footprint`;
//! * `refused`, a call, spawn or cancel that wasn't performed: it has no effect and no footprint;
//! * `withheld` and `failed`, as recorded.
//!
//! An event of a child run has no `state`: a child's state is its own, and `state` is always the root's.
//!
//! Properties are checked against `event_type`, a record of every field some event has, at the type every event that
//! has it gives it. A property may read a field the event at hand doesn't have (that atom is then false), but not one
//! no event has, and not at a type no event gives it.

use crate::check::infer;
use crate::harness::{Harness, Interface, input_type};
use crate::ir::Type;
use crate::run::Event;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// The event as a property reads it.
pub fn view(event: &Event) -> Value {
    let mut fields = Map::new();
    let root = |origin: &Vec<u64>| origin.is_empty();
    match event {
        Event::In {
            origin,
            input,
            label,
            state,
        } => {
            fields.insert("kind".into(), json!("in"));
            fields.insert("origin".into(), json!(origin));
            fields.insert("input".into(), input.clone());
            fields.insert("label".into(), json!(label));
            if root(origin) {
                fields.insert("state".into(), state.clone());
            }
        }
        Event::Out {
            origin,
            call,
            action,
            performed,
            effect,
            footprint,
            labels,
            state,
        } => {
            let mut action = action.clone();
            if let Value::Object(a) = &mut action {
                if let Some(k) = call {
                    a.insert("call".into(), json!(k));
                }
                if let Some(f) = footprint {
                    a.insert("footprint".into(), f.clone());
                }
            }
            let kind = if *performed { "out" } else { "refused" };
            fields.insert("kind".into(), json!(kind));
            fields.insert("origin".into(), json!(origin));
            fields.insert("action".into(), action);
            if *performed {
                fields.insert("effect".into(), json!(effect));
            }
            fields.insert("labels".into(), labels.clone());
            if root(origin) {
                fields.insert("state".into(), state.clone());
            }
        }
        recorded => return serde_json::to_value(recorded).unwrap_or(Value::Null),
    }
    Value::Object(fields)
}

/// What a property may read of `event` in a run of this harness against this interface.
pub fn event_type(harness: &Harness, interface: &Interface) -> Type {
    readable(&exact(harness, interface))
}

fn record(fields: Vec<(&str, Type)>) -> Type {
    Type::Record(
        fields
            .into_iter()
            .map(|(n, t)| (n.to_string(), t))
            .collect(),
    )
}

fn variant(tag: &str, cases: Vec<(&str, Type)>) -> Type {
    Type::Variant {
        tag: tag.to_string(),
        cases: cases.into_iter().map(|(n, t)| (n.to_string(), t)).collect(),
    }
}

fn list(t: Type) -> Type {
    Type::List(Box::new(t))
}

/// The events as a variant over `kind`. A child's start value and result are its own, so they are `Any` here.
fn exact(harness: &Harness, interface: &Interface) -> Type {
    let origin = || list(Type::Int);
    let sources = || list(Type::String);
    let mut started = harness.clone();
    started.arg = Type::Any;
    let input = input_type(&started, interface);
    let calls = interface
        .operations
        .iter()
        .map(|(name, o)| {
            let mut fields = vec![("args", o.input.clone()), ("call", Type::Int)];
            if let Some(f) = &o.footprint {
                let (ty, _) = infer(f, vec![("args".to_string(), o.input.clone())]);
                fields.push(("footprint", ty.unwrap_or(Type::Any)));
            }
            (name.clone(), record(fields))
        })
        .collect::<BTreeMap<_, _>>();
    let calls = Type::Variant {
        tag: "op".to_string(),
        cases: calls,
    };
    let spawn = record(vec![
        ("call", Type::Int),
        ("name", Type::String),
        ("harness", Type::Any),
        ("arg", Type::Any),
        ("narrow", Type::Any),
    ]);
    let cancel = record(vec![("call", Type::Int)]);
    let performed = variant(
        "kind",
        vec![
            ("call", calls.clone()),
            ("spawn", spawn.clone()),
            ("cancel", cancel.clone()),
            ("halt", record(vec![("value", Type::Any)])),
        ],
    );
    let labels = labels_of(&readable(&performed));
    let refused = variant(
        "kind",
        vec![("call", calls), ("spawn", spawn), ("cancel", cancel)],
    );
    let world = variant(
        "kind",
        vec![
            (
                "start",
                record(vec![("value", Type::Any), ("label", sources())]),
            ),
            (
                "ret",
                record(vec![("call", Type::Int), ("value", Type::Any)]),
            ),
            ("end", record(vec![("call", Type::Int)])),
            (
                "failed",
                record(vec![("call", Type::Int), ("error", Type::Any)]),
            ),
            (
                "msg",
                record(vec![("channel", Type::String), ("value", Type::Any)]),
            ),
        ],
    );
    let reason = variant(
        "kind",
        vec![
            ("noSuchRun", record(vec![])),
            ("ended", record(vec![])),
            ("notStarted", record(vec![])),
            ("alreadyStarted", record(vec![])),
            ("notPending", record(vec![("call", Type::Int)])),
            ("notAStream", record(vec![("call", Type::Int)])),
            ("noSuchChannel", record(vec![("channel", Type::String)])),
            (
                "mismatch",
                record(vec![("expected", Type::Any), ("got", Type::Any)]),
            ),
            ("postcondition", record(vec![("call", Type::Int)])),
            ("invariant", record(vec![("channel", Type::String)])),
            ("returnLabel", record(vec![("call", Type::Int)])),
        ],
    );
    let step_error = variant(
        "kind",
        vec![
            ("eval", record(vec![("error", Type::Any)])),
            ("notAStep", record(vec![("got", Type::Any)])),
            ("badAction", record(vec![("action", Type::Any)])),
            ("runaway", record(vec![])),
            (
                "result",
                record(vec![("expected", Type::Any), ("got", Type::Any)]),
            ),
        ],
    );
    variant(
        "kind",
        vec![
            (
                "in",
                record(vec![
                    ("origin", origin()),
                    ("input", input.clone()),
                    ("label", sources()),
                    ("state", harness.state.clone()),
                ]),
            ),
            (
                "out",
                record(vec![
                    ("origin", origin()),
                    ("action", performed),
                    ("effect", Type::Bool),
                    ("labels", labels.clone()),
                    ("state", harness.state.clone()),
                ]),
            ),
            (
                "refused",
                record(vec![
                    ("origin", origin()),
                    ("action", refused),
                    ("labels", labels),
                    ("state", harness.state.clone()),
                ]),
            ),
            (
                "withheld",
                record(vec![
                    ("origin", origin()),
                    ("input", world),
                    ("reason", reason),
                ]),
            ),
            (
                "failed",
                record(vec![
                    ("origin", origin()),
                    ("input", input),
                    ("error", step_error),
                ]),
            ),
        ],
    )
}

/// Every field some value of `ty` has, at the type every value that has it gives it: a variant becomes the record of
/// its cases' fields, and its tag.
pub fn readable(ty: &Type) -> Type {
    match ty {
        Type::Variant { tag, cases } => {
            let mut fields = BTreeMap::from([(tag.clone(), Type::String)]);
            for case in cases.values() {
                match readable(case) {
                    Type::Record(more) => {
                        for (name, t) in more {
                            let merged = match fields.remove(&name) {
                                Some(existing) => union(&existing, &t),
                                None => t,
                            };
                            fields.insert(name, merged);
                        }
                    }
                    _ => return Type::Any,
                }
            }
            Type::Record(fields)
        }
        Type::Record(fields) => Type::Record(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), readable(t)))
                .collect(),
        ),
        Type::List(t) => list(readable(t)),
        other => other.clone(),
    }
}

/// The type of a field two cases both have: a record has the fields either has.
fn union(a: &Type, b: &Type) -> Type {
    match (a, b) {
        (Type::Record(x), Type::Record(y)) => {
            let mut fields = x.clone();
            for (name, t) in y {
                let merged = match fields.remove(name) {
                    Some(existing) => union(&existing, t),
                    None => t.clone(),
                };
                fields.insert(name.clone(), merged);
            }
            Type::Record(fields)
        }
        (Type::List(x), Type::List(y)) => list(union(x, y)),
        (Type::Int, Type::Float) | (Type::Float, Type::Int) => Type::Float,
        (a, b) if a == b => a.clone(),
        _ => Type::Any,
    }
}

/// The labels of a value of `ty`, shaped like it: each scalar replaced by its sources.
fn labels_of(ty: &Type) -> Type {
    match ty {
        Type::Any => Type::Any,
        Type::List(t) => list(labels_of(t)),
        Type::Record(fields) => Type::Record(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), labels_of(t)))
                .collect(),
        ),
        Type::Variant { .. } => labels_of(&readable(ty)),
        _ => list(Type::String),
    }
}
