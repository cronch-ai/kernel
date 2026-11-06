//! The contracts in `examples/claude-code/`, at run level through the kernel's interface: no harness of T1 here, only
//! one that makes whatever call it is told to. What `net.fetch` and `shell.online` may do depends on `netAllow` alone.

mod common;
use common::*;
use serde_json::{Value, json};

const CAPABILITIES: [&str; 5] = ["brain", "fs", "shell", "net", "session"];

fn capability(name: &str) -> Value {
    let path = format!(
        "{}/../../examples/claude-code/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn installed() -> Value {
    let installations: Vec<Value> = CAPABILITIES
        .iter()
        .map(|c| json!({ "prefix": c, "capability": capability(c) }))
        .collect();
    let out = call(json!({ "op": "install", "capabilities": installations }));
    assert_eq!(out["kind"], "installed", "{out}");
    out["interface"].clone()
}

/// Makes the call a message on `act` holds.
fn puppet() -> Value {
    json!({
        "arg": "Int",
        "result": "String",
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![("msg", match_(
                var("input"),
                "channel",
                vec![("act", "m", acting(var("state"), list(vec![path("m", &["value"])])))],
                Some(step(var("state"), vec![])),
            ))],
            step(var("state"), vec![]),
        ),
    })
}

fn init_with(harness: Value) -> Value {
    call(json!({ "op": "init", "harness": harness, "interface": acting_on(installed()) }))
}

/// Started, and a person has allowed each of `allowed` through `net.ask`: the one way an entry gets in.
fn started(allowed: &[&str]) -> Value {
    let ready = init_with(puppet());
    assert_eq!(ready["kind"], "ready", "{ready}");
    let mut system = deliver_all(ready["system"].clone(), vec![start(json!(0))]).0;
    for (k, url) in allowed.iter().enumerate() {
        let asking = act("net.ask", json!({ "url": url }));
        system = deliver_all(system, vec![asking, ret(k as u64, json!("allow"))]).0;
    }
    assert_eq!(system["root"]["grant"]["netAllow"], json!(allowed));
    system
}

fn act(operation: &str, args: Value) -> Value {
    msg(
        "act",
        json!({ "kind": "call", "op": operation, "args": args }),
    )
}

/// What the kernel did with the one call this delivery makes.
fn verdict(system: Value, operation: &str, args: Value) -> (Value, bool) {
    let (system, events) = deliver_all(system, vec![act(operation, args)]);
    let outs: Vec<&Value> = events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["op"] == operation)
        .collect();
    assert_eq!(outs.len(), 1, "{events:?}");
    (system, outs[0]["performed"] == true)
}

fn fetch(url: &str) -> (&'static str, Value) {
    ("net.fetch", json!({ "url": url }))
}

fn online(url: &str) -> (&'static str, Value) {
    ("shell.online", json!({ "command": "curl", "url": url }))
}

const A: &str = "https://a.example/";
const B: &str = "https://b.example/";

#[test]
fn the_contracts_install_and_share_one_grant() {
    let interface = installed();
    assert_eq!(
        interface["grant"],
        json!({ "Record": { "netAllow": { "List": "String" } } })
    );
    let mut names: Vec<&String> = interface["operations"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "brain.choose",
            "fs.read",
            "fs.write",
            "net.ask",
            "net.fetch",
            "shell.online",
            "shell.run"
        ]
    );
}

#[test]
fn the_model_is_offered_each_tool_with_its_input_and_cannot_ask() {
    let interface = installed();
    let cases = interface["operations"]["brain.choose"]["output"]["Variant"]["cases"]
        .as_object()
        .unwrap()
        .clone();
    let mut tools: Vec<&String> = cases.keys().filter(|c| *c != "answer").collect();
    tools.sort();
    assert_eq!(
        tools,
        [
            "fs.read",
            "fs.write",
            "net.fetch",
            "shell.online",
            "shell.run"
        ]
    );
    for tool in tools {
        assert_eq!(
            cases[tool], interface["operations"][tool]["input"],
            "{tool}"
        );
    }
}

#[test]
fn every_run_starts_with_no_entries() {
    let out = init_with(puppet());
    assert_eq!(out["kind"], "ready", "{out}");
    assert_eq!(out["system"]["root"]["grant"], json!({ "netAllow": [] }));
}

