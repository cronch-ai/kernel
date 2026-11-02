//! Harnesses and their runs, through the kernel's interface: checking, stepping, refusals, concurrency, streams,
//! cancellation, withheld inputs and failure.

mod common;
use common::*;
use serde_json::{Value, json};

fn int_op() -> Value {
    json!({ "input": { "Record": { "n": "Int" } }, "output": "Int" })
}

/// Start: call `double(n)`. A result: halt with it. Anything else: halt with "refused".
fn double_harness() -> Value {
    let calls = rec(vec![(
        "calls",
        op("add", vec![path("state", &["calls"]), lit(json!(1))]),
    )]);
    json!({
        "arg": "Int",
        "state": { "Record": { "calls": "Int" } },
        "init": { "calls": 0 },
        "step": on(
            vec![
                ("start", step(calls, vec![call_action("double", rec(vec![("n", path("input", &["value"]))]))])),
                ("ret", step(var("state"), vec![halt(path("input", &["value"]))])),
            ],
            step(var("state"), vec![halt(lit(json!("refused")))]),
        ),
    })
}

fn doubling() -> Value {
    json!({ "operations": { "double": int_op() } })
}

#[test]
fn harnesses_are_checked() {
    let mut bad = double_harness();
    bad["init"] = json!({ "calls": "none" });
    bad["step"] = op("add", vec![var("state"), lit(json!(1))]);
    let problems =
        call(json!({ "op": "checkHarness", "harness": bad, "interface": {} }))["problems"].clone();
    let kinds: Vec<&str> = problems
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["init", "expr"]);

    let mut unbound = double_harness();
    unbound["step"] = var("nowhere");
    let problems =
        call(json!({ "op": "checkHarness", "harness": unbound, "interface": {} }))["problems"]
            .clone();
    assert_eq!(
        problems[0]["problem"],
        json!({ "kind": "unknownVariable", "at": "", "name": "nowhere" })
    );
    let refused = call(json!({ "op": "init", "harness": unbound, "interface": {} }));
    assert_eq!(refused["kind"], "refused");

    let problems =
        call(json!({ "op": "checkHarness", "harness": double_harness(), "interface": doubling() }))
            ["problems"]
            .clone();
    assert_eq!(problems, json!([]));
}

#[test]
fn a_run_calls_and_halts() {
    let system = init(double_harness(), doubling());
    let (system, events) = deliver_all(system, vec![start(json!(21))]);
    assert_eq!(shape(&events), ["in:start", "out:call"]);
    assert_eq!(events[1]["call"], json!(0));
    assert_eq!(events[1]["performed"], json!(true));
    assert_eq!(events[1]["action"]["args"], json!({ "n": 21 }));
    // Every event of a step carries the state it produced.
    assert_eq!(events[0]["state"], json!({ "calls": 1 }));
    assert_eq!(system["root"]["pending"][0]["op"], "double");

    let (system, events) = deliver_all(system, vec![ret(0, json!(42))]);
    assert_eq!(shape(&events), ["in:ret", "out:halt"]);
    assert_eq!(
        system["root"]["outcome"],
        json!({ "kind": "halted", "value": 42 })
    );
    assert_eq!(system["root"]["pending"], json!([]));
}

#[test]
fn a_call_the_interface_lacks_is_caught_before_the_run_and_a_failed_precondition_when_it_happens() {
    // The step can call `double`, which this interface lacks: that is a problem with the harness, found before it
    // runs.
    let refused = call(json!({ "op": "init", "harness": double_harness(), "interface": {} }));
    assert_eq!(refused["kind"], "refused");
    assert_eq!(refused["error"]["kind"], "harness");

    // This step reads its start value without matching on the input's kind, so the argument is `Any` to the checker,
    // and one of the wrong type is refused when the call is made.
    let mut wrong = double_harness();
    wrong["arg"] = json!("String");
    let system = init(wrong, doubling());
    let (_, events) = deliver_all(system, vec![start(json!("one"))]);
    assert_eq!(events[1]["performed"], json!(false));

    // A precondition depends on the run, so a call that fails one is refused when it is made, before the world
    // moves.
    let mut closed = doubling();
    closed["operations"]["double"]["pre"] = lit(json!(false));
    let system = init(double_harness(), closed);
    let (system, events) = deliver_all(system, vec![start(json!(1))]);
    assert_eq!(
        shape(&events),
        ["in:start", "out:call", "in:refused", "out:halt"]
    );
    assert_eq!(events[1]["performed"], json!(false));
    assert_eq!(
        events[2]["input"],
        json!({ "kind": "refused", "call": 0, "op": "double" })
    );
    assert_eq!(system["root"]["outcome"]["value"], "refused");
}

