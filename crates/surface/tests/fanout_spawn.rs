//! Fan-out, one call per item of a list issued in one step, and spawning a registered harness as a child run.

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};

fn call(request: Value) -> Value {
    let response = decode(&surface::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

fn math() -> Value {
    let int = json!({ "input": { "Record": { "n": "Int" } }, "output": "Int" });
    let stream =
        json!({ "input": { "Record": { "n": "Int" } }, "output": "Int", "mode": "stream" });
    json!({ "math": { "operations": { "double": int, "square": int, "count": stream } } })
}

fn interface() -> Value {
    let capabilities = json!([{ "prefix": "m", "capability": math()["math"] }]);
    call(json!({ "op": "install", "capabilities": capabilities }))["interface"].clone()
}

fn elaborate(source: &str, harnesses: Value) -> Value {
    call(
        json!({ "op": "elaborate", "source": source, "capabilities": math(), "harnesses": harnesses }),
    )
}

fn elaborated(source: &str, harnesses: Value) -> Value {
    let out = elaborate(source, harnesses);
    assert_eq!(out["kind"], "elaborated", "{out:#}");
    out
}

/// The outcome after these deliveries, and every action the root performed.
fn run(harness: &Value, deliveries: &[Value]) -> (Value, Vec<Value>) {
    let ready = call(json!({ "op": "init", "harness": harness, "interface": interface() }));
    assert_eq!(ready["kind"], "ready", "{ready}");
    let mut system = ready["system"].clone();
    let mut actions = Vec::new();
    for d in deliveries {
        let out = call(json!({ "op": "deliver", "system": system, "delivery": d }));
        system = out["system"].clone();
        for e in out["events"].as_array().unwrap() {
            if e["kind"] == "out" && e["origin"] == json!([]) {
                actions.push(e["action"].clone());
            }
        }
    }
    (system["root"]["outcome"].clone(), actions)
}

fn to_root(input: Value) -> Value {
    json!({ "input": input })
}

fn start(value: Value) -> Value {
    to_root(json!({ "kind": "start", "value": value }))
}

fn ret(call: u64, value: Value) -> Value {
    to_root(json!({ "kind": "ret", "call": call, "value": value }))
}

fn failed(call: u64) -> Value {
    to_root(json!({ "kind": "failed", "call": call, "error": "down" }))
}

const DOUBLE_ALL: &str = r#"
harness double_all(xs: List[Int]) -> List[Int]
uses m = "math"
doubled = for x in xs: m.double(n: x + 1)
return doubled
"#;

#[test]
fn a_fan_out_issues_every_call_in_one_step_and_keeps_the_list_order() {
    let harness = elaborated(DOUBLE_ALL, json!({}))["harness"].clone();
    let (outcome, actions) = run(&harness, &[start(json!([1, 2, 3]))]);
    assert_eq!(outcome, Value::Null);
    let args: Vec<Value> = actions.iter().map(|a| a["args"]["n"].clone()).collect();
    assert_eq!(args, [json!(2), json!(3), json!(4)]);

    let (outcome, _) = run(
        &harness,
        &[
            start(json!([1, 2, 3])),
            ret(2, json!(8)),
            ret(0, json!(4)),
            ret(1, json!(6)),
        ],
    );
    assert_eq!(outcome, json!({ "kind": "halted", "value": [4, 6, 8] }));
}

#[test]
fn a_fan_out_over_nothing_settles_at_once() {
    let harness = elaborated(DOUBLE_ALL, json!({}))["harness"].clone();
    let (outcome, actions) = run(&harness, &[start(json!([]))]);
    assert_eq!(actions, [json!({ "kind": "halt", "value": [] })]);
    assert_eq!(outcome["value"], json!([]));
}

#[test]
fn a_fan_out_fails_if_any_of_its_calls_does() {
    let harness = elaborated(DOUBLE_ALL, json!({}))["harness"].clone();
    let (outcome, _) = run(
        &harness,
        &[start(json!([1, 2])), ret(0, json!(4)), failed(1)],
    );
    assert_eq!(outcome["value"], json!({ "failed": "doubled" }));

    let handled = r#"
harness h(xs: List[Int]) -> Int
uses m = "math"
doubled = for x in xs: m.double(n: x)
return match doubled { ok(ys) => len(ys), failed(e) => -1, refused => -2 }
"#;
    let harness = elaborated(handled, json!({}))["harness"].clone();
    let (outcome, _) = run(
        &harness,
        &[start(json!([1, 2])), failed(1), ret(0, json!(2))],
    );
    assert_eq!(outcome["value"], json!(-1));
}

const CHILD: &str = r#"
harness child(n: Int) -> Int
uses m = "math"
a = m.double(n: n)
return a
"#;

#[test]
fn a_spawned_harness_runs_as_a_child_and_its_value_comes_back() {
    let child = elaborated(CHILD, json!({}))["harness"].clone();
    let parent = r#"
harness parent(n: Int) -> Any
uses m = "math"
uses doubler = harness "registered:doubler"
r = spawn doubler(n + 1) only m.double
return r
"#;
    let out = elaborated(parent, json!({ "registered:doubler": child }));
    assert_eq!(
        out["harnesses"],
        json!([{ "name": "doubler", "harness": "registered:doubler" }])
    );
    let harness = out["harness"].clone();
    let (outcome, actions) = run(
        &harness,
        &[
            start(json!(4)),
            json!({ "to": [0], "input": { "kind": "ret", "call": 0, "value": 10 } }),
        ],
    );
    assert_eq!(actions[0]["kind"], "spawn");
    assert_eq!(actions[0]["arg"], json!(5));
    assert_eq!(
        actions[0]["narrow"],
        json!([{ "kind": "only", "ops": ["m.double"] }])
    );
    assert_eq!(outcome["value"], json!(10));
}

#[test]
fn spawns_and_fan_outs_are_checked() {
    let child = elaborated(CHILD, json!({}))["harness"].clone();
    let harnesses = json!({ "doubler": child });
    let problems = |source: &str| {
        let out = elaborate(source, harnesses.clone());
        assert_eq!(out["kind"], "problems", "{out:#}");
        out["problems"].as_array().unwrap().clone()
    };
    let header =
        "harness h(n: Int) -> Any\nuses m = \"math\"\nuses doubler = harness \"doubler\"\n";

    // The child calls `m.double`, which a narrowing to `m.square` takes away.
    let p = problems(&format!(
        "{header}r = spawn doubler(n) only m.square\nreturn r\n"
    ));
    assert_eq!(p[0]["kind"], "child");
    let p = problems(&format!(
        "{header}r = spawn doubler(n) without m.triple\nreturn r\n"
    ));
    assert_eq!(p[0]["kind"], "unknownOperation");
    let p = problems(&format!("{header}r = spawn doubler(\"n\")\nreturn r\n"));
    assert_eq!(p[0]["kind"], "mismatch");
    let p = problems(&format!("{header}r = spawn m(n)\nreturn r\n"));
    assert_eq!(p[0]["kind"], "notAHarness");
    let p = problems("harness h(n: Int) -> Any\nuses ghost = harness \"nowhere\"\nreturn 1\n");
    assert_eq!(p[0]["kind"], "unknownHarness");

    let p = problems(&format!(
        "{header}d = for x in n: m.double(n: x)\nreturn d\n"
    ));
    assert_eq!(p[0]["kind"], "mismatch");
    let p = problems(&format!(
        "{header}d = for x in [n]: m.count(n: x)\nreturn d\n"
    ));
    assert_eq!(p[0]["kind"], "argument");
    let p = problems(&format!(
        "{header}s = m.double(n: n)\nd = for x in [n]: m.double(n: x) until s\nreturn d\n"
    ));
    assert_eq!(p[0]["kind"], "until");
    let p = problems(&format!(
        "{header}d = len(for x in [n]: m.double(n: x))\nreturn d\n"
    ));
    assert_eq!(p[0]["kind"], "syntax");
}
