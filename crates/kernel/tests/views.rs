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
    let ready = call(json!({ "op": "init", "harness": actor(), "interface": tools() }));
    let system = deliver_all(ready["system"].clone(), vec![start(json!({}))]).0;
    let refused = msg(
        "act",
        json!({ "kind": "call", "op": "fs.write", "args": { "path": "/etc/passwd" } }),
    );
    let child = msg(
        "act",
        json!({ "kind": "spawn", "name": "worker", "harness": written(&worker()), "narrow": "[]", "arg": { "kind": "call", "op": "fs.write", "args": { "path": "/workspace/a" } } }),
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
    let footprinted = acting_on(json!({
        "operations": {
            "fs.write": { "input": { "Record": { "path": "String" } }, "output": "Int", "effect": true,
                "footprint": rec(vec![("writes", path("args", &["path"]))]) },
        },
    }));
    let ty =
        call(json!({ "op": "eventType", "harness": actor(), "interface": footprinted }))["type"]
            .clone();
    assert_eq!(
        ty["Variant"]["cases"]["out"]["Record"]["action"]["Variant"]["cases"]["call"]["Variant"]["cases"]
            ["fs.write"]["Record"]["footprint"],
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

/// Fails on `failing`'s kind of input and otherwise does nothing: its step divides by zero there.
fn failing_on(kind: &str) -> Value {
    let idle = || step(var("state"), vec![]);
    let by_zero = let_("x", op("idiv", vec![lit(json!(1)), lit(json!(0))]), idle());
    json!({
        "arg": "Int", "result": "String", "state": { "Record": {} }, "init": {},
        "step": on(vec![(kind, by_zero)], idle()),
    })
}

/// A property that fails where `holds` does, accepted by the checker of this harness: the event it first fails at.
fn first_where(holds: Value, harness: &Value, interface: &Value, trace: &[Value]) -> Option<u64> {
    let property = json!({ "name": "never", "always": not(atom(holds)) });
    let problems = call(
        json!({ "op": "checkProperty", "property": property, "harness": harness, "interface": interface }),
    );
    assert_eq!(problems["problems"], json!([]), "{property}");
    call(json!({ "op": "monitor", "property": property, "trace": trace }))["at"].as_u64()
}

/// `then`, at a failed step's input of the kind `kind`, its case bound as `i`.
fn at_failed_input(kind: &str, then: Value) -> Value {
    at_event(
        "failed",
        when_case(path("e", &["input"]), "kind", kind, "i", then),
    )
}

#[test]
fn a_failed_step_s_start_value_is_text() {
    let harness = failing_on("start");
    let interface = json!({});
    let (_, trace) = deliver_all(
        init(harness.clone(), interface.clone()),
        vec![start(json!(1))],
    );
    assert_eq!(shape(&trace), ["failed:"]);

    // The trace keeps what it recorded; a property reads what `eventType` says.
    assert_eq!(trace[0]["input"]["value"], 1);
    assert_eq!(
        views(&trace)[0]["input"],
        json!({ "kind": "start", "value": "1" })
    );
    let ty = call(json!({ "op": "eventType", "harness": harness, "interface": interface }))["type"]
        .clone();
    assert_eq!(
        ty["Variant"]["cases"]["failed"]["Record"]["input"]["Variant"]["cases"]["start"]["Record"]
            ["value"],
        "String"
    );
    let reads = |value: Value| {
        let holds = at_failed_input("start", eq(path("i", &["value"]), lit(value)));
        first_where(holds, &harness, &interface, &trace)
    };
    assert_eq!(reads(json!("1")), Some(0));
    let extends = at_failed_input(
        "start",
        eq(
            op("concat", vec![path("i", &["value"]), lit(json!("!"))]),
            lit(json!("1!")),
        ),
    );
    assert_eq!(first_where(extends, &harness, &interface, &trace), Some(0));
}

#[test]
fn a_failed_child_s_start_value_is_text() {
    let child = failing_on("start");
    let spawned = rec(vec![
        ("kind", lit(json!("spawn"))),
        ("name", lit(json!("kid"))),
        ("harness", lit(written(&child))),
        ("narrow", lit(json!("[]"))),
        ("arg", path("input", &["value"])),
    ]);
    let idle = || step(var("state"), vec![]);
    let parent = json!({
        "arg": "Int", "result": "String", "state": { "Record": {} }, "init": {},
        "spawns": { "kid": { "arg": "Int", "result": "String" } },
        "step": on(vec![("start", step(var("state"), vec![spawned]))], idle()),
    });
    let interface = json!({});
    let (_, trace) = deliver_all(
        init(parent.clone(), interface.clone()),
        vec![start(json!(1))],
    );
    let failed = trace.iter().position(|e| e["kind"] == "failed").unwrap();
    assert_eq!(trace[failed]["origin"], json!([0]));
    let holds = at_failed_input("start", eq(path("i", &["value"]), lit(json!("1"))));
    assert_eq!(
        first_where(holds, &parent, &interface, &trace),
        Some(failed as u64)
    );
}

#[test]
fn a_failed_step_s_other_inputs_keep_their_exact_types() {
    // The same as at an `in` event: only a start value crosses between runs.
    let harness = failing_on("msg");
    let interface = json!({ "channels": { "count": { "type": "Int" } } });
    let (_, trace) = deliver_all(
        init(harness.clone(), interface.clone()),
        vec![start(json!(1)), msg("count", json!(5))],
    );
    assert_eq!(shape(&trace), ["in:start", "failed:"]);
    assert_eq!(views(&trace)[1]["input"]["value"], 5);
    let over_four = at_failed_input(
        "msg",
        when_case(
            var("i"),
            "channel",
            "count",
            "m",
            op("gt", vec![path("m", &["value"]), lit(json!(4))]),
        ),
    );
    assert_eq!(
        first_where(over_four, &harness, &interface, &trace),
        Some(1)
    );
}