#[test]
fn inputs_the_environment_could_not_produce_are_withheld() {
    let system = init(double_harness(), doubling());
    let (system, events) = deliver_all(
        system.clone(),
        vec![
            ret(0, json!(1)),
            start(json!("text")),
            start(json!(1)),
            start(json!(1)),
        ],
    );
    assert_eq!(
        shape(&events),
        [
            "withheld:notStarted",
            "withheld:mismatch",
            "in:start",
            "out:call",
            "withheld:alreadyStarted"
        ]
    );
    let (system, events) = deliver_all(
        system,
        vec![
            ret(7, json!(1)),
            ret(0, json!("forty-two")),
            msg("user", json!("hi")),
        ],
    );
    assert_eq!(
        shape(&events),
        [
            "withheld:notPending",
            "withheld:mismatch",
            "withheld:noSuchChannel"
        ]
    );
    // Withheld inputs change nothing: the call is still pending, and a good result still ends the run.
    let (system, _) = deliver_all(system, vec![ret(0, json!(2)), ret(0, json!(2))]);
    assert_eq!(system["root"]["outcome"]["value"], json!(2));
    let (_, events) = deliver_all(system, vec![ret(0, json!(2))]);
    assert_eq!(shape(&events), ["withheld:ended"]);
}

/// Start: call `a` and `b` at once. Collect results as they arrive; halt with them once both are in.
fn both_harness() -> Value {
    let got = op(
        "append",
        vec![path("state", &["got"]), path("input", &["value"])],
    );
    json!({
        "arg": { "Record": {} },
        "state": { "Record": { "got": { "List": "Int" } } },
        "init": { "got": [] },
        "step": on(
            vec![
                ("start", step(var("state"), vec![
                    call_action("a", rec(vec![("n", lit(json!(1)))])),
                    call_action("b", rec(vec![("n", lit(json!(2)))])),
                ])),
                ("ret", let_("got", got,
                    if_(eq(op("len", vec![var("got")]), lit(json!(2))),
                        step(rec(vec![("got", var("got"))]), vec![halt(var("got"))]),
                        step(rec(vec![("got", var("got"))]), vec![])))),
            ],
            step(var("state"), vec![]),
        ),
    })
}

#[test]
fn concurrent_calls_return_in_any_order() {
    let interface = json!({ "operations": { "a": int_op(), "b": int_op() } });
    let system = init(both_harness(), interface);
    let (system, events) = deliver_all(system, vec![start(json!({}))]);
    assert_eq!(shape(&events), ["in:start", "out:call", "out:call"]);
    assert_eq!(system["root"]["next"], json!(2));

    let (a_first, _) = deliver_all(system.clone(), vec![ret(0, json!(10)), ret(1, json!(20))]);
    let (b_first, _) = deliver_all(system, vec![ret(1, json!(20)), ret(0, json!(10))]);
    assert_eq!(a_first["root"]["outcome"]["value"], json!([10, 20]));
    assert_eq!(b_first["root"]["outcome"]["value"], json!([20, 10]));
}

#[test]
fn streams_deliver_items_then_end_and_cancellation_is_a_request() {
    let tokens = json!({ "input": { "Record": {} }, "output": "String", "mode": "stream" });
    let interface = json!({
        "operations": { "tokens": tokens },
        "channels": { "user": { "type": "String" } }
    });
    let text = op(
        "concat",
        vec![path("state", &["text"]), path("input", &["value"])],
    );
    let harness = json!({
        "arg": { "Record": {} },
        "state": { "Record": { "text": "String" } },
        "init": { "text": "" },
        "step": on(
            vec![
                ("start", step(var("state"), vec![call_action("tokens", rec(vec![]))])),
                ("ret", step(rec(vec![("text", text)]), vec![])),
                ("end", step(var("state"), vec![halt(path("state", &["text"]))])),
                ("msg", step(var("state"), vec![cancel(lit(json!(0)))])),
            ],
            step(var("state"), vec![]),
        ),
    });
    let system = init(harness, interface);
    let end = json!({ "input": { "kind": "end", "call": 0 } });
    let (system, events) = deliver_all(
        system,
        vec![
            start(json!({})),
            ret(0, json!("he")),
            msg("user", json!("stop")),
            ret(0, json!("llo")),
        ],
    );
    // The call stays pending after a cancel, and its items still arrive.
    assert_eq!(shape(&events)[3..], ["in:msg", "out:cancel", "in:ret"]);
    assert_eq!(events[4]["performed"], json!(true));
    let (system, _) = deliver_all(system, vec![end]);
    assert_eq!(system["root"]["outcome"]["value"], "hello");

    // `end` is only for streams.
    let system = init(double_harness(), doubling());
    let (_, events) = deliver_all(
        system,
        vec![
            start(json!(1)),
            json!({ "input": { "kind": "end", "call": 0 } }),
        ],
    );
    assert_eq!(shape(&events)[2], "withheld:notAStream");
}

