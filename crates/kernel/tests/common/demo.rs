//! The demonstration: harnesses over fetch, send, write and a user channel, and the properties they should keep.

use super::*;

pub fn interface() -> Value {
    let record = |fields: Value| json!({ "Record": fields });
    json!({
        "operations": {
            "net.fetch": { "input": record(json!({ "url": "String" })), "output": "String", "ret": list(vec![lit(json!("web"))]) },
            "net.send": { "input": record(json!({ "url": "String", "body": "String" })), "output": "Int", "effect": true },
            "fs.write": { "input": record(json!({ "path": "String" })), "output": "Int", "effect": true },
        },
        "channels": {
            "user": { "type": "String" },
            "act": { "type": record(json!({ "kind": "String" })) },
        }
    })
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
    json!({ "arg": { "Record": {} }, "state": { "Record": {} }, "init": {}, "step": step_fn })
}

/// Fetches a page and sends what it got.
pub fn leaky() -> Value {
    empty_state(on(
        vec![
            ("start", step(var("state"), vec![fetch()])),
            (
                "ret",
                if_(
                    eq(path("input", &["call"]), lit(json!(0))),
                    step(var("state"), vec![send(path("input", &["value"]))]),
                    step(var("state"), vec![]),
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
            if_(
                eq(path("input", &["channel"]), lit(json!("act"))),
                step(var("state"), vec![dynamic(path("input", &["value"]))]),
                step(var("state"), vec![]),
            ),
        )],
        step(var("state"), vec![]),
    ))
}

/// Like `stubborn`, but after the user says "stop" it does nothing until the user says something else.
pub fn obedient() -> Value {
    let user_said = |s: &str| eq(path("input", &["value"]), lit(json!(s)));
    let on_user = step(rec(vec![("stopped", user_said("stop"))]), vec![]);
    let on_act = if_(
        path("state", &["stopped"]),
        step(var("state"), vec![]),
        step(var("state"), vec![dynamic(path("input", &["value"]))]),
    );
    json!({
        "arg": { "Record": {} },
        "state": { "Record": { "stopped": "Bool" } },
        "init": { "stopped": false },
        "step": on(
            vec![("msg", if_(eq(path("input", &["channel"]), lit(json!("user"))), on_user, on_act))],
            step(var("state"), vec![]),
        ),
    })
}

fn performed_call() -> Value {
    all(vec![
        eq(ev(&["kind"]), lit(json!("out"))),
        eq(ev(&["action", "kind"]), lit(json!("call"))),
        ev(&["performed"]),
    ])
}

/// No untrusted data reaches the body of a send.
pub fn no_untrusted_send() -> Value {
    json!({ "name": "noUntrustedSend", "always": implies(
        atom(all(vec![performed_call(), eq(ev(&["action", "op"]), lit(json!("net.send")))])),
        atom(eq(op("len", vec![ev(&["labels", "args", "body"])]), lit(json!(0)))),
    )})
}

fn user_says(text: &str) -> Value {
    atom(all(vec![
        eq(ev(&["kind"]), lit(json!("in"))),
        eq(ev(&["input", "kind"]), lit(json!("msg"))),
        eq(ev(&["input", "channel"]), lit(json!("user"))),
        eq(ev(&["input", "value"]), lit(json!(text))),
    ]))
}

fn user_says_other_than_stop() -> Value {
    atom(all(vec![
        eq(ev(&["kind"]), lit(json!("in"))),
        eq(ev(&["input", "kind"]), lit(json!("msg"))),
        eq(ev(&["input", "channel"]), lit(json!("user"))),
        op("ne", vec![ev(&["input", "value"]), lit(json!("stop"))]),
    ]))
}

/// No effect after the user says "stop", until they say something else.
pub fn stop_means_stop() -> Value {
    json!({ "name": "stopMeansStop", "always": implies(
        atom(all(vec![performed_call(), ev(&["effect"])])),
        not(since(not(user_says_other_than_stop()), user_says("stop"))),
    )})
}

/// Every result answers a call that was performed earlier.
pub fn results_answer_calls() -> Value {
    let is_ret = atom(all(vec![
        eq(ev(&["kind"]), lit(json!("in"))),
        eq(ev(&["input", "kind"]), lit(json!("ret"))),
    ]));
    json!({ "name": "resultsAnswerCalls", "always": implies(is_ret, exists("k", ev(&["call"]), and(vec![
        atom(eq(ev(&["input", "call"]), var("k"))),
        once(atom(all(vec![performed_call(), eq(ev(&["call"]), var("k"))]))),
    ])))})
}

/// Performs the action each message on `act` names, whatever it is. Ignores everything else.
pub fn actor() -> Value {
    json!({
        "arg": { "Record": {} },
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![("msg", step(var("state"), vec![dynamic(path("input", &["value"]))]))],
            step(var("state"), vec![]),
        ),
    })
}

/// Calls `{op, args}` from its start value, then halts with the result, or "denied" or "failed".
pub fn worker() -> Value {
    json!({
        "arg": { "Record": { "op": "String" } },
        "state": { "Record": {} },
        "init": {},
        "step": on(
            vec![
                ("start", step(var("state"), vec![dynamic(rec(vec![
                    ("kind", lit(json!("call"))),
                    ("op", path("input", &["value", "op"])),
                    ("args", path("input", &["value", "args"])),
                ]))])),
                ("ret", step(var("state"), vec![halt(path("input", &["value"]))])),
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
    json!({
        "operations": {
            "fs.write": { "input": { "Record": { "path": "String" } }, "output": "Int", "effect": true, "pre": within("/workspace/") },
            "net.fetch": { "input": { "Record": { "url": "String" } }, "output": "String", "ret": list(vec![lit(json!("web"))]) },
            "shell.run": { "input": argv, "output": "String", "effect": true, "pre": allowed, "onCall": used_up },
            "shell.approve": { "input": argv, "output": "String", "ret": list(vec![]), "onResult": answered },
        },
        "channels": { "act": { "type": { "Record": { "kind": "String" } } } }
    })
}
