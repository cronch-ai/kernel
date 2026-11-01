//! Evidence: counterexamples replayed, bounded and exhaustive searches, and re-checking what someone else produced.

mod common;
use common::demo::*;
use common::*;
use serde_json::{Value, json};

fn prove(harness: Value, property: Value, method: Value) -> Value {
    call(
        json!({ "op": "prove", "harness": harness, "interface": interface(), "property": property, "method": method }),
    )
}

fn verify(harness: Value, property: Value, evidence: &Value) -> Value {
    call(
        json!({ "op": "verify", "harness": harness, "interface": interface(), "property": property, "evidence": evidence }),
    )
}

fn fetch_alphabet() -> Value {
    json!([
        { "input": { "kind": "start", "value": {} } },
        { "input": { "kind": "ret", "value": "page", "op": "net.fetch" } },
        { "input": { "kind": "ret", "value": 200, "op": "net.send" } },
    ])
}

fn stop_alphabet() -> Value {
    json!([
        { "input": { "kind": "start", "value": {} } },
        { "input": { "kind": "msg", "channel": "user", "value": "stop" } },
        { "input": { "kind": "msg", "channel": "user", "value": "go on" } },
        { "input": { "kind": "msg", "channel": "act", "value": { "kind": "call", "op": "fs.write", "args": { "path": "/a" } } } },
    ])
}

#[test]
fn a_counterexample_is_replayed_not_trusted() {
    let deliveries = json!([start(json!({})), ret(0, json!("page"))]);
    let out = prove(
        leaky(),
        no_untrusted_send(),
        json!({ "kind": "counterexample", "deliveries": deliveries }),
    );
    assert_eq!(out["kind"], "evidence", "{out}");
    let evidence = &out["evidence"];
    assert_eq!(evidence["verdict"], "refuted");
    assert_eq!(evidence["at"], json!(3));
    assert_eq!(
        verify(leaky(), no_untrusted_send(), evidence),
        json!({ "valid": true })
    );

    // A delivery the environment couldn't make: there is no call 5.
    let bad = json!([start(json!({})), ret(5, json!("page"))]);
    let out = prove(
        leaky(),
        no_untrusted_send(),
        json!({ "kind": "counterexample", "deliveries": bad }),
    );
    assert_eq!(out["error"]["kind"], "inadmissible");
    assert_eq!(out["error"]["index"], json!(1));

    // A run that keeps the property refutes nothing.
    let fine = json!([start(json!({})), ret(1, json!("page"))]);
    let out = prove(
        careful(),
        no_untrusted_send(),
        json!({ "kind": "counterexample", "deliveries": fine }),
    );
    assert_eq!(out["error"]["kind"], "notViolated");
}

#[test]
fn bounded_search_finds_counterexamples_or_holds_to_its_depth() {
    let method = json!({ "kind": "bounded", "depth": 4, "alphabet": fetch_alphabet() });
    let refuted = prove(leaky(), no_untrusted_send(), method.clone());
    assert_eq!(refuted["evidence"]["verdict"], "refuted");
    assert_eq!(
        refuted["evidence"]["counterexample"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        verify(leaky(), no_untrusted_send(), &refuted["evidence"])["valid"],
        json!(true)
    );

    let holds = prove(careful(), no_untrusted_send(), method);
    assert_eq!(holds["evidence"]["verdict"], "holdsUpTo");
    assert_eq!(holds["evidence"]["depth"], json!(4));
    let kinds: Vec<&Value> = holds["evidence"]["assumptions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| &a["kind"])
        .collect();
    assert_eq!(
        kinds,
        [
            &json!("contracts"),
            &json!("checker"),
            &json!("environment"),
            &json!("runtime")
        ]
    );
    assert_eq!(
        verify(careful(), no_untrusted_send(), &holds["evidence"])["valid"],
        json!(true)
    );

    let method = json!({ "kind": "bounded", "depth": 4, "alphabet": stop_alphabet() });
    let stubborn_verdict = prove(stubborn(), stop_means_stop(), method.clone());
    assert_eq!(stubborn_verdict["evidence"]["verdict"], "refuted");
    let obedient_verdict = prove(obedient(), stop_means_stop(), method);
    assert_eq!(obedient_verdict["evidence"]["verdict"], "holdsUpTo");
}

#[test]
fn exhaustive_search_proves_for_every_depth_when_the_configurations_run_out() {
    let method = json!({ "kind": "exhaustive", "alphabet": fetch_alphabet() });
    let holds = prove(careful(), no_untrusted_send(), method.clone());
    assert_eq!(holds["evidence"]["verdict"], "holds", "{holds}");
    assert_eq!(
        verify(careful(), no_untrusted_send(), &holds["evidence"])["valid"],
        json!(true)
    );

    let refuted = prove(leaky(), no_untrusted_send(), method);
    assert_eq!(refuted["evidence"]["verdict"], "refuted");

    // Every write takes a fresh call identifier, so a harness that writes on demand never runs out of
    // configurations: the search says so rather than claim more than it explored.
    let method = json!({ "kind": "exhaustive", "alphabet": stop_alphabet(), "limit": 200 });
    let out = prove(obedient(), stop_means_stop(), method);
    assert_eq!(
        out["error"],
        json!({ "kind": "inconclusive", "explored": 200 })
    );

    let method = json!({ "kind": "exhaustive", "alphabet": fetch_alphabet() });
    let out = prove(careful(), results_answer_calls(), method);
    assert_eq!(out["error"]["kind"], "quantified");
}

#[test]
fn evidence_is_about_one_subject_and_one_verdict() {
    let method = json!({ "kind": "bounded", "depth": 3, "alphabet": fetch_alphabet() });
    let out = prove(careful(), no_untrusted_send(), method);
    let evidence = out["evidence"].clone();
    assert_eq!(
        verify(leaky(), no_untrusted_send(), &evidence)["error"]["kind"],
        "wrongSubject"
    );

    let mut overclaimed = evidence.clone();
    overclaimed["verdict"] = json!("holds");
    overclaimed.as_object_mut().unwrap().remove("depth");
    assert_eq!(
        verify(careful(), no_untrusted_send(), &overclaimed)["error"]["kind"],
        "disagrees"
    );

    // Keys identify content: the same evidence has the same key.
    let again = prove(
        careful(),
        no_untrusted_send(),
        json!({ "kind": "bounded", "depth": 3, "alphabet": fetch_alphabet() }),
    );
    assert_eq!(again["key"], out["key"]);
    let harness_key =
        call(json!({ "op": "objectKey", "kind": "harness", "object": careful() }))["key"].clone();
    assert_eq!(evidence["harness"], harness_key);
}
