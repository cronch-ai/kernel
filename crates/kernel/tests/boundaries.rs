//! Each row of the table in `docs/2026-11-05-boundaries.md`: a value crossing a boundary has a declared type, and the
//! interpreter checks it on arrival. What doesn't fit doesn't cross.

mod common;
use common::*;
use serde_json::{Value, json};

/// Makes the call a message on `model` holds, and halts with what a message on `stop` holds. The model's choice has a
/// type: the calls of the interface, as plain data.
fn puppet(result: Value) -> Value {
    json!({
        "arg": "Int",
        "result": result,
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![("msg", match_(
                var("input"),
                "channel",
                vec![
                    ("model", "m", acting(var("state"), list(vec![path("m", &["value"])]))),
                    ("stop", "m", step(var("state"), vec![halt(path("m", &["value"]))])),
                ],
                Some(step(var("state"), vec![])),
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
            "model": { "type": { "Variant": { "tag": "kind", "cases": { "call": { "Variant": { "tag": "op", "cases": {
                "fetch": { "Record": { "args": { "Record": { "url": "String" } } } },
            }}}}}}, "label": ["model"] },
            "stop": { "type": "String" },
            "count": { "type": "Int", "inv": op("gt", vec![var("value"), lit(json!(0))]) },
        },
    })
}

fn started() -> Value {
    let system = init(puppet(json!("String")), interface());
    deliver_all(system, vec![start(json!(1))]).0
}

fn chosen(op_: &str, args: Value) -> Value {
    msg("model", json!({ "kind": "call", "op": op_, "args": args }))
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
    let system = init(puppet(json!("String")), interface());
    let (_, events) = deliver_all(system, vec![start(json!("one"))]);
    let reason = &withheld(&events).expect("withheld")["reason"];
    assert_eq!(reason["kind"], "mismatch", "{events:?}");
}

#[test]
fn a_chosen_call_has_the_operation_s_input_type() {
    // None of these has the channel's type, so none reaches the harness. See `chosen_call.rs` for a model's answer.
    for (operation, args) in [
        ("fetch", json!({ "url": 1 })),
        ("fetch", json!({})),
        ("delete", json!({ "url": "x" })),
    ] {
        let (_, events) = deliver_all(started(), vec![chosen(operation, args)]);
        assert_eq!(
            withheld(&events).expect("withheld")["reason"]["kind"],
            "mismatch"
        );
        assert!(!performed(&events), "{events:?}");
    }
    let (_, events) = deliver_all(started(), vec![chosen("fetch", json!({ "url": "x" }))]);
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

/// Takes a record, calls `get` on start, and listens on `note`: each of its boundaries has a record type.
fn recording() -> (Value, Value) {
    let harness = json!({
        "arg": { "Record": { "url": "String" } },
        "result": "String",
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![("start", step(var("state"), vec![call_action(
                "get",
                rec(vec![("url", path("input", &["value", "url"]))]),
            )]))],
            step(var("state"), vec![]),
        ),
    });
    let interface = json!({
        "operations": {
            "get": {
                "input": { "Record": { "url": "String" } },
                "output": { "Record": { "pages": { "List": { "Record": { "text": "String" } } } } },
            },
        },
        "channels": { "note": { "type": { "Record": { "text": "String" } } } },
    });
    (harness, interface)
}

#[test]
fn a_field_the_type_doesn_t_declare_doesn_t_cross() {
    let (harness, interface) = recording();
    let fresh = || init(harness.clone(), interface.clone());
    let reason = |events: &[Value]| withheld(events).map(|w| w["reason"]["kind"].clone());

    // A start value.
    let (_, events) = deliver_all(fresh(), vec![start(json!({ "url": "u", "x": 1 }))]);
    assert_eq!(reason(&events), Some(json!("mismatch")), "{events:#?}");
    let (system, events) = deliver_all(fresh(), vec![start(json!({ "url": "u" }))]);
    assert_eq!(reason(&events), None, "{events:#?}");

    // A result, at any depth: here inside a list's items.
    for extra in [
        json!({ "pages": [], "x": 1 }),
        json!({ "pages": [{ "text": "a" }, { "text": "b", "x": 1 }] }),
    ] {
        let (_, events) = deliver_all(system.clone(), vec![ret(0, extra)]);
        assert_eq!(reason(&events), Some(json!("mismatch")), "{events:#?}");
    }
    let (_, events) = deliver_all(
        system.clone(),
        vec![ret(0, json!({ "pages": [{ "text": "a" }] }))],
    );
    assert_eq!(reason(&events), None, "{events:#?}");

    // A message.
    let (_, events) = deliver_all(
        system.clone(),
        vec![msg("note", json!({ "text": "t", "x": 1 }))],
    );
    assert_eq!(reason(&events), Some(json!("mismatch")), "{events:#?}");
    let (_, events) = deliver_all(system, vec![msg("note", json!({ "text": "t" }))]);
    assert_eq!(reason(&events), None, "{events:#?}");
}

#[test]
fn a_chosen_call_with_a_field_it_doesn_t_declare_is_withheld() {
    // A variant's tags sit beside its case's fields, so `kind` and `op` are declared. Anything else isn't: beside
    // them, or in the arguments.
    for answer in [
        json!({ "kind": "call", "op": "fetch", "args": { "url": "x", "x": 1 } }),
        json!({ "kind": "call", "op": "fetch", "args": { "url": "x" }, "why": "because" }),
    ] {
        let (_, events) = deliver_all(started(), vec![msg("model", answer)]);
        assert_eq!(
            withheld(&events).expect("withheld")["reason"]["kind"],
            "mismatch"
        );
        assert!(!performed(&events), "{events:?}");
    }
}

#[test]
fn a_halt_has_the_harness_s_result_type() {
    // Checked before the run starts: this step halts with `String` where the harness declares `Int`.
    let out =
        call(json!({ "op": "init", "harness": puppet(json!("Int")), "interface": interface() }));
    assert_eq!(out["kind"], "refused", "{out}");
    let (system, _) = deliver_all(started(), vec![msg("stop", json!("done"))]);
    assert_eq!(system["root"]["outcome"]["value"], "done", "{system}");
}

#[test]
fn a_grant_has_the_interface_s_grant_type() {
    let mut interface = interface();
    interface["grant"] = json!({ "Record": { "allow": { "List": "String" } } });
    let bad = call(
        json!({ "op": "init", "harness": puppet(json!("String")), "interface": interface, "grant": { "allow": 1 } }),
    );
    assert_ne!(bad["kind"], "ready", "{bad}");
    let good = call(
        json!({ "op": "init", "harness": puppet(json!("String")), "interface": interface, "grant": { "allow": [] } }),
    );
    assert_eq!(good["kind"], "ready", "{good}");
}

#[test]
fn an_update_has_the_interface_s_grant_type() {
    let mut interface = interface();
    interface["grant"] = json!({ "Record": { "allow": { "List": "String" } } });
    interface["operations"]["fetch"]["onResult"] = rec(vec![("allow", lit(json!(1)))]);
    let out = call(
        json!({ "op": "init", "harness": puppet(json!("String")), "interface": interface, "grant": { "allow": [] } }),
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

fn to(path: Vec<u64>, delivery: Value) -> Value {
    let mut d = delivery;
    d["to"] = json!(path);
    d
}

/// A child taking and giving `Int`, whose step is `step`.
fn kid(step: Value) -> Value {
    json!({ "arg": "Int", "result": "Int", "state": { "Record": {} }, "init": {}, "step": step })
}

/// A child that waits, and halts with what a message on `stop` holds.
fn waiting_kid() -> Value {
    let idle = || step(var("state"), vec![]);
    kid(on(
        vec![(
            "msg",
            on_channel(
                "stop",
                step(var("state"), vec![halt(path("m", &["value"]))]),
                idle(),
            ),
        )],
        idle(),
    ))
}

/// Spawns `child` as its declared `kid`, which takes and gives `Int`, and otherwise does nothing.
fn parent_of(child: Value) -> Value {
    let spawned = rec(vec![
        ("kind", lit(json!("spawn"))),
        ("name", lit(json!("kid"))),
        ("harness", lit(written(&child))),
        ("arg", path("input", &["value"])),
        ("narrow", lit(json!("[]"))),
    ]);
    json!({
        "arg": "Int", "result": "Int", "state": { "Record": {} }, "init": {},
        "spawns": { "kid": { "arg": "Int", "result": "Int" } },
        "step": on(vec![("start", step(var("state"), vec![spawned]))], step(var("state"), vec![])),
    })
}

fn stops() -> Value {
    json!({ "channels": { "stop": { "type": "Int" } } })
}

/// A parent whose `child` has started.
fn spawned(child: Value) -> (Value, Vec<Value>) {
    deliver_all(init(parent_of(child), stops()), vec![start(json!(1))])
}

/// What the root's step received, of the inputs that came back from a child.
fn heard(events: &[Value]) -> Vec<&Value> {
    events
        .iter()
        .filter(|e| e["kind"] == "in" && e["origin"] == json!([]) && e["input"]["kind"] != "start")
        .map(|e| &e["input"])
        .collect()
}

// The check at a halt is for what the static pass didn't stop. To reach it whatever that pass stops, the tests of it
// replace the step of a running configuration: `deliver` takes a system as data, and doesn't check it again.
#[test]
fn a_childs_result_is_checked_when_it_starts() {
    // Its step halts outside its own result type, so it never runs.
    let (system, events) = spawned(kid(step(var("state"), vec![halt(lit(json!("five")))])));
    assert_eq!(heard(&events)[0]["kind"], "refused", "{events:#?}");
    assert_eq!(heard(&events)[0]["op"], "spawn.kid");
    assert!(
        events.iter().all(|e| e["origin"] == json!([])),
        "{events:#?}"
    );
    assert_eq!(system["root"]["children"], Value::Null);

    // This one keeps its own promise, but it isn't the signature's.
    let mut other = kid(step(var("state"), vec![halt(lit(json!("five")))]));
    other["result"] = json!("String");
    let (_, events) = spawned(other);
    assert_eq!(heard(&events)[0]["kind"], "refused", "{events:#?}");

    // One that fits starts.
    let (system, events) = spawned(waiting_kid());
    assert_eq!(heard(&events), Vec::<&Value>::new(), "{events:#?}");
    assert_eq!(system["root"]["children"][0]["op"], "spawn.kid");
}

#[test]
fn a_childs_result_arrives_at_the_signature_s_type() {
    let (system, _) = spawned(waiting_kid());
    let (system, events) = deliver_all(system, vec![to(vec![0], msg("stop", json!(5)))]);
    assert_eq!(
        heard(&events),
        [&json!({ "kind": "ret", "call": 0, "op": "spawn.kid", "value": 5 })]
    );
    assert_eq!(system["root"]["children"], Value::Null);
}

#[test]
fn a_child_that_halts_outside_its_result_fails_and_its_parent_hears() {
    let (system, _) = spawned(waiting_kid());
    let replaced = |value: Value| {
        let mut system = system.clone();
        system["root"]["children"][0]["run"]["harness"]["step"] =
            step(var("state"), vec![halt(lit(value))]);
        system
    };

    // The replacement alone changes nothing: a halt that fits still arrives.
    let (_, events) = deliver_all(replaced(json!(5)), vec![to(vec![0], msg("stop", json!(0)))]);
    assert_eq!(heard(&events)[0]["kind"], "ret", "{events:#?}");

    let (system, events) = deliver_all(
        replaced(json!("five")),
        vec![to(vec![0], msg("stop", json!(0)))],
    );
    let failed = events
        .iter()
        .find(|e| e["kind"] == "failed")
        .expect("failed");
    assert_eq!(failed["origin"], json!([0]));
    let expected = json!({ "kind": "result", "expected": "Int", "got": "five" });
    assert_eq!(failed["error"], expected);

    // The value doesn't cross: the parent hears the failure, as text, and goes on.
    let inputs = heard(&events);
    assert_eq!(inputs.len(), 1, "{events:#?}");
    assert_eq!(inputs[0]["kind"], "failed");
    assert_eq!(inputs[0]["call"], 0);
    assert_eq!(inputs[0]["op"], "spawn.kid");
    assert_eq!(inputs[0]["error"]["kind"], "child");
    let said: Value = serde_json::from_str(inputs[0]["error"]["error"].as_str().unwrap()).unwrap();
    assert_eq!(said, expected);
    assert_eq!(system["root"]["children"], Value::Null);
    assert_eq!(system["root"]["outcome"], Value::Null);
}

/// Keeps the last `tick`, and calls `fetch` on each one, so a step's actions show whether they ran.
fn ticking() -> Value {
    let idle = || step(var("state"), vec![]);
    let fetches = call_action("fetch", rec(vec![("url", lit(json!("x")))]));
    json!({
        "arg": "Int", "result": "String", "state": { "Record": { "n": "Int" } }, "init": { "n": 0 },
        "step": on(
            vec![(
                "msg",
                on_channel(
                    "count",
                    step(rec(vec![("n", path("m", &["value"]))]), vec![fetches]),
                    idle(),
                ),
            )],
            idle(),
        ),
    })
}

#[test]
fn a_next_state_outside_the_state_type_is_caught_before_the_run() {
    let mut wrong = ticking();
    let next = rec(vec![("n", lit(json!("one")))]);
    wrong["step"] = step(next, vec![]);
    let out = call(json!({ "op": "init", "harness": wrong, "interface": interface() }));
    assert_eq!(out["kind"], "refused", "{out}");
    assert_eq!(out["error"]["kind"], "harness", "{out}");
}

// As for a child's result: to reach the check whatever the static pass stops, these replace the step of a running
// configuration, which `deliver` doesn't check again.
#[test]
fn a_next_state_outside_the_state_type_fails_the_run() {
    let system = init(ticking(), interface());
    let (system, _) = deliver_all(system, vec![start(json!(1)), msg("count", json!(1))]);
    assert_eq!(
        system["root"]["state"]["record"]["fields"]["n"]["scalar"]["value"], 1,
        "{system}"
    );
    let call_ = || call_action("fetch", rec(vec![("url", lit(json!("y")))]));
    let cases = [
        rec(vec![("n", lit(json!("one")))]),
        rec(vec![("other", lit(json!(1)))]),
        lit(json!(3)),
    ];
    for next in cases {
        let mut replaced = system.clone();
        replaced["root"]["harness"]["step"] = step(next.clone(), vec![call_()]);
        let pending = replaced["root"]["pending"].clone();
        let (after, events) = deliver_all(replaced, vec![msg("count", json!(2))]);

        // One event: the failure. The step's call isn't made, and the state stays what it was.
        assert_eq!(events.len(), 1, "{events:#?}");
        assert_eq!(events[0]["kind"], "failed");
        assert_eq!(events[0]["error"]["kind"], "notAStep");
        assert_eq!(after["root"]["outcome"]["kind"], "failed");
        assert_eq!(after["root"]["outcome"]["error"]["kind"], "notAStep");
        assert_eq!(after["root"]["state"], system["root"]["state"]);
        assert_eq!(after["root"]["pending"], pending);

        // It's over: nothing more is delivered.
        let (_, events) = deliver_all(after, vec![msg("count", json!(3))]);
        assert_eq!(shape(&events), ["withheld:ended"]);
    }
}

#[test]
fn the_next_state_is_checked_after_each_step() {
    // Fine until the message that takes it outside the type, so not only the first step is checked.
    let mut system = init(ticking(), interface());
    let next = if_(
        eq(path("m", &["value"]), lit(json!(2))),
        rec(vec![("n", lit(json!("two")))]),
        rec(vec![("n", path("m", &["value"]))]),
    );
    let mut step_ = system["root"]["harness"]["step"].clone();
    step_["cases"]["msg"]["body"]["cases"]["count"]["body"] = acting(next, list(vec![]));
    system["root"]["harness"]["step"] = step_;

    let (system, events) = deliver_all(system, vec![start(json!(1)), msg("count", json!(1))]);
    assert_eq!(shape(&events), ["in:start", "in:msg"], "{events:#?}");
    assert_eq!(
        system["root"]["state"]["record"]["fields"]["n"]["scalar"]["value"], 1,
        "{system}"
    );

    let (system, events) = deliver_all(system, vec![msg("count", json!(2))]);
    assert_eq!(events[0]["kind"], "failed", "{events:#?}");
    assert_eq!(system["root"]["outcome"]["error"]["kind"], "notAStep");
}

#[test]
fn a_childs_next_state_outside_its_type_fails_the_child_alone() {
    let (mut system, _) = spawned(waiting_kid());
    system["root"]["children"][0]["run"]["harness"]["step"] = step(lit(json!(3)), vec![]);
    let (system, events) = deliver_all(system, vec![to(vec![0], msg("stop", json!(0)))]);
    assert_eq!(events[0]["kind"], "failed", "{events:#?}");
    assert_eq!(events[0]["origin"], json!([0]));
    assert_eq!(events[0]["error"]["kind"], "notAStep");
    assert_eq!(heard(&events)[0]["kind"], "failed");
    assert_eq!(heard(&events)[0]["error"]["kind"], "child");
    assert_eq!(system["root"]["outcome"], Value::Null);
}

/// Performs a chosen call, keeps the last `tick` and each result, and halts with what a message on `end` holds. Every
/// type it can touch is `Int` but the arguments, so a property reading one as text is caught.
fn observed() -> Value {
    let idle = || step(var("state"), vec![]);
    let keep = |of: Value| rec(vec![("last", of)]);
    let msgs = match_(
        var("input"),
        "channel",
        vec![
            (
                "act",
                "m",
                acting(var("state"), list(vec![path("m", &["value"])])),
            ),
            ("tick", "m", step(keep(path("m", &["value"])), vec![])),
            (
                "end",
                "m",
                step(var("state"), vec![halt(path("m", &["value"]))]),
            ),
        ],
        Some(idle()),
    );
    let results = on_op(
        vec![("fetch", step(keep(path("input", &["value"])), vec![]))],
        idle(),
    );
    json!({
        "arg": "Int", "result": "Int", "state": { "Record": { "last": "Int" } }, "init": { "last": 0 },
        "step": on(vec![("msg", msgs), ("ret", results)], idle()),
    })
}

fn watched() -> Value {
    let mut interface = json!({
        "operations": { "fetch": { "input": { "Record": { "url": "String" } }, "output": "Int" } },
        "channels": { "tick": { "type": "Int" }, "end": { "type": "Int" } },
    });
    interface["channels"]["act"] = json!({ "type": calls_of(&interface) });
    interface
}

/// The events of a real run of `observed`: each kind of value a property can meet, with what crosses shown by its text.
fn watched_run() -> Vec<Value> {
    let fetch = |url: &str| {
        msg(
            "act",
            json!({ "kind": "call", "op": "fetch", "args": { "url": url } }),
        )
    };
    let anything = json!({ "any": [1, "shape"] });
    let trace = deliver_all(
        init(observed(), watched()),
        vec![
            start(json!(7)),
            msg("tick", json!(3)),
            fetch("a"),
            ret(0, json!(5)),
            msg("tick", anything.clone()),
            fetch("b"),
            json!({ "input": { "kind": "failed", "call": 1, "error": anything } }),
            msg("end", json!(9)),
        ],
    )
    .1;
    assert_eq!(
        shape(&trace),
        [
            "in:start",
            "in:msg",
            "in:msg",
            "out:call",
            "in:ret",
            "withheld:mismatch",
            "in:msg",
            "out:call",
            "in:failed",
            "in:msg",
            "out:halt",
        ]
    );
    trace
}

/// The first event of `trace` where `holds` does, by the monitor, for a property the checker accepts of this harness:
/// the way to see what a real property reads.
fn first_where(holds: Value, harness: &Value, interface: &Value, trace: &[Value]) -> Option<u64> {
    let property = json!({ "name": "never", "always": not(atom(holds)) });
    let problems = call(
        json!({ "op": "checkProperty", "property": property, "harness": harness, "interface": interface }),
    );
    assert_eq!(problems["problems"], json!([]), "{property}");
    let verdict = call(json!({ "op": "monitor", "property": property, "trace": trace }));
    verdict["at"].as_u64()
}

#[test]
fn a_property_reads_what_a_contract_typed_at_its_exact_type() {
    let trace = watched_run();
    let at = |holds| first_where(holds, &observed(), &watched(), &trace);
    let over_two = |of: Value| op("gt", vec![of, lit(json!(2))]);

    // A message, a result, the root's state and a call's arguments, each read as what its contract says, under `match`.
    let tick = at_input(
        "msg",
        when_case(
            var("i"),
            "channel",
            "tick",
            "m",
            over_two(path("m", &["value"])),
        ),
    );
    assert_eq!(at(tick), Some(1));
    let result = at_input(
        "ret",
        when_case(
            var("i"),
            "op",
            "fetch",
            "r",
            eq(path("r", &["value"]), lit(json!(5))),
        ),
    );
    assert_eq!(at(result), Some(4));
    assert_eq!(
        at(at_event("in", over_two(path("e", &["state", "last"])))),
        Some(1)
    );
    let url = at_call("fetch", eq(path("c", &["args", "url"]), lit(json!("b"))));
    assert_eq!(at(url), Some(7));
}

#[test]
fn a_property_reads_as_text_what_one_type_cant_cover() {
    let trace = watched_run();
    let at = |holds| first_where(holds, &observed(), &watched(), &trace);

    // A start value and a halt value cross between runs, so a property never has them at the run's own type.
    let started = at_input("start", eq(path("i", &["value"]), lit(json!("7"))));
    assert_eq!(at(started), Some(0));
    let halted = at_event(
        "out",
        when_case(
            path("e", &["action"]),
            "kind",
            "halt",
            "a",
            eq(path("a", &["value"]), lit(json!("9"))),
        ),
    );
    assert_eq!(at(halted), Some(10));

    // What was withheld, and why.
    let text = r#"{"any":[1,"shape"]}"#;
    let held = at_event(
        "withheld",
        when_case(
            path("e", &["input"]),
            "kind",
            "msg",
            "w",
            eq(path("w", &["value"]), lit(json!(text))),
        ),
    );
    assert_eq!(at(held), Some(5));
    let why = at_event(
        "withheld",
        when_case(
            path("e", &["reason"]),
            "kind",
            "mismatch",
            "r",
            all(vec![
                eq(path("r", &["expected"]), lit(json!("Int"))),
                eq(path("r", &["got"]), lit(json!(text))),
            ]),
        ),
    );
    assert_eq!(at(why), Some(5));

    // What the world said of a failure.
    let said = at_input(
        "failed",
        when_case(
            path("i", &["error"]),
            "kind",
            "world",
            "w",
            eq(path("w", &["detail"]), lit(json!(text))),
        ),
    );
    assert_eq!(at(said), Some(8));
}

#[test]
fn a_property_reading_either_at_the_other_type_is_refused_by_the_checker() {
    let check = |holds: Value| {
        let property = json!({ "name": "never", "always": not(atom(holds)) });
        call(json!({ "op": "checkProperty", "property": property, "harness": observed(), "interface": watched() }))["problems"][0]
            ["problem"]["kind"]
            .clone()
    };
    let result_as_text = at_input(
        "ret",
        when_case(
            var("i"),
            "op",
            "fetch",
            "r",
            eq(
                op("concat", vec![path("r", &["value"]), lit(json!("!"))]),
                lit(json!("5!")),
            ),
        ),
    );
    assert_eq!(check(result_as_text), "operator");
    let start_as_number = at_input(
        "start",
        op("gt", vec![path("i", &["value"]), lit(json!(0))]),
    );
    assert_eq!(check(start_as_number), "operator");
}

#[test]
fn a_property_reads_a_childs_result_at_the_signature_s_type_and_its_crossing_as_text() {
    let (parent, interface) = (parent_of(waiting_kid()), stops());
    let (system, mut trace) = spawned(waiting_kid());
    trace.extend(deliver_all(system, vec![to(vec![0], msg("stop", json!(5)))]).1);
    let at = |holds| first_where(holds, &parent, &interface, &trace);

    // A spawn's parts are text.
    let spawn = |holds: Value| {
        at_event(
            "out",
            when_case(path("e", &["action"]), "kind", "spawn", "a", holds),
        )
    };
    assert_eq!(at(spawn(eq(path("a", &["arg"]), lit(json!("1"))))), Some(1));
    assert_eq!(
        at(spawn(eq(path("a", &["narrow"]), lit(json!("[]"))))),
        Some(1)
    );
    let names_step = op(
        "contains",
        vec![path("a", &["harness"]), lit(json!("\"step\""))],
    );
    assert_eq!(at(spawn(names_step)), Some(1));

    // Its result is what the signature says, not text.
    let result = at_input(
        "ret",
        when_case(
            var("i"),
            "op",
            "spawn.kid",
            "r",
            op("gt", vec![path("r", &["value"]), lit(json!(4))]),
        ),
    );
    let at_ret = trace
        .iter()
        .position(|e| e["input"]["op"] == "spawn.kid")
        .unwrap() as u64;
    assert_eq!(at(result), Some(at_ret));
}

#[test]
fn a_property_reads_a_failed_steps_details_as_text() {
    let eq_text = |field: &str, text: &str| eq(path("f", &[field]), lit(json!(text)));
    let idle = || step(var("state"), vec![]);
    let bad_action = rec(vec![("kind", lit(json!("nope")))]);
    let by_zero = op("idiv", vec![lit(json!(1)), lit(json!(0))]);
    let cases = [
        (
            step(var("state"), vec![halt(lit(json!(1)))]),
            "result",
            all(vec![
                eq_text("expected", r#"{"List":"Int"}"#),
                eq_text("got", "1"),
            ]),
        ),
        (lit(json!(3)), "notAStep", eq_text("got", "3")),
        (
            acting(var("state"), list(vec![bad_action])),
            "badAction",
            eq_text("action", r#"{"kind":"nope"}"#),
        ),
        (
            let_("x", by_zero, idle()),
            "eval",
            op(
                "contains",
                vec![path("f", &["error"]), lit(json!("divisionByZero"))],
            ),
        ),
    ];
    for (replacement, kind, details) in cases {
        let mut system = init(observed(), watched());
        system["root"]["harness"]["result"] = json!({ "List": "Int" });
        system["root"]["harness"]["step"] = replacement;
        let (_, trace) = deliver_all(system, vec![start(json!(7))]);
        let failed = trace.iter().position(|e| e["kind"] == "failed").unwrap() as u64;
        let holds = at_event(
            "failed",
            when_case(path("e", &["error"]), "kind", kind, "f", details),
        );
        assert_eq!(
            first_where(holds, &observed(), &watched(), &trace),
            Some(failed),
            "{trace:#?}"
        );
    }
}

#[test]
fn a_property_reads_what_a_parent_hears_of_a_failed_child_as_text() {
    let (parent, interface) = (parent_of(waiting_kid()), stops());
    let (mut system, _) = spawned(waiting_kid());
    system["root"]["children"][0]["run"]["harness"]["step"] =
        step(var("state"), vec![halt(lit(json!("five")))]);
    let (_, trace) = deliver_all(system, vec![to(vec![0], msg("stop", json!(0)))]);
    let hears = at_input(
        "failed",
        when_case(
            path("i", &["error"]),
            "kind",
            "child",
            "c",
            op("contains", vec![path("c", &["error"]), lit(json!("five"))]),
        ),
    );
    let at = trace
        .iter()
        .position(|e| e["kind"] == "in" && e["input"]["kind"] == "failed");
    assert_eq!(
        first_where(hears, &parent, &interface, &trace),
        at.map(|i| i as u64)
    );
}

#[test]
fn no_event_value_is_refused() {
    // Whatever shape the world gave, a property over the run gets a verdict from both monitors.
    let trace = watched_run();
    let always = |truth: bool| json!({ "name": "p", "always": atom(lit(json!(truth))) });
    let holds = always(true);
    assert_eq!(
        call(json!({ "op": "monitor", "property": holds, "trace": trace }))["verdict"],
        "holds"
    );
    assert_eq!(
        call(json!({ "op": "monitor", "property": always(false), "trace": trace }))["verdict"],
        "violated"
    );
    let stepped = call(json!({ "op": "monitorStep", "property": holds, "events": trace }));
    assert_eq!(stepped["kind"], "stepped", "{stepped}");
    assert_eq!(stepped["verdict"], "holds", "{stepped}");

    // A recorded value outside the type the checker gave the property: nothing checks it on the way in. The atom reading
    // it fails to evaluate, so it is false, and the property still has a verdict.
    let mut doctored = trace.clone();
    doctored[4]["input"]["value"] = json!({ "not": "an Int" });
    let over_two = at_input(
        "ret",
        when_case(
            var("i"),
            "op",
            "fetch",
            "r",
            op("gt", vec![path("r", &["value"]), lit(json!(2))]),
        ),
    );
    let at = |t: &[Value]| first_where(over_two.clone(), &observed(), &watched(), t);
    assert_eq!(at(&trace), Some(4));
    assert_eq!(at(&doctored), None);
    let view = call(json!({ "op": "view", "events": doctored }));
    assert_eq!(
        view["events"][4]["input"]["value"],
        json!({ "not": "an Int" })
    );
}