#[test]
fn a_step_that_fails_ends_the_run() {
    let harness = json!({
        "arg": "Int",
        "state": "Int",
        "init": 0,
        "step": step(op("idiv", vec![lit(json!(1)), path("input", &["value"])]), vec![]),
    });
    let system = init(harness, json!({}));
    let (system, events) = deliver_all(system, vec![start(json!(0)), start(json!(1))]);
    assert_eq!(shape(&events), ["failed:", "withheld:ended"]);
    assert_eq!(system["root"]["outcome"]["kind"], "failed");
    assert_eq!(
        system["root"]["outcome"]["error"]["error"]["error"]["kind"],
        "divisionByZero"
    );
}

#[test]
fn labels_flow_from_inputs_into_state_and_calls() {
    let system = init(double_harness(), doubling());
    let (system, events) = deliver_all(
        system,
        vec![json!({ "input": { "kind": "start", "value": 5, "label": ["user"] } })],
    );
    assert_eq!(events[0]["label"], json!(["user"]));
    // The argument came from the untrusted start, so the result of the call carries its label.
    let (_, events) = deliver_all(system, vec![ret(0, json!(10))]);
    assert_eq!(events[0]["label"], json!(["user"]));
}

/// On a `net.fetch` result: send a fixed body, then one that depends on whether the page said "go".
fn reacts_to_fetch() -> Value {
    let send = |body: Value| {
        call_action(
            "net.send",
            rec(vec![
                ("url", lit(json!("https://example.com"))),
                ("body", body),
            ]),
        )
    };
    let fixed = send(lit(json!("arrived")));
    let chosen = if_(
        eq(path("input", &["value"]), lit(json!("go"))),
        send(lit(json!("went"))),
        send(lit(json!("stayed"))),
    );
    let decided = step(var("state"), vec![fixed, chosen]);
    json!({
        "arg": { "Record": {} }, "state": { "Record": {} }, "init": {},
        "step": on(
            vec![
                ("start", step(var("state"), vec![call_action("net.fetch", rec(vec![("url", lit(json!("https://example.com/page")))]))])),
                ("ret", decided),
            ],
            step(var("state"), vec![]),
        ),
    })
}

#[test]
fn only_what_arrived_carries_its_sources() {
    let system = init(reacts_to_fetch(), common::demo::interface());
    let (_, events) = deliver_all(system, vec![start(json!({})), ret(0, json!("go"))]);
    let sends: Vec<&Value> = events
        .iter()
        .filter(|e| e["kind"] == "out" && e["action"]["op"] == "net.send")
        .collect();
    assert_eq!(sends.len(), 2);
    // Which call the result answers is the interpreter's: reacting to its arrival adds no label...
    assert_eq!(sends[0]["labels"]["args"]["body"], json!([]));
    // ...but deciding on what it says does.
    assert_eq!(sends[1]["labels"]["args"]["body"], json!(["web"]));
}

#[test]
fn a_failure_says_whose_it_is() {
    let system = init(double_harness(), doubling());
    let failure = json!({ "input": { "kind": "failed", "call": 0, "error": "down" } });
    let (_, events) = deliver_all(system, vec![start(json!(1)), failure]);
    let failed = events
        .iter()
        .find(|e| e["input"]["kind"] == "failed")
        .unwrap();
    // What the world said is kept as it said it, under the kind the interpreter gives it.
    assert_eq!(
        failed["input"]["error"],
        json!({ "kind": "world", "detail": "down" })
    );

    // So an obligation leaves out only the cases whose detail it can't read, not every failure.
    let ty = call(
        json!({ "op": "inputType", "harness": double_harness(), "interface": doubling() }),
    )["type"]
        .clone();
    let cases = &ty["Variant"]["cases"]["failed"]["Record"]["error"]["Variant"]["cases"];
    assert_eq!(cases["cancelled"], json!({ "Record": {} }));
    assert_eq!(cases["world"], json!({ "Record": { "detail": "Any" } }));
}
