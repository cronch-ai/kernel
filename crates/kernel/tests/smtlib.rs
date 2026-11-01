//! The SMT-LIB export of each fixture in `tests/smtlib/`, through the kernel's interface as a host calls it, is exactly
//! the `.smt2` beside it. `BLESS=1 cargo test` rewrites them; review the diff. Whether a solver reads them as the
//! kernel evaluates is checked by the application (`cronch-ai/cronch-october2026`), with z3.

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};
use std::path::PathBuf;

fn call(request: Value) -> Value {
    let response = decode(&kernel::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

fn fixtures() -> Vec<(String, Value)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/smtlib");
    let mut found: Vec<(String, Value)> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            let name = path
                .file_name()?
                .to_str()?
                .strip_suffix(".json")?
                .to_string();
            Some((
                name,
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            ))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

#[test]
fn each_fixture_exports_as_its_smt2() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/smtlib");
    let bless = std::env::var_os("BLESS").is_some();
    let all = fixtures();
    assert!(all.len() >= 5);
    for (name, fixture) in all {
        let exported = call(json!({
            "op": "smtlib",
            "ask": fixture["ask"],
            "world": fixture.get("world").cloned().unwrap_or(json!({})),
            "answer": fixture["answer"],
        }));
        assert_eq!(exported["kind"], "exported", "{name}: {exported}");
        let text = exported["text"].as_str().unwrap();
        let path = dir.join(format!("{name}.smt2"));
        if bless {
            std::fs::write(&path, text).unwrap();
        } else {
            let expected = std::fs::read_to_string(&path).unwrap_or_default();
            assert_eq!(
                text, expected,
                "{name}: the export differs from {name}.smt2 (BLESS=1 to rewrite it)"
            );
        }
    }
}

#[test]
fn a_part_with_no_reading_is_refused_with_where_and_why() {
    let refused = call(json!({
        "op": "smtlib",
        "ask": { "spec": { "name": "Empty", "output": "Int" }, "args": {} },
        "answer": { "procedure": { "e": "op", "op": "len", "args": [{ "e": "list", "items": [] }] } },
    }));
    assert_eq!(
        refused,
        json!({ "kind": "refused", "error": {
            "kind": "noReading", "place": { "in": "body", "at": "/args/0" }, "reason": { "kind": "any" }
        } })
    );
}
