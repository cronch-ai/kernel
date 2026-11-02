//! `Json`: any JSON value. Every type conforms to it and it conforms to nothing else, so what it holds is read only
//! after `is` has tested it against a type.

mod common;
use common::*;
use serde_json::{Value, json};

fn infer(expr: Value, vars: Value) -> Value {
    call(json!({ "op": "infer", "expr": expr, "vars": vars }))
}

fn evaluate(expr: Value, vars: Value) -> Value {
    call(json!({ "op": "evaluate", "expr": expr, "vars": vars }))["value"].clone()
}

/// One more than `x`, if it is an integer; 0 otherwise.
fn successor() -> Value {
    is_type(
        var("x"),
        json!("Int"),
        "n",
        op("add", vec![var("n"), lit(json!(1))]),
        lit(json!(0)),
    )
}

#[test]
fn json_is_read_only_after_a_test() {
    // Anything goes in.
    let wrapped = json!({ "e": "let", "name": "j", "type": "Json", "be": rec(vec![("a", lit(json!(1)))]), "in": var("j") });
    assert_eq!(
        infer(wrapped, json!({})),
        json!({ "type": "Json", "problems": [] })
    );

    // Nothing comes out unchecked.
    let unchecked = infer(
        op("add", vec![var("x"), lit(json!(1))]),
        json!({ "x": "Json" }),
    );
    assert_eq!(unchecked["problems"][0]["kind"], "operator");
    let unread = infer(get(var("x"), "a"), json!({ "x": "Json" }));
    assert_eq!(unread["problems"][0]["kind"], "notARecord");

    // Tested, it has the type it was tested against.
    assert_eq!(
        infer(successor(), json!({ "x": "Json" })),
        json!({ "type": "Int", "problems": [] })
    );
}

#[test]
fn a_test_takes_the_branch_the_value_decides() {
    assert_eq!(evaluate(successor(), json!({ "x": 41 })), json!(42));
    assert_eq!(evaluate(successor(), json!({ "x": "41" })), json!(0));
    assert_eq!(evaluate(successor(), json!({ "x": 1.5 })), json!(0));

    // A record type is tested all the way down; extra fields are allowed, as everywhere.
    let call = json!({ "Record": { "op": "String", "args": { "Record": { "text": "String" } } } });
    let text = is_type(
        var("x"),
        call,
        "c",
        path("c", &["args", "text"]),
        lit(json!("")),
    );
    let good = json!({ "op": "notes.append", "args": { "text": "hi" }, "why": "asked" });
    assert_eq!(evaluate(text.clone(), json!({ "x": good })), json!("hi"));
    let bad = json!({ "op": "notes.append", "args": { "text": 7 } });
    assert_eq!(evaluate(text, json!({ "x": bad })), json!(""));
}

/// Calls `notes.append` with what a model chose, if it is a call to it at all, and otherwise halts saying so.
fn chooser() -> Value {
    let append = json!({ "Record": { "args": { "Record": { "text": "String" } } } });
    let chosen = is_type(
        path("s", &["value"]),
        append,
        "c",
        step(
            var("state"),
            vec![call_action("notes.append", path("c", &["args"]))],
        ),
        step(var("state"), vec![halt(lit(json!("not a call")))]),
    );
    json!({
        "arg": "Json", "state": { "Record": {} }, "init": {},
        "step": match_(
            var("input"), "kind",
            vec![
                ("start", "s", chosen),
                ("ret", "r", step(var("state"), vec![halt(lit(json!("appended")))])),
            ],
            Some(step(var("state"), vec![])),
        ),
    })
}

fn notes() -> Value {
    json!({ "operations": { "notes.append": { "input": { "Record": { "text": "String" } }, "output": "String", "effect": true } } })
}

#[test]
fn a_harness_checks_what_a_model_chose_against_what_it_may_call() {
    let problems =
        call(json!({ "op": "checkHarness", "harness": chooser(), "interface": notes() }));
    assert_eq!(problems["problems"], json!([]));

    let system = init(chooser(), notes());
    let (_, events) = deliver_all(
        system.clone(),
        vec![start(json!({ "args": { "text": "hi" } }))],
    );
    assert_eq!(events[1]["action"]["op"], "notes.append");
    assert_eq!(events[1]["performed"], true);

    let (after, events) = deliver_all(system, vec![start(json!({ "args": { "path": "/etc" } }))]);
    assert_eq!(
        events[1]["action"],
        json!({ "kind": "halt", "value": "not a call" })
    );
    assert_eq!(after["root"]["outcome"]["value"], "not a call");
}

#[test]
fn a_started_value_its_test_rejects_still_taints_the_fallback() {
    // A start labelled as the model's: the halt it causes depends on what the model said, so it carries its label.
    let system = init(chooser(), notes());
    let started = json!({ "input": { "kind": "start", "value": 5, "label": ["model"] } });
    let (after, _) = deliver_all(system, vec![started]);
    assert_eq!(after["root"]["outcome"]["value"], "not a call");
    assert_eq!(after["root"]["outcome"]["label"], json!(["model"]));
}

#[test]
fn json_has_no_smt_reading_yet() {
    let harness = json!({
        "arg": "Int", "state": { "Record": { "n": "Int" } }, "init": { "n": 0 },
        "step": match_(var("input"), "kind", vec![
            ("start", "s", step(rec(vec![("n", is_type(path("s", &["value"]), json!("Int"), "k", var("k"), lit(json!(0))))]), vec![])),
        ], Some(step(var("state"), vec![]))),
    });
    let out =
        call(json!({ "op": "obligation", "harness": harness, "invariant": lit(json!(true)) }));
    assert_eq!(out["kind"], "refused", "{out}");
    assert_eq!(out["error"]["refusal"]["reason"]["kind"], "json", "{out}");
}
