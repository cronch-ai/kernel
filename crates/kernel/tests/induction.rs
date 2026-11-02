//! Inductive invariants: certificates the kernel checks without searching, and obligations over every state whose
//! counterexamples the kernel checks.

mod common;
use common::demo::*;
use common::*;
use serde_json::{Value, json};

fn fetch_alphabet() -> Value {
    json!([
        { "input": { "kind": "start", "value": {} } },
        { "input": { "kind": "ret", "value": "page", "op": "net.fetch" } },
        { "input": { "kind": "ret", "value": 200, "op": "net.send" } },
    ])
}

fn subject(harness: Value) -> Value {
    json!({ "harness": harness, "interface": interface(), "property": no_untrusted_send() })
}

fn with(mut request: Value, extra: Value) -> Value {
    for (k, v) in extra.as_object().unwrap() {
        request[k] = v.clone();
    }
    request
}

/// No run makes more than `n` calls: `config.system.root.next <= n`.
fn at_most_calls(n: i64) -> Value {
    op(
        "le",
        vec![path("config", &["system", "root", "next"]), lit(json!(n))],
    )
}

#[test]
fn a_certificate_is_checked_without_searching() {
    let found = call(with(
        subject(careful()),
        json!({ "op": "certificate", "alphabet": fetch_alphabet() }),
    ));
    assert_eq!(found["kind"], "certificate", "{found}");
    let configurations = found["configurations"].as_array().unwrap().clone();

    let method = |invariant: Value, configurations: &[Value]| json!({ "kind": "invariant", "invariant": invariant, "configurations": configurations, "alphabet": fetch_alphabet() });
    let proved = call(with(
        subject(careful()),
        json!({ "op": "prove", "method": method(at_most_calls(2), &configurations) }),
    ));
    assert_eq!(proved["evidence"]["verdict"], "holds", "{proved}");
    let verified = call(with(
        subject(careful()),
        json!({ "op": "verify", "evidence": proved["evidence"] }),
    ));
    assert_eq!(verified["valid"], json!(true));

    // An invariant that doesn't hold of every listed configuration.
    let out = call(with(
        subject(careful()),
        json!({ "op": "prove", "method": method(at_most_calls(1), &configurations) }),
    ));
    assert_eq!(out["error"]["kind"], "notInvariant");

    // A certificate missing the configurations a step reaches.
    let out = call(with(
        subject(careful()),
        json!({ "op": "prove", "method": method(at_most_calls(2), &configurations[..1]) }),
    ));
    assert_eq!(out["error"]["kind"], "notClosed");

    // A certificate missing the initial configuration.
    let out = call(with(
        subject(careful()),
        json!({ "op": "prove", "method": method(lit(json!(true)), &configurations[1..]) }),
    ));
    assert_eq!(out["error"]["kind"], "initiation");

    // A harness that leaks has no certificate: the search finds the counterexample instead.
    let out = call(with(
        subject(leaky()),
        json!({ "op": "certificate", "alphabet": fetch_alphabet() }),
    ));
    assert_eq!(out["error"]["kind"], "refuted");
}

fn counter_interface() -> Value {
    json!({ "operations": { "count": { "input": { "Record": {} }, "output": "Int" } } })
}

/// Counts results: on start, calls `count`; on each count, adds `update(n, value)` to its state.
fn counter(update: Value) -> Value {
    let on_ret = match_(
        var("r"),
        "op",
        vec![("count", "c", step(rec(vec![("n", update)]), vec![]))],
        Some(step(var("state"), vec![])),
    );
    json!({
        "arg": { "Record": {} },
        "state": { "Record": { "n": "Int" } },
        "init": { "n": 0 },
        "step": match_(
            var("input"),
            "kind",
            vec![
                ("start", "s", step(var("state"), vec![call_action("count", rec(vec![]))])),
                ("ret", "r", on_ret),
            ],
            Some(step(var("state"), vec![])),
        ),
    })
}

fn non_negative() -> Value {
    op("ge", vec![path("state", &["n"]), lit(json!(0))])
}

#[test]
fn consecution_is_a_query_whose_counterexamples_the_kernel_checks() {
    let adds_value = counter(op(
        "add",
        vec![path("state", &["n"]), path("c", &["value"])],
    ));
    let out = call(
        json!({ "op": "obligation", "harness": adds_value, "interface": counter_interface(), "invariant": non_negative() }),
    );
    assert_eq!(out["kind"], "obligation", "{out}");
    assert_eq!(out["initiation"], json!(true));
    assert_eq!(
        out["excluded"],
        json!(["failed.child", "failed.world", "ret.spawn"])
    );
    assert_eq!(out["reads"][0], json!({ "Record": { "n": "Int" } }));
    let text = out["text"].as_str().unwrap();
    assert!(text.contains("(declare-const |q:state| |record:"), "{text}");
    assert!(
        text.contains("(declare-const |q:input| |variant:"),
        "{text}"
    );
    assert!(text.contains("(check-sat)"), "{text}");

    // What a solver would find: a count of -1 takes n from 0 below zero.
    let check = |harness: &Value, state: Value, input: Value| {
        call(
            json!({ "op": "checkInduction", "harness": harness, "interface": counter_interface(),
                     "invariant": non_negative(), "state": state, "input": input }),
        )
    };
    let negative = json!({ "kind": "ret", "op": "count", "call": 0, "value": -1 });
    assert_eq!(
        check(&adds_value, json!({ "n": 0 }), negative.clone()),
        json!({ "kind": "notInductive", "next": { "n": -1 } })
    );

    // Counting each result as one keeps it non-negative.
    let adds_one = counter(op("add", vec![path("state", &["n"]), lit(json!(1))]));
    assert_eq!(
        check(&adds_one, json!({ "n": 0 }), negative.clone()),
        json!({ "kind": "preserved", "next": { "n": 1 } })
    );

    // A candidate the kernel won't accept: the invariant doesn't hold to begin with, or the input isn't one.
    assert_eq!(
        check(&adds_value, json!({ "n": -5 }), negative)["error"]["kind"],
        "notHeld"
    );
    let not_an_input = json!({ "kind": "ret", "op": "launch", "call": 0, "value": 1 });
    assert_eq!(
        check(&adds_value, json!({ "n": 0 }), not_an_input)["error"]["kind"],
        "input"
    );
}

#[test]
fn projection_keeps_only_the_next_state() {
    // `if` and `match` both survive projection; the actions, which may fail to evaluate, are dropped.
    let failing_actions = json!({
        "arg": { "Record": {} },
        "state": { "Record": { "n": "Int" } },
        "init": { "n": 0 },
        "step": rec(vec![
            ("state", rec(vec![("n", lit(json!(1)))])),
            ("actions", list(vec![op("idiv", vec![lit(json!(1)), lit(json!(0))])])),
        ]),
    });
    let out = call(
        json!({ "op": "obligation", "harness": failing_actions, "interface": {}, "invariant": non_negative() }),
    );
    assert_eq!(out["kind"], "obligation", "{out}");
    assert!(!out["text"].as_str().unwrap().contains("div"));
}
