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
        ("harness", lit(child)),
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

    // An argument outside the signature is refused too: the child would accept it, the parent didn't declare it.
    let mut loose = increment();
    loose["arg"] = json!("Any");
    let mut p = parent(loose);
    p["arg"] = json!("Any");
    assert_eq!(
        outcome(p, json!("four")),
        json!({ "kind": "halted", "value": -1 })
    );
}

#[test]
fn a_halt_outside_the_result_type_fails_the_run() {
    let mut liar = increment();
    liar["step"]["cases"]["start"]["body"] = step(var("state"), vec![halt(lit(json!("five")))]);
    let ended = outcome(liar, json!(4));
    assert_eq!(ended["kind"], "failed");
    assert_eq!(ended["error"]["kind"], "result");

    // Without a result type, any value will do, as before.
    let mut untyped = increment();
    untyped.as_object_mut().unwrap().remove("result");
    assert_eq!(outcome(untyped, json!(4))["value"], json!(5));
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
