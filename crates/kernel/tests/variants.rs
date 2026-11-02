//! Variant types: what a harness receives is typed by its interface, and `match` reads it precisely.

mod common;
use common::*;
use serde_json::{Value, json};

fn interface() -> Value {
    json!({
        "operations": {
            "count": { "input": { "Record": {} }, "output": "Int" },
            "fetch": { "input": { "Record": {} }, "output": "String" },
        },
        "channels": { "user": { "type": "String" } }
    })
}

/// Start: count. A count comes back: halt with one more. A fetch, a message or anything else: halt with -1.
fn counter(on_count: Value) -> Value {
    let ret = match_(
        var("r"),
        "op",
        vec![("count", "c", step(var("state"), vec![halt(on_count)]))],
        Some(step(var("state"), vec![halt(lit(json!(-1)))])),
    );
    json!({
        "arg": { "Record": {} },
        "state": { "Record": {} },
        "init": {},
        "step": match_(
            var("input"),
            "kind",
            vec![
                ("start", "s", step(var("state"), vec![call_action("count", rec(vec![]))])),
                ("ret", "r", ret),
            ],
            Some(step(var("state"), vec![halt(lit(json!(-1)))])),
        ),
    })
}

fn check(harness: Value) -> Value {
    call(json!({ "op": "checkHarness", "harness": harness, "interface": interface() }))["problems"]
        .clone()
}

#[test]
fn a_matched_result_has_its_operations_type() {
    let plus_one = op("add", vec![path("c", &["value"]), lit(json!(1))]);
    assert_eq!(check(counter(plus_one.clone())), json!([]));

    // The count is an Int, so concatenating it is caught before the harness ever runs.
    let wrong = op("concat", vec![path("c", &["value"]), lit(json!("!"))]);
    let problems = check(counter(wrong));
    assert_eq!(problems[0]["problem"]["kind"], "operator");
    assert_eq!(
        problems[0]["problem"]["at"],
        "/cases/ret/body/cases/count/body/fields/actions/items/0/fields/value"
    );

    let ready =
        call(json!({ "op": "init", "harness": counter(plus_one), "interface": interface() }));
    let (system, _) = deliver_all(
        ready["system"].clone(),
        vec![start(json!({})), ret(0, json!(41))],
    );
    assert_eq!(system["root"]["outcome"]["value"], json!(42));
}

#[test]
fn matches_must_cover_their_variant() {
    let partial = json!({
        "arg": { "Record": {} }, "state": { "Record": {} }, "init": {},
        "step": match_(var("input"), "kind", vec![("start", "s", step(var("state"), vec![]))], None),
    });
    let problems = check(partial);
    assert_eq!(problems[0]["problem"]["kind"], "notExhaustive");
    assert_eq!(
        problems[0]["problem"]["missing"],
        json!(["end", "failed", "msg", "refused", "ret"])
    );

    let unknown = json!({
        "arg": { "Record": {} }, "state": { "Record": {} }, "init": {},
        "step": match_(var("input"), "kind", vec![("stop", "s", step(var("state"), vec![]))], Some(step(var("state"), vec![]))),
    });
    assert_eq!(check(unknown)[0]["problem"]["kind"], "noSuchCase");
}

#[test]
fn the_input_type_follows_the_interface() {
    let ty = call(
        json!({ "op": "inputType", "harness": counter(lit(json!(0))), "interface": interface() }),
    )["type"]
        .clone();
    let cases = &ty["Variant"]["cases"];
    assert_eq!(
        cases["ret"]["Variant"]["cases"]["count"]["Record"]["value"],
        "Int"
    );
    assert_eq!(
        cases["ret"]["Variant"]["cases"]["spawn"]["Record"]["value"],
        "Json"
    );
    assert_eq!(
        cases["msg"]["Variant"]["cases"]["user"]["Record"]["value"],
        "String"
    );
    assert_eq!(cases["end"]["Variant"]["cases"], json!({}));
}

