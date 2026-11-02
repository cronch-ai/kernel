//! Elaboration, through the same interface the WebAssembly build exposes: harnesses written in source run, and prove,
//! as their hand-written terms do, and what is wrong with a source is reported where it is.
//!
//! The fixtures are the platform's: its `model`, `approve` and `notes` contracts, the hand-written `approved-note`
//! harness, and the `no-effect-without-approval` property.

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};

fn call(request: Value) -> Value {
    let response = decode(&surface::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn platform() -> Value {
    json!({ "model": fixture("model"), "approve": fixture("approve"), "notes": fixture("notes") })
}

fn platform_interface() -> Value {
    let capabilities: Vec<Value> = ["model", "approve", "notes"]
        .iter()
        .map(|c| json!({ "prefix": c, "capability": fixture(c) }))
        .collect();
    call(json!({ "op": "install", "capabilities": capabilities }))["interface"].clone()
}

const APPROVED_NOTE: &str = r#"
harness approved_note(prompt: String) -> Any

uses model, approve, notes

draft    = model.generate(messages: [{role: "user", content: prompt}])
decision = approve.ask(action: "notes.append", args: {text: draft})
saved    = notes.append(text: draft) when decision != "deny"

return match saved {
  ok(note) => note
  skipped  => "denied"
}

otherwise return "stopped"
"#;

fn elaborate(source: &str, capabilities: Value) -> Value {
    call(json!({ "op": "elaborate", "source": source, "capabilities": capabilities }))
}

fn elaborated(source: &str, capabilities: Value) -> Value {
    let out = elaborate(source, capabilities);
    assert_eq!(out["kind"], "elaborated", "{out:#}");
    out
}

fn problems(source: &str, capabilities: Value) -> Vec<Value> {
    let out = elaborate(source, capabilities);
    assert_eq!(out["kind"], "problems", "{out:#}");
    out["problems"].as_array().unwrap().clone()
}

/// The run's outcome and every call it performed, in order, after these inputs.
fn run(harness: &Value, interface: &Value, inputs: &[Value]) -> (Value, Vec<Value>) {
    let ready = call(json!({
        "op": "init", "harness": harness, "interface": interface, "grant": { "allow": [], "once": [] },
    }));
    assert_eq!(ready["kind"], "ready", "{ready}");
    let mut system = ready["system"].clone();
    let mut calls = Vec::new();
    for input in inputs {
        let out =
            call(json!({ "op": "deliver", "system": system, "delivery": { "input": input } }));
        system = out["system"].clone();
        for e in out["events"].as_array().unwrap() {
            if e["kind"] == "out" && e["action"]["kind"] == "call" {
                calls.push(json!([
                    e["action"]["op"],
                    e["action"]["args"],
                    e["performed"]
                ]));
            }
        }
    }
    (system["root"]["outcome"].clone(), calls)
}

fn start(value: Value) -> Value {
    json!({ "kind": "start", "value": value })
}

fn ret(call: u64, value: Value) -> Value {
    json!({ "kind": "ret", "call": call, "value": value })
}

fn failed(call: u64) -> Value {
    json!({ "kind": "failed", "call": call, "error": "unavailable" })
}

#[test]
fn approved_note_runs_as_the_hand_written_harness_does() {
    let out = elaborated(APPROVED_NOTE, platform());
    assert_eq!(
        out["uses"],
        json!([
            { "name": "model", "capability": "model" },
            { "name": "approve", "capability": "approve" },
            { "name": "notes", "capability": "notes" },
        ])
    );
    let interface = platform_interface();
    let scenarios = [
        vec![
            start(json!("a haiku")),
            ret(0, json!("draft")),
            ret(1, json!("once")),
            ret(2, json!("id")),
        ],
        vec![
            start(json!("a haiku")),
            ret(0, json!("draft")),
            ret(1, json!("always")),
            failed(2),
        ],
        vec![
            start(json!("a haiku")),
            ret(0, json!("draft")),
            ret(1, json!("deny")),
        ],
        vec![start(json!("a haiku")), failed(0)],
        vec![start(json!("a haiku")), ret(0, json!("draft")), failed(1)],
    ];
    for inputs in scenarios {
        let (mut source, calls) = run(&out["harness"], &interface, &inputs);
        let (hand_written, expected) = run(&fixture("approved-note"), &interface, &inputs);
        assert_eq!(calls, expected, "after {inputs:?}");
        // The whole input of a step is labelled with its result's sources, so the slot kinds a step writes after the
        // model's result carry `model`, and so does anything decided on them later. The hand-written harness keeps no
        // such state. Labels only ever over-approximate, so this is lost precision, not lost safety.
        let mut hand_written = hand_written;
        let label = |outcome: &mut Value| outcome.as_object_mut().and_then(|o| o.remove("label"));
        let (source_label, hand_written_label) = (label(&mut source), label(&mut hand_written));
        assert_eq!(source_label, Some(json!(["model"])), "after {inputs:?}");
        assert!(
            hand_written_label.is_none() || hand_written_label == source_label,
            "after {inputs:?}"
        );
        assert_eq!(source, hand_written, "after {inputs:?}");
    }
    let (outcome, calls) = run(
        &out["harness"],
        &interface,
        &[
            start(json!("a haiku")),
            ret(0, json!("draft")),
            ret(1, json!("once")),
            ret(2, json!("id")),
        ],
    );
    assert_eq!(
        outcome,
        json!({ "kind": "halted", "value": "id", "label": ["model"] })
    );
    assert_eq!(calls[2], json!(["notes.append", { "text": "draft" }, true]));
}

#[test]
fn approved_note_proves_as_the_hand_written_harness_does() {
    let mut alphabet = vec![
        json!({ "input": { "kind": "start", "value": "a haiku" } }),
        json!({ "input": { "kind": "ret", "op": "model.generate", "value": "draft" } }),
    ];
    for answer in ["once", "always", "deny"] {
        alphabet.push(json!({ "input": { "kind": "ret", "op": "approve.ask", "value": answer } }));
    }
    alphabet.push(json!({ "input": { "kind": "ret", "op": "notes.append", "value": "id" } }));
    alphabet.push(
        json!({ "input": { "kind": "failed", "op": "notes.append", "error": "unavailable" } }),
    );
    let prove = |harness: Value, grant: Value| {
        let out = call(json!({
            "op": "prove", "harness": harness, "interface": platform_interface(), "grant": grant,
            "property": fixture("no-effect-without-approval"),
            "method": { "kind": "exhaustive", "alphabet": alphabet },
        }));
        assert_eq!(out["kind"], "evidence", "{out}");
        out["evidence"]["verdict"].clone()
    };
    let harness = elaborated(APPROVED_NOTE, platform())["harness"].clone();
    assert_eq!(
        prove(harness.clone(), json!({ "allow": [], "once": [] })),
        "holds"
    );
    assert_eq!(
        prove(fixture("approved-note"), json!({ "allow": [], "once": [] })),
        "holds"
    );

    // Without the guard, only the grant stands between the harness and the effect.
    let reckless = APPROVED_NOTE.replace(r#" when decision != "deny""#, "");
    let reckless = elaborated(&reckless, platform())["harness"].clone();
    let allowed = json!({ "allow": ["notes.append"], "once": [] });
    assert_eq!(prove(reckless, allowed), "refuted");
}

fn math() -> Value {
    let int = json!({ "input": { "Record": { "n": "Int" } }, "output": "Int" });
    json!({ "math": { "operations": { "double": int, "square": int } } })
}

fn math_interface() -> Value {
    let capabilities = json!([{ "prefix": "m", "capability": math()["math"] }]);
    call(json!({ "op": "install", "capabilities": capabilities }))["interface"].clone()
}

#[test]
fn independent_calls_go_out_together_and_results_arrive_in_any_order() {
    let source = r#"
harness both(n: Int) -> Int
uses m = "math"
total = a + b       # a pure node, read by `return`
a = m.double(n: n)
b = m.square(n: n + 1)
return total
otherwise return -1
"#;
    let out = elaborated(source, math());
    assert_eq!(out["uses"], json!([{ "name": "m", "capability": "math" }]));
    let interface = math_interface();
    let (outcome, calls) = run(&out["harness"], &interface, &[start(json!(3))]);
    assert_eq!(outcome, Value::Null);
    assert_eq!(
        calls,
        [
            json!(["m.double", { "n": 3 }, true]),
            json!(["m.square", { "n": 4 }, true])
        ]
    );
    let (outcome, _) = run(
        &out["harness"],
        &interface,
        &[start(json!(3)), ret(1, json!(16)), ret(0, json!(6))],
    );
    assert_eq!(outcome, json!({ "kind": "halted", "value": 22 }));
}

#[test]
fn a_failure_ends_the_run_unless_a_match_handles_it() {
    let interface = math_interface();
    // `{"failed": node}` is what a run halts with when nothing says otherwise, so the result type must admit it.
    let unhandled = r#"
harness h(n: Int) -> Any
uses m = "math"
a = m.double(n: n)
return a
"#;
    let out = elaborated(unhandled, math());
    let (outcome, _) = run(&out["harness"], &interface, &[start(json!(1)), failed(0)]);
    assert_eq!(
        outcome,
        json!({ "kind": "halted", "value": { "failed": "a" } })
    );

    let handled = r#"
harness h(n: Int) -> Int
uses m = "math"
a = m.double(n: n)
b = m.square(n: n) when n > 0
return match a {
  ok(v) => v + (match b { ok(w) => w, _ => 0 })
  failed(e) => -1
  refused => -2
}
"#;
    let out = elaborated(handled, math());
    let (outcome, calls) = run(
        &out["harness"],
        &interface,
        &[start(json!(2)), failed(0), ret(1, json!(4))],
    );
    assert_eq!(calls.len(), 2);
    assert_eq!(outcome, json!({ "kind": "halted", "value": -1 }));
    let (outcome, calls) = run(
        &out["harness"],
        &interface,
        &[start(json!(-2)), ret(0, json!(-4))],
    );
    assert_eq!(calls.len(), 1, "b is skipped");
    assert_eq!(outcome, json!({ "kind": "halted", "value": -4 }));

    // The interpreter refuses a call whose precondition fails, and the harness handles the refusal.
    let refused = r#"
harness h(n: Int) -> Int
uses m = "math"
a = m.double(n: n)
return match a { ok(v) => v, failed(e) => -1, refused => -2 }
"#;
    let out = elaborated(refused, math());
    let mut closed = math_interface();
    closed["operations"]["m.double"]["pre"] = json!({ "e": "lit", "v": false });
    let (outcome, _) = run(&out["harness"], &closed, &[start(json!(1))]);
    assert_eq!(outcome, json!({ "kind": "halted", "value": -2 }));
}

#[test]
fn guards_and_dependencies_skip_what_cannot_run() {
    let source = r#"
harness h(n: Int) -> Int
uses m = "math"
type Unused = a | b {x: Int}
a = m.double(n: n) when n > 10
b = m.square(n: a)
return match b { ok(v) => v, skipped => 0 }
otherwise return -1
"#;
    let out = elaborated(source, math());
    let (outcome, calls) = run(&out["harness"], &math_interface(), &[start(json!(1))]);
    assert_eq!(calls, Vec::<Value>::new());
    assert_eq!(outcome, json!({ "kind": "halted", "value": 0 }));
}

fn kinds(problems: &[Value]) -> Vec<&str> {
    problems
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect()
}

#[test]
fn problems_are_reported_where_they_are() {
    let header = "harness h(n: Int) -> Any\nuses m = \"math\"\n";
    let at = |source: &str| problems(&format!("{header}{source}"), math());

    let p = at("a = m.double(n: n\nreturn a\n");
    assert_eq!(kinds(&p), ["syntax"]);

    let p = at("a = m.double(n: x)\nreturn a\n");
    assert_eq!(p[0]["kind"], "unknownName");
    assert_eq!(
        p[0]["span"],
        json!({ "start": { "line": 3, "column": 17 }, "end": { "line": 3, "column": 18 } })
    );

    let p = at("a = m.double(n: b)\nb = m.double(n: a)\nreturn a\n");
    assert_eq!(
        p[0],
        json!({
            "kind": "cycle", "nodes": ["a", "b"],
            "span": { "start": { "line": 3, "column": 1 }, "end": { "line": 3, "column": 2 } },
        })
    );

    let p = at("a = m.double(n: n) when n > 0\nreturn a\n");
    assert_eq!(p[0]["kind"], "mayBeSkipped");
    assert_eq!(p[0]["node"], "a");

    let p = at("a = m.double(n: \"two\")\nreturn a\n");
    assert_eq!(p[0]["kind"], "mismatch");
    assert_eq!(p[0]["span"]["start"], json!({ "line": 3, "column": 17 }));

    let p = at("a = m.double(n: m.square(n: 1))\nreturn a\n");
    assert_eq!(p[0]["kind"], "nestedCall");

    let p = at("a = m.triple(n: 1)\nreturn a\n");
    assert_eq!(
        p[0],
        json!({
            "kind": "unknownOperation", "op": "m.triple",
            "span": { "start": { "line": 3, "column": 5 }, "end": { "line": 3, "column": 13 } },
        })
    );

    let p = at("a = m.double(n: 1)\nreturn a\notherwise return a\n");
    assert_eq!(kinds(&p), ["otherwiseReadsNode"]);

    let p = at("a = m.double(n: 1)\nreturn match a { failed(e) => 0, _ => 1 } + len(a)\n");
    assert_eq!(kinds(&p), ["type"]);

    let p = problems(
        "harness h(n: Int) -> Int\nuses m = \"math\"\na = m.double(n: n)\nreturn a\n",
        math(),
    );
    assert_eq!(p[0]["kind"], "needsOtherwise");
    assert_eq!(p[0]["node"], "a");

    let p = problems("harness h(n: Int) -> Int\nuses q\nreturn 1\n", math());
    assert_eq!(p[0]["kind"], "unknownCapability");
    assert_eq!(p[0]["capability"], "q");

    let p = at("a = m.double(n: 1)\n");
    assert_eq!(kinds(&p), ["noReturn"]);
}
