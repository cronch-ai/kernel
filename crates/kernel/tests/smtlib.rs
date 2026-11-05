//! What SMT-LIB reads of typed values: lists item by item, and the text `str` writes, which a solver chooses.

mod common;
use common::*;
use serde_json::{Value, json};

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
fn a_list_reads_as_a_wider_list_item_by_item() {
    let ints = json!({ "List": "Int" });
    let floats = json!({ "List": "Float" });
    let declared = |text: &str| text.matches("(define-funs-rec ((|map:coerce.").count();

    // Declared once, however often it is used.
    let text = exported(
        json!([{ "name": "xs", "type": ints }]),
        rec(vec![("a", var("xs")), ("b", var("xs"))]),
        Some(json!({ "Record": { "a": floats, "b": floats } })),
    );
    assert_eq!(declared(&text), 1, "{text}");

    // A list of lists: the inner map is declared before the outer one, which calls it.
    let text = exported(
        json!([{ "name": "xss", "type": { "List": ints } }]),
        var("xss"),
        Some(json!({ "List": floats })),
    );
    assert_eq!(declared(&text), 2, "{text}");
    let first = text.find("(define-funs-rec ((|map:coerce.").unwrap();
    let outer = text.rfind("(define-funs-rec ((|map:coerce.").unwrap();
    let inner = text[first + "(define-funs-rec ((".len()..]
        .split_whitespace()
        .next()
        .unwrap();
    assert!(text[outer..].contains(inner), "{text}");
}

#[test]
fn the_text_of_a_value_is_unconstrained_but_per_type() {
    let text = exported(
        json!([
            { "name": "r", "type": { "Record": { "a": "Int" } } },
            { "name": "x", "type": "Float" },
        ]),
        rec(vec![
            ("r", op("str", vec![var("r")])),
            ("again", op("str", vec![var("r")])),
            ("x", op("str", vec![var("x")])),
            ("n", op("str", vec![lit(json!(3))])),
        ]),
        None,
    );
    // One function for the record type and one for Float; an Int's text is exact.
    assert_eq!(text.matches("(declare-fun |fn:text.").count(), 2, "{text}");
    assert!(text.contains("str.from_int"), "{text}");
}

#[test]
fn a_literal_with_no_type_of_its_own_is_a_problem() {
    let infer = |expr: Value| call(json!({ "op": "infer", "expr": expr, "vars": {} }));
    for untyped in [lit(json!(null)), list(vec![]), lit(json!([]))] {
        let out = infer(untyped);
        assert_eq!(out["problems"][0]["kind"], "untyped", "{out}");
    }
    // Where a type is expected, it takes it.
    let typed = json!({ "e": "let", "name": "x", "type": { "List": "Int" }, "be": list(vec![]), "in": var("x") });
    assert_eq!(infer(typed)["problems"], json!([]));
    // And `[]` takes its type from the branches that say what a list holds.
    let branches = if_(lit(json!(true)), list(vec![lit(json!(1))]), list(vec![]));
    let out = infer(branches);
    assert_eq!(out["type"], json!({ "List": "Int" }), "{out}");
}
