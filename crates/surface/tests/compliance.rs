//! Compliance vectors for elaboration: exchanges every implementation of `elaborate` must reproduce exactly.
//!
//! Each file in `compliance/surface/` is a sequence of requests and the responses they must get, as for the kernel's
//! own vectors in `compliance/cases/`. They are defined below and written by running with `CRONCH_BLESS=1`; a case
//! whose definition changes must be blessed again, and the diff reviewed as a change to the specification.

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../compliance/surface")
}

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn call(request: &Value) -> Value {
    decode(&surface::api::call(&encode(request))).unwrap()
}

type Case = (&'static str, &'static str, fn() -> Vec<Value>);

fn cases() -> Vec<Case> {
    vec![(
        "elaborate",
        "The platform's approved-note harness elaborated from source, and a source with problems.",
        || {
            let capabilities = json!({ "model": fixture("model"), "approve": fixture("approve"), "notes": fixture("notes") });
            let source = "\
harness approved_note(prompt: String) -> Any

uses model, approve, notes

draft    = model.generate(messages: [{role: \"user\", content: prompt}])
decision = approve.ask(action: \"notes.append\", args: {text: draft})
saved    = notes.append(text: draft) when decision != \"deny\"

return match saved {
  ok(note) => note
  skipped  => \"denied\"
}

otherwise return \"stopped\"
";
            let broken = "\
harness broken(prompt: String) -> String
uses model
draft = model.generate(messages: [{role: \"user\", content: topic}])
return draft
";
            vec![
                json!({ "op": "elaborate", "source": source, "capabilities": capabilities }),
                json!({ "op": "elaborate", "source": broken, "capabilities": capabilities }),
            ]
        },
    )]
}

#[test]
fn every_vector_replays_exactly() {
    let cases = cases();
    if std::env::var("CRONCH_BLESS").is_ok() {
        std::fs::create_dir_all(dir()).unwrap();
        for (name, description, requests) in &cases {
            let exchanges: Vec<Value> = requests()
                .into_iter()
                .map(|request| {
                    let response = call(&request);
                    assert_eq!(response["ok"], json!(true), "{response}");
                    json!({ "request": request, "response": response })
                })
                .collect();
            let vector = json!({ "description": description, "exchanges": exchanges });
            let text = serde_json::to_string_pretty(&vector).unwrap() + "\n";
            std::fs::write(dir().join(format!("{name}.json")), text).unwrap();
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
            assert_eq!(
                call(&exchange["request"]),
                exchange["response"],
                "{name}, exchange {i}"
            );
        }
    }
}
