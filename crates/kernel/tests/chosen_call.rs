//! A model chooses a call with pure datatypes: its answer is a variant of the tools it is offered, and a harness reads
//! it with `match`. No type here depends on the interface.

mod common;
use common::*;
use serde_json::{Value, json};

fn brain(tools: Value) -> Value {
    let mut cases = json!({ "answer": { "Record": { "text": "String" } } });
    for (name, args) in tools.as_object().unwrap() {
        cases[name] = args.clone();
    }
    json!({
        "input": { "Record": { "prompt": "String" } },
        "output": { "Variant": { "tag": "kind", "cases": cases } },
        "ret": list(vec![lit(json!("model"))]),
    })
}

fn url() -> Value {
    json!({ "Record": { "url": "String" } })
}
fn text() -> Value {
    json!({ "Record": { "text": "String" } })
}

fn interface() -> Value {
    let out = call(json!({ "op": "install", "capabilities": [
        { "prefix": "brain", "capability": { "operations": {
            "choose": brain(json!({ "net.fetch": url(), "notes.append": text() })),
            "readonly": brain(json!({ "net.fetch": url() })),
        }}},
        { "prefix": "net", "capability": {
            "operations": { "fetch": {
                "input": url(), "output": "String",
                "pre": op("contains", vec![path("grant", &["netAllow"]), path("args", &["url"])]),
            }},
            "grant": { "Record": { "netAllow": { "List": "String" } } },
        }},
        { "prefix": "notes", "capability": { "operations": {
            "append": { "input": text(), "output": "String", "effect": true },
        }}},
    ]}));
    assert_eq!(out["kind"], "installed", "{out}");
    out["interface"].clone()
}

fn ask(brain_op: &str, prompt: Value) -> Value {
    call_action(brain_op, rec(vec![("prompt", prompt)]))
}

fn seen(prefix: &str) -> Value {
    op(
        "concat",
        vec![
            lit(json!(prefix)),
            op("str", vec![path("input", &["value"])]),
        ],
    )
}

/// Asks the model, does what it chose, tells it what happened, and goes round until it answers.
fn agent(brain_op: &str, tools: &[&str]) -> Value {
    let keep = || var("state");
    let mut chosen = vec![(
        "answer",
        "c",
        step(keep(), vec![halt(path("c", &["text"]))]),
    )];
    if tools.contains(&"net.fetch") {
        chosen.push((
            "net.fetch",
            "c",
            step(
                keep(),
                vec![call_action(
                    "net.fetch",
                    rec(vec![("url", path("c", &["url"]))]),
                )],
            ),
        ));
    }
    if tools.contains(&"notes.append") {
        chosen.push((
            "notes.append",
            "c",
            step(
                keep(),
                vec![call_action(
                    "notes.append",
                    rec(vec![("text", path("c", &["text"]))]),
                )],
            ),
        ));
    }
    let mut results = vec![(
        brain_op,
        match_(path("input", &["value"]), "kind", chosen, None),
    )];
    if tools.contains(&"net.fetch") {
        results.push((
            "net.fetch",
            step(keep(), vec![ask(brain_op, seen("fetched: "))]),
        ));
    }
    if tools.contains(&"notes.append") {
        results.push((
            "notes.append",
            step(keep(), vec![ask(brain_op, seen("noted: "))]),
        ));
    }
    json!({
        "arg": "String",
        "result": "String",
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![
                ("start", step(keep(), vec![ask(brain_op, path("input", &["value"]))])),
                ("ret", on_op(results, step(keep(), vec![]))),
                ("refused", step(keep(), vec![ask(brain_op, lit(json!("refused")))])),
            ],
            step(keep(), vec![]),
        ),
    })
}

fn started(harness: Value, allowed: Value) -> Value {
    let ready = call(
        json!({ "op": "init", "harness": harness, "interface": interface(), "grant": { "netAllow": allowed } }),
    );
    assert_eq!(ready["kind"], "ready", "{ready}");
    deliver_all(ready["system"].clone(), vec![start(json!("go"))]).0
}

fn full() -> Value {
    agent("brain.choose", &["net.fetch", "notes.append"])
}

fn performed(events: &[Value]) -> Vec<(String, bool)> {
    events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["kind"] == "call")
        .map(|e| {
            (
                e["action"]["op"].as_str().unwrap().to_string(),
                e["performed"].as_bool().unwrap(),
            )
        })
        .collect()
}

fn withheld(events: &[Value]) -> Option<String> {
    events
        .iter()
        .find(|e| e["kind"] == "withheld")
        .map(|e| e["reason"]["kind"].as_str().unwrap().to_string())
}

#[test]
fn what_the_model_chooses_is_done_and_it_hears_back() {
    let sys = started(full(), json!(["https://a"]));
    let (sys, ev) = deliver_all(
        sys,
        vec![ret(0, json!({ "kind": "net.fetch", "url": "https://a" }))],
    );
    assert_eq!(performed(&ev), [("net.fetch".to_string(), true)], "{ev:?}");
    let (sys, ev) = deliver_all(sys, vec![ret(1, json!("page"))]);
    assert_eq!(
        performed(&ev),
        [("brain.choose".to_string(), true)],
        "{ev:?}"
    );
    let (sys, _) = deliver_all(
        sys,
        vec![ret(2, json!({ "kind": "answer", "text": "done" }))],
    );
    assert_eq!(sys["root"]["outcome"]["kind"], "halted", "{sys}");
    assert_eq!(sys["root"]["outcome"]["value"], "done", "{sys}");
}

