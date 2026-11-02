//! Builders for writing harnesses as data in tests, and a way to call the kernel as a host does.
#![allow(dead_code)]

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};

pub fn call(request: Value) -> Value {
    let response = decode(&kernel::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

pub fn lit(v: Value) -> Value {
    json!({ "e": "lit", "v": v })
}

pub fn var(name: &str) -> Value {
    json!({ "e": "var", "name": name })
}

pub fn get(of: Value, field: &str) -> Value {
    json!({ "e": "get", "of": of, "field": field })
}

/// `a.b.c` from a variable.
pub fn path(name: &str, fields: &[&str]) -> Value {
    fields.iter().fold(var(name), |of, f| get(of, f))
}

pub fn op(name: &str, args: Vec<Value>) -> Value {
    json!({ "e": "op", "op": name, "args": args })
}

pub fn eq(a: Value, b: Value) -> Value {
    op("eq", vec![a, b])
}

pub fn if_(cond: Value, then: Value, otherwise: Value) -> Value {
    json!({ "e": "if", "cond": cond, "then": then, "else": otherwise })
}

pub fn let_(name: &str, be: Value, body: Value) -> Value {
    json!({ "e": "let", "name": name, "be": be, "in": body })
}

pub fn rec(fields: Vec<(&str, Value)>) -> Value {
    let fields: serde_json::Map<String, Value> = fields
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    json!({ "e": "record", "fields": fields })
}

pub fn list(items: Vec<Value>) -> Value {
    json!({ "e": "list", "items": items })
}

pub fn fold(over: Value, init: Value, acc: &str, item: &str, body: Value) -> Value {
    json!({ "e": "fold", "over": over, "init": init, "acc": acc, "item": item, "body": body })
}

pub fn label(of: Value) -> Value {
    json!({ "e": "label", "of": of })
}

/// `input.kind == kind`.
pub fn is(kind: &str) -> Value {
    eq(path("input", &["kind"]), lit(json!(kind)))
}

/// A step's result.
pub fn step(state: Value, actions: Vec<Value>) -> Value {
    rec(vec![("state", state), ("actions", list(actions))])
}

pub fn call_action(op: &str, args: Value) -> Value {
    rec(vec![
        ("kind", lit(json!("call"))),
        ("op", lit(json!(op))),
        ("args", args),
    ])
}

/// An action made at run time, from data: bound at `Any`, so the checker leaves it to the interpreter, which refuses
/// it when it runs if it isn't one. A harness that performs whatever it is told needs nothing else until `Any` goes.
pub fn dynamic(action: Value) -> Value {
    json!({ "e": "let", "name": "@action", "type": "Any", "be": action, "in": var("@action") })
}

pub fn halt(value: Value) -> Value {
    rec(vec![("kind", lit(json!("halt"))), ("value", value)])
}

pub fn cancel(call: Value) -> Value {
    rec(vec![("kind", lit(json!("cancel"))), ("call", call)])
}

/// A dispatch on `input.kind`, with a default for every other kind.
pub fn on(cases: Vec<(&str, Value)>, otherwise: Value) -> Value {
    cases
        .into_iter()
        .rev()
        .fold(otherwise, |rest, (kind, then)| if_(is(kind), then, rest))
}

pub fn init(harness: Value, interface: Value) -> Value {
    let ready = call(json!({ "op": "init", "harness": harness, "interface": interface }));
    assert_eq!(ready["kind"], "ready", "{ready}");
    ready["system"].clone()
}

/// Deliver in order; the final system and every event.
pub fn deliver_all(mut system: Value, deliveries: Vec<Value>) -> (Value, Vec<Value>) {
    let mut events = Vec::new();
    for d in deliveries {
        let out = call(json!({ "op": "deliver", "system": system, "delivery": d }));
        system = out["system"].clone();
        events.extend(out["events"].as_array().unwrap().iter().cloned());
    }
    (system, events)
}

pub fn start(value: Value) -> Value {
    json!({ "input": { "kind": "start", "value": value } })
}

pub fn ret(call: u64, value: Value) -> Value {
    json!({ "input": { "kind": "ret", "call": call, "value": value } })
}

pub fn msg(channel: &str, value: Value) -> Value {
    json!({ "input": { "kind": "msg", "channel": channel, "value": value } })
}

/// The kinds of events, in order, with the action or input kind after a colon.
pub fn shape(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .map(|e| {
            let detail = match e["kind"].as_str().unwrap() {
                "in" => e["input"]["kind"].clone(),
                "out" => e["action"]["kind"].clone(),
                "withheld" => e["reason"]["kind"].clone(),
                _ => json!(""),
            };
            format!(
                "{}:{}",
                e["kind"].as_str().unwrap(),
                detail.as_str().unwrap_or("")
            )
        })
        .collect()
}

pub mod demo;

/// `event.a.b`.
pub fn ev(fields: &[&str]) -> Value {
    path("event", fields)
}

pub fn atom(holds: Value) -> Value {
    json!({ "f": "atom", "holds": holds })
}

pub fn not(of: Value) -> Value {
    json!({ "f": "not", "of": of })
}

pub fn and(of: Vec<Value>) -> Value {
    json!({ "f": "and", "of": of })
}

pub fn implies(condition: Value, then: Value) -> Value {
    json!({ "f": "implies", "if": condition, "then": then })
}

pub fn since(left: Value, right: Value) -> Value {
    json!({ "f": "since", "left": left, "right": right })
}

pub fn once(of: Value) -> Value {
    json!({ "f": "once", "of": of })
}

pub fn exists(var_name: &str, of: Value, body: Value) -> Value {
    json!({ "f": "exists", "var": var_name, "of": of, "body": body })
}

/// The conjunction of expressions, as one expression.
pub fn all(exprs: Vec<Value>) -> Value {
    exprs
        .into_iter()
        .reduce(|a, b| op("and", vec![a, b]))
        .unwrap_or(lit(json!(true)))
}

/// `match of on tag { case as bind: body, ... } otherwise`.
pub fn match_(
    of: Value,
    on: &str,
    cases: Vec<(&str, &str, Value)>,
    otherwise: Option<Value>,
) -> Value {
    let cases: serde_json::Map<String, Value> = cases
        .into_iter()
        .map(|(case, bind, body)| (case.to_string(), json!({ "bind": bind, "body": body })))
        .collect();
    let mut m = json!({ "e": "match", "of": of, "on": on, "cases": cases });
    if let Some(o) = otherwise {
        m["otherwise"] = o;
    }
    m
}
