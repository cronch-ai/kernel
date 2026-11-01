//! `fold`, the one form of iteration: checked, evaluated and exported through the kernel's interface.

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};

fn call(request: Value) -> Value {
    let response = decode(&kernel::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

fn sum_ask(xs: Value) -> Value {
    json!({
        "spec": { "name": "Sum", "params": [{ "name": "xs", "type": { "List": "Int" } }], "output": "Int" },
        "args": { "xs": xs }
    })
}

fn var(name: &str) -> Value {
    json!({ "e": "var", "name": name })
}

fn fold(over: Value, init: Value, body: Value) -> Value {
    json!({ "procedure": { "e": "fold", "over": over, "init": init, "acc": "acc", "item": "x", "body": body } })
}

fn add(a: Value, b: Value) -> Value {
    json!({ "e": "op", "op": "add", "args": [a, b] })
}

fn assess(ask: Value, answer: Value, resolved: Value) -> Value {
    call(json!({ "op": "assess", "ask": ask, "answer": answer, "resolved": resolved }))
}

#[test]
fn a_fold_runs_once_per_item_in_order() {
    let sum = fold(
        var("xs"),
        json!({ "e": "lit", "v": 0 }),
        add(var("acc"), var("x")),
    );
    let report = assess(sum_ask(json!([1, 2, 3, 4])), sum.clone(), json!({}));
    assert_eq!(report["state"], "satisfied");
    assert_eq!(report["value"], json!(10));

    let empty = assess(sum_ask(json!([])), sum, json!({}));
    assert_eq!(empty["value"], json!(0));

    // Order matters: the last item seen is the one that stays.
    let last = fold(var("xs"), json!({ "e": "lit", "v": 0 }), var("x"));
    let report = assess(sum_ask(json!([7, 8, 9])), last, json!({}));
    assert_eq!(report["value"], json!(9));
}

#[test]
fn every_items_hole_is_reported_at_once() {
    let double =
        json!({ "name": "Double", "params": [{ "name": "n", "type": "Int" }], "output": "Int" });
    let answer = fold(
        var("xs"),
        json!({ "e": "lit", "v": 0 }),
        add(
            var("acc"),
            json!({ "e": "ask", "spec": double, "args": { "n": var("x") } }),
        ),
    );
    let partial = assess(sum_ask(json!([1, 2, 3])), answer.clone(), json!({}));
    assert_eq!(partial["state"], "constructed");
    let open = partial["open"].as_array().unwrap();
    let args: Vec<&Value> = open.iter().map(|o| &o["ask"]["args"]["n"]).collect();
    assert_eq!(args, [&json!(1), &json!(2), &json!(3)]);

    let resolved: serde_json::Map<String, Value> = open
        .iter()
        .map(|o| {
            let n = o["ask"]["args"]["n"].as_i64().unwrap();
            (o["key"].as_str().unwrap().to_string(), json!(n * 2))
        })
        .collect();
    let done = assess(sum_ask(json!([1, 2, 3])), answer, Value::Object(resolved));
    assert_eq!(done["state"], "satisfied");
    assert_eq!(done["value"], json!(12));
}

#[test]
fn a_fold_must_be_over_a_list_and_keep_its_accumulators_type() {
    let not_a_list = fold(
        json!({ "e": "lit", "v": 3 }),
        json!({ "e": "lit", "v": 0 }),
        var("acc"),
    );
    let report = assess(sum_ask(json!([])), not_a_list, json!({}));
    assert_eq!(report["state"], "illFormed");
    assert_eq!(
        report["problems"][0],
        json!({ "kind": "notAList", "at": "/over", "got": "Int" })
    );

    let changes_type = fold(
        var("xs"),
        json!({ "e": "lit", "v": 0 }),
        json!({ "e": "lit", "v": "text" }),
    );
    let report = assess(sum_ask(json!([1])), changes_type, json!({}));
    assert_eq!(report["state"], "illFormed");
    assert_eq!(
        report["problems"][0],
        json!({ "kind": "mismatch", "at": "/body", "expected": "Int", "got": "String" })
    );
}

#[test]
fn a_fold_has_no_smtlib_reading() {
    let sum = fold(
        var("xs"),
        json!({ "e": "lit", "v": 0 }),
        add(var("acc"), var("x")),
    );
    let result = call(json!({ "op": "smtlib", "ask": sum_ask(json!([1])), "answer": sum }));
    assert_eq!(result["kind"], "refused");
    assert_eq!(result["error"]["reason"], json!({ "kind": "fold" }));
    assert_eq!(result["error"]["place"], json!({ "in": "body", "at": "" }));
}
