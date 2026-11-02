//! Spawning: children run against a narrowed interface, share the grant state, answer their parent with labelled
//! results, and are revoked by cancellation or by their parent ending.

mod common;
use common::demo::{actor, tools, worker};
use common::*;
use serde_json::{Value, json};

fn system() -> Value {
    let ready = call(
        json!({ "op": "init", "harness": actor(), "interface": tools(), "grant": { "allow": [], "once": [] } }),
    );
    assert_eq!(ready["kind"], "ready", "{ready}");
    deliver_all(ready["system"].clone(), vec![start(json!({}))]).0
}

fn act(action: Value) -> Value {
    msg("act", action)
}

fn spawn(op_name: &str, args: Value, narrow: Value) -> Value {
    act(
        json!({ "kind": "spawn", "harness": worker(), "narrow": narrow, "arg": { "op": op_name, "args": args } }),
    )
}

fn to(path: Vec<u64>, delivery: Value) -> Value {
    let mut d = delivery;
    d["to"] = json!(path);
    d
}

fn outs(events: &[Value]) -> Vec<(Value, bool)> {
    events
        .iter()
        .filter(|e| e["kind"] == "out")
        .map(|e| (e["origin"].clone(), e["performed"].as_bool().unwrap()))
        .collect()
}

#[test]
fn a_child_calls_through_its_parents_interface_and_answers_it() {
    let (system, events) = deliver_all(
        system(),
        vec![spawn(
            "fs.write",
            json!({ "path": "/workspace/a" }),
            json!([]),
        )],
    );
    assert_eq!(
        shape(&events),
        ["in:msg", "out:spawn", "in:start", "out:call"]
    );
    assert_eq!(events[3]["origin"], json!([0]));
    assert_eq!(events[3]["effect"], json!(true));
    assert_eq!(
        system["root"]["children"][0]["run"]["pending"][0]["op"],
        "fs.write"
    );

    // The world answers the child's call at the child's path; the child halts, and its parent hears.
    let (system, events) = deliver_all(system, vec![to(vec![0], ret(0, json!(1)))]);
    assert_eq!(shape(&events), ["in:ret", "out:halt", "in:ret"]);
    assert_eq!(events[2]["origin"], json!([]));
    assert_eq!(
        events[2]["input"],
        json!({ "kind": "ret", "call": 0, "op": "spawn", "value": 1 })
    );
    assert_eq!(system["root"]["children"], Value::Null);
}

#[test]
fn a_child_cannot_exceed_its_grant() {
    // Dropped: the child can't write at all, though its parent could.
    let (_, events) = deliver_all(
        system(),
        vec![spawn(
            "fs.write",
            json!({ "path": "/workspace/a" }),
            json!([{ "kind": "drop", "op": "fs.write" }]),
        )],
    );
    assert_eq!(events.last().unwrap()["input"]["value"], "denied");

    // Restricted: only under /workspace/sub/, and still never outside /workspace/.
    let sub = op(
        "prefix",
        vec![lit(json!("/workspace/sub/")), path("args", &["path"])],
    );
    let narrow = json!([{ "kind": "restrict", "op": "fs.write", "pre": sub }]);
    let (_, inside) = deliver_all(
        system(),
        vec![spawn(
            "fs.write",
            json!({ "path": "/workspace/sub/a" }),
            narrow.clone(),
        )],
    );
    let (_, outside) = deliver_all(
        system(),
        vec![spawn("fs.write", json!({ "path": "/workspace/a" }), narrow)],
    );
    assert_eq!(outs(&inside).last().unwrap(), &(json!([0]), true));
    assert_eq!(outside.last().unwrap()["input"]["value"], "denied");
}

#[test]
fn approving_a_child_always_widens_its_ancestors() {
    let publish = json!(["npm", "publish"]);
    let run = act(json!({ "kind": "call", "op": "shell.run", "args": { "argv": publish } }));
    let (system, events) = deliver_all(system(), vec![run.clone()]);
    assert_eq!(outs(&events), [(json!([]), false)]);

    let (system, _) = deliver_all(
        system,
        vec![spawn(
            "shell.approve",
            json!({ "argv": publish }),
            json!([]),
        )],
    );
    // The person answers the child's question "always".
    let (system, _) = deliver_all(system, vec![to(vec![1], ret(0, json!("always")))]);
    assert_eq!(system["root"]["grant"]["allow"], json!([publish]));
    let (_, events) = deliver_all(system, vec![run]);
    assert_eq!(outs(&events), [(json!([]), true)]);
}

#[test]
fn an_approval_never_reaches_a_sibling() {
    let publish = json!(["npm", "publish"]);
    // A sibling that stays running: its write is never answered.
    let (system, _) = deliver_all(
        system(),
        vec![
            spawn("fs.write", json!({ "path": "/workspace/a" }), json!([])),
            spawn("shell.approve", json!({ "argv": publish }), json!([])),
        ],
    );
    let (system, _) = deliver_all(system, vec![to(vec![1], ret(0, json!("always")))]);
    assert_eq!(system["root"]["grant"]["allow"], json!([publish]));
    assert_eq!(
        system["root"]["children"][0]["run"]["grant"]["allow"],
        json!([])
    );
}

