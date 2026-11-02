//! Properties monitored over the traces of real runs.

mod common;
use common::demo::*;
use common::*;
use serde_json::{Value, json};

fn trace(harness: Value, deliveries: Vec<Value>) -> Vec<Value> {
    let ready = call(json!({ "op": "init", "harness": harness, "interface": interface() }));
    assert_eq!(ready["kind"], "ready", "{ready}");
    deliver_all(ready["system"].clone(), deliveries).1
}

fn monitor(property: Value, trace: &[Value]) -> Value {
    call(json!({ "op": "monitor", "property": property, "trace": trace }))
}

fn write() -> Value {
    msg(
        "act",
        json!({ "kind": "call", "op": "fs.write", "args": { "path": "/a" } }),
    )
}

#[test]
fn untrusted_data_reaching_a_send_is_caught() {
    let leaked = trace(
        leaky(),
        vec![start(json!({})), ret(0, json!("secret page"))],
    );
    let verdict = monitor(no_untrusted_send(), &leaked);
    assert_eq!(verdict["verdict"], "violated");
    let at = verdict["at"].as_u64().unwrap() as usize;
    assert_eq!(leaked[at]["action"]["op"], "net.send");
    assert_eq!(leaked[at]["labels"]["args"]["body"], json!(["web"]));

    let fine = trace(
        careful(),
        vec![start(json!({})), ret(1, json!("secret page"))],
    );
    assert_eq!(monitor(no_untrusted_send(), &fine)["verdict"], "holds");
}

#[test]
fn effects_after_stop_are_caught_until_the_user_speaks_again() {
    let deliveries = vec![
        start(json!({})),
        write(),
        msg("user", json!("stop")),
        write(),
        msg("user", json!("carry on")),
        write(),
    ];
    let stubborn_run = trace(stubborn(), deliveries.clone());
    let verdict = monitor(stop_means_stop(), &stubborn_run);
    assert_eq!(verdict["verdict"], "violated");
    // The second write, right after "stop".
    assert_eq!(verdict["at"], json!(5));

    let obedient_run = trace(obedient(), deliveries);
    assert_eq!(
        monitor(stop_means_stop(), &obedient_run)["verdict"],
        "holds"
    );
    // It did write again once the user carried on.
    let writes = obedient_run
        .iter()
        .filter(|e| e["kind"] == "out" && e["performed"] == true)
        .count();
    assert_eq!(writes, 2);
}

#[test]
fn quantifiers_connect_results_to_their_calls() {
    let run = trace(
        careful(),
        vec![start(json!({})), ret(1, json!("page")), ret(0, json!(200))],
    );
    assert_eq!(monitor(results_answer_calls(), &run)["verdict"], "holds");

    // A doctored trace whose result answers nothing performed.
    let mut forged = run.clone();
    forged[3]["input"]["call"] = json!(9);
    assert_eq!(
        monitor(results_answer_calls(), &forged),
        json!({ "verdict": "violated", "at": 3 })
    );
}

#[test]
fn properties_are_checked() {
    let bad =
        json!({ "name": "bad", "always": atom(op("add", vec![lit(json!(1)), lit(json!(2))])) });
    let check = |property: Value| {
        call(json!({ "op": "checkProperty", "property": property, "harness": obedient(), "interface": interface() }))
            ["problems"]
            .clone()
    };
    let problems = check(bad);
    assert_eq!(problems[0]["at"], "/holds");
    assert_eq!(problems[0]["problem"]["kind"], "mismatch");
    for good in [
        no_untrusted_send(),
        stop_means_stop(),
        results_answer_calls(),
    ] {
        assert_eq!(check(good), json!([]));
    }

    // A field no event has, and a field read at a type no event gives it.
    let problems = check(misspelled());
    assert_eq!(problems[0]["problem"]["kind"], "noSuchField");
    assert_eq!(problems[0]["problem"]["field"], "efect");
    let problems = check(mistyped());
    assert_eq!(problems[0]["problem"]["kind"], "operator");
}
