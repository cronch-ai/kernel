//! T1's harness: the Claude Code-style agent in `examples/claude-code/agent.cronch`, against the capabilities beside it.
//! The model chooses a tool; a network tool is performed only after a person allows its URL through `net.ask`. These
//! runs show what the harness does. That it can't reach the network without an allow, whatever it does and whatever
//! the model says, is the rules' claim, proved in `proofs/t1.test.mjs`.

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};

fn call(request: Value) -> Value {
    let response = decode(&surface::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

fn example(name: &str) -> String {
    let path = format!(
        "{}/../../examples/claude-code/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).unwrap()
}

const CAPABILITIES: [&str; 5] = ["brain", "fs", "shell", "net", "session"];

fn capabilities() -> Value {
    CAPABILITIES
        .iter()
        .map(|c| {
            let contract: Value = serde_json::from_str(&example(&format!("{c}.json"))).unwrap();
            (c.to_string(), contract)
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

fn interface() -> Value {
    let installations: Vec<Value> = CAPABILITIES
        .iter()
        .map(|c| json!({ "prefix": c, "capability": capabilities()[c] }))
        .collect();
    let out = call(json!({ "op": "install", "capabilities": installations }));
    assert_eq!(out["kind"], "installed", "{out}");
    out["interface"].clone()
}

fn harness() -> Value {
    let out = call(
        json!({ "op": "elaborate", "source": example("agent.cronch"), "capabilities": capabilities() }),
    );
    assert_eq!(out["kind"], "elaborated", "{out:#}");
    out["harness"].clone()
}

/// The events of the run these inputs give, and the system it ends in.
fn run(inputs: &[Value]) -> (Vec<Value>, Value) {
    let ready = call(json!({ "op": "init", "harness": harness(), "interface": interface() }));
    assert_eq!(ready["kind"], "ready", "{ready}");
    let mut system = ready["system"].clone();
    let mut events = Vec::new();
    for input in inputs {
        let out =
            call(json!({ "op": "deliver", "system": system, "delivery": { "input": input } }));
        system = out["system"].clone();
        let new = out["events"].as_array().unwrap();
        assert!(
            new.iter()
                .all(|e| e["kind"] != "withheld" && e["kind"] != "failed"),
            "{input}: {new:#?}"
        );
        events.extend(new.iter().cloned());
    }
    (events, system)
}

fn start() -> Value {
    json!({ "kind": "start", "value": "fix the build" })
}

fn ret(call: u64, value: Value) -> Value {
    json!({ "kind": "ret", "call": call, "value": value })
}

fn chooses(tool: &str, args: Value) -> Value {
    let mut choice = args;
    choice["kind"] = json!(tool);
    choice
}

fn answers(text: &str) -> Value {
    json!({ "kind": "answer", "text": text })
}

/// The calls the run performed, in order.
fn performed(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["kind"] == "call" && e["performed"] == true)
        .map(|e| e["action"]["op"].as_str().unwrap().to_string())
        .collect()
}

fn halted(system: &Value) -> Value {
    assert_eq!(system["root"]["outcome"]["kind"], "halted", "{system:#}");
    system["root"]["outcome"]["value"].clone()
}

const A: &str = "https://a.example/";
const B: &str = "https://b.example/";

#[test]
fn it_elaborates_against_the_t1_capabilities() {
    let problems =
        call(json!({ "op": "checkHarness", "harness": harness(), "interface": interface() }));
    assert_eq!(problems["problems"], json!([]), "{problems:#}");
}

#[test]
fn a_denied_url_is_not_fetched() {
    let (events, system) = run(&[
        start(),
        ret(0, chooses("net.fetch", json!({ "url": A }))),
        ret(1, json!("deny")),
        ret(2, answers("couldn't fetch it")),
    ]);
    assert_eq!(
        performed(&events),
        ["brain.choose", "net.ask", "brain.choose"]
    );
    assert_eq!(system["root"]["grant"]["netAllow"], json!([]));
    assert_eq!(halted(&system), "couldn't fetch it");
}

#[test]
fn an_allowed_url_is_fetched_and_the_model_hears_the_page() {
    let (events, system) = run(&[
        start(),
        ret(0, chooses("net.fetch", json!({ "url": A }))),
        ret(1, json!("allow")),
        ret(2, json!("<html>")),
        ret(3, answers("done")),
    ]);
    assert_eq!(
        performed(&events),
        ["brain.choose", "net.ask", "net.fetch", "brain.choose"]
    );
    let second = events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["op"] == "brain.choose")
        .nth(1)
        .unwrap();
    let history = second["action"]["args"]["messages"].as_array().unwrap();
    assert_eq!(
        history.last().unwrap(),
        &json!({ "role": "tool", "content": "<html>" })
    );
    assert_eq!(system["root"]["grant"]["netAllow"], json!([A]));
    assert_eq!(halted(&system), "done");
}

#[test]
fn a_local_tool_needs_no_ask() {
    let (events, system) = run(&[
        start(),
        ret(0, chooses("fs.read", json!({ "path": "Cargo.toml" }))),
        ret(1, json!("[workspace]")),
        ret(2, chooses("shell.run", json!({ "command": "cargo build" }))),
        ret(3, json!("ok")),
        ret(4, answers("built")),
    ]);
    assert_eq!(
        performed(&events),
        [
            "brain.choose",
            "fs.read",
            "brain.choose",
            "shell.run",
            "brain.choose"
        ]
    );
    assert_eq!(halted(&system), "built");
}

#[test]
fn a_shell_with_a_network_asks_for_its_url() {
    let (events, _) = run(&[
        start(),
        ret(
            0,
            chooses("shell.online", json!({ "command": "curl", "url": A })),
        ),
        ret(1, json!("allow")),
        ret(2, json!("200")),
    ]);
    assert_eq!(
        performed(&events),
        ["brain.choose", "net.ask", "shell.online", "brain.choose"]
    );
}

#[test]
fn each_network_call_asks_for_its_own_url() {
    let (events, system) = run(&[
        start(),
        ret(0, chooses("net.fetch", json!({ "url": A }))),
        ret(1, json!("allow")),
        ret(2, json!("a")),
        ret(3, chooses("net.fetch", json!({ "url": B }))),
        ret(4, json!("deny")),
        ret(5, answers("only a")),
    ]);
    assert_eq!(
        performed(&events),
        [
            "brain.choose",
            "net.ask",
            "net.fetch",
            "brain.choose",
            "net.ask",
            "brain.choose"
        ]
    );
    let asked: Vec<&Value> = events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["op"] == "net.ask")
        .map(|e| &e["action"]["args"]["url"])
        .collect();
    assert_eq!(asked, [&json!(A), &json!(B)]);
    assert_eq!(system["root"]["grant"]["netAllow"], json!([A]));
}

#[test]
fn the_user_can_stop_it() {
    let (_, system) = run(&[
        start(),
        json!({ "kind": "msg", "channel": "session.user", "value": "stop" }),
    ]);
    assert_eq!(halted(&system), "interrupted");
}
