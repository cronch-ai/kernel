//! Compliance vectors: exchanges with the kernel's interface that every implementation must reproduce exactly.
//!
//! Each file in `compliance/cases/` is a sequence of requests and the responses they must get. This test replays
//! them. The cases are defined below and written by running with `CRONCH_BLESS=1`; a case whose definition changes
//! must be blessed again, and the diff reviewed as a change to the specification.

mod common;
use common::demo::*;
use common::*;
use kernel::cbor::{decode, encode};
use serde_json::{Value, json};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../compliance/cases")
}

/// Records each exchange as a case runs.
struct Recorder {
    exchanges: Vec<Value>,
}

impl Recorder {
    fn call(&mut self, request: Value) -> Value {
        let response = decode(&kernel::api::call(&encode(&request))).unwrap();
        self.exchanges
            .push(json!({ "request": request, "response": response }));
        assert_eq!(response["ok"], json!(true), "{response}");
        response["result"].clone()
    }

    fn init(&mut self, harness: Value, interface: Value, grant: Value) -> Value {
        let ready = self.call(
            json!({ "op": "init", "harness": harness, "interface": interface, "grant": grant }),
        );
        ready["system"].clone()
    }

    fn deliver(&mut self, system: Value, delivery: Value) -> (Value, Value) {
        let out = self.call(json!({ "op": "deliver", "system": system, "delivery": delivery }));
        (out["system"].clone(), out["events"].clone())
    }
}

type Case = (&'static str, &'static str, fn(&mut Recorder));

