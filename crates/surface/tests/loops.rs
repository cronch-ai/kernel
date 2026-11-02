//! Agentic loops: a harness that runs its graph round after round, carrying values from each round to the next, and
//! performs actions a model chose. The agent below is the plan's Claude Code agent loop, with `notes` as its only
//! tool: it asks the model what to do, asks the owner to approve it, performs it, and goes round again until the model
//! answers, the user interrupts, or it has taken three turns.

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

/// A model that answers or chooses a call of whatever the interface it is called in allows.
fn brain() -> Value {
    let message = json!({ "Record": { "role": "String", "content": "String" } });
    let reply = json!({ "Variant": { "tag": "kind", "cases": {
        "answer": { "Record": { "text": "String" } },
        "call": "Call",
    }}});
    json!({ "operations": { "choose": {
        "input": { "Record": { "messages": { "List": message } } },
        "output": reply,
        "ret": { "e": "list", "items": [{ "e": "lit", "v": "model" }] },
    }}})
}

fn capabilities() -> Value {
    json!({
        "brain": brain(),
        "approve": fixture("approve"),
        "notes": fixture("notes"),
        "session": { "channels": { "user": { "type": "String" } } },
    })
}

fn interface() -> Value {
    let installations: Vec<Value> = ["brain", "approve", "notes", "session"]
        .iter()
        .map(|c| json!({ "prefix": c, "capability": capabilities()[c] }))
        .collect();
    call(json!({ "op": "install", "capabilities": installations }))["interface"].clone()
}

const AGENT: &str = r#"
harness agent(task: String) -> String

uses brain, approve, notes, session

type Message = {role: String, content: String}

carry history: List[Message] = [{role: "user", content: task}]
carry turns: Int = 0

stop   = receive said from session.user when said == "stop"
reply  = brain.choose(messages: history) until stop
choice = reply
wants  = match choice { call(c) => c.op, answer(a) => "" }
asked  = approve.ask(op: wants, call: str(choice)) when wants != "" until stop
result = perform choice when (match asked { ok(a) => a != "deny", _ => false }) until stop

answer = match choice { answer(a) => a.text, call(c) => "" }
seen   = match result {
  ok(v)     => str(v)
  failed(e) => concat("failed: ", e)
  refused   => "not allowed"
  skipped   => "denied"
}
heard  = match stop { ok(s) => true, skipped => false }

next history = append(append(history, {role: "assistant", content: str(choice)}), {role: "tool", content: seen})
next turns   = turns + 1

return (if heard then "interrupted" else answer) when heard or answer != "" or turns >= 3

otherwise return "interrupted"

property approved_since_chosen:
  effect() implies (not returned("brain.choose")) since returned("approve.ask", value in ["once", "always"])

invariant bounded: turns <= 3
"#;

fn elaborate(source: &str) -> Value {
    call(json!({ "op": "elaborate", "source": source, "capabilities": capabilities() }))
}

fn elaborated(source: &str) -> Value {
    let out = elaborate(source);
    assert_eq!(out["kind"], "elaborated", "{out:#}");
    out
}

fn problems(source: &str) -> Vec<Value> {
    let out = elaborate(source);
    assert_eq!(out["kind"], "problems", "{out:#}");
    out["problems"].as_array().unwrap().clone()
}

fn no_grant() -> Value {
    json!({ "allow": [], "once": [] })
}

/// Every event of the run these inputs give, and the system it ends in.
fn run(harness: &Value, grant: Value, inputs: &[Value]) -> (Vec<Value>, Value) {
    let ready =
        call(json!({ "op": "init", "harness": harness, "interface": interface(), "grant": grant }));
    assert_eq!(ready["kind"], "ready", "{ready}");
    let mut system = ready["system"].clone();
    let mut events = Vec::new();
    for input in inputs {
        let out =
            call(json!({ "op": "deliver", "system": system, "delivery": { "input": input } }));
        system = out["system"].clone();
        events.extend(out["events"].as_array().unwrap().iter().cloned());
    }
    (events, system)
}

fn start() -> Value {
    json!({ "kind": "start", "value": "keep a note" })
}

fn ret(call: u64, value: Value) -> Value {
    json!({ "kind": "ret", "call": call, "value": value })
}

fn says(text: &str) -> Value {
    json!({ "kind": "msg", "channel": "session.user", "value": text })
}

fn chooses(op: &str, args: Value) -> Value {
    json!({ "kind": "call", "op": op, "args": args })
}

fn answers(text: &str) -> Value {
    json!({ "kind": "answer", "text": text })
}

