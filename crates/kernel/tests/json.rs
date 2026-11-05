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
        "result": "Json", "arg": "Json", "state": { "Record": {} }, "init": {},
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

/// Keeps the last start value, whatever it is.
fn keeper() -> Value {
    json!({
        "result": "Json", "arg": "Json", "state": { "Record": { "last": "Json" } }, "init": { "last": null },
        "step": match_(var("input"), "kind", vec![
            ("start", "s", step(rec(vec![("last", path("s", &["value"]))]), vec![])),
        ], Some(step(var("state"), vec![]))),
    })
}

/// The last value, if a whole number, isn't negative.
fn not_negative() -> Value {
    is_type(
        path("state", &["last"]),
        json!("Int"),
        "n",
        op("ge", vec![var("n"), lit(json!(0))]),
        lit(json!(true)),
    )
}

#[test]
fn json_in_a_state_has_an_smt_reading() {
    let out = call(json!({ "op": "obligation", "harness": keeper(), "invariant": not_negative() }));
    assert_eq!(out["kind"], "obligation", "{out}");
    let text = out["text"].as_str().unwrap();
    assert!(text.contains("(declare-datatypes ((|json:Json| 0) (|json:Fields| 0))"));
    // What the query quantifies over is a value the kernel could hold.
    assert!(text.contains("(assert (|json:ok| (|record:"), "{text}");

    // A model is read back as JSON, records in key order and whole numbers as integers.
    let reply = r#"((|q:state| (|record:x| (|json:record| (|json:field| "a" (|json:number| (fp #b0 #b10000000000 #x0000000000000)) (|json:field| "b" |json:null| |json:end|))))) (|q:input| x))"#;
    let state = json!({ "Record": { "last": "Json" } });
    let read = call(json!({ "op": "readValues", "reply": reply, "types": [state, "Json"] }));
    assert_eq!(read["kind"], "unreadable", "{read}");
    let reply = r#"((a (|json:record| (|json:field| "a" (|json:number| (fp #b0 #b10000000000 #x0000000000000)) (|json:field| "b" (|json:list| (seq.unit |json:null|)) |json:end|)))))"#;
    let read = call(json!({ "op": "readValues", "reply": reply, "types": ["Json"] }));
    assert_eq!(read["values"], json!([{ "a": 2, "b": [null] }]), "{read}");
    let twice = r#"((a (|json:record| (|json:field| "a" |json:null| (|json:field| "a" |json:null| |json:end|)))))"#;
    let read = call(json!({ "op": "readValues", "reply": twice, "types": ["Json"] }));
    assert_eq!(read["kind"], "unreadable", "{read}");
}

/// Performs what each message names, if it is an action this run may take.
fn performer() -> Value {
    let counted = rec(vec![(
        "acted",
        op("add", vec![path("state", &["acted"]), lit(json!(1))]),
    )]);
    let chosen = is_type(
        path("input", &["value"]),
        json!("Action"),
        "a",
        step(counted, vec![var("a")]),
        step(var("state"), vec![]),
    );
    json!({
        "result": "Json", "arg": "Json", "state": { "Record": { "acted": "Int" } }, "init": { "acted": 0 },
        "step": on(vec![("msg", chosen)], step(var("state"), vec![])),
    })
}

fn tools() -> Value {
    json!({
        "operations": {
            "notes.append": { "input": { "Record": { "text": "String" } }, "output": "String", "effect": true },
            "notes.read": { "input": { "Record": {} }, "output": "String" },
        },
        "channels": { "pick": { "type": "Json" } },
    })
}

