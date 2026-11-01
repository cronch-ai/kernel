//! Claims, through the kernel's interface as a host calls it. Each fixture in `tests/claims/` lists witnesses and the
//! verdict the kernel gives each, and its first query is exactly the `.smt2` beside it (`BLESS=1 cargo test` rewrites
//! them; review the diff). Whether z3 finds what the fixture says, and the kernel confirms it, is checked by the
//! application (`cronch-ai/cronch-october2026`).

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};
use std::path::PathBuf;

fn call(request: Value) -> Value {
    let response = decode(&kernel::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/claims")
}

fn fixtures() -> Vec<(String, Value)> {
    let mut found: Vec<(String, Value)> = std::fs::read_dir(dir())
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

/// The verdict's kind, or `incomplete`.
fn outcome(assessment: &Value) -> String {
    match assessment["kind"].as_str() {
        Some("assessed") => assessment["verdict"]["kind"].as_str().unwrap().to_string(),
        other => other.unwrap().to_string(),
    }
}

#[test]
fn each_witness_gets_the_verdict_its_fixture_states() {
    let all = fixtures();
    assert!(all.len() >= 6);
    for (name, fixture) in all {
        assert_eq!(
            call(json!({ "op": "checkClaim", "claim": fixture["claim"] })),
            json!({ "problems": [] }),
            "{name}"
        );
        for case in fixture["witnesses"].as_array().unwrap() {
            let assessment = call(json!({
                "op": "assessClaim",
                "claim": fixture["claim"],
                "answer": fixture["answer"],
                "witness": case["witness"],
            }));
            assert_eq!(
                outcome(&assessment),
                case["verdict"],
                "{name} at {}: {assessment}",
                case["witness"]
            );
            // A refutation comes with a script that confirms it without the kernel; nothing else does.
            let refuted = matches!(outcome(&assessment).as_str(), "refutes" | "inconsistent");
            assert_eq!(
                assessment["validation"].is_string(),
                refuted,
                "{name}: {assessment}"
            );
        }
    }
}

/// The first query, exact (`<name>.smt2`), and with floats read as reals (`<name>.real.smt2`) where that differs.
#[test]
fn each_first_query_is_its_smt2() {
    let bless = std::env::var_os("BLESS").is_some();
    for (name, fixture) in fixtures() {
        let query = |floats: &str| {
            let query = call(json!({
                "op": "claimQuery", "claim": fixture["claim"], "answer": fixture["answer"], "floats": floats,
            }));
            assert_eq!(query["kind"], "query", "{name}: {query}");
            query
        };
        let exact = query("exact");
        let vars: Vec<&str> = fixture["claim"]["vars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        let reads: Vec<&str> = exact["reads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            reads, vars,
            "{name}: the first query reads the claim's variables"
        );
        let real = query("real");
        let real = (real["script"] != exact["script"]).then_some(real);
        for (suffix, query) in [("smt2", Some(exact)), ("real.smt2", real)] {
            let path = dir().join(format!("{name}.{suffix}"));
            match (query, bless) {
                (Some(query), true) => {
                    std::fs::write(&path, query["script"].as_str().unwrap()).unwrap()
                }
                (None, true) => {
                    let _ = std::fs::remove_file(&path);
                }
                (Some(query), false) => {
                    let expected = std::fs::read_to_string(&path).unwrap_or_default();
                    assert_eq!(
                        query["script"],
                        json!(expected),
                        "{name}.{suffix} differs (BLESS=1 to rewrite it)"
                    );
                }
                (None, false) => assert!(
                    !path.exists(),
                    "{name}.{suffix} is stale (BLESS=1 to remove it)"
                ),
            }
        }
    }
}

#[test]
fn a_query_goes_on_to_the_nested_asks_the_witness_reaches() {
    let (_, fixture) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "per-capita")
        .unwrap();
    let query = call(json!({
        "op": "claimQuery",
        "claim": fixture["claim"],
        "answer": fixture["answer"],
        "witness": { "vars": { "country": "Kenya" } },
    }));
    let reads = query["reads"].as_array().unwrap();
    assert_eq!(
        reads
            .iter()
            .map(|r| r["ask"]["spec"]["name"].clone())
            .collect::<Vec<_>>(),
        [json!("GDP"), json!("Population")]
    );
    let script = query["script"].as_str().unwrap();
    assert!(
        script.contains("(assert (= |v:country| \"Kenya\"))"),
        "{script}"
    );
    assert!(script.contains("(get-value ((|ask:GDP:"), "{script}");

    // The reply is read at the types the reads name, in order.
    let reply = "((|ask:GDP:x| (fp #b1 #b01111111111 #x0000000000000)) (|ask:Population:x| 3))";
    let values = call(json!({ "op": "readValues", "reply": reply, "reads": reads }));
    assert_eq!(values, json!({ "kind": "values", "values": [-1, 3] }));
    let lying = call(
        json!({ "op": "readValues", "reply": "((|ask:GDP:x| true) (|ask:Population:x| 3))", "reads": reads }),
    );
    assert_eq!(lying["kind"], "unreadable");
}

#[test]
fn a_claim_may_ask_only_what_it_is_about() {
    let other = json!({ "name": "Other", "output": "Int" });
    let claim = json!({
        "name": "Mixed",
        "about": { "name": "Abs", "params": [{ "name": "n", "type": "Int" }], "output": "Int" },
        "vars": [{ "name": "n", "type": "Int" }],
        "holds": { "e": "op", "op": "ge", "args": [{ "e": "ask", "spec": other, "args": {} }, { "e": "var", "name": "n" }] },
    });
    assert_eq!(
        call(json!({ "op": "checkClaim", "claim": claim })),
        json!({ "problems": [{ "kind": "foreignAsk", "at": "/args/0", "ask": "Other" }] })
    );
    let assume = json!({ "e": "assume", "v": true, "type": "Bool" });
    let claim = json!({ "name": "Taken", "about": other, "holds": assume });
    let problems = call(json!({ "op": "checkClaim", "claim": claim }));
    assert_eq!(
        problems["problems"][0]["problem"]["kind"], "notDerived",
        "{problems}"
    );
}

#[test]
fn claims_are_keyed_by_their_content() {
    let (_, fixture) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "abs")
        .unwrap();
    let key = |claim: &Value| {
        call(json!({ "op": "claimKey", "claim": claim }))["key"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let mut renamed = fixture["claim"].clone();
    renamed["name"] = json!("AtLeastZero");
    assert!(key(&fixture["claim"]).starts_with("sha256:"));
    assert_eq!(key(&fixture["claim"]), key(&fixture["claim"].clone()));
    assert_ne!(key(&fixture["claim"]), key(&renamed));
}