/// What the run did: each action's kind, and its operation for a call.
fn actions(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["kind"] == "out")
        .map(|e| {
            let a = &e["action"];
            match a["op"].as_str() {
                Some(op) => format!("{} {op}", a["kind"].as_str().unwrap()),
                None => a["kind"].as_str().unwrap().to_string(),
            }
        })
        .collect()
}

#[test]
fn the_agent_goes_round_until_the_model_answers() {
    let harness = elaborated(AGENT)["harness"].clone();
    let note = json!({ "text": "buy milk" });
    let (events, system) = run(
        &harness,
        no_grant(),
        &[
            start(),
            ret(0, chooses("notes.append", note.clone())),
            ret(1, json!("once")),
            ret(2, json!("note-1")),
            ret(3, answers("noted")),
        ],
    );
    assert_eq!(
        actions(&events),
        [
            "call brain.choose",
            "call approve.ask",
            "call notes.append",
            "call brain.choose",
            "halt"
        ]
    );
    // The model's choice is performed as it chose it.
    let performed = events
        .iter()
        .find(|e| e["kind"] == "out" && e["action"]["op"] == "notes.append")
        .unwrap();
    assert_eq!(performed["action"]["args"], note);
    // The second round's model sees the first round's choice and what came of it.
    let second = events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["op"] == "brain.choose")
        .nth(1)
        .unwrap();
    let history = &second["action"]["args"]["messages"];
    assert_eq!(history.as_array().unwrap().len(), 3);
    // What came of it is the result as it arrived, with the operation that gave it.
    assert_eq!(history[2]["role"], "tool");
    let result: Value = serde_json::from_str(history[2]["content"].as_str().unwrap()).unwrap();
    assert_eq!(result["op"], "notes.append");
    assert_eq!(result["value"], "note-1");
    assert_eq!(system["root"]["outcome"]["value"], json!("noted"));
}

#[test]
fn a_choice_that_is_denied_or_not_an_action_is_not_performed() {
    let harness = elaborated(AGENT)["harness"].clone();
    // Denied: the owner says no.
    let (events, _) = run(
        &harness,
        no_grant(),
        &[
            start(),
            ret(0, chooses("notes.append", json!({ "text": "x" }))),
            ret(1, json!("deny")),
        ],
    );
    assert_eq!(
        actions(&events),
        ["call brain.choose", "call approve.ask", "call brain.choose"]
    );
    // Not a call the interface has: its result type says so, so it is withheld, and the harness never hears of it.
    let system = run(&harness, no_grant(), &[start()]).1;
    let delivered = call(json!({
        "op": "deliver", "system": system,
        "delivery": { "input": ret(0, chooses("notes.delete", json!({}))) },
    }));
    assert_eq!(delivered["events"][0]["kind"], "withheld", "{delivered}");
    assert_eq!(delivered["events"][0]["reason"]["kind"], "mismatch");
}

#[test]
fn a_performed_call_the_grant_forbids_is_refused() {
    // Without the approval guard, the choice is performed at once, alongside the approval, and the precondition
    // refuses it.
    let heedless = AGENT.replace(
        r#" when (match asked { ok(a) => a != "deny", _ => false })"#,
        "",
    );
    let harness = elaborated(&heedless)["harness"].clone();
    let (events, _) = run(
        &harness,
        no_grant(),
        &[
            start(),
            ret(0, chooses("notes.append", json!({ "text": "x" }))),
        ],
    );
    let refused = |e: &&Value| e["kind"] == "in" && e["input"]["kind"] == "refused";
    assert_eq!(events.iter().filter(refused).count(), 1, "{events:#?}");
    assert_eq!(
        actions(&events),
        ["call brain.choose", "call approve.ask", "call notes.append"]
    );
}

#[test]
fn an_interrupt_ends_the_turn_wherever_it_comes() {
    let harness = elaborated(AGENT)["harness"].clone();
    // While the model thinks: its call is cancelled, and its failure ends the run through `otherwise`.
    let (events, system) = run(&harness, no_grant(), &[start(), says("stop")]);
    assert_eq!(actions(&events), ["call brain.choose", "cancel", "halt"]);
    assert_eq!(system["root"]["outcome"]["value"], json!("interrupted"));
    // While the owner decides: the approval is cancelled, nothing is performed, and the round ends interrupted.
    let (events, system) = run(
        &harness,
        no_grant(),
        &[
            start(),
            ret(0, chooses("notes.append", json!({ "text": "x" }))),
            says("stop"),
        ],
    );
    assert_eq!(
        actions(&events),
        ["call brain.choose", "call approve.ask", "cancel", "halt"]
    );
    assert_eq!(system["root"]["outcome"]["value"], json!("interrupted"));
    // Something else said doesn't interrupt.
    let (events, _) = run(
        &harness,
        no_grant(),
        &[
            start(),
            says("hurry"),
            ret(0, chooses("notes.append", json!({ "text": "x" }))),
        ],
    );
    assert_eq!(actions(&events), ["call brain.choose", "call approve.ask"]);
}

