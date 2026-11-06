//! Result types and declared children: a run halts only with a value of its harness's result type, and a declared
//! child's result arrives on its own operation, at its signature's type.

mod common;
use common::*;
use serde_json::{Value, json};

/// Halts with one more than it was started with.
fn increment() -> Value {
    json!({
        "arg": "Int", "result": "Int", "state": { "Record": {} }, "init": {},
        "step": match_(
            var("input"),
            "kind",
            vec![("start", "s", step(var("state"), vec![halt(op("add", vec![path("s", &["value"]), lit(json!(1))]))]))],
            Some(step(var("state"), vec![])),
        ),
    })
}

/// Spawns `child` as its declared `inc`, and halts with what it gives, or -1 if it can't.
fn parent(child: Value) -> Value {
    let spawned = rec(vec![
        ("kind", lit(json!("spawn"))),
        ("name", lit(json!("inc"))),
        ("harness", lit(written(&child))),
        ("narrow", lit(json!("[]"))),
        ("arg", path("s", &["value"])),
    ]);
    let results = match_(
        var("r"),
        "op",
        vec![(
            "spawn.inc",
            "c",
            step(var("state"), vec![halt(path("c", &["value"]))]),
        )],
        Some(step(var("state"), vec![])),
    );
    json!({
        "arg": "Int", "result": "Int", "state": { "Record": {} }, "init": {},
        "spawns": { "inc": { "arg": "Int", "result": "Int" } },
        "step": match_(
            var("input"),
            "kind",
            vec![
                ("start", "s", step(var("state"), vec![spawned])),
                ("ret", "r", results),
                ("refused", "f", step(var("state"), vec![halt(lit(json!(-1)))])),
            ],
            Some(step(var("state"), vec![])),
        ),
    })
}

fn outcome(harness: Value, arg: Value) -> Value {
    let system = init(harness, json!({}));
    let (system, _) = deliver_all(system, vec![start(arg)]);
    system["root"]["outcome"].clone()
}

#[test]
fn a_declared_childs_result_arrives_at_its_type() {
    let p = parent(increment());
    let problems = call(json!({ "op": "checkHarness", "harness": p, "interface": {} }));
    assert_eq!(problems["problems"], json!([]));
    let ty = call(json!({ "op": "inputType", "harness": p, "interface": {} }))["type"].clone();
    assert_eq!(
        ty["Variant"]["cases"]["ret"]["Variant"]["cases"]["spawn.inc"]["Record"]["value"],
        "Int"
    );
    assert_eq!(
        outcome(p, json!(4)),
        json!({ "kind": "halted", "value": 5 })
    );

    // Reading the child's value as a string is caught before anything runs.
    let mut wrong = parent(increment());
    wrong["step"]["cases"]["ret"]["body"]["cases"]["spawn.inc"]["body"] = step(
        var("state"),
        vec![halt(op(
            "concat",
            vec![path("c", &["value"]), lit(json!("!"))],
        ))],
    );
    let problems = call(json!({ "op": "checkHarness", "harness": wrong, "interface": {} }));
    assert_eq!(problems["problems"][0]["problem"]["kind"], "operator");
}

#[test]
fn a_child_without_the_declared_signature_is_refused() {
    let mut chatty = increment();
    chatty["result"] = json!("String");
    assert_eq!(
        outcome(parent(chatty), json!(4)),
        json!({ "kind": "halted", "value": -1 })
    );

    // An argument outside the signature doesn't check: the child would accept it, but the parent didn't declare it,
    // and a spawn's argument has the signature's `arg`.
    let mut loose = increment();
    loose["arg"] = json!("Float");
    loose["step"]["cases"]["start"]["body"] = step(var("state"), vec![halt(lit(json!(5)))]);
    let mut p = parent(loose);
    p["arg"] = json!("Float");
    let refused = call(json!({ "op": "init", "harness": p, "interface": {} }));
    assert_eq!(refused["error"]["kind"], "harness", "{refused}");
}

#[test]
fn a_halt_outside_the_result_type_fails_the_run() {
    // A literal of the wrong type is caught before the run.
    let mut liar = increment();
    liar["step"]["cases"]["start"]["body"] = step(var("state"), vec![halt(lit(json!("five")))]);
    let refused = call(json!({ "op": "init", "harness": liar, "interface": {} }));
    assert_eq!(refused["error"]["kind"], "harness");

    // So is a value of another type.
    let mut echo = increment();
    echo["arg"] = json!("String");
    echo["step"]["cases"]["start"]["body"] = step(var("state"), vec![halt(path("s", &["value"]))]);
    let refused = call(json!({ "op": "init", "harness": echo, "interface": {} }));
    assert_eq!(refused["error"]["kind"], "harness");

    // A harness without a result type is malformed: every harness says what it halts with.
    let mut untyped = increment();
    untyped.as_object_mut().unwrap().remove("result");
    let response = kernel::cbor::decode(&kernel::api::call(&kernel::cbor::encode(
        &json!({ "op": "init", "harness": untyped, "interface": {} }),
    )))
    .unwrap();
    assert_eq!(response["error"]["kind"], "malformed", "{response}");
}

#[test]
fn a_declared_child_cant_shadow_an_operation() {
    let clash =
        json!({ "operations": { "spawn.inc": { "input": { "Record": {} }, "output": "Int" } } });
    let problems =
        call(json!({ "op": "checkHarness", "harness": parent(increment()), "interface": clash }));
    assert_eq!(
        problems["problems"][0],
        json!({ "kind": "spawnClash", "name": "inc" })
    );
}
