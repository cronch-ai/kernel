//! Harnesses and their runs, through the kernel's interface: checking, stepping, refusals, concurrency, streams,
//! cancellation, withheld inputs and failure.

mod common;
use common::*;
use serde_json::{Value, json};

fn int_op() -> Value {
    json!({ "input": { "Record": { "n": "Int" } }, "output": "Int" })
}

/// Start: call `double(n)`. A result: halt with it. Anything else: halt with -1.
fn double_harness() -> Value {
    let calls = rec(vec![(
        "calls",
        op("add", vec![path("state", &["calls"]), lit(json!(1))]),
    )]);
    json!({
        "result": "Int", "arg": "Int",
        "state": { "Record": { "calls": "Int" } },
        "init": { "calls": 0 },
        "step": on(
            vec![
                ("start", step(calls, vec![call_action("double", rec(vec![("n", path("input", &["value"]))]))])),
                ("ret", step(var("state"), vec![halt(path("input", &["value"]))])),
            ],
            step(var("state"), vec![halt(lit(json!(-1)))]),
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

    // An argument of the wrong type for the call it is passed to is caught too.
    let mut wrong = double_harness();
    wrong["arg"] = json!("String");
    let refused = call(json!({ "op": "init", "harness": wrong, "interface": doubling() }));
    assert_eq!(refused["error"]["kind"], "harness");

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
    assert_eq!(system["root"]["outcome"]["value"], json!(-1));
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
    let collect = || {
        let got = op(
            "append",
            vec![path("state", &["got"]), path("input", &["value"])],
        );
        let_(
            "got",
            got,
            if_(
                eq(op("len", vec![var("got")]), lit(json!(2))),
                step(rec(vec![("got", var("got"))]), vec![halt(var("got"))]),
                step(rec(vec![("got", var("got"))]), vec![]),
            ),
        )
    };
    json!({
        "result": { "List": "Int" }, "arg": { "Record": {} },
        "state": { "Record": { "got": { "List": "Int" } } },
        "init": { "got": [] },
        "step": on(
            vec![
                ("start", step(var("state"), vec![
                    call_action("a", rec(vec![("n", lit(json!(1)))])),
                    call_action("b", rec(vec![("n", lit(json!(2)))])),
                ])),
                ("ret", on_op(vec![("a", collect()), ("b", collect())], step(var("state"), vec![]))),
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
        "result": "String", "arg": { "Record": {} },
        "state": { "Record": { "text": "String" } },
        "init": { "text": "" },
        "step": on(
            vec![
                ("start", step(var("state"), vec![call_action("tokens", rec(vec![]))])),
                ("ret", on_op(vec![("tokens", step(rec(vec![("text", text)]), vec![]))], step(var("state"), vec![]))),
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
        "result": "String", "arg": "Int",
        "state": "Int",
        "init": 0,
        "step": on(
            vec![("start", step(op("idiv", vec![lit(json!(1)), path("input", &["value"])]), vec![]))],
            step(var("state"), vec![]),
        ),
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
        "result": "String", "arg": { "Record": {} }, "state": { "Record": {} }, "init": {},
        "step": on(
            vec![
                ("start", step(var("state"), vec![call_action("net.fetch", rec(vec![("url", lit(json!("https://example.com/page")))]))])),
                ("ret", on_op(vec![("net.fetch", decided)], step(var("state"), vec![]))),
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

    // No contract types what the world or a failed child says, so it arrives as text.
    let ty = call(
        json!({ "op": "inputType", "harness": double_harness(), "interface": doubling() }),
    )["type"]
        .clone();
    let cases = &ty["Variant"]["cases"]["failed"]["Record"]["error"]["Variant"]["cases"];
    assert_eq!(cases["cancelled"], json!({ "Record": {} }));
    assert_eq!(cases["world"], json!({ "Record": { "detail": "String" } }));
    assert_eq!(cases["child"], json!({ "Record": { "error": "String" } }));
}

/// A harness of this state and result, whose step is `step`.
fn typed_step(state: Value, init: Value, result: Value, step: Value) -> Value {
    json!({ "arg": "Int", "result": result, "state": state, "init": init, "step": step })
}

fn problem_kinds(harness: &Value) -> Vec<Value> {
    let out = call(json!({ "op": "checkHarness", "harness": harness, "interface": {} }));
    out["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["problem"]["kind"].clone())
        .collect()
}

/// Terms that gave a value outside their type, each with the static pass finding nothing.
fn dishonest_terms() -> Vec<(&'static str, Value, Value)> {
    let mixed = || lit(json!([1, "x"]));
    let c = || is("msg");
    let let_x = |be: Value| let_("x", be, var("x"));
    let arm_else = || if_(c(), lit(json!(1)), list(vec![]));
    let arm_then = || if_(c(), list(vec![]), lit(json!(1)));
    let arm_case = || {
        match_(
            var("input"),
            "kind",
            vec![("start", "i", list(vec![]))],
            Some(lit(json!(1))),
        )
    };
    let arm_otherwise = || {
        match_(
            var("input"),
            "kind",
            vec![("start", "i", lit(json!(1)))],
            Some(list(vec![])),
        )
    };
    let ints = || json!({ "List": "Int" });
    vec![
        ("a list of mixed items", ints(), mixed()),
        (
            "[] after a non-list `then`",
            json!("Int"),
            let_x(arm_else()),
        ),
        (
            "[] after a non-list `else`",
            json!("Int"),
            let_x(arm_then()),
        ),
        (
            "[] beside a non-list otherwise",
            json!("Int"),
            let_x(arm_case()),
        ),
        (
            "[] beside a non-list case",
            json!("Int"),
            let_x(arm_otherwise()),
        ),
    ]
}

#[test]
fn a_literal_or_a_branch_with_no_honest_type_is_refused_as_a_state_or_a_result() {
    for (what, ty, term) in dishonest_terms() {
        let state = json!({ "Record": { "v": ty } });
        let init = if ty == json!("Int") {
            json!({ "v": 0 })
        } else {
            json!({ "v": [] })
        };
        let as_state = typed_step(
            state,
            init,
            json!("Int"),
            step(rec(vec![("v", term.clone())]), vec![]),
        );
        assert_ne!(
            problem_kinds(&as_state),
            Vec::<Value>::new(),
            "state: {what}"
        );
        let refused = call(json!({ "op": "init", "harness": as_state, "interface": {} }));
        assert_eq!(refused["kind"], "refused", "state: {what}");

        let as_result = typed_step(
            json!({ "Record": {} }),
            json!({}),
            ty,
            step(var("state"), vec![halt(term)]),
        );
        assert_ne!(
            problem_kinds(&as_result),
            Vec::<Value>::new(),
            "result: {what}"
        );
    }
}

#[test]
fn a_child_with_no_honest_result_cant_start() {
    for (what, ty, term) in dishonest_terms() {
        let child = typed_step(
            json!({ "Record": {} }),
            json!({}),
            ty.clone(),
            step(var("state"), vec![halt(term)]),
        );
        let spawned = rec(vec![
            ("kind", lit(json!("spawn"))),
            ("name", lit(json!("kid"))),
            ("harness", lit(written(&child))),
            ("narrow", lit(json!("[]"))),
            ("arg", path("input", &["value"])),
        ]);
        let parent = json!({
            "arg": "Int", "result": "String", "state": { "Record": {} }, "init": {},
            "spawns": { "kid": { "arg": "Int", "result": ty } },
            "step": on(vec![("start", step(var("state"), vec![spawned]))], step(var("state"), vec![])),
        });
        let (_, events) = deliver_all(init(parent, json!({})), vec![start(json!(1))]);
        let last = events.last().unwrap();
        assert_eq!(last["origin"], json!([]), "{what}: {events:#?}");
        assert_eq!(last["input"]["kind"], "refused", "{what}: {events:#?}");
    }
}

#[test]
fn a_literal_list_has_the_join_of_its_items() {
    let inferred = |v: Value| call(json!({ "op": "infer", "expr": lit(v) }));
    let of = |v: Value| inferred(v)["type"].clone();
    assert_eq!(of(json!([1, 2])), json!({ "List": "Int" }));
    assert_eq!(of(json!([1, 1.5])), json!({ "List": "Float" }));
    assert_eq!(of(json!([1.5, 1])), json!({ "List": "Float" }));
    assert_eq!(of(json!([[1], []])), json!({ "List": { "List": "Int" } }));
    assert_eq!(
        of(json!([{ "a": 1, "b": 2 }, { "a": 3 }])),
        json!({ "List": { "Record": { "a": "Int" } } })
    );
    for dishonest in [json!([1, "x"]), json!([1, null]), json!([[1], ["x"]])] {
        let out = inferred(dishonest.clone());
        assert_eq!(out["problems"][0]["kind"], "untyped", "{dishonest} {out}");
    }
}

#[test]
fn an_empty_branch_beside_a_list_or_with_an_expected_type_still_checks() {
    let ints = || json!({ "List": "Int" });
    let c = || is("msg");
    let one = || lit(json!([1]));
    let state = || json!({ "Record": { "v": ints() } });
    let holding = |v: Value| step(rec(vec![("v", v)]), vec![]);
    let terms = [
        // Inferred, the other branch saying what the list holds.
        let_("x", if_(c(), one(), list(vec![])), holding(var("x"))),
        let_("x", if_(c(), list(vec![]), one()), holding(var("x"))),
        let_(
            "x",
            match_(
                var("input"),
                "kind",
                vec![("start", "i", list(vec![]))],
                Some(one()),
            ),
            holding(var("x")),
        ),
        let_(
            "x",
            match_(
                var("input"),
                "kind",
                vec![("start", "i", one())],
                Some(list(vec![])),
            ),
            holding(var("x")),
        ),
        // Expected.
        holding(if_(c(), list(vec![]), list(vec![]))),
        holding(list(vec![])),
        holding(lit(json!([]))),
    ];
    for term in terms {
        let harness = typed_step(state(), json!({ "v": [] }), json!("Int"), term.clone());
        assert_eq!(problem_kinds(&harness), Vec::<Value>::new(), "{term}");
    }
}