#[test]
fn the_rounds_are_bounded() {
    let out = elaborated(AGENT);
    let harness = out["harness"].clone();
    let note = || chooses("notes.append", json!({ "text": "x" }));
    let (events, system) = run(
        &harness,
        no_grant(),
        &[
            start(),
            ret(0, note()),
            ret(1, json!("deny")),
            ret(2, note()),
            ret(3, json!("deny")),
            ret(4, note()),
            ret(5, json!("deny")),
            ret(6, note()),
            ret(7, json!("deny")),
        ],
    );
    // `when` reads the count before `next` moves it, so the fourth round is the last.
    assert_eq!(system["root"]["outcome"]["value"], json!(""));
    assert_eq!(
        actions(&events)
            .iter()
            .filter(|a| *a == "call brain.choose")
            .count(),
        4
    );
    // The invariant reads the carried count, and holds of every state the run passed through.
    let invariant = out["invariants"][0]["expr"].clone();
    for e in events.iter().filter(|e| e["kind"] == "in") {
        let held =
            call(json!({ "op": "evaluate", "expr": invariant, "vars": { "state": e["state"] } }));
        assert_eq!(held["value"], json!(true), "{}", e["state"]);
    }
    let last = events.iter().rev().find(|e| e["kind"] == "in").unwrap();
    let mut past = last["state"].clone();
    past["carry"]["turns"] = json!(4);
    let held = call(json!({ "op": "evaluate", "expr": invariant, "vars": { "state": past } }));
    assert_eq!(held["value"], json!(false));
}

#[test]
fn approval_since_the_last_choice_holds_up_to_a_depth_and_fails_without_it() {
    let alphabet = json!([
        { "input": start() },
        { "input": { "kind": "ret", "op": "brain.choose", "value": chooses("notes.append", json!({ "text": "x" })) } },
        { "input": { "kind": "ret", "op": "brain.choose", "value": answers("done") } },
        { "input": { "kind": "ret", "op": "approve.ask", "value": "always" } },
        { "input": { "kind": "ret", "op": "approve.ask", "value": "deny" } },
        { "input": { "kind": "ret", "op": "notes.append", "value": "n" } },
        { "input": says("stop") },
    ]);
    // Under a grant that already allows appends, only the harness stands between the model and the notes.
    let allowed = json!({ "allow": ["notes.append"], "once": [] });
    let prove = |source: &str| {
        let out = elaborated(source);
        let out = call(json!({
            "op": "prove", "harness": out["harness"], "interface": interface(), "grant": allowed,
            "property": out["properties"][0],
            "method": { "kind": "bounded", "depth": 6, "alphabet": alphabet },
        }));
        assert_eq!(out["kind"], "evidence", "{out}");
        out["evidence"]["verdict"].clone()
    };
    assert_eq!(prove(AGENT), "holdsUpTo");
    let heedless = AGENT.replace(
        r#" when (match asked { ok(a) => a != "deny", _ => false })"#,
        "",
    );
    assert_eq!(prove(&heedless), "refuted");
}

#[test]
fn a_loop_without_calls_to_wait_on_stalls() {
    let counter = r#"
harness count(n: Int) -> {failed: String}
uses notes
carry k: Int = n
seen = notes.read()
next k = k + 1
return {failed: str(k)} when k >= n + 2
"#;
    let harness = elaborated(counter)["harness"].clone();
    let (events, system) = run(
        &harness,
        no_grant(),
        &[
            json!({ "kind": "start", "value": 5 }),
            ret(0, json!([])),
            ret(1, json!([])),
            ret(2, json!([])),
        ],
    );
    assert_eq!(actions(&events).len(), 4);
    assert_eq!(system["root"]["outcome"]["value"], json!({ "failed": "7" }));

    // Every call skipped: the next round would wait on nothing, so the run stalls.
    let idle = counter.replace("notes.read()", "notes.read() when k < 0");
    let harness = elaborated(&idle)["harness"].clone();
    let (_, system) = run(
        &harness,
        no_grant(),
        &[json!({ "kind": "start", "value": 5 })],
    );
    assert_eq!(
        system["root"]["outcome"]["value"],
        json!({ "failed": "round" })
    );
    let idle = format!("{idle}otherwise return {{failed: str(k)}}\n");
    let harness = elaborated(&idle)["harness"].clone();
    let (_, system) = run(
        &harness,
        no_grant(),
        &[json!({ "kind": "start", "value": 5 })],
    );
    assert_eq!(system["root"]["outcome"]["value"], json!({ "failed": "6" }));
}

