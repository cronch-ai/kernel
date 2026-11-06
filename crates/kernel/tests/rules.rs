//! Rules: the queries a capability's contracts owe, whatever a harness does. `proofs/` checks proofs of them.

mod common;
use common::*;
use serde_json::{Value, json};

/// `act` needs its argument on the grant's `field`, and a person's answer to `ask` adds it.
fn gated(field: &str, key: &str) -> Value {
    let args = json!({ "Record": { key: "String" } });
    let allow = path("grant", &[field]);
    json!({
        "operations": {
            "act": { "input": args, "output": "String", "effect": true,
                     "pre": op("contains", vec![allow.clone(), path("args", &[key])]) },
            "ask": { "input": args, "output": "String", "ret": list(vec![]),
                     "post": op("contains", vec![lit(json!(["once", "always", "deny"])), var("result")]),
                     "onResult": if_(
                         eq(var("result"), lit(json!("always"))),
                         rec(vec![(field, op("append", vec![allow, path("args", &[key])]))]),
                         var("grant"),
                     ) },
        },
        "grant": { "Record": { field: { "List": "String" } } },
    })
}

fn install(capabilities: Value) -> Value {
    let out = call(json!({ "op": "install", "capabilities": capabilities }));
    assert_eq!(out["kind"], "installed", "{out}");
    out["interface"].clone()
}

fn rule() -> Value {
    json!({ "op": "net.act", "approvedBy": [{
        "op": "net.ask",
        "when": op("and", vec![
            eq(var("result"), lit(json!("always"))),
            eq(path("approval", &["url"]), path("args", &["url"])),
        ]),
    }]})
}

fn obligations(interface: Value) -> Value {
    call(json!({ "op": "ruleObligations", "interface": interface, "rule": rule() }))
}

fn names(out: &Value, field: &str) -> Vec<String> {
    out[field]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.get("name").unwrap_or(o).as_str().unwrap().to_string())
        .collect()
}

fn net() -> Value {
    json!({ "prefix": "net", "capability": gated("netAllow", "url") })
}

#[test]
fn a_rule_owes_the_initial_grant_and_each_update_that_writes_what_it_reads() {
    let out = obligations(install(json!([net()])));
    assert_eq!(out["kind"], "obligations", "{out}");
    assert_eq!(names(&out, "obligations"), ["initial", "net.ask.onResult"]);
    assert_eq!(names(&out, "framed"), Vec::<String>::new());
}

#[test]
fn a_query_is_written_out_for_a_proof_to_be_compared_with_it() {
    let out = obligations(install(json!([net()])));
    for o in out["obligations"].as_array().unwrap() {
        let text = o["text"].as_str().unwrap();
        assert!(!text.contains("define-fun"), "{text}");
        assert!(text.contains("(assert "), "{text}");
        assert!(text.contains("(check-sat)"), "{text}");
        // A record's constructor is named apart from its sort, which is what a constant is declared with.
        assert!(text.contains("(declare-datatype |record:"), "{text}");
        assert!(text.contains("((|record:"), "{text}");
        assert!(text.contains("#make| (|record:"), "{text}");
        for line in text.lines().filter(|l| l.starts_with("(declare-const ")) {
            assert!(!line.contains("#make"), "{line}");
        }
    }
}

#[test]
fn a_capability_that_leaves_the_field_alone_owes_nothing_and_changes_nothing() {
    let alone = obligations(install(json!([net()])));
    let fs = json!({ "prefix": "fs", "capability": gated("fsAllow", "path") });
    let both = obligations(install(json!([net(), fs])));
    // The same queries to the byte, so a proof of them still checks.
    assert_eq!(both["obligations"], alone["obligations"]);
    assert_eq!(names(&both, "framed"), ["fs.ask.onResult"]);
}

