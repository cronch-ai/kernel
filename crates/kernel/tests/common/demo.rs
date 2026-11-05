//! The demonstration: harnesses over fetch, send, write and a user channel, and the properties they should keep.

use super::*;

pub fn interface() -> Value {
    let record = |fields: Value| json!({ "Record": fields });
    acting_on(json!({
        "operations": {
            "net.fetch": { "input": record(json!({ "url": "String" })), "output": "String", "ret": list(vec![lit(json!("web"))]) },
            "net.send": { "input": record(json!({ "url": "String", "body": "String" })), "output": "Int", "effect": true },
            "fs.write": { "input": record(json!({ "path": "String" })), "output": "Int", "effect": true },
        },
        "channels": {
            "user": { "type": "String" },
        }
    }))
}

fn send(body: Value) -> Value {
    call_action(
        "net.send",
        rec(vec![
            ("url", lit(json!("https://example.com"))),
            ("body", body),
        ]),
    )
}

fn fetch() -> Value {
    call_action(
        "net.fetch",
        rec(vec![("url", lit(json!("https://example.com/page")))]),
    )
}

fn empty_state(step_fn: Value) -> Value {
    json!({ "result": "String", "arg": { "Record": {} }, "state": { "Record": {} }, "init": {}, "step": step_fn })
}

/// Fetches a page and sends what it got.
pub fn leaky() -> Value {
    empty_state(on(
        vec![
            ("start", step(var("state"), vec![fetch()])),
            (
                "ret",
                match_(
                    var("input"),
                    "op",
                    vec![(
                        "net.fetch",
                        "page",
                        step(var("state"), vec![send(path("page", &["value"]))]),
                    )],
                    Some(step(var("state"), vec![])),
                ),
            ),
        ],
        step(var("state"), vec![]),
    ))
}

/// Sends a fixed greeting and fetches a page, but never sends anything after hearing back.
pub fn careful() -> Value {
    empty_state(on(
        vec![(
            "start",
            step(var("state"), vec![send(lit(json!("hello"))), fetch()]),
        )],
        step(var("state"), vec![]),
    ))
}

/// Performs every action it is told to on `act`, whatever the user says.
pub fn stubborn() -> Value {
    empty_state(on(
        vec![(
            "msg",
            on_channel("act", performing(), step(var("state"), vec![])),
        )],
        step(var("state"), vec![]),
    ))
}

/// Like `stubborn`, but after the user says "stop" it does nothing until the user says something else.
pub fn obedient() -> Value {
    let on_user = step(
        rec(vec![(
            "stopped",
            eq(path("m", &["value"]), lit(json!("stop"))),
        )]),
        vec![],
    );
    let on_act = if_(
        path("state", &["stopped"]),
        step(var("state"), vec![]),
        performing(),
    );
    let msg = match_(
        var("input"),
        "channel",
        vec![("user", "m", on_user), ("act", "m", on_act)],
        Some(step(var("state"), vec![])),
    );
    json!({
        "result": "String", "arg": { "Record": {} },
        "state": { "Record": { "stopped": "Bool" } },
        "init": { "stopped": false },
        "step": on(vec![("msg", msg)], step(var("state"), vec![])),
    })
}

/// No untrusted data reaches the body of a send.
pub fn no_untrusted_send() -> Value {
    json!({ "name": "noUntrustedSend", "always": implies(
        atom(at_call("net.send", lit(json!(true)))),
        atom(at_event("out", eq(op("len", vec![path("e", &["labels", "args", "body"])]), lit(json!(0))))),
    )})
}

fn user_says(text: &str) -> Value {
    atom(at_input(
        "msg",
        when_case(
            var("i"),
            "channel",
            "user",
            "m",
            eq(path("m", &["value"]), lit(json!(text))),
        ),
    ))
}

fn user_says_other_than_stop() -> Value {
    atom(at_input(
        "msg",
        when_case(
            var("i"),
            "channel",
            "user",
            "m",
            op("ne", vec![path("m", &["value"]), lit(json!("stop"))]),
        ),
    ))
}

/// No effect after the user says "stop", until they say something else.
pub fn stop_means_stop() -> Value {
    json!({ "name": "stopMeansStop", "always": implies(
        atom(at_any_call(path("e", &["effect"]))),
        not(since(not(user_says_other_than_stop()), user_says("stop"))),
    )})
}

