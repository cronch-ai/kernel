//! Properties and invariants written in source: they reach the same verdicts as the platform's hand-written ones,
//! read node slots by name, and report what is wrong with them where it is.

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};

fn call(request: Value) -> Value {
    let response = decode(&surface::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn platform() -> Value {
    json!({ "model": fixture("model"), "approve": fixture("approve"), "notes": fixture("notes") })
}

fn interface() -> Value {
    let capabilities: Vec<Value> = ["model", "approve", "notes"]
        .iter()
        .map(|c| json!({ "prefix": c, "capability": fixture(c) }))
        .collect();
    call(json!({ "op": "install", "capabilities": capabilities }))["interface"].clone()
}

const APPROVED_NOTE: &str = r#"
harness approved_note(prompt: String) -> Any

uses model, approve, notes

draft    = model.generate(messages: [{role: "user", content: prompt}])
decision = approve.ask(action: "notes.append", args: {text: draft})
saved    = notes.append(text: draft) when decision != "deny"

return match saved {
  ok(note) => note
  skipped  => "denied"
}

otherwise return "stopped"

property no_effect_without_approval:
  effect() implies once(returned(approve.ask, value in ["once", "always"]))

property saved_only_after_a_yes:
  running(saved) implies decision.value != "deny"

invariant saved_only_after_a_yes:
  saved.kind in ["running", "ok"] implies decision.kind == "ok" and decision.value != "deny"
"#;

fn elaborated(source: &str) -> Value {
    let out = call(json!({ "op": "elaborate", "source": source, "capabilities": platform() }));
    assert_eq!(out["kind"], "elaborated", "{out:#}");
    out
}

fn problems(source: &str) -> Vec<Value> {
    let out = call(json!({ "op": "elaborate", "source": source, "capabilities": platform() }));
    assert_eq!(out["kind"], "problems", "{out:#}");
    out["problems"].as_array().unwrap().clone()
}

fn alphabet() -> Value {
    let mut letters = vec![
        json!({ "input": { "kind": "start", "value": "a haiku" } }),
        json!({ "input": { "kind": "ret", "op": "model.generate", "value": "draft" } }),
    ];
    for answer in ["once", "always", "deny"] {
        letters.push(json!({ "input": { "kind": "ret", "op": "approve.ask", "value": answer } }));
    }
    letters.push(json!({ "input": { "kind": "ret", "op": "notes.append", "value": "id" } }));
    letters.push(
        json!({ "input": { "kind": "failed", "op": "notes.append", "error": "unavailable" } }),
    );
    json!(letters)
}

fn verdict(harness: &Value, property: &Value, grant: Value) -> Value {
    let out = call(json!({
        "op": "prove", "harness": harness, "interface": interface(), "grant": grant, "property": property,
        "method": { "kind": "exhaustive", "alphabet": alphabet() },
    }));
    assert_eq!(out["kind"], "evidence", "{out}");
    out["evidence"]["verdict"].clone()
}

/// Every event of the run these inputs give.
fn trace(harness: &Value, grant: Value, inputs: &[Value]) -> Vec<Value> {
    let ready =
        call(json!({ "op": "init", "harness": harness, "interface": interface(), "grant": grant }));
    let mut system = ready["system"].clone();
    let mut events = Vec::new();
    for input in inputs {
        let out =
            call(json!({ "op": "deliver", "system": system, "delivery": { "input": input } }));
        system = out["system"].clone();
        events.extend(out["events"].as_array().unwrap().iter().cloned());
    }
    events
}

fn start() -> Value {
    json!({ "kind": "start", "value": "a haiku" })
}

fn ret(call: u64, value: Value) -> Value {
    json!({ "kind": "ret", "call": call, "value": value })
}

#[test]
fn a_property_in_source_reaches_the_hand_written_ones_verdicts() {
    let out = elaborated(APPROVED_NOTE);
    let harness = out["harness"].clone();
    let source = out["properties"][0].clone();
    assert_eq!(source["name"], "no_effect_without_approval");
    let hand_written = fixture("no-effect-without-approval");
    let none = json!({ "allow": [], "once": [] });
    let allowed = json!({ "allow": ["notes.append"], "once": [] });

    assert_eq!(verdict(&harness, &source, none.clone()), "holds");
    let reckless = APPROVED_NOTE.replace(r#" when decision != "deny""#, "");
    let reckless = elaborated(&reckless)["harness"].clone();
    for property in [&source, &hand_written] {
        assert_eq!(verdict(&reckless, property, allowed.clone()), "refuted");
    }

    // The two agree event by event, including where the reckless harness writes without a yes.
    let runs = [
        (
            harness.clone(),
            none.clone(),
            vec![
                start(),
                ret(0, json!("d")),
                ret(1, json!("once")),
                ret(2, json!("id")),
            ],
        ),
        (
            harness.clone(),
            none.clone(),
            vec![start(), ret(0, json!("d")), ret(1, json!("deny"))],
        ),
        (
            reckless.clone(),
            allowed.clone(),
            vec![
                start(),
                ret(0, json!("d")),
                ret(1, json!("deny")),
                ret(2, json!("id")),
            ],
        ),
    ];
    for (h, grant, inputs) in runs {
        let events = trace(&h, grant, &inputs);
        let a = call(json!({ "op": "monitor", "property": source, "trace": events }));
        let b = call(json!({ "op": "monitor", "property": hand_written, "trace": events }));
        assert_eq!(a, b, "{inputs:?}");
    }
}

#[test]
fn properties_and_invariants_read_node_slots_by_name() {
    let out = elaborated(APPROVED_NOTE);
    let harness = out["harness"].clone();
    let slots = out["properties"][1].clone();
    let none = json!({ "allow": [], "once": [] });
    assert_eq!(verdict(&harness, &slots, none.clone()), "holds");

    // The invariant holds of every state of every run, and of the initial state.
    let invariant = out["invariants"][0]["expr"].clone();
    assert_eq!(out["invariants"][0]["name"], "saved_only_after_a_yes");
    let runs = [
        vec![
            start(),
            ret(0, json!("d")),
            ret(1, json!("once")),
            ret(2, json!("id")),
        ],
        vec![start(), ret(0, json!("d")), ret(1, json!("deny"))],
    ];
    for inputs in runs {
        for e in trace(&harness, none.clone(), &inputs) {
            let held = call(
                json!({ "op": "evaluate", "expr": invariant, "vars": { "state": e["state"] } }),
            );
            assert_eq!(held["value"], json!(true), "{e}");
        }
    }
    let obligation = call(json!({
        "op": "obligation", "harness": harness, "interface": interface(), "invariant": invariant,
    }));
    assert_eq!(obligation["kind"], "obligation", "{obligation}");
    assert_eq!(obligation["initiation"], json!(true));

    // And it fails of a state where the note was saved after a no.
    let bad = json!({ "arg": "", "next": 3, "nodes": {
        "draft": { "kind": "ok", "call": 0, "value": "d", "error": "" },
        "decision": { "kind": "ok", "call": 1, "value": "deny", "error": "" },
        "saved": { "kind": "running", "call": 2, "value": "", "error": "" },
    }});
    let held = call(json!({ "op": "evaluate", "expr": invariant, "vars": { "state": bad } }));
    assert_eq!(held["value"], json!(false));

    assert_eq!(
        out["sourceMap"]["nodes"]["saved"]["start"],
        json!({ "line": 8, "column": 1 })
    );
    assert_eq!(
        out["sourceMap"]["properties"]["no_effect_without_approval"]["start"]["line"],
        json!(17)
    );
}

#[test]
fn quantifiers_and_since_connect_events() {
    // Every append answers an ask that was answered with a yes since the last append.
    let source = format!(
        "{APPROVED_NOTE}\nproperty fresh_yes:\n  called(notes.append) implies prev((not called(notes.append)) since returned(approve.ask, value != \"deny\"))\n\nproperty every_result_answers_a_call:\n  (event.kind == \"in\" and event.input.kind == \"ret\") implies exists k in event.call: k == event.input.call\n"
    );
    let out = elaborated(&source);
    let none = json!({ "allow": [], "once": [] });
    let fresh = &out["properties"][2];
    assert_eq!(verdict(&out["harness"], fresh, none.clone()), "holds");

    // The kernel searches only quantifier-free properties, so the quantified one is monitored.
    let answers = &out["properties"][3];
    let inputs = [
        start(),
        ret(0, json!("d")),
        ret(1, json!("once")),
        ret(2, json!("id")),
    ];
    let events = trace(&out["harness"], none, &inputs);
    let monitored = call(json!({ "op": "monitor", "property": answers, "trace": events }));
    assert_eq!(monitored["verdict"], "holds", "{monitored}");
}

#[test]
fn claims_are_checked_where_they_are() {
    let header = "harness h(prompt: String) -> Any\nuses model, approve, notes\ndraft = model.generate(messages: [])\ntext = draft\nreturn draft\n";
    let at = |claim: &str| problems(&format!("{header}{claim}\n"));

    let p = at("property p: called(notes.remove)");
    assert_eq!(p[0]["kind"], "unknownOperation");
    let p = at("property p: once(always(effect()))");
    assert_eq!(p[0]["kind"], "nestedAlways");
    let p = at("property p: once(effect(), effect())");
    assert_eq!(p[0]["kind"], "helper");
    let p = at("property p: text == \"\"");
    assert_eq!(
        p[0],
        json!({
            "kind": "notASlot", "node": "text",
            "span": { "start": { "line": 6, "column": 13 }, "end": { "line": 6, "column": 23 } },
        })
    );
    let p = at("property p: msg(model.generate)");
    assert_eq!(p[0]["kind"], "unknownChannel");
    let p = at("invariant i: once(effect())");
    assert_eq!(p[0]["kind"], "unknownFunction");
    let p = at("invariant i: draft.kind");
    assert_eq!(p[0]["kind"], "mismatch");
    let p = problems("harness h(n: Int) -> Any\nx = n since n\nreturn x\n");
    assert_eq!(p[0]["kind"], "propertyOnly");
    let p = at("property p: effect()\nproperty p: effect()");
    assert_eq!(p[0]["kind"], "duplicate");
}