#[test]
fn network_calls_are_refused_without_an_entry() {
    for allowed in [&[][..], &[B][..], &["https://a.example"][..]] {
        for (operation, args) in [fetch(A), online(A)] {
            let (_, performed) = verdict(started(allowed), operation, args);
            assert!(!performed, "{operation} under {allowed:?}");
        }
    }
}

#[test]
fn network_calls_are_performed_with_an_entry() {
    for (operation, args) in [fetch(A), online(A)] {
        let (_, performed) = verdict(started(&[A]), operation, args);
        assert!(performed, "{operation}");
    }
}

#[test]
fn an_entry_stands_and_covers_both_network_tools() {
    let (system, _) = verdict(started(&[A]), "net.fetch", json!({ "url": A }));
    let (system, performed) = verdict(system, "net.fetch", json!({ "url": A }));
    assert!(performed);
    let (_, performed) = verdict(system, "shell.online", online(A).1);
    assert!(performed);
}

#[test]
fn only_a_person_s_allow_adds_an_entry() {
    // Each call in a run takes the next identifier.
    let asked = |answer: Value| {
        let (system, performed) = verdict(started(&[]), "net.ask", json!({ "url": A }));
        assert!(performed);
        deliver_all(system, vec![ret(0, answer)])
    };
    let (system, events) = asked(json!("allow"));
    assert!(events.iter().all(|e| e["kind"] != "withheld"), "{events:?}");
    let (system, performed) = verdict(system, "net.fetch", json!({ "url": A }));
    assert!(performed);
    let (_, performed) = verdict(system, "net.fetch", json!({ "url": B }));
    assert!(!performed, "an approval is for its URL");

    let (system, _) = asked(json!("deny"));
    let (_, performed) = verdict(system, "shell.online", online(A).1);
    assert!(!performed);

    for bad in [json!("yes"), json!(true)] {
        let (system, events) = asked(bad.clone());
        assert!(
            events.iter().any(|e| e["kind"] == "withheld"),
            "{bad}: {events:?}"
        );
        let (_, performed) = verdict(system, "net.fetch", json!({ "url": A }));
        assert!(!performed, "{bad}");
    }
}

#[test]
fn what_is_not_network_needs_no_entry() {
    // `shell.run`'s lack of a network is the host's, so its contract has nothing to check. `fs` is unrestricted.
    for (operation, args) in [
        ("shell.run", json!({ "command": "ls" })),
        ("fs.read", json!({ "path": "/etc/passwd" })),
        ("fs.write", json!({ "path": "/etc/passwd", "text": "x" })),
    ] {
        let (_, performed) = verdict(started(&[]), operation, args);
        assert!(performed, "{operation}");
    }
}

#[test]
fn malformed_arguments_are_refused_before_the_run() {
    let calling = |operation: &str, args: Value| {
        json!({
            "arg": "Int",
            "result": "String",
            "state": { "Record": {} },
            "init": {},
            "step": step(var("state"), vec![call_action(operation, args)]),
        })
    };
    for (operation, args) in [
        ("net.fetch", rec(vec![("url", lit(json!(1)))])),
        ("net.fetch", rec(vec![])),
        ("net.ask", rec(vec![("url", lit(json!(1)))])),
        ("shell.online", rec(vec![("command", lit(json!("curl")))])),
        ("shell.online", rec(vec![("url", lit(json!(A)))])),
        ("shell.run", rec(vec![("command", lit(json!(1)))])),
        ("fs.write", rec(vec![("path", lit(json!("/x")))])),
        ("net.delete", rec(vec![("url", lit(json!(A)))])),
    ] {
        let out = init_with(calling(operation, args.clone()));
        assert_eq!(out["kind"], "refused", "{operation} {args}: {out}");
    }
}

#[test]
fn malformed_arguments_at_run_time_do_not_reach_a_call() {
    for (operation, args) in [
        ("net.fetch", json!({ "url": 1 })),
        ("net.fetch", json!({})),
        ("shell.online", json!({ "url": A })),
        ("net.delete", json!({ "url": A })),
    ] {
        let (_, events) = deliver_all(started(&[A]), vec![act(operation, args.clone())]);
        let reason = events.iter().find(|e| e["kind"] == "withheld");
        assert_eq!(
            reason.expect("withheld")["reason"]["kind"],
            "mismatch",
            "{args}"
        );
        assert!(events.iter().all(|e| e["kind"] != "out"), "{events:?}");
    }
}