#[test]
fn loops_are_checked() {
    let header = "harness h(n: Int) -> String\nuses brain, notes, session\n";
    let kinds = |body: &str| -> Vec<String> {
        problems(&format!("{header}{body}"))
            .iter()
            .map(|p| p["kind"].as_str().unwrap().to_string())
            .collect()
    };
    let read = "seen = notes.read()\n";
    assert_eq!(
        kinds(&format!("{read}next n = 1\nreturn \"\" when true\n")),
        ["notACarry"]
    );
    assert_eq!(
        kinds(&format!(
            "carry k: Int = 0\n{read}next k = 1\nreturn \"\"\n"
        )),
        ["nextWithoutLoop"]
    );
    assert_eq!(
        kinds(&format!(
            "carry k: Int = 0\n{read}next k = 1\nnext k = 2\nreturn \"\" when true\notherwise return \"\"\n"
        )),
        ["duplicate"]
    );
    assert_eq!(
        kinds(&format!(
            "carry k: Int = len(seen)\n{read}return \"\" when true\notherwise return \"\"\n"
        )),
        ["carryReads"]
    );
    assert_eq!(
        kinds(&format!(
            "carry j: Int = 0\ncarry k: Int = j\n{read}return \"\" when true\notherwise return \"\"\n"
        )),
        ["carryReads"]
    );
    assert!(
        kinds(&format!("carry k: Int = 0\n{read}return \"\" when k > 1\n"))
            .contains(&"roundNeedsOtherwise".to_string())
    );
    assert_eq!(
        kinds(&format!(
            "carry k: String = \"\"\n{read}next k = 1\nreturn \"\" when true\notherwise return \"\"\n"
        )),
        ["mismatch"]
    );
    // A performed call may be skipped, so the end of a round reads it through `match`.
    assert_eq!(
        kinds(
            "carry k: String = \"\"\nr = brain.choose(messages: [])\nx = perform r\nnext k = str(x)\nreturn \"\" when true\notherwise return \"\"\n"
        ),
        ["mayBeSkipped"]
    );
    // Only a call can be performed: a value of any other type has nothing to say what it would do.
    assert_eq!(
        kinds("x = perform n\ny = match x { ok(v) => 1, _ => 0 }\nreturn str(y)\n"),
        ["notPerformable"]
    );
    assert_eq!(
        kinds(
            "x = perform {kind: \"call\", op: \"notes.read\", args: {}}\ny = match x { ok(v) => 1, _ => 0 }\nreturn str(y)\n"
        ),
        ["notPerformable"]
    );
    assert_eq!(kinds("x = 1 + perform n\nreturn \"\"\n"), ["syntax"]);
    // A receive read through `match` hasn't always heard anything.
    assert_eq!(
        kinds("m = receive from session.user\nb = match m { ok(s) => s }\nreturn b\n"),
        ["uncovered"]
    );
}

#[test]
fn a_loops_invariant_has_an_induction_obligation() {
    let source = r#"
harness count(n: Int) -> Int
uses notes
carry k: Int = 0
saved = notes.append(text: "x")
next k = k + 1
return k when k >= 3
otherwise return -1
invariant small: k <= 3
"#;
    let out = elaborated(source);
    let obligation = call(json!({
        "op": "obligation", "harness": out["harness"], "interface": interface(),
        "invariant": out["invariants"][0]["expr"],
    }));
    assert_eq!(obligation["kind"], "obligation", "{obligation}");
    assert_eq!(obligation["initiation"], json!(true));
    assert_eq!(
        obligation["reads"][0]["Record"]["carry"],
        json!({ "Record": { "k": "Int" } })
    );

    // The agent's too. Its state and input are typed, including what it performs and what comes back, so `Json` is
    // nowhere in the query. Only `str` of a typed value is text a solver chooses.
    let agent = elaborated(AGENT);
    let obligation = call(json!({
        "op": "obligation", "harness": agent["harness"], "interface": interface(),
        "invariant": agent["invariants"][0]["expr"],
    }));
    assert_eq!(obligation["kind"], "obligation", "{obligation}");
    let text = obligation["text"].as_str().unwrap();
    assert!(!text.contains("|json:"), "{text}");
    assert!(text.contains("(declare-fun |fn:text."), "{text}");
}