#[test]
fn variants_and_matches_export_to_smtlib_and_read_back() {
    let shape = json!({ "Variant": { "tag": "kind", "cases": {
        "circle": { "Record": { "r": "Int" } },
        "square": { "Record": { "side": "Int" } },
    }}});
    let area = match_(
        var("s"),
        "kind",
        vec![
            (
                "circle",
                "c",
                op(
                    "mul",
                    vec![
                        lit(json!(3)),
                        op("mul", vec![path("c", &["r"]), path("c", &["r"])]),
                    ],
                ),
            ),
            (
                "square",
                "q",
                op("mul", vec![path("q", &["side"]), path("q", &["side"])]),
            ),
        ],
        None,
    );
    let out = call(json!({ "op": "smtlib", "definitions": [
        { "name": "area", "params": [{ "name": "s", "type": shape }], "body": area },
        { "name": "kind", "params": [{ "name": "s", "type": shape }], "body": path("s", &["kind"]) },
    ]}));
    assert_eq!(out["kind"], "exported", "{out}");
    let text = out["text"].as_str().unwrap();
    assert!(text.contains("(declare-datatype |variant:"), "{text}");
    assert!(text.contains("(_ is |variant:"), "{text}");

    let name = text
        .split("(declare-datatype |")
        .nth(1)
        .unwrap()
        .split('|')
        .next()
        .unwrap()
        .to_string();
    let reply = format!("((s (|{name}#square| 4)))");
    let values = call(json!({ "op": "readValues", "reply": reply, "types": [shape] }));
    assert_eq!(values["values"], json!([{ "kind": "square", "side": 4 }]));

    let evaluated = call(
        json!({ "op": "evaluate", "expr": area, "vars": { "s": { "kind": "circle", "r": 2 } } }),
    );
    assert_eq!(evaluated["value"], json!(12));
    let none =
        call(json!({ "op": "evaluate", "expr": area, "vars": { "s": { "kind": "hexagon" } } }));
    assert_eq!(none["error"]["kind"], "noCase");
}

fn slot() -> Value {
    json!({ "Variant": { "tag": "kind", "cases": {
        "waiting": { "Record": {} },
        "ok": { "Record": { "value": "Int" } },
    }}})
}

/// A state with a variant in it, set from record literals: start, count, and keep the count in the slot.
fn slotted() -> Value {
    let state_type = json!({ "Record": { "slot": slot() } });
    let counted = json!({
        "e": "let", "name": "next", "type": state_type,
        "be": rec(vec![("slot", rec(vec![("kind", lit(json!("ok"))), ("value", path("c", &["value"]))]))]),
        "in": step(var("next"), vec![halt(path("c", &["value"]))]),
    });
    let ret = match_(
        var("r"),
        "op",
        vec![("count", "c", counted)],
        Some(step(var("state"), vec![])),
    );
    json!({
        "arg": { "Record": {} },
        "state": state_type,
        "init": { "slot": { "kind": "waiting" } },
        "step": match_(
            var("input"),
            "kind",
            vec![
                ("start", "s", step(rec(vec![("slot", rec(vec![("kind", lit(json!("waiting")))]))]), vec![call_action("count", rec(vec![]))])),
                ("ret", "r", ret),
            ],
            Some(step(var("state"), vec![])),
        ),
    })
}

#[test]
fn a_step_can_build_its_state_at_a_variant_type() {
    assert_eq!(check(slotted()), json!([]));
    let ready = call(json!({ "op": "init", "harness": slotted(), "interface": interface() }));
    let (system, events) = deliver_all(
        ready["system"].clone(),
        vec![start(json!({})), ret(0, json!(7))],
    );
    assert_eq!(system["root"]["outcome"]["value"], json!(7));
    let last = events.last().unwrap();
    assert_eq!(
        last["state"],
        json!({ "slot": { "kind": "ok", "value": 7 } })
    );

    // A field of the case at the wrong type is caught where it is.
    let mut wrong = slotted();
    wrong["step"]["cases"]["start"]["body"]["fields"]["state"]["fields"]["slot"]["fields"]["kind"] =
        lit(json!("ok"));
    let problems = check(wrong);
    assert_eq!(problems[0]["problem"]["kind"], "mismatch", "{problems}");

    // The state has a sort, so an invariant over it gets an obligation, with the case's constructor in it.
    let invariant = match_(
        path("state", &["slot"]),
        "kind",
        vec![
            (
                "ok",
                "o",
                op("ge", vec![path("o", &["value"]), lit(json!(0))]),
            ),
            ("waiting", "w", lit(json!(true))),
        ],
        None,
    );
    let out = call(
        json!({ "op": "obligation", "harness": slotted(), "interface": interface(), "invariant": invariant }),
    );
    assert_eq!(out["kind"], "obligation", "{out}");
    assert_eq!(out["initiation"], json!(true));
    assert!(
        out["text"].as_str().unwrap().contains("#ok"),
        "{}",
        out["text"]
    );
}
