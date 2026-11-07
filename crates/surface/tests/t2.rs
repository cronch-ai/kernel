//! T2's harness: the MCP proxy in `examples/mcp-proxy/proxy.pata`. An outside agent plays the model's part: each
//! `agent.next` hands it the last result and waits for its next tool call, which the harness performs through the
//! owner's `mcp:<key>` servers. A tool its owner marked `noEffect` goes through; any other asks first, through
//! `net.ask`, for its server's URL.

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};

fn call(request: Value) -> Value {
    let response = decode(&surface::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

fn contract(name: &str) -> Value {
    let path = format!(
        "{}/../../examples/mcp-proxy/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The owner's servers, by key, and the prefix the harness installs each under.
const SERVERS: [(&str, &str); 2] = [("docs", "docs"), ("mail", "mail")];

/// The capabilities, by the names `uses` gives: the agent, `net` for asking, and each server as `mcp:<key>`.
fn capabilities() -> Value {
    let mut out = serde_json::Map::new();
    out.insert("agent".into(), contract("agent"));
    out.insert("net".into(), contract("net"));
    for (key, _) in SERVERS {
        out.insert(format!("mcp:{key}"), contract(key));
    }
    out.into()
}

/// The interface, built from the server keys, each installed under its own prefix.
fn interface() -> Value {
    let caps = capabilities();
    let mut installations = vec![
        json!({ "prefix": "agent", "capability": caps["agent"] }),
        json!({ "prefix": "net", "capability": caps["net"] }),
    ];
    for (key, prefix) in SERVERS {
        installations.push(json!({ "prefix": prefix, "capability": caps[format!("mcp:{key}")] }));
    }
    let out = call(json!({ "op": "install", "capabilities": installations }));
    assert_eq!(out["kind"], "installed", "{out}");
    out["interface"].clone()
}

fn harness() -> Value {
    let path = format!(
        "{}/../../examples/mcp-proxy/proxy.pata",
        env!("CARGO_MANIFEST_DIR")
    );
    let source = std::fs::read_to_string(path).unwrap();
    let out = call(json!({ "op": "elaborate", "source": source, "capabilities": capabilities() }));
    assert_eq!(out["kind"], "elaborated", "{out:#}");
    out["harness"].clone()
}

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
    json!({ "kind": "start", "value": "owner" })
}

fn ret(call: u64, value: Value) -> Value {
    json!({ "kind": "ret", "call": call, "value": value })
}

/// The agent calls `tool` with `input`, its arguments as JSON text.
fn calls(tool: &str, input: &str) -> Value {
    json!({ "kind": tool, "input": input })
}

fn close() -> Value {
    json!({ "kind": "close" })
}

fn performed(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["kind"] == "call" && e["performed"] == true)
        .map(|e| e["action"]["op"].as_str().unwrap().to_string())
        .collect()
}

/// What each `agent.next` handed the agent: the result of its last call.
fn heard(events: &[Value]) -> Vec<Value> {
    events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["op"] == "agent.next")
        .map(|e| e["action"]["args"]["result"].clone())
        .collect()
}

fn halted(system: &Value) -> Value {
    assert_eq!(system["root"]["outcome"]["kind"], "halted", "{system:#}");
    system["root"]["outcome"]["value"].clone()
}

const MAIL: &str = "https://mail.example/mcp";

#[test]
fn it_elaborates_against_the_owners_servers() {
    let problems =
        call(json!({ "op": "checkHarness", "harness": harness(), "interface": interface() }));
    assert_eq!(problems["problems"], json!([]), "{problems:#}");
}

#[test]
fn an_allowed_call_goes_through() {
    let (events, system) = run(&[
        start(),
        ret(0, calls("docs.search", r#"{"q":"pata"}"#)),
        ret(1, json!("3 results")),
        ret(2, close()),
    ]);
    assert_eq!(
        performed(&events),
        ["agent.next", "docs.search", "agent.next"]
    );
    assert_eq!(heard(&events), [json!(""), json!("3 results")]);
    assert_eq!(halted(&system), "closed");
}

#[test]
fn an_unallowed_call_asks_and_goes_through_once_allowed() {
    let (events, system) = run(&[
        start(),
        ret(0, calls("mail.send", r#"{"to":"a@example.com"}"#)),
        ret(1, json!("allow")),
        ret(2, json!("sent")),
        ret(3, close()),
    ]);
    assert_eq!(
        performed(&events),
        ["agent.next", "net.ask", "mail.send", "agent.next"]
    );
    let asked: Vec<&Value> = events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["op"] == "net.ask")
        .map(|e| &e["action"]["args"]["url"])
        .collect();
    assert_eq!(asked, [&json!(MAIL)]);
    assert_eq!(heard(&events), [json!(""), json!("sent")]);
    assert_eq!(system["root"]["grant"]["netAllow"], json!([MAIL]));
    assert_eq!(halted(&system), "closed");
}

#[test]
fn a_denied_call_is_not_performed() {
    let (events, system) = run(&[
        start(),
        ret(0, calls("mail.send", r#"{"to":"a@example.com"}"#)),
        ret(1, json!("deny")),
        ret(2, close()),
    ]);
    assert_eq!(performed(&events), ["agent.next", "net.ask", "agent.next"]);
    assert_eq!(heard(&events), [json!(""), json!("denied")]);
    assert_eq!(system["root"]["grant"]["netAllow"], json!([]));
    assert_eq!(halted(&system), "closed");
}