/// Every result answers a call that was performed earlier.
pub fn results_answer_calls() -> Value {
    let is_ret = atom(at_input("ret", lit(json!(true))));
    let calls = match_(
        var("event"),
        "kind",
        vec![(
            "out",
            "e",
            match_(
                path("e", &["action"]),
                "kind",
                vec![("call", "a", list(vec![path("a", &["call"])]))],
                Some(list(vec![])),
            ),
        )],
        Some(list(vec![])),
    );
    json!({ "name": "resultsAnswerCalls", "always": implies(is_ret, exists("k", calls, and(vec![
        atom(at_input("ret", eq(path("i", &["call"]), var("k")))),
        once(atom(at_any_call(eq(path("a", &["call"]), var("k"))))),
    ])))})
}

/// `stopMeansStop` with `effect` misspelled: without its type, the property would hold of every run.
pub fn misspelled() -> Value {
    json!({ "name": "misspelled", "always": implies(
        atom(at_any_call(path("e", &["efect"]))),
        not(since(not(user_says_other_than_stop()), user_says("stop"))),
    )})
}

/// A send's body used as a number.
pub fn mistyped() -> Value {
    let length = op("add", vec![path("c", &["args", "body"]), lit(json!(1))]);
    json!({ "name": "mistyped", "always": atom(at_call("net.send", op("gt", vec![length, lit(json!(0))]))) })
}

/// Performs the action each message on `act` holds. Ignores everything else.
pub fn actor() -> Value {
    json!({
        "result": "String", "arg": { "Record": {} },
        "spawns": { "worker": { "arg": calls_of(&tools()), "result": "String" } },
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![("msg", on_channel("act", performing(), step(var("state"), vec![])))],
            step(var("state"), vec![]),
        ),
    })
}

/// Makes the call its start value holds, then halts with the result as text, or "denied" if the call is refused, or
/// "failed". A narrowing that took the call away refuses the spawn: the start value wouldn't be a call of the child's.
pub fn worker() -> Value {
    json!({
        "result": "String", "arg": calls_of(&tools()),
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![
                ("start", acting(var("state"), list(vec![path("input", &["value"])]))),
                ("ret", step(var("state"), vec![halt(op("str", vec![var("input")]))])),
                ("refused", step(var("state"), vec![halt(lit(json!("denied")))])),
            ],
            step(var("state"), vec![halt(lit(json!("failed")))]),
        ),
    })
}

/// Writes within /workspace/, fetches, and runs argvs a grant allowlists or approved once. A person's "always" extends
/// the allowlist, and "once" approves one run, which the run uses up.
pub fn tools() -> Value {
    let within = |dir: &str| op("prefix", vec![lit(json!(dir)), path("args", &["path"])]);
    let allowed = fold(
        path("grant", &["allow"]),
        lit(json!(false)),
        "ok",
        "p",
        op(
            "or",
            vec![
                var("ok"),
                op("prefix", vec![var("p"), path("args", &["argv"])]),
            ],
        ),
    );
    let argv_of = || path("args", &["argv"]);
    let grant = |allow: Value, once: Value| rec(vec![("allow", allow), ("once", once)]);
    let answered = if_(
        eq(var("result"), lit(json!("always"))),
        grant(
            op("append", vec![path("grant", &["allow"]), argv_of()]),
            path("grant", &["once"]),
        ),
        if_(
            eq(var("result"), lit(json!("once"))),
            grant(
                path("grant", &["allow"]),
                op("append", vec![path("grant", &["once"]), argv_of()]),
            ),
            var("grant"),
        ),
    );
    let allowed = op(
        "or",
        vec![
            allowed,
            op("contains", vec![path("grant", &["once"]), argv_of()]),
        ],
    );
    let used_up = grant(
        path("grant", &["allow"]),
        fold(
            path("grant", &["once"]),
            list(vec![]),
            "kept",
            "x",
            if_(
                eq(var("x"), argv_of()),
                var("kept"),
                op("append", vec![var("kept"), var("x")]),
            ),
        ),
    );
    let argv = json!({ "Record": { "argv": { "List": "String" } } });
    acting_on(json!({
        "operations": {
            "fs.write": { "input": { "Record": { "path": "String" } }, "output": "Int", "effect": true, "pre": within("/workspace/") },
            "net.fetch": { "input": { "Record": { "url": "String" } }, "output": "String", "ret": list(vec![lit(json!("web"))]) },
            "shell.run": { "input": argv, "output": "String", "effect": true, "pre": allowed, "onCall": used_up },
            "shell.approve": { "input": argv, "output": "String", "ret": list(vec![]), "onResult": answered },
        },
        // The shell's part of the grant state: argv prefixes allowed always, and argvs approved once.
        "grant": { "Record": { "allow": { "List": { "List": "String" } }, "once": { "List": { "List": "String" } } } },
    }))
}