fn cases() -> Vec<Case> {
    vec![
        (
            "expressions",
            "Typing and evaluating expressions, including a fold and a label.",
            |r| {
                let sum = fold(
                    var("xs"),
                    lit(json!(0)),
                    "a",
                    "x",
                    op("add", vec![var("a"), var("x")]),
                );
                r.call(json!({ "op": "infer", "expr": sum, "vars": { "xs": { "List": "Int" } } }));
                r.call(json!({ "op": "evaluate", "expr": sum, "vars": { "xs": [1, 2, 3] } }));
                r.call(json!({ "op": "evaluate", "expr": label(var("x")), "vars": { "x": 1 } }));
                r.call(json!({ "op": "evaluate", "expr": op("idiv", vec![lit(json!(1)), lit(json!(0))]) }));
            },
        ),
        (
            "checking",
            "Problems with a harness, an interface and a property.",
            |r| {
                let mut bad = careful();
                bad["init"] = json!(1);
                r.call(json!({ "op": "checkHarness", "harness": bad, "interface": interface() }));
                r.call(
                    json!({ "op": "checkHarness", "harness": careful(), "interface": interface() }),
                );
                let x = json!({ "operations": { "x": { "input": { "Record": {} }, "output": "Int", "pre": lit(json!(1)) } } });
                r.call(json!({ "op": "checkInterface", "interface": x }));
                r.call(
                    json!({ "op": "eventType", "harness": obedient(), "interface": interface() }),
                );
                for property in [stop_means_stop(), misspelled(), mistyped()] {
                    r.call(json!({ "op": "checkProperty", "property": property, "harness": obedient(), "interface": interface() }));
                }
            },
        ),
        (
            "expected",
            "Checking against an expected type: a record literal naming a case is that variant, in a state too.",
            |r| {
                let slot = json!({ "Variant": { "tag": "kind", "cases": {
                    "waiting": { "Record": {} },
                    "ok": { "Record": { "value": "Int" } },
                }}});
                let ok = rec(vec![("kind", lit(json!("ok"))), ("value", var("n"))]);
                r.call(
                    json!({ "op": "infer", "expr": ok, "vars": { "n": "Int" }, "expected": slot }),
                );
                r.call(json!({ "op": "infer", "expr": ok, "vars": { "n": "String" }, "expected": slot }));
                r.call(
                    json!({ "op": "infer", "expr": list(vec![]), "expected": { "List": slot } }),
                );
                let state = json!({ "Record": { "slot": slot } });
                let set = json!({ "e": "let", "name": "next", "type": state,
                    "be": rec(vec![("slot", ok)]), "in": step(var("next"), vec![]) });
                let harness = json!({
                    "arg": "Int", "state": state, "init": { "slot": { "kind": "waiting" } },
                    "step": let_("n", lit(json!(1)), set),
                });
                r.call(json!({ "op": "checkHarness", "harness": harness, "interface": {} }));
                let invariant = eq(path("state", &["slot", "kind"]), lit(json!("ok")));
                r.call(json!({ "op": "obligation", "harness": harness, "invariant": invariant }));
            },
        ),
        (
            "run",
            "A run that fetches and sends, a refused call, withheld inputs, and a harness its interface can't run.",
            |r| {
                let system = r.init(leaky(), interface(), Value::Null);
                let (system, _) = r.deliver(system, start(json!({})));
                let (system, _) = r.deliver(system, ret(7, json!("no such call")));
                let (system, _) = r.deliver(system, ret(0, json!(3)));
                r.deliver(system, ret(0, json!("page")));
                let mut closed = interface();
                closed["operations"]["net.fetch"]["pre"] = lit(json!(false));
                let refusing = r.init(leaky(), closed, Value::Null);
                r.deliver(refusing, start(json!({})));
                r.call(json!({ "op": "init", "harness": leaky(), "interface": {} }));
            },
        ),
        (
            "grants",
            "A child narrowed and approved: an \"always\" in the child widens its parent.",
            |r| {
                let system = r.init(actor(), tools(), json!({ "allow": [], "once": [] }));
                let (system, _) = r.deliver(system, start(json!({})));
                let publish = json!(["npm", "publish"]);
                let run = msg(
                    "act",
                    json!({ "kind": "call", "op": "shell.run", "args": { "argv": publish } }),
                );
                let (system, _) = r.deliver(system, run.clone());
                let spawn = msg(
                    "act",
                    json!({ "kind": "spawn", "harness": worker(), "narrow": [{ "kind": "only", "ops": ["shell.approve"] }],
                                            "arg": { "op": "shell.approve", "args": { "argv": publish } } }),
                );
                let (system, _) = r.deliver(system, spawn);
                let mut answer = ret(0, json!("always"));
                answer["to"] = json!([1]);
                let (system, _) = r.deliver(system, answer);
                r.deliver(system, run);
            },
        ),
        (
            "monitoring",
            "Properties monitored over traces, whole and step by step, reading each event as its view: a leak, and effects after stop.",
            |r| {
                let system = r.init(leaky(), interface(), Value::Null);
                let (system, a) = r.deliver(system, start(json!({})));
                let (_, b) = r.deliver(system, ret(0, json!("page")));
                let trace: Vec<Value> = a
                    .as_array()
                    .unwrap()
                    .iter()
                    .chain(b.as_array().unwrap())
                    .cloned()
                    .collect();
                // What the properties read of it.
                r.call(json!({ "op": "view", "events": trace }));
                r.call(json!({ "op": "monitor", "property": no_untrusted_send(), "trace": trace }));
                r.call(
                    json!({ "op": "monitor", "property": results_answer_calls(), "trace": trace }),
                );
                // The same verdict, one delivery's events at a time; `at` counts from the step's first event.
                let started =
                    r.call(json!({ "op": "monitorStart", "property": no_untrusted_send() }));
                let first = r.call(json!({ "op": "monitorStep", "property": no_untrusted_send(), "state": started["state"], "events": a }));
                r.call(json!({ "op": "monitorStep", "property": no_untrusted_send(), "state": first["state"], "events": b }));
                // Refused: a quantified property, and a state that isn't this property's.
                r.call(json!({ "op": "monitorStart", "property": results_answer_calls() }));
                r.call(json!({ "op": "monitorStep", "property": no_untrusted_send(), "state": { "last": [true] }, "events": b }));
            },
        ),
        (
            "evidence",
            "Evidence by counterexample, bounded and exhaustive search, and its re-checking.",
            |r| {
                let alphabet = json!([
                    { "input": { "kind": "start", "value": {} } },
                    { "input": { "kind": "ret", "value": "page", "op": "net.fetch" } },
                    { "input": { "kind": "ret", "value": 200, "op": "net.send" } },
                ]);
                let subject = |harness: Value| json!({ "harness": harness, "interface": interface(), "property": no_untrusted_send() });
                let with = |harness: Value, extra: Value| {
                    let mut request = subject(harness);
                    for (k, v) in extra.as_object().unwrap() {
                        request[k] = v.clone();
                    }
                    request
                };
                let counterexample = json!({ "kind": "counterexample", "deliveries": [start(json!({})), ret(0, json!("page"))] });
                let refuted = r.call(with(
                    leaky(),
                    json!({ "op": "prove", "method": counterexample }),
                ));
                r.call(with(
                    leaky(),
                    json!({ "op": "verify", "evidence": refuted["evidence"] }),
                ));
                r.call(with(careful(), json!({ "op": "prove", "method": { "kind": "bounded", "depth": 3, "alphabet": alphabet } })));
                let holds = r.call(with(careful(), json!({ "op": "prove", "method": { "kind": "exhaustive", "alphabet": alphabet } })));
                r.call(with(
                    careful(),
                    json!({ "op": "verify", "evidence": holds["evidence"] }),
                ));
            },
        ),
        (
            "induction",
            "An invariant certificate checked, and consecution as a query whose counterexample the kernel checks.",
            |r| {
                let alphabet = json!([
                    { "input": { "kind": "start", "value": {} } },
                    { "input": { "kind": "ret", "value": "page", "op": "net.fetch" } },
                    { "input": { "kind": "ret", "value": 200, "op": "net.send" } },
                ]);
                let mut request = json!({ "op": "certificate", "harness": careful(), "interface": interface(), "property": no_untrusted_send(), "alphabet": alphabet });
                let found = r.call(request.clone());
                request["op"] = json!("prove");
                request["method"] = json!({ "kind": "invariant", "invariant": lit(json!(true)), "configurations": found["configurations"], "alphabet": alphabet });
                r.call(request);
                let count = json!({ "operations": { "count": { "input": { "Record": {} }, "output": "Int" } } });
                let counter = json!({
                    "arg": { "Record": {} }, "state": { "Record": { "n": "Int" } }, "init": { "n": 0 },
                    "step": match_(var("input"), "kind", vec![
                        ("ret", "r", match_(var("r"), "op", vec![
                            ("count", "c", step(rec(vec![("n", op("add", vec![path("state", &["n"]), path("c", &["value"])]))]), vec![])),
                        ], Some(step(var("state"), vec![])))),
                    ], Some(step(var("state"), vec![]))),
                });
                let invariant = op("ge", vec![path("state", &["n"]), lit(json!(0))]);
                r.call(json!({ "op": "obligation", "harness": counter, "interface": count, "invariant": invariant }));
                r.call(json!({ "op": "checkInduction", "harness": counter, "interface": count, "invariant": invariant,
                           "state": { "n": 0 }, "input": { "kind": "ret", "op": "count", "call": 0, "value": -1 } }));
            },
        ),
        ("keys", "Content keys of harness objects.", |r| {
            r.call(json!({ "op": "objectKey", "kind": "harness", "object": careful() }));
            r.call(json!({ "op": "objectKey", "kind": "property", "object": no_untrusted_send() }));
            r.call(json!({ "op": "objectKey", "kind": "interface", "object": interface() }));
            r.call(json!({ "op": "objectKey", "kind": "logEntry", "object": {
                "seq": 0,
                "prev": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "delivery": start(json!({})),
                "events": [],
            } }));
            r.call(json!({ "op": "objectKey", "kind": "module", "object": {
                "main": "capability.js",
                "modules": { "capability.js": "export class Capability {}" },
            } }));
        }),
        (
            "smtlib",
            "A precondition exported to SMT-LIB, a refusal, and a solver's values read back.",
            |r| {
                let pre = op(
                    "prefix",
                    vec![lit(json!("/workspace/")), path("args", &["path"])],
                );
                r.call(json!({ "op": "smtlib", "definitions": [{ "name": "pre", "params": [{ "name": "args", "type": { "Record": { "path": "String" } } }], "body": pre }] }));
                r.call(json!({ "op": "smtlib", "definitions": [{ "name": "f", "body": fold(lit(json!([1])), lit(json!(0)), "a", "x", var("a")) }] }));
                r.call(json!({ "op": "readValues", "reply": "((x (- 5)) (y true))", "types": ["Int", "Bool"] }));
            },
        ),
    ]
}

