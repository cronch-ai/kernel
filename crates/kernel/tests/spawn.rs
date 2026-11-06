//! Spawning: children run against a narrowed interface, share the grant state, answer their parent with labelled
//! results, and are revoked by cancellation or by their parent ending.

mod common;
use common::demo::{actor, tools, worker};
use common::*;
use serde_json::{Value, json};

fn system() -> Value {
    let ready = call(json!({ "op": "init", "harness": actor(), "interface": tools() }));
    assert_eq!(ready["kind"], "ready", "{ready}");
    deliver_all(ready["system"].clone(), vec![start(json!({}))]).0
}

fn act(action: Value) -> Value {
    msg("act", action)
}

fn spawn(op_name: &str, args: Value, narrow: Value) -> Value {
    act(
        json!({ "kind": "spawn", "name": "worker", "harness": written(&worker()), "narrow": written(&narrow), "arg": { "kind": "call", "op": op_name, "args": args } }),
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
    assert_eq!(events[2]["input"]["op"], "spawn.worker");
    let said: Value = serde_json::from_str(events[2]["input"]["value"].as_str().unwrap()).unwrap();
    assert_eq!(said["value"], 1);
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
    // Its start value wouldn't be a call of the child's, so the spawn is refused.
    assert_eq!(events.last().unwrap()["input"]["kind"], "refused");
    assert_eq!(events.last().unwrap()["input"]["op"], "spawn.worker");

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
    let write = json!({ "kind": "call", "op": "fs.write", "args": { "path": "/workspace/a" } });
    let spawning = |harness: Value, narrow: Value| {
        act(
            json!({ "kind": "spawn", "name": "worker", "harness": harness, "narrow": narrow, "arg": write }),
        )
    };
    let (_, events) = deliver_all(
        system(),
        vec![
            // An ill-formed harness, a narrowing of what the interface lacks, and text that isn't a harness.
            spawning(written(&bad), json!("[]")),
            spawning(
                written(&worker()),
                written(&json!([{ "kind": "drop", "op": "fs.delete" }])),
            ),
            spawning(json!("not a harness"), json!("[]")),
        ],
    );
    let refused: Vec<&Value> = events
        .iter()
        .filter(|e| e["input"]["kind"] == "refused")
        .map(|e| &e["input"]["op"])
        .collect();
    assert_eq!(
        refused,
        [
            &json!("spawn.worker"),
            &json!("spawn.worker"),
            &json!("spawn.worker")
        ]
    );
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
        "result": "String", "arg": { "Record": {} }, "state": { "Record": {} }, "init": {},
        "step": step(var("state"), vec![halt(lit(json!(0)))]),
    });
    let again = rec(vec![
        ("kind", lit(json!("spawn"))),
        ("name", lit(json!("quitter"))),
        ("harness", lit(written(&quitter))),
        ("narrow", lit(json!("[]"))),
        ("arg", rec(vec![])),
    ]);
    let looper = json!({
        "result": "String", "arg": { "Record": {} }, "state": { "Record": {} }, "init": {},
        "spawns": { "quitter": { "arg": { "Record": {} }, "result": "String" } },
        "step": step(var("state"), vec![again]),
    });
    let ready = call(json!({ "op": "init", "harness": looper, "interface": {} }));
    let (system, events) = deliver_all(ready["system"].clone(), vec![start(json!({}))]);
    assert_eq!(system["root"]["outcome"]["error"]["kind"], "runaway");
    assert_eq!(events.last().unwrap()["kind"], "failed");
}

#[test]
fn a_spawn_must_name_a_declared_child() {
    // A spawn is a variant over the declared children: one naming no child, or one the harness doesn't declare,
    // isn't an action of it, so its message is withheld.
    let write = json!({ "kind": "call", "op": "fs.write", "args": { "path": "/workspace/a" } });
    for action in [
        json!({ "kind": "spawn", "harness": written(&worker()), "narrow": "[]", "arg": write }),
        json!({ "kind": "spawn", "name": "stranger", "harness": written(&worker()), "narrow": "[]", "arg": write }),
    ] {
        let (system, events) = deliver_all(system(), vec![act(action)]);
        assert_eq!(system["root"]["outcome"], Value::Null, "{events:#?}");
        assert_eq!(shape(&events), ["withheld:mismatch"]);
    }
}

