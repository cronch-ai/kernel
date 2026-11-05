//! Each row of the table in `docs/2026-11-05-boundaries.md`: a value crossing a boundary has a declared type, and the
//! interpreter checks it on arrival. What doesn't fit doesn't cross.

mod common;
use common::*;
use serde_json::{Value, json};

/// Calls whatever a message on `model` says, as a harness following a model would, and halts when told what to halt
/// with on `stop`. Its argument, state and what it reads of a message promise nothing about the call.
fn puppet(result: Value) -> Value {
    let call = |of: Value| {
        let shaped = json!({ "Record": { "op": "String", "args": "Json" } });
        let made = rec(vec![
            ("kind", lit(json!("call"))),
            ("op", path("v", &["op"])),
            ("args", path("v", &["args"])),
        ]);
        is_type(
            of,
            shaped,
            "v",
            acting(var("state"), dynamic(made)),
            step(var("state"), vec![]),
        )
    };
    let message = path("input", &["value"]);
    json!({
        "arg": "Int",
        "result": result,
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![("msg", if_(
                eq(path("input", &["channel"]), lit(json!("model"))),
                call(message.clone()),
                step(var("state"), vec![halt(message)]),
            ))],
            step(var("state"), vec![]),
        ),
    })
}

fn interface() -> Value {
    json!({
        "operations": {
            "fetch": {
                "input": { "Record": { "url": "String" } },
                "output": "String",
                "post": op("not", vec![eq(var("result"), lit(json!("bad")))]),
            },
        },
        "channels": {
            "model": { "type": "Json", "label": ["model"] },
            "stop": { "type": "Json" },
            "count": { "type": "Int", "inv": op("gt", vec![var("value"), lit(json!(0))]) },
        },
    })
}

fn started() -> Value {
    let system = init(puppet(json!("Json")), interface());
    deliver_all(system, vec![start(json!(1))]).0
}

fn chosen(op_: &str, args: Value) -> Value {
    msg("model", json!({ "op": op_, "args": args }))
}

fn withheld(events: &[Value]) -> Option<&Value> {
    events.iter().find(|e| e["kind"] == "withheld")
}

/// Whether a delivery led to a call being performed.
fn performed(events: &[Value]) -> bool {
    events
        .iter()
        .any(|e| e["kind"] == "out" && e["action"]["kind"] == "call" && e["performed"] == true)
}

#[test]
fn a_start_value_has_the_harness_s_argument_type() {
    let system = init(puppet(json!("Json")), interface());
    let (_, events) = deliver_all(system, vec![start(json!("one"))]);
    let reason = &withheld(&events).expect("withheld")["reason"];
    assert_eq!(reason["kind"], "mismatch", "{events:?}");
}

#[test]
fn a_chosen_call_has_the_operation_s_input_type_whatever_the_harness_held() {
    // Today a harness can read these as `Json`, which goes: see `chosen_call.rs`. Until then it must narrow one to
    // `Action`, whose cases are this interface's operations and their input types, so none of these happens.
    let (system, events) = deliver_all(started(), vec![chosen("fetch", json!({ "url": 1 }))]);
    assert!(!performed(&events), "{events:?}");
    let (system, events) = deliver_all(system, vec![chosen("fetch", json!({}))]);
    assert!(!performed(&events), "{events:?}");
    let (system, events) = deliver_all(system, vec![chosen("delete", json!({ "url": "x" }))]);
    assert!(!performed(&events), "{events:?}");
    let (_, events) = deliver_all(system, vec![chosen("fetch", json!({ "url": "x" }))]);
    assert!(performed(&events), "{events:?}");
}

#[test]
fn a_result_has_the_operation_s_output_type_and_postcondition() {
    let (system, _) = deliver_all(started(), vec![chosen("fetch", json!({ "url": "x" }))]);
    let (system, events) = deliver_all(system, vec![ret(0, json!(1))]);
    assert_eq!(
        withheld(&events).expect("withheld")["reason"]["kind"],
        "mismatch"
    );
    let (system, events) = deliver_all(system, vec![ret(0, json!("bad"))]);
    assert_eq!(
        withheld(&events).expect("withheld")["reason"]["kind"],
        "postcondition"
    );
    let (_, events) = deliver_all(system, vec![ret(0, json!("good"))]);
    assert_eq!(withheld(&events), None, "{events:?}");
}

#[test]
fn a_message_has_the_channel_s_type_and_invariant() {
    let (system, events) = deliver_all(started(), vec![msg("count", json!("one"))]);
    assert_eq!(
        withheld(&events).expect("withheld")["reason"]["kind"],
        "mismatch"
    );
    let (system, events) = deliver_all(system, vec![msg("count", json!(0))]);
    assert_eq!(
        withheld(&events).expect("withheld")["reason"]["kind"],
        "invariant"
    );
    let (_, events) = deliver_all(system, vec![msg("count", json!(1))]);
    assert_eq!(withheld(&events), None, "{events:?}");
}

#[test]
fn a_halt_has_the_harness_s_result_type() {
    // Checked before the run starts: this step halts with `Json` where the harness declares `String`.
    let out =
        call(json!({ "op": "init", "harness": puppet(json!("String")), "interface": interface() }));
    assert_eq!(out["kind"], "refused", "{out}");
    assert_eq!(
        out["error"]["problems"][0]["problem"]["kind"], "mismatch",
        "{out}"
    );
    // `Json` promises nothing, so anything halts.
    let (system, _) = deliver_all(started(), vec![msg("stop", json!(1))]);
    assert_eq!(system["root"]["outcome"]["kind"], "halted", "{system}");
}

#[test]
fn a_grant_has_the_interface_s_grant_type() {
    let mut interface = interface();
    interface["grant"] = json!({ "Record": { "allow": { "List": "String" } } });
    let bad = call(
        json!({ "op": "init", "harness": puppet(json!("Json")), "interface": interface, "grant": { "allow": 1 } }),
    );
    assert_ne!(bad["kind"], "ready", "{bad}");
    let good = call(
        json!({ "op": "init", "harness": puppet(json!("Json")), "interface": interface, "grant": { "allow": [] } }),
    );
    assert_eq!(good["kind"], "ready", "{good}");
}

#[test]
fn an_update_has_the_interface_s_grant_type() {
    let mut interface = interface();
    interface["grant"] = json!({ "Record": { "allow": { "List": "String" } } });
    interface["operations"]["fetch"]["onResult"] = rec(vec![("allow", lit(json!(1)))]);
    let out = call(
        json!({ "op": "init", "harness": puppet(json!("Json")), "interface": interface, "grant": { "allow": [] } }),
    );
    assert_eq!(out["kind"], "refused", "{out}");
}

#[test]
fn what_the_world_says_of_a_failure_is_not_checked() {
    // The one row where the world picks the shape, so whatever it says reaches a step as text.
    let (system, _) = deliver_all(started(), vec![chosen("fetch", json!({ "url": "x" }))]);
    let failure = json!({ "input": { "kind": "failed", "call": 0, "error": { "kind": "world", "detail": { "any": [1, "shape"] } } } });
    let (_, events) = deliver_all(system, vec![failure]);
    assert_eq!(withheld(&events), None, "{events:?}");
}