#[test]
fn a_call_the_contract_forbids_is_refused_whatever_the_model_says() {
    let sys = started(full(), json!([]));
    let (sys, ev) = deliver_all(
        sys,
        vec![ret(
            0,
            json!({ "kind": "net.fetch", "url": "https://evil" }),
        )],
    );
    // Refused at the call, and the model hears so.
    assert_eq!(
        performed(&ev),
        [
            ("net.fetch".to_string(), false),
            ("brain.choose".to_string(), true)
        ],
        "{ev:?}"
    );
    assert_eq!(sys["root"]["outcome"], Value::Null);
}

#[test]
fn a_choice_of_the_wrong_shape_never_reaches_the_harness() {
    for bad in [
        json!({ "kind": "net.fetch", "url": 1 }),
        json!({ "kind": "net.fetch" }),
        json!({ "kind": "shell.run", "argv": ["rm"] }),
        json!("just text"),
    ] {
        let (_, ev) = deliver_all(
            started(full(), json!(["https://a"])),
            vec![ret(0, bad.clone())],
        );
        assert_eq!(withheld(&ev).as_deref(), Some("mismatch"), "{bad} {ev:?}");
        assert_eq!(performed(&ev), [], "{bad} {ev:?}");
    }
}

/// Spawns `reader`, narrowed to what a reader may do, on the first result.
fn parent(child: Value) -> Value {
    let narrow = json!([
        { "kind": "drop", "op": "notes.append" },
        { "kind": "drop", "op": "brain.choose" },
    ]);
    let spawned = rec(vec![
        ("kind", lit(json!("spawn"))),
        ("name", lit(json!("reader"))),
        ("harness", lit(written(&child))),
        ("narrow", lit(written(&narrow))),
        ("arg", path("input", &["value"])),
    ]);
    json!({
        "arg": "String",
        "result": "String",
        "spawns": { "reader": { "arg": "String", "result": "String" } },
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![
                ("start", step(var("state"), vec![spawned])),
                ("ret", on_op(
                    vec![("spawn.reader", step(var("state"), vec![halt(path("input", &["value"]))]))],
                    step(var("state"), vec![]),
                )),
            ],
            step(var("state"), vec![]),
        ),
    })
}

#[test]
fn a_child_is_offered_only_what_its_narrowing_leaves() {
    let child = agent("brain.readonly", &["net.fetch"]);
    let sys = started(parent(child), json!(["https://a"]));
    assert_eq!(
        sys["root"]["children"].as_array().map(Vec::len),
        Some(1),
        "{sys}"
    );
    let to = |d: Value| {
        let mut d = d;
        d["to"] = json!([0]);
        d
    };
    let call_id = sys["root"]["children"][0]["run"]["pending"][0]["call"]
        .as_u64()
        .unwrap();
    // The tool the narrowing took away isn't a case of this model's answer.
    let (sys, ev) = deliver_all(
        sys,
        vec![to(ret(
            call_id,
            json!({ "kind": "notes.append", "text": "x" }),
        ))],
    );
    assert_eq!(withheld(&ev).as_deref(), Some("mismatch"), "{ev:?}");
    let (sys, ev) = deliver_all(
        sys,
        vec![to(ret(
            call_id,
            json!({ "kind": "answer", "text": "read" }),
        ))],
    );
    assert_eq!(withheld(&ev), None, "{ev:?}");
    assert_eq!(sys["root"]["outcome"]["value"], "read", "{sys}");
}

#[test]
fn a_child_that_calls_what_its_narrowing_took_away_can_t_start() {
    let sys = started(parent(full()), json!([]));
    assert_eq!(
        sys["root"]["children"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0),
        0,
        "{sys}"
    );
}

#[test]
fn a_withheld_choice_changes_nothing_and_a_failure_still_ends_the_call() {
    let failure =
        || json!({ "input": { "kind": "failed", "call": 0, "error": "no answer that fits" } });
    let heedless = started(full(), json!(["https://a"]));
    let mut hears = full();
    hears["step"]["cases"]["failed"] = json!({
        "bind": "input",
        "body": step(var("state"), vec![ask("brain.choose", lit(json!("try again")))]),
    });
    let listening = started(hears, json!(["https://a"]));

    let unfit = json!({ "kind": "shell.run", "argv": ["rm"] });
    let (unchanged, ev) = deliver_all(listening.clone(), vec![ret(0, unfit)]);
    assert_eq!(withheld(&ev).as_deref(), Some("mismatch"), "{ev:?}");
    assert_eq!(unchanged, listening);

    // The harness hears the failure, labelled as the model's result would be, as if nothing had been withheld.
    let (after, ev) = deliver_all(unchanged, vec![failure()]);
    let (alone, ev_alone) = deliver_all(listening, vec![failure()]);
    assert_eq!((&after, &ev), (&alone, &ev_alone));
    assert_eq!(ev[0]["kind"], "in", "{ev:?}");
    assert_eq!(
        ev[0]["input"],
        json!({ "kind": "failed", "call": 0, "op": "brain.choose",
                "error": { "kind": "world", "detail": "no answer that fits" } })
    );
    assert_eq!(ev[0]["label"], json!(["model"]));
    assert_eq!(
        performed(&ev),
        [("brain.choose".to_string(), true)],
        "{ev:?}"
    );

    // The call has ended: nothing more can answer it.
    let answer = ret(0, json!({ "kind": "answer", "text": "late" }));
    for late in [answer, failure()] {
        let (same, ev) = deliver_all(after.clone(), vec![late]);
        assert_eq!(withheld(&ev).as_deref(), Some("notPending"), "{ev:?}");
        assert_eq!(same, after);
    }

    // A step that doesn't read the failure hears it and does nothing: no call pending, no outcome, and no event says so.
    let (stuck, ev) = deliver_all(heedless, vec![failure()]);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(stuck["root"]["pending"], json!([]), "{stuck}");
    assert_eq!(stuck["root"]["outcome"], Value::Null, "{stuck}");
}