#[test]
fn what_one_run_uses_up_is_used_up_everywhere() {
    let publish = json!(["npm", "publish"]);
    let run = act(json!({ "kind": "call", "op": "shell.run", "args": { "argv": publish } }));
    let (system, _) = deliver_all(
        system(),
        vec![spawn(
            "shell.approve",
            json!({ "argv": publish }),
            json!([]),
        )],
    );
    let (system, _) = deliver_all(system, vec![to(vec![0], ret(0, json!("once")))]);
    assert_eq!(system["root"]["grant"]["once"], json!([publish]));
    // A child spawned now copies the approval, and uses it: the parent's copy is gone too.
    let (system, events) = deliver_all(
        system,
        vec![spawn("shell.run", json!({ "argv": publish }), json!([]))],
    );
    assert_eq!(outs(&events).last().unwrap(), &(json!([1]), true));
    assert_eq!(system["root"]["grant"]["once"], json!([]));
    let (_, events) = deliver_all(system, vec![run]);
    assert_eq!(outs(&events), [(json!([]), false)]);
}

#[test]
fn a_childs_result_carries_what_it_depended_on() {
    let (system, _) = deliver_all(
        system(),
        vec![spawn(
            "net.fetch",
            json!({ "url": "https://example.com" }),
            json!([]),
        )],
    );
    let (_, events) = deliver_all(system, vec![to(vec![0], ret(0, json!("<html>")))]);
    let parent_hears = events.last().unwrap();
    assert_eq!(parent_hears["origin"], json!([]));
    assert_eq!(parent_hears["label"], json!(["web"]));
}

#[test]
fn cancelling_a_spawn_revokes_the_child() {
    let (system, _) = deliver_all(
        system(),
        vec![spawn(
            "fs.write",
            json!({ "path": "/workspace/a" }),
            json!([]),
        )],
    );
    let (system, events) = deliver_all(system, vec![act(json!({ "kind": "cancel", "call": 0 }))]);
    assert_eq!(shape(&events), ["in:msg", "out:cancel", "in:failed"]);
    assert_eq!(events[2]["input"]["error"], json!({ "kind": "cancelled" }));
    // The child is gone: its call's result has nowhere to go.
    let (_, events) = deliver_all(system, vec![to(vec![0], ret(0, json!(1)))]);
    assert_eq!(shape(&events), ["withheld:noSuchRun"]);
}

#[test]
fn a_spawn_that_cannot_start_is_refused() {
    let mut bad = worker();
    bad["init"] = json!(3);
    let (_, events) = deliver_all(
        system(),
        vec![
            act(json!({ "kind": "spawn", "harness": bad, "arg": { "op": "x" } })),
            spawn(
                "x",
                json!({}),
                json!([{ "kind": "drop", "op": "fs.delete" }]),
            ),
            act(json!({ "kind": "spawn", "harness": worker(), "arg": { "op": 3 } })),
        ],
    );
    let refused: Vec<&Value> = events
        .iter()
        .filter(|e| e["input"]["kind"] == "refused")
        .map(|e| &e["input"]["op"])
        .collect();
    assert_eq!(refused, [&json!("spawn"), &json!("spawn"), &json!("spawn")]);
}

#[test]
fn a_run_that_ends_takes_its_children_with_it() {
    let (system, _) = deliver_all(
        system(),
        vec![
            spawn("fs.write", json!({ "path": "/workspace/a" }), json!([])),
            act(json!({ "kind": "halt", "value": "done" })),
        ],
    );
    assert_eq!(system["root"]["children"], Value::Null);
    let (_, events) = deliver_all(system, vec![to(vec![0], ret(0, json!(1)))]);
    assert_eq!(shape(&events), ["withheld:noSuchRun"]);
}

#[test]
fn runs_that_only_answer_each_other_are_stopped() {
    let quitter = json!({
        "arg": { "Record": {} }, "state": { "Record": {} }, "init": {},
        "step": step(var("state"), vec![halt(lit(json!(0)))]),
    });
    let again = rec(vec![
        ("kind", lit(json!("spawn"))),
        ("harness", lit(quitter)),
        ("arg", rec(vec![])),
    ]);
    let looper = json!({
        "arg": { "Record": {} }, "state": { "Record": {} }, "init": {},
        "step": step(var("state"), vec![again]),
    });
    let ready = call(json!({ "op": "init", "harness": looper, "interface": {} }));
    let (system, events) = deliver_all(ready["system"].clone(), vec![start(json!({}))]);
    assert_eq!(system["root"]["outcome"]["error"]["kind"], "runaway");
    assert_eq!(events.last().unwrap()["kind"], "failed");
}
