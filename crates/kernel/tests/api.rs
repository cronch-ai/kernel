//! The kernel's interface for expressions and SMT-LIB, and how it answers requests it doesn't understand.

mod common;
use common::*;
use kernel::cbor::{decode, encode};
use serde_json::json;

#[test]
fn expressions_are_typed_and_evaluated() {
    let sum = fold(
        var("xs"),
        lit(json!(0)),
        "a",
        "x",
        op("add", vec![var("a"), var("x")]),
    );
    let typed = call(json!({ "op": "infer", "expr": sum, "vars": { "xs": { "List": "Int" } } }));
    assert_eq!(typed, json!({ "type": "Int", "problems": [] }));
    let value = call(json!({ "op": "evaluate", "expr": sum, "vars": { "xs": [1, 2, 3, 4] } }));
    assert_eq!(value, json!({ "kind": "value", "value": 10 }));
    let failed =
        call(json!({ "op": "evaluate", "expr": op("idiv", vec![lit(json!(1)), lit(json!(0))]) }));
    assert_eq!(failed["error"]["error"]["kind"], "divisionByZero");
}

#[test]
fn definitions_export_to_smtlib_and_solver_values_read_back() {
    let pre = op(
        "prefix",
        vec![lit(json!("/workspace/")), path("args", &["path"])],
    );
    let out = call(json!({ "op": "smtlib", "definitions": [{
        "name": "pre",
        "params": [{ "name": "args", "type": { "Record": { "path": "String" } } }],
        "body": pre,
        "description": "fs.write's precondition",
    }]}));
    assert_eq!(out["kind"], "exported");
    let text = out["text"].as_str().unwrap();
    assert!(text.contains("; fs.write's precondition"), "{text}");
    assert!(
        text.contains("(define-fun |def:pre| ((|v:args| |record:"),
        "{text}"
    );

    let refused = call(
        json!({ "op": "smtlib", "definitions": [{ "name": "labelled", "body": label(lit(json!(1))) }] }),
    );
    assert_eq!(refused["error"]["definition"], "labelled");
    assert_eq!(refused["error"]["reason"]["kind"], "label");

    let values = call(
        json!({ "op": "readValues", "reply": "((x 5) (y \"hi\"))", "types": ["Int", "String"] }),
    );
    assert_eq!(values, json!({ "kind": "values", "values": [5, "hi"] }));
}

#[test]
fn malformed_requests_are_reported_not_panicked() {
    let response = decode(&kernel::api::call(&encode(&json!({ "op": "nope" })))).unwrap();
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["error"]["kind"], "malformed");
    let response = decode(&kernel::api::call(b"{\"op\":\"key\"}")).unwrap();
    assert_eq!(response["error"]["kind"], "undecodable");
    let response = decode(&kernel::api::call(&encode(&json!({ "op": "assess" })))).unwrap();
    assert_eq!(response["error"]["kind"], "malformed");
}
