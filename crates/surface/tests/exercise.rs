//! The design session's Claude Code exercise, one turn of it: the model streams a reply, which is written to the
//! workspace, while the user may interrupt on their channel. P4, "an interrupt stops effects until the next prompt",
//! holds on its monitored runs and up to a depth, and fails for the same turn without `until`.

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};

fn call(request: Value) -> Value {
    let response = decode(&surface::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

fn capabilities() -> Value {
    let message = json!({ "Record": { "role": "String", "content": "String" } });
    json!({
        "model": { "operations": { "generate": {
            "input": { "Record": { "messages": { "List": message } } },
            "output": "String",
            "mode": "stream",
            "ret": { "e": "list", "items": [{ "e": "lit", "v": "model" }] },
        }}},
        "fs": { "operations": { "write": {
            "input": { "Record": { "path": "String", "text": "String" } },
            "output": "Int",
            "effect": true,
        }}},
        "session": { "channels": { "user": { "type": "String" } } },
    })
}

fn interface() -> Value {
    let installations: Vec<Value> = ["model", "fs", "session"]
        .iter()
        .map(|c| json!({ "prefix": c, "capability": capabilities()[c] }))
        .collect();
    call(json!({ "op": "install", "capabilities": installations }))["interface"].clone()
}

const TURN: &str = r#"
harness turn(prompt: String) -> Any

uses model, fs, session

stop   = receive said from session.user when said == "stop"
chunks = model.generate(messages: [{role: "user", content: prompt}]) until stop
reply  = fold chunks from "" with (text, chunk) => concat(text, chunk)
saved  = fs.write(path: "/workspace/reply.md", text: reply) until stop

return saved

otherwise return "interrupted"
"#;

fn elaborated(source: &str) -> Value {
    let out = call(json!({ "op": "elaborate", "source": source, "capabilities": capabilities() }));
    assert_eq!(out["kind"], "elaborated", "{out:#}");
    out["harness"].clone()
}

fn lit(v: Value) -> Value {
    json!({ "e": "lit", "v": v })
}

fn ev(fields: &[&str]) -> Value {
    fields.iter().fold(
        json!({ "e": "var", "name": "event" }),
        |of, f| json!({ "e": "get", "of": of, "field": f }),
    )
}

fn eq(a: Value, b: Value) -> Value {
    json!({ "e": "op", "op": "eq", "args": [a, b] })
}

fn all(exprs: Vec<Value>) -> Value {
    exprs
        .into_iter()
        .reduce(|a, b| json!({ "e": "op", "op": "and", "args": [a, b] }))
        .unwrap()
}

fn atom(holds: Value) -> Value {
    json!({ "f": "atom", "holds": holds })
}

fn not(of: Value) -> Value {
    json!({ "f": "not", "of": of })
}

fn user_says(text: &str, same: bool) -> Value {
    let op = if same { "eq" } else { "ne" };
    atom(all(vec![
        eq(ev(&["kind"]), lit(json!("in"))),
        eq(ev(&["input", "kind"]), lit(json!("msg"))),
        eq(ev(&["input", "channel"]), lit(json!("session.user"))),
        json!({ "e": "op", "op": op, "args": [ev(&["input", "value"]), lit(json!(text))] }),
    ]))
}

/// P4: no effect is performed after the user says "stop", until they say something else.
fn p4() -> Value {
    let effect = atom(all(vec![
        eq(ev(&["kind"]), lit(json!("out"))),
        eq(ev(&["action", "kind"]), lit(json!("call"))),
        ev(&["effect"]),
    ]));
    let interrupted = json!({ "f": "since", "left": not(user_says("stop", false)), "right": user_says("stop", true) });
    json!({ "name": "p4", "always": { "f": "implies", "if": effect, "then": not(interrupted) } })
}

/// Every event of the run these inputs give, and how it ended.
fn run(harness: &Value, inputs: &[Value]) -> (Vec<Value>, Value) {
    let ready = call(json!({ "op": "init", "harness": harness, "interface": interface() }));
    assert_eq!(ready["kind"], "ready", "{ready}");
    let mut system = ready["system"].clone();
    let mut events = Vec::new();
    for input in inputs {
        let out =
            call(json!({ "op": "deliver", "system": system, "delivery": { "input": input } }));
        system = out["system"].clone();
        events.extend(out["events"].as_array().unwrap().iter().cloned());
    }
    (events, system["root"]["outcome"].clone())
}

fn start() -> Value {
    json!({ "kind": "start", "value": "write a haiku" })
}

fn item(call: u64, value: Value) -> Value {
    json!({ "kind": "ret", "call": call, "value": value })
}

fn end(call: u64) -> Value {
    json!({ "kind": "end", "call": call })
}

fn says(text: &str) -> Value {
    json!({ "kind": "msg", "channel": "session.user", "value": text })
}

fn actions(events: &[Value]) -> Vec<Value> {
    events
        .iter()
        .filter(|e| e["kind"] == "out")
        .map(|e| json!([e["action"]["kind"], e["action"]["op"], e["action"]["args"]]))
        .collect()
}

#[test]
fn a_streamed_reply_is_written_once_it_ends() {
    let harness = elaborated(TURN);
    let (events, outcome) = run(
        &harness,
        &[
            start(),
            item(0, json!("old pond, ")),
            says("keep going"),
            item(0, json!("a frog")),
            end(0),
            item(1, json!(17)),
        ],
    );
    let written = json!({ "path": "/workspace/reply.md", "text": "old pond, a frog" });
    assert_eq!(actions(&events)[1], json!(["call", "fs.write", written]));
    assert_eq!(outcome["value"], json!(17));
}

#[test]
fn an_interrupt_cancels_the_stream_and_ends_the_turn() {
    let harness = elaborated(TURN);
    let (events, outcome) = run(
        &harness,
        &[start(), item(0, json!("old pond")), says("stop")],
    );
    let acted: Vec<Value> = actions(&events).iter().map(|a| a[0].clone()).collect();
    assert_eq!(acted, [json!("call"), json!("cancel"), json!("halt")]);
    assert_eq!(outcome["value"], json!("interrupted"));
}

#[test]
fn p4_holds_on_monitored_runs() {
    let harness = elaborated(TURN);
    let runs = [
        vec![start(), item(0, json!("a")), end(0), item(1, json!(1))],
        vec![start(), item(0, json!("a")), says("stop")],
        vec![start(), says("stop")],
        vec![
            start(),
            item(0, json!("a")),
            end(0),
            says("stop"),
            item(1, json!(1)),
        ],
    ];
    for inputs in runs {
        let (trace, _) = run(&harness, &inputs);
        let verdict = call(json!({ "op": "monitor", "property": p4(), "trace": trace }));
        assert_eq!(verdict["verdict"], "holds", "{inputs:?}: {verdict}");
    }
}

#[test]
fn p4_holds_up_to_a_depth_and_fails_without_until() {
    let alphabet = json!([
        { "input": { "kind": "start", "value": "write a haiku" } },
        { "input": { "kind": "ret", "op": "model.generate", "value": "a" } },
        { "input": { "kind": "end", "op": "model.generate" } },
        { "input": { "kind": "ret", "op": "fs.write", "value": 1 } },
        { "input": { "kind": "msg", "channel": "session.user", "value": "stop" } },
        { "input": { "kind": "msg", "channel": "session.user", "value": "go on" } },
    ]);
    let prove = |harness: Value| {
        let out = call(json!({
            "op": "prove", "harness": harness, "interface": interface(), "property": p4(),
            "method": { "kind": "bounded", "depth": 6, "alphabet": alphabet },
        }));
        assert_eq!(out["kind"], "evidence", "{out}");
        out["evidence"]["verdict"].clone()
    };
    assert_eq!(prove(elaborated(TURN)), "holdsUpTo");
    let heedless = TURN.replace(" until stop", "");
    assert_eq!(prove(elaborated(&heedless)), "refuted");
}

#[test]
fn receives_and_untils_are_checked() {
    let header = "harness h(n: Int) -> Any\nuses model, fs, session\n";
    let problems = |body: &str| {
        let out = call(
            json!({ "op": "elaborate", "source": format!("{header}{body}"), "capabilities": capabilities() }),
        );
        assert_eq!(out["kind"], "problems", "{out:#}");
        out["problems"].as_array().unwrap().clone()
    };
    let p = problems("m = receive from session.nobody\nreturn 1\n");
    assert_eq!(p[0]["kind"], "unknownChannel");

    let p = problems("w = fs.write(path: \"a\", text: \"b\") until nothing\nreturn w\n");
    assert_eq!(
        p[0],
        json!({
            "kind": "until", "name": "nothing",
            "span": { "start": { "line": 3, "column": 42 }, "end": { "line": 3, "column": 49 } },
        })
    );

    let p = problems(
        "w = fs.write(path: \"a\", text: \"b\")\nm = receive x from session.user when x == w\nreturn w\n",
    );
    assert_eq!(p[0]["kind"], "filterReadsNode");

    let p = problems("w = fs.write(path: \"a\", text: \"b\")\nreturn receive from session.user\n");
    assert_eq!(p[0]["kind"], "syntax");
}

#[test]
fn a_received_message_is_a_value() {
    let source = r#"
harness echo(n: Int) -> String
uses session
heard = receive said from session.user when said != ""
return heard
"#;
    let harness = elaborated(source);
    let (_, outcome) = run(
        &harness,
        &[
            json!({ "kind": "start", "value": 1 }),
            says(""),
            says("hello"),
        ],
    );
    assert_eq!(outcome["value"], json!("hello"));
}
