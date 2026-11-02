//! `Call`: a call of one of the operations of the interface a harness runs against. A model whose answer holds one is
//! typed by whatever that interface allows, so a step performs its choice with no test, and a narrowed child's model
//! can't choose what the child may not do.

mod common;
use common::*;
use serde_json::{Value, json};

fn capabilities() -> Value {
    let answer = json!({ "Variant": { "tag": "kind", "cases": {
        "answer": { "Record": { "text": "String" } },
        "call": "Call",
    }}});
    json!([
        { "prefix": "brain", "capability": { "operations": {
            "choose": { "input": { "Record": {} }, "output": answer },
        }}},
        { "prefix": "t", "capability": { "operations": {
            "ping": { "input": { "Record": {} }, "output": "Int" },
            "echo": { "input": { "Record": { "text": "String" } }, "output": "String" },
        }}},
    ])
}

fn interface() -> Value {
    call(json!({ "op": "install", "capabilities": capabilities() }))["interface"].clone()
}

/// Asks the model, performs the call it chooses, and halts with its answer, or once the call returns.
fn chooser() -> Value {
    let choice = match_(
        path("input", &["value"]),
        "kind",
        vec![
            ("call", "c", acting(var("state"), list(vec![var("c")]))),
            (
                "answer",
                "a",
                step(var("state"), vec![halt(path("a", &["text"]))]),
            ),
        ],
        None,
    );
    let results = on_op(
        vec![("brain.choose", choice)],
        step(var("state"), vec![halt(lit(json!("done")))]),
    );
    json!({
        "arg": "String", "result": "String", "state": { "Record": {} }, "init": {},
        "step": on(
            vec![
                ("start", step(var("state"), vec![call_action("brain.choose", rec(vec![]))])),
                ("ret", results),
            ],
            step(var("state"), vec![]),
        ),
    })
}

fn problems(harness: &Value, interface: &Value) -> Vec<Value> {
    call(json!({ "op": "checkHarness", "harness": harness, "interface": interface }))["problems"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn a_chosen_call_is_performed_without_a_test() {
    let iface = interface();
    assert_eq!(problems(&chooser(), &iface), Vec::<Value>::new());
    let system = init(chooser(), iface);
    let (system, events) = deliver_all(
        system,
        vec![
            start(json!("go")),
            ret(
                0,
                json!({ "kind": "call", "op": "t.echo", "args": { "text": "hi" } }),
            ),
            ret(1, json!("hi")),
        ],
    );
    let performed: Vec<&Value> = events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["kind"] == "call")
        .map(|e| &e["action"]["op"])
        .collect();
    assert_eq!(performed, [&json!("brain.choose"), &json!("t.echo")]);
    assert_eq!(system["root"]["outcome"]["value"], json!("done"));
}

#[test]
fn a_call_is_typed_by_the_interface_it_is_made_in() {
    // Narrowed, the model's answer can only name what is left.
    let narrowed = call(json!({
        "op": "narrow", "interface": interface(), "narrowings": [{ "kind": "drop", "op": "t.echo" }],
    }))["interface"]
        .clone();
    assert_eq!(problems(&chooser(), &narrowed), Vec::<Value>::new());
    let system = init(chooser(), narrowed);
    let echo = json!({ "kind": "call", "op": "t.echo", "args": { "text": "hi" } });
    let out =
        call(json!({ "op": "deliver", "system": system.clone(), "delivery": start(json!("go")) }));
    let out = call(json!({ "op": "deliver", "system": out["system"], "delivery": ret(0, echo) }));
    assert_eq!(out["events"][0]["kind"], "withheld", "{out}");
    assert_eq!(out["events"][0]["reason"]["kind"], "mismatch", "{out}");
    let ping = json!({ "kind": "call", "op": "t.ping", "args": {} });
    let out = call(json!({ "op": "deliver", "system": out["system"], "delivery": ret(0, ping) }));
    assert_eq!(out["events"][0]["kind"], "in", "{out}");

    // A call with the wrong arguments is no call at all.
    let system = init(chooser(), interface());
    let out = call(json!({ "op": "deliver", "system": system, "delivery": start(json!("go")) }));
    let wrong = json!({ "kind": "call", "op": "t.echo", "args": { "text": 1 } });
    let out = call(json!({ "op": "deliver", "system": out["system"], "delivery": ret(0, wrong) }));
    assert_eq!(out["events"][0]["reason"]["kind"], "mismatch", "{out}");
}

#[test]
fn call_is_refused_where_the_interface_defines_its_own_types() {
    let check = |capability: Value| -> Vec<Value> {
        let iface = call(
            json!({ "op": "install", "capabilities": [{ "prefix": "x", "capability": capability }] }),
        );
        call(json!({ "op": "checkInterface", "interface": iface["interface"] }))["problems"]
            .as_array()
            .unwrap()
            .clone()
    };
    let input = check(json!({ "operations": {
        "approve": { "input": { "Record": { "call": "Call" } }, "output": "Bool" },
    }}));
    assert_eq!(
        input,
        [
            json!({ "kind": "selfReference", "place": { "in": "operation", "name": "x.approve", "part": "input" } })
        ]
    );
    let grant = check(json!({
        "operations": { "a": { "input": { "Record": {} }, "output": "Bool" } },
        "grant": { "Record": { "once": { "List": "Call" } } },
    }));
    assert_eq!(
        grant,
        [json!({ "kind": "selfReference", "place": { "in": "grant" } })]
    );
    // An output may hold one: it is resolved against the interface the call is made in.
    assert!(check(capabilities()[0]["capability"].clone()).is_empty());
}

#[test]
fn a_contract_reads_the_name_its_operation_is_installed_under() {
    let notes = json!({
        "operations": { "append": {
            "input": { "Record": { "text": "String" } }, "output": "String", "effect": true,
            "pre": op("contains", vec![path("grant", &["allow"]), var("op")]),
        }},
        "grant": { "Record": { "allow": { "List": "String" } } },
    });
    let iface =
        call(json!({ "op": "install", "capabilities": [{ "prefix": "n", "capability": notes }] }))
            ["interface"]
            .clone();
    let writer = json!({
        "arg": "String", "result": "String", "state": { "Record": {} }, "init": {},
        "step": on(
            vec![("start", step(var("state"), vec![call_action("n.append", rec(vec![("text", lit(json!("x")))]))]))],
            step(var("state"), vec![halt(lit(json!("done")))]),
        ),
    });
    let performed = |grant: Value| {
        let ready =
            call(json!({ "op": "init", "harness": writer, "interface": iface, "grant": grant }));
        let out = call(
            json!({ "op": "deliver", "system": ready["system"], "delivery": start(json!("go")) }),
        );
        out["events"].as_array().unwrap().iter().any(|e| {
            e["kind"] == "out" && e["action"]["op"] == "n.append" && e["performed"] != json!(false)
        })
    };
    assert!(performed(json!({ "allow": ["n.append"] })));
    assert!(!performed(json!({ "allow": ["notes.append"] })));
}