fn record(case: &Case) -> Value {
    let mut recorder = Recorder {
        exchanges: Vec::new(),
    };
    (case.2)(&mut recorder);
    json!({ "description": case.1, "exchanges": recorder.exchanges })
}

#[test]
fn every_vector_replays_exactly() {
    let cases = cases();
    if std::env::var("CRONCH_BLESS").is_ok() {
        std::fs::create_dir_all(dir()).unwrap();
        for case in &cases {
            let text = serde_json::to_string_pretty(&record(case)).unwrap() + "\n";
            std::fs::write(dir().join(format!("{}.json", case.0)), text).unwrap();
        }
    }
    let mut names: Vec<String> = std::fs::read_dir(dir())
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".json").map(str::to_string))
        .collect();
    names.sort();
    let mut defined: Vec<String> = cases.iter().map(|c| c.0.to_string()).collect();
    defined.sort();
    assert_eq!(
        names, defined,
        "every case has a vector, and every vector a case"
    );
    for name in names {
        let vector: Value = serde_json::from_str(
            &std::fs::read_to_string(dir().join(format!("{name}.json"))).unwrap(),
        )
        .unwrap();
        for (i, exchange) in vector["exchanges"].as_array().unwrap().iter().enumerate() {
            let response = decode(&kernel::api::call(&encode(&exchange["request"]))).unwrap();
            assert_eq!(response, exchange["response"], "{name}, exchange {i}");
        }
    }
}
