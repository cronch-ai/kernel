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
//! What no contract has typed for the root is text, as `str` writes it, and its labels a list of sources: the values
//! that cross between runs (a start value, a halt value, a spawn's harness, argument and narrowings), a withheld
//! delivery's value and why it was withheld, and a step error's details.
//!
//! Properties are checked against `event_type`, the variant over `kind` of what each event has, so an atom reads a
//! case's fields after `match`, and a misspelt field is a problem with the property, not a property that holds of every
//! run. The one thing the type can't say is that a child's events have no `state`, so reading it at one fails to
//! evaluate, and the atom is false: read it where `origin` is empty.

use crate::check::infer;
use crate::harness::{Harness, Interface, action_type, input_type};
use crate::ir::Type;
use crate::ops::text;
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
            let mut input = input.clone();
            if input["kind"] == "start" {
                as_text(&mut input, &["value"]);
            }
            fields.insert("kind".into(), json!("in"));
            fields.insert("origin".into(), json!(origin));
            fields.insert("input".into(), input);
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
            let mut labels = labels.clone();
            let crossing: &[&str] = match action["kind"].as_str() {
                Some("halt") => &["value"],
                Some("spawn") => &["harness", "arg", "narrow"],
                _ => &[],
            };
            as_text(&mut action, crossing);
            flatten(&mut labels, crossing);
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
            fields.insert("labels".into(), labels);
            if root(origin) {
                fields.insert("state".into(), state.clone());
            }
        }
        Event::Withheld { .. } => {
            let mut recorded = serde_json::to_value(event).unwrap_or(Value::Null);
            as_text(&mut recorded["input"], &["value", "error"]);
            as_text(&mut recorded["reason"], &["expected", "got"]);
            return recorded;
        }
        Event::Failed { .. } => {
            let mut recorded = serde_json::to_value(event).unwrap_or(Value::Null);
            as_text(
                &mut recorded["error"],
                &["error", "got", "action", "expected"],
            );
            return recorded;
        }
    }
    Value::Object(fields)
}

/// These fields of a record, where it has them, as text.
fn as_text(record: &mut Value, names: &[&str]) {
    if let Value::Object(fields) = record {
        for name in names {
            if let Some(v) = fields.get_mut(*name) {
                *v = Value::String(text(v));
            }
        }
    }
}

/// These fields of a value's labels, where it has them, as one list of every source under them: the labels of text.
fn flatten(labels: &mut Value, names: &[&str]) {
    fn sources(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(items) => items.iter().for_each(|i| sources(i, out)),
            Value::Object(fields) => fields.values().for_each(|f| sources(f, out)),
            _ => {}
        }
    }
    if let Value::Object(fields) = labels {
        for name in names {
            if let Some(v) = fields.get_mut(*name) {
                let mut out = Vec::new();
                sources(v, &mut out);
                out.sort();
                out.dedup();
                *v = json!(out);
            }
        }
    }
}

/// What a property may read of `event` in a run of this harness against this interface.
pub fn event_type(harness: &Harness, interface: &Interface) -> Type {
    exact(harness, interface).resolved(&action_type(&harness.result, interface))
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

/// The events as a variant over `kind`. What no contract has typed for the root is text.
fn exact(harness: &Harness, interface: &Interface) -> Type {
    let origin = || list(Type::Int);
    let sources = || list(Type::String);
    let mut started = harness.clone();
    started.arg = Type::String;
    let input = input_type(&started, interface);
    let calls = interface
        .operations
        .iter()
        .map(|(name, o)| {
            let mut fields = vec![("args", o.input.clone()), ("call", Type::Int)];
            if let Some(f) = &o.footprint {
                let (ty, _) = infer(f, vec![("args".to_string(), o.input.clone())]);
                fields.push(("footprint", ty.unwrap_or(Type::Json)));
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
        ("harness", Type::String),
        ("arg", Type::String),
        ("narrow", Type::String),
    ]);
    let cancel = record(vec![("call", Type::Int)]);
    let performed = variant(
        "kind",
        vec![
            ("call", calls.clone()),
            ("spawn", spawn.clone()),
            ("cancel", cancel.clone()),
            ("halt", record(vec![("value", Type::String)])),
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
                record(vec![("value", Type::String), ("label", sources())]),
            ),
            (
                "ret",
                record(vec![("call", Type::Int), ("value", Type::String)]),
            ),
            ("end", record(vec![("call", Type::Int)])),
            (
                "failed",
                record(vec![("call", Type::Int), ("error", Type::String)]),
            ),
            (
                "msg",
                record(vec![("channel", Type::String), ("value", Type::String)]),
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
                record(vec![("expected", Type::String), ("got", Type::String)]),
            ),
            ("postcondition", record(vec![("call", Type::Int)])),
            ("invariant", record(vec![("channel", Type::String)])),
            ("returnLabel", record(vec![("call", Type::Int)])),
        ],
    );
    let step_error = variant(
        "kind",
        vec![
            ("eval", record(vec![("error", Type::String)])),
            ("notAStep", record(vec![("got", Type::String)])),
            ("badAction", record(vec![("action", Type::String)])),
            ("runaway", record(vec![])),
            (
                "result",
                record(vec![("expected", Type::String), ("got", Type::String)]),
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
/// its cases' fields, and its tag. A field whose cases shape it differently isn't there.
fn readable(ty: &Type) -> Type {
    match ty {
        Type::Variant { tag, cases } => {
            let mut fields = BTreeMap::from([(tag.clone(), Type::String)]);
            let mut disputed = Vec::new();
            for case in cases.values() {
                if let Type::Record(more) = readable(case) {
                    for (name, t) in more {
                        let merged = match fields.get(&name) {
                            Some(existing) => union(existing, &t),
                            None => Some(t),
                        };
                        match merged {
                            Some(m) => {
                                fields.insert(name, m);
                            }
                            None => disputed.push(name),
                        }
                    }
                }
            }
            for name in disputed {
                fields.remove(&name);
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

/// The type of a field two cases both have: a record has the fields either has. Types that disagree have none.
fn union(a: &Type, b: &Type) -> Option<Type> {
    match (a, b) {
        (Type::Record(x), Type::Record(y)) => {
            let mut fields = x.clone();
            for (name, t) in y {
                let merged = match fields.remove(name) {
                    Some(existing) => union(&existing, t),
                    None => Some(t.clone()),
                };
                if let Some(m) = merged {
                    fields.insert(name.clone(), m);
                }
            }
            Some(Type::Record(fields))
        }
        (Type::List(x), Type::List(y)) => union(x, y).map(list),
        (Type::Int, Type::Float) | (Type::Float, Type::Int) => Some(Type::Float),
        (a, b) if a == b => Some(a.clone()),
        _ => None,
    }
}

/// The labels of a value of `ty`, shaped like it: each scalar replaced by its sources.
fn labels_of(ty: &Type) -> Type {
    match ty {
        Type::Json => Type::Json,
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
