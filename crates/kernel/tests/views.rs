//! What a property reads: each recorded event as a view with every field it can have, and its type.

mod common;
use common::demo::{actor, interface, leaky, tools, worker};
use common::*;
use serde_json::{Value, json};

fn views(events: &[Value]) -> Vec<Value> {
    let out = call(json!({ "op": "view", "events": events }));
    out["events"].as_array().unwrap().clone()
}

#[test]
fn a_call_carries_its_identifier_and_a_refusal_is_its_own_kind() {
    let system = init(leaky(), interface());
    let (system, events) = deliver_all(system, vec![start(json!({}))]);
    let (_, more) = deliver_all(system, vec![ret(0, json!("page"))]);
    let all: Vec<Value> = events.into_iter().chain(more).collect();
    let seen = views(&all);

    // The fetch: performed, its identifier inside the action, and no effect.
    assert_eq!(seen[1]["kind"], "out");
    assert_eq!(seen[1]["action"]["call"], 0);
    assert_eq!(seen[1]["effect"], false);
    assert!(seen[1].get("call").is_none() && seen[1].get("performed").is_none());

    // The input that started it carries an empty label rather than none.
    assert_eq!(seen[0]["label"], json!([]));

    // Recording is unchanged: the trace keeps what it always kept.
    assert_eq!(all[1]["call"], 0);
    assert_eq!(all[1]["performed"], true);
}

#[test]
fn a_refused_call_has_no_effect_and_a_child_has_no_state() {
    let ready = call(
        json!({ "op": "init", "harness": actor(), "interface": tools(), "grant": { "allow": [], "once": [] } }),
    );
    let system = deliver_all(ready["system"].clone(), vec![start(json!({}))]).0;
    let refused = msg(
        "act",
        json!({ "kind": "call", "op": "fs.write", "args": { "path": "/etc/passwd" } }),
    );
    let child = msg(
        "act",
        json!({ "kind": "spawn", "harness": worker(), "narrow": [], "arg": { "op": "fs.write", "args": { "path": "/workspace/a" } } }),
    );
    let (_, events) = deliver_all(system, vec![refused, child]);
    let seen = views(&events);
    let kinds: Vec<(&str, &Value)> = seen
        .iter()
        .map(|e| (e["kind"].as_str().unwrap(), &e["origin"]))
        .collect();
    assert_eq!(
        kinds,
        [
            ("in", &json!([])),
            ("refused", &json!([])),
            ("in", &json!([])),
            ("in", &json!([])),
            ("out", &json!([])),
            ("in", &json!([1])),
            ("out", &json!([1])),
        ]
    );
    assert_eq!(seen[1]["action"]["call"], 0);
    assert!(seen[1].get("effect").is_none());
    assert_eq!(seen[4]["action"]["kind"], "spawn");
    assert_eq!(seen[4]["action"]["call"], 1);

    // The child's write: an effect, under its own origin, without the root's state or its own.
    assert_eq!(seen[6]["effect"], true);
    assert!(seen[6].get("state").is_none());
    assert!(seen[4].get("state").is_some());
}

#[test]
fn an_operations_footprint_is_on_its_call() {
    let footprinted = json!({
        "operations": {
            "fs.write": { "input": { "Record": { "path": "String" } }, "output": "Int", "effect": true,
                "footprint": rec(vec![("writes", path("args", &["path"]))]) },
        },
        "channels": { "act": { "type": { "Record": { "kind": "String" } } } },
    });
    let ty =
        call(json!({ "op": "eventType", "harness": actor(), "interface": footprinted }))["type"]
            .clone();
    assert_eq!(
        ty["Record"]["action"]["Record"]["footprint"],
        json!({ "Record": { "writes": "String" } })
    );
    let system = init(actor(), footprinted);
    let (_, events) = deliver_all(
        system,
        vec![
            start(json!({})),
            msg(
                "act",
                json!({ "kind": "call", "op": "fs.write", "args": { "path": "/a" } }),
            ),
        ],
    );
    let seen = views(&events);
    let out = seen.iter().find(|e| e["kind"] == "out").unwrap();
    assert_eq!(out["action"]["footprint"], json!({ "writes": "/a" }));
}