#[test]
fn a_spawn_carries_exactly_its_declared_fields() {
    let write = json!({ "kind": "call", "op": "fs.write", "args": { "path": "/workspace/a" } });
    for action in [
        // The child's harness as a term, not text: a term has no Cronch type.
        json!({ "kind": "spawn", "name": "worker", "harness": worker(), "narrow": "[]", "arg": write }),
        // An argument outside the signature's `arg`.
        json!({ "kind": "spawn", "name": "worker", "harness": written(&worker()), "narrow": "[]", "arg": { "op": 3 } }),
        // No narrowing.
        json!({ "kind": "spawn", "name": "worker", "harness": written(&worker()), "arg": write }),
        // A field the action doesn't declare.
        json!({ "kind": "spawn", "name": "worker", "harness": written(&worker()), "narrow": "[]", "arg": write, "x": 1 }),
    ] {
        let (_, events) = deliver_all(system(), vec![act(action)]);
        assert_eq!(shape(&events), ["withheld:mismatch"], "{events:#?}");
    }
}

#[test]
fn a_step_that_spawns_an_undeclared_child_does_not_check() {
    let spawned = rec(vec![
        ("kind", lit(json!("spawn"))),
        ("name", lit(json!("stranger"))),
        ("harness", lit(written(&worker()))),
        ("narrow", lit(json!("[]"))),
        ("arg", rec(vec![])),
    ]);
    let harness = json!({
        "result": "String", "arg": { "Record": {} }, "state": { "Record": {} }, "init": {},
        "step": step(var("state"), vec![spawned]),
    });
    let problems =
        call(json!({ "op": "checkHarness", "harness": harness, "interface": {} }))["problems"]
            .clone();
    assert_ne!(problems, json!([]), "{problems:#}");
}

/// A child whose every value is a literal: it writes a fixed path, then halts with "done", or fails if the write does.
fn scribe() -> Value {
    json!({
        "result": "String", "arg": calls_of(&tools()),
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![
                ("start", step(var("state"), vec![call_action("fs.write", rec(vec![("path", lit(json!("/workspace/a")))]))])),
                ("ret", step(var("state"), vec![halt(lit(json!("done")))])),
            ],
            // Dividing by zero can't evaluate, so this step fails the child.
            step(var("state"), vec![halt(op("str", vec![op("div", vec![lit(json!(1)), lit(json!(0))])]))]),
        ),
    })
}

#[test]
fn a_child_written_by_an_untrusted_source_carries_its_label() {
    let mut interface = tools();
    interface["channels"]["act"]["label"] = json!(["model"]);
    let ready = call(json!({ "op": "init", "harness": actor(), "interface": interface }));
    assert_eq!(ready["kind"], "ready", "{ready}");
    let system = deliver_all(ready["system"].clone(), vec![start(json!({}))]).0;
    let spawn = act(
        json!({ "kind": "spawn", "name": "worker", "harness": written(&scribe()), "narrow": written(&json!([])),
        "arg": { "kind": "call", "op": "net.fetch", "args": { "url": "https://example.com" } } }),
    );
    let (system, events) = deliver_all(system, vec![spawn]);
    assert_eq!(
        shape(&events),
        ["in:msg", "out:spawn", "in:start", "out:call"]
    );
    // The literal path the child writes depends on whoever wrote the child.
    assert_eq!(events[3]["labels"]["args"]["path"], json!(["model"]));

    // So does the literal it halts with, and so what its parent hears.
    let (_, events) = deliver_all(system.clone(), vec![to(vec![0], ret(0, json!(1)))]);
    let parent_hears = events.last().unwrap();
    assert_eq!(parent_hears["origin"], json!([]));
    assert_eq!(parent_hears["label"], json!(["model"]));

    // And a failure of the child.
    let failed =
        json!({ "to": [0], "input": { "kind": "failed", "call": 0, "error": "unreachable" } });
    let (_, events) = deliver_all(system, vec![failed]);
    let parent_hears = events.last().unwrap();
    assert_eq!(parent_hears["input"]["kind"], "failed", "{events:?}");
    assert_eq!(parent_hears["label"], json!(["model"]));
}