#[test]
fn action_is_what_the_interface_it_runs_against_allows() {
    // One term, checked against each interface it runs against.
    let problems =
        call(json!({ "op": "checkHarness", "harness": performer(), "interface": tools() }));
    assert_eq!(problems["problems"], json!([]));
    let append = json!({ "kind": "call", "op": "notes.append", "args": { "text": "hi" } });
    let (_, events) = deliver_all(
        init(performer(), tools()),
        vec![start(json!({})), msg("pick", append.clone())],
    );
    assert_eq!(events.last().unwrap()["action"], append);

    // Narrowed, an append is no longer an action it may take, so the same term doesn't take it.
    let read_only = call(json!({ "op": "narrow", "interface": tools(), "narrowings": [{ "kind": "drop", "op": "notes.append" }] }))["interface"].clone();
    let (_, events) = deliver_all(
        init(performer(), read_only.clone()),
        vec![start(json!({})), msg("pick", append)],
    );
    assert!(events.iter().all(|e| e["kind"] == "in"), "{events:?}");

    // What isn't an action at all, or calls with the wrong arguments, isn't one either.
    let wrong = json!({ "kind": "call", "op": "notes.append", "args": { "text": 3 } });
    let (_, events) = deliver_all(
        init(performer(), tools()),
        vec![
            start(json!({})),
            msg("pick", wrong),
            msg("pick", json!("hello")),
        ],
    );
    assert!(events.iter().all(|e| e["kind"] == "in"), "{events:?}");

    // `actionType` says what `Action` is against an interface.
    let ty = call(json!({ "op": "actionType", "interface": read_only }))["type"].clone();
    assert_eq!(
        ty["Variant"]["cases"]["call"]["Variant"]["cases"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["notes.read"]
    );

    // And an obligation reads it resolved.
    let out = call(
        json!({ "op": "obligation", "harness": performer(), "interface": tools(),
        "invariant": op("ge", vec![path("state", &["acted"]), lit(json!(0))]) }),
    );
    assert_eq!(out["kind"], "obligation", "{out}");
}

/// One definition as SMT-LIB text, which must export.
fn exported(params: Value, body: Value, result: Option<Value>) -> String {
    let mut definition = json!({ "name": "f", "params": params, "body": body });
    if let Some(r) = result {
        definition["result"] = r;
    }
    let out = call(json!({ "op": "smtlib", "definitions": [definition] }));
    assert_eq!(out["kind"], "exported", "{out}");
    out["text"].as_str().unwrap().to_string()
}

#[test]
fn lists_are_read_as_json_item_by_item() {
    let ints = json!({ "List": "Int" });
    let declared = |text: &str, kind: &str| {
        text.matches(&format!("(define-funs-rec ((|map:{kind}."))
            .count()
    };
    // A list of Int as Json, through a map declared once however often it is used.
    let as_json =
        |xs: Value| json!({ "e": "let", "name": "j", "type": "Json", "be": xs, "in": var("j") });
    let text = exported(
        json!([{ "name": "xs", "type": ints }]),
        rec(vec![("a", as_json(var("xs"))), ("b", as_json(var("xs")))]),
        None,
    );
    assert_eq!(declared(&text, "json"), 1, "{text}");
    assert!(text.contains("(|json:list| (|map:json."), "{text}");

    // Json tested against, then read at, a list of Int.
    let text = exported(
        json!([{ "name": "j", "type": "Json" }]),
        is_type(var("j"), ints.clone(), "xs", var("xs"), list(vec![])),
        Some(ints.clone()),
    );
    assert_eq!(declared(&text, "is"), 1, "{text}");
    assert_eq!(declared(&text, "read"), 1, "{text}");

    // A list of Int where a list of Float is wanted.
    let text = exported(
        json!([{ "name": "xs", "type": ints.clone() }]),
        var("xs"),
        Some(json!({ "List": "Float" })),
    );
    assert_eq!(declared(&text, "coerce"), 1, "{text}");

    // A list of lists: the inner map is declared before the outer one, which calls it.
    let text = exported(
        json!([{ "name": "xss", "type": { "List": ints } }]),
        as_json(var("xss")),
        None,
    );
    assert_eq!(declared(&text, "json"), 2, "{text}");
    let first = text.find("(define-funs-rec ((|map:json.").unwrap();
    let outer = text.rfind("(define-funs-rec ((|map:json.").unwrap();
    let inner_name = &text[first + "(define-funs-rec ((".len()..]
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    assert!(text[outer..].contains(inner_name.as_str()), "{text}");
}

#[test]
fn the_text_of_a_value_is_unconstrained_but_per_type() {
    let text = exported(
        json!([
            { "name": "r", "type": { "Record": { "a": "Int" } } },
            { "name": "j", "type": "Json" },
        ]),
        rec(vec![
            ("r", op("str", vec![var("r")])),
            ("again", op("str", vec![var("r")])),
            ("j", op("str", vec![var("j")])),
            ("n", op("str", vec![lit(json!(3))])),
        ]),
        None,
    );
    // One function for the record type and one for Json; an Int's text is exact.
    assert_eq!(text.matches("(declare-fun |fn:text.").count(), 2, "{text}");
    assert!(text.contains("str.from_int"), "{text}");
}