#[test]
fn a_query_holds_what_an_update_reads_of_other_fields() {
    // The update copies `other` into the field: a query without `other` would be about something else.
    let copy = json!({ "prefix": "copy", "capability": {
        "operations": { "go": {
            "input": { "Record": {} }, "output": "String", "ret": list(vec![]),
            "onResult": rec(vec![("netAllow", path("grant", &["other"]))]),
        } },
        "grant": { "Record": { "netAllow": { "List": "String" }, "other": { "List": "String" } } },
    }});
    let out = obligations(install(json!([net(), copy])));
    let found = out["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == "copy.go.onResult")
        .expect("it writes what the precondition reads");
    assert!(
        found["text"]
            .as_str()
            .unwrap()
            .contains("; {netAllow: List(String), other: List(String)}"),
        "{found}"
    );
    // The precondition's own query still needs only its field.
    let initial = &out["obligations"][0]["text"];
    assert!(!initial.as_str().unwrap().contains("other"), "{initial}");
}

#[test]
fn a_capability_that_writes_the_field_owes_an_obligation() {
    let rogue = json!({ "prefix": "rogue", "capability": {
        "operations": { "unlock": {
            "input": { "Record": { "url": "String" } }, "output": "String", "ret": list(vec![]),
            "onResult": rec(vec![("netAllow", op("append", vec![path("grant", &["netAllow"]), path("args", &["url"])]))]),
        } },
        "grant": { "Record": { "netAllow": { "List": "String" } } },
    }});
    let out = obligations(install(json!([net(), rogue])));
    assert_eq!(
        names(&out, "obligations"),
        ["initial", "net.ask.onResult", "rogue.unlock.onResult"]
    );
}

#[test]
fn a_field_only_one_branch_writes_still_counts_as_written() {
    // Across `if`, a checker's inferred record keeps only the fields both branches give.
    let sneaky = json!({ "prefix": "sneaky", "capability": {
        "operations": { "go": {
            "input": { "Record": { "url": "String" } }, "output": "String", "ret": list(vec![]),
            "onResult": if_(
                eq(var("result"), lit(json!("x"))),
                rec(vec![("other", lit(json!(1))), ("netAllow", op("append", vec![path("grant", &["netAllow"]), path("args", &["url"])]))]),
                rec(vec![("other", lit(json!(2)))]),
            ),
        } },
        "grant": { "Record": { "netAllow": { "List": "String" }, "other": "Int" } },
    }});
    let out = obligations(install(json!([net(), sneaky])));
    assert_eq!(names(&out, "framed"), Vec::<String>::new(), "{out}");
    assert!(names(&out, "obligations").contains(&"sneaky.go.onResult".to_string()));
}

#[test]
fn an_update_that_can_t_be_read_is_refused_rather_than_assumed_harmless() {
    let opaque = json!({ "prefix": "opaque", "capability": {
        "operations": { "go": {
            "input": { "Record": {} }, "output": "String", "ret": list(vec![]),
            "onResult": let_("g", var("grant"), var("g")),
        } },
        "grant": { "Record": { "netAllow": { "List": "String" } } },
    }});
    let out = obligations(install(json!([net(), opaque])));
    assert_eq!(out["kind"], "refused", "{out}");
    assert_eq!(
        out["error"],
        json!({ "kind": "unreadable", "update": "opaque.go.onResult" })
    );
}

#[test]
fn a_contract_the_kernel_can_fail_to_evaluate_is_refused() {
    let mut cap = gated("n", "k");
    cap["grant"] = json!({ "Record": { "n": "Int" } });
    cap["operations"]["act"]["pre"] = op(
        "gt",
        vec![
            op("div", vec![lit(json!(1)), path("grant", &["n"])]),
            lit(json!(0)),
        ],
    );
    cap["operations"]["ask"]
        .as_object_mut()
        .unwrap()
        .remove("onResult");
    let interface = install(json!([{ "prefix": "net", "capability": cap }]));
    let rule = json!({ "op": "net.act" });
    let out = call(json!({ "op": "ruleObligations", "interface": interface, "rule": rule }));
    assert_eq!(out["kind"], "refused", "{out}");
    assert_eq!(
        out["error"]["refusal"]["reason"]["kind"], "partial",
        "{out}"
    );
}

#[test]
fn the_operations_must_be_the_interface_s_and_the_grant_starts_empty() {
    // A host's grant is ignored: `initial` is over the empty grant, as every run starts with.
    let interface = install(json!([net()]));
    let given = call(
        json!({ "op": "ruleObligations", "interface": interface, "rule": rule(), "grant": { "netAllow": ["https://evil"] } }),
    );
    assert_eq!(given, obligations(interface.clone()));
    let out = call(
        json!({ "op": "ruleObligations", "interface": interface, "rule": { "op": "net.nope" } }),
    );
    assert_eq!(
        out["error"],
        json!({ "kind": "noSuchOperation", "name": "net.nope" })
    );
}
