//! Constrained reasoning: an action that follows only from a checked proof, from a fixed theory and from facts that
//! trusted results gave. A refund agent: `refund.issue` needs `refundable(order)` derived, and `policy.prove_refundable`
//! is the only thing that derives it, when the checker it stands for accepts a proof from the policy and every fact in
//! the grant. Facts and derived atoms are grant fields, one per predicate, so preconditions read them as lists of
//! strings, which SMT-LIB and Lean read too. The harness is a puppet that makes whatever call the model tells it to, so
//! what is shown holds whatever the model says.

mod common;
use common::*;
use serde_json::{Value, json};

/// One of the capabilities in `examples/refund/`. `orders.lookup` and `carrier.claim` give facts, each the owner of
/// its predicates. `policy.prove_refundable` stands for the checker: a call passes every fact in the grant, as its
/// precondition requires, so the statement checked is `Gamma -> facts -> refundable(order)` with no fact left out or
/// made up, and its trusted result derives `refundable(order)`. `refund.issue` needs that, and refunds an order at most
/// once, counting it refunded when the call is made.
fn example(name: &str) -> Value {
    let path = format!(
        "{}/../../examples/refund/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn capabilities() -> Value {
    json!([
        { "prefix": "orders", "capability": example("orders") },
        { "prefix": "carrier", "capability": example("carrier") },
        { "prefix": "policy", "capability": example("policy") },
        { "prefix": "refund", "capability": example("refund") },
    ])
}

fn tools() -> Value {
    let out = call(json!({ "op": "install", "capabilities": capabilities() }));
    assert_eq!(out["kind"], "installed", "{out}");
    out["interface"].clone()
}

/// The tools, and a channel on which the model says what to call.
fn installed() -> Value {
    let calls = calls_of(&tools());
    let mut all = capabilities();
    all.as_array_mut().unwrap().push(json!({ "capability": {
        "channels": { "model": { "type": calls, "label": ["model"] } },
    }}));
    let out = call(json!({ "op": "install", "capabilities": all }));
    assert_eq!(out["kind"], "installed", "{out}");
    out["interface"].clone()
}

fn rule() -> Value {
    json!({ "op": "refund.issue", "approvedBy": [{
        "op": "policy.prove_refundable",
        "when": op("and", vec![
            path("result", &["ok"]),
            eq(path("approval", &["order"]), path("args", &["order"])),
        ]),
    }]})
}

fn names(out: &Value, field: &str) -> Vec<String> {
    out[field]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.get("name").unwrap_or(o).as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_refund_owes_its_derivation_and_facts_owe_nothing() {
    for format in ["smtlib", "lean"] {
        let out = call(json!({
            "op": "ruleObligations", "interface": tools(), "rule": rule(), "format": format,
        }));
        assert_eq!(out["kind"], "obligations", "{out}");
        // The derivation, and the refund's own update, which only narrows what it allows.
        assert_eq!(
            names(&out, "obligations"),
            [
                "initial",
                "policy.prove_refundable.onResult",
                "refund.issue.onCall"
            ]
        );
        // What writes facts writes nothing a refund reads, so a capability that adds facts re-proves nothing.
        assert_eq!(
            names(&out, "framed"),
            ["carrier.claim.onResult", "orders.lookup.onResult"]
        );
    }
}

/// The puppet: it makes each call the model's channel holds.
fn puppet() -> Value {
    let msg = match_(
        var("input"),
        "channel",
        vec![("model", "m", performing())],
        Some(step(var("state"), vec![])),
    );
    json!({
        "result": "String", "arg": { "Record": {} },
        "state": { "Record": {} },
        "init": {},
        "step": on(vec![("msg", msg)], step(var("state"), vec![])),
    })
}

fn model(op_name: &str, args: Value) -> Value {
    msg(
        "model",
        json!({ "kind": "call", "op": op_name, "args": args }),
    )
}

fn issue(order: &str) -> Value {
    model("refund.issue", json!({ "order": order }))
}

fn prove(order: &str, late: Value, premium: Value, damaged: Value) -> Value {
    model(
        "policy.prove_refundable",
        json!({ "order": order, "proof": "...", "late": late, "premium": premium, "damaged": damaged }),
    )
}

/// Whether each delivery's call was performed.
fn performed(system: Value, deliveries: Vec<Value>) -> (Value, Vec<bool>) {
    let (system, events) = deliver_all(system, deliveries);
    let calls = events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["kind"] == "call")
        .map(|e| e["performed"].as_bool().unwrap())
        .collect();
    (system, calls)
}

fn started() -> Value {
    let (system, _) = deliver_all(init(puppet(), installed()), vec![start(json!({}))]);
    system
}

#[test]
fn a_refund_happens_only_after_its_derivation_and_only_once() {
    let none = || json!([]);
    let mine = || json!(["812"]);

    // Call 0: straight to the refund, with nothing derived. Call 1: the lookup, whose trusted result gives the facts.
    let (system, calls) = performed(
        started(),
        vec![issue("812"), model("orders.lookup", json!({ "id": "812" }))],
    );
    assert_eq!(calls, [false, true]);
    let (system, _) = deliver_all(
        system,
        vec![ret(1, json!({ "late": true, "premium": true }))],
    );
    assert_eq!(system["root"]["grant"]["late"], mine());
    assert_eq!(system["root"]["grant"]["premium"], mine());

    // Call 2: the customer said it arrived damaged, and the model cites that. Call 3: it leaves out a fact. Neither is
    // the grant's facts, so neither reaches the checker. Call 4: every fact, as the grant has them.
    let (system, calls) = performed(
        system,
        vec![
            prove("812", mine(), mine(), mine()),
            prove("812", mine(), none(), none()),
            prove("812", mine(), mine(), none()),
        ],
    );
    assert_eq!(calls, [false, false, true]);

    // The checker refuses the proof, and nothing is derived.
    let (system, calls) = performed(system, vec![ret(4, json!({ "ok": false })), issue("812")]);
    assert_eq!(calls, [false]);
    assert_eq!(system["root"]["grant"]["refundable"], none());

    // Call 6: a proof the checker accepts. Then a refund of another order, of this one, and of this one again.
    let (system, _) = performed(system, vec![prove("812", mine(), mine(), none())]);
    let (system, calls) = performed(
        system,
        vec![
            ret(6, json!({ "ok": true })),
            issue("813"),
            issue("812"),
            issue("812"),
        ],
    );
    assert_eq!(system["root"]["grant"]["refundable"], mine());
    assert_eq!(calls, [false, true, false]);
    assert_eq!(system["root"]["grant"]["refunded"], mine());
}

#[test]
fn a_model_can_not_write_a_fact_or_a_derivation() {
    // The same operations, with their results marked as coming from the model: as if the model answered for the
    // lookup and the checker. Untrusted results change no grant.
    let narrowed = call(
        json!({ "op": "narrow", "interface": installed(), "narrowings": [
            { "kind": "raise", "op": "orders.lookup", "label": ["model"] },
            { "kind": "raise", "op": "policy.prove_refundable", "label": ["model"] },
        ]}),
    )["interface"]
        .clone();
    let (system, _) = deliver_all(init(puppet(), narrowed), vec![start(json!({}))]);
    let (system, _) = performed(
        system,
        vec![
            model("orders.lookup", json!({ "id": "812" })),
            prove("812", json!([]), json!([]), json!([])),
        ],
    );
    let (system, calls) = performed(
        system,
        vec![
            ret(0, json!({ "late": true, "premium": true })),
            ret(1, json!({ "ok": true })),
            issue("812"),
        ],
    );
    assert_eq!(calls, [false]);
    assert_eq!(system["root"]["grant"]["late"], json!([]));
    assert_eq!(system["root"]["grant"]["refundable"], json!([]));
}
