//! Asks answered by Procedures with nested Asks, raw values and assumptions, run against
//! the kernel's interface exactly as a host would call it: deterministic CBOR in and out.

use kernel::cbor::{decode, encode};
use serde_json::{Value, json};

fn call(request: Value) -> Value {
    let response = decode(&kernel::api::call(&encode(&request))).unwrap();
    assert_eq!(response["ok"], json!(true), "{response}");
    response["result"].clone()
}

fn gdp_spec() -> Value {
    json!({
        "name": "GDP",
        "params": [{ "name": "country", "type": "String" }],
        "output": "Float",
        "description": "Nominal GDP in current US dollars, for the latest year available."
    })
}

fn population_spec() -> Value {
    json!({
        "name": "Population",
        "params": [{ "name": "country", "type": "String" }],
        "output": "Int",
        "description": "Total population, for the same year as the GDP figure."
    })
}

fn gdp_per_capita(country: &str) -> Value {
    json!({
        "spec": {
            "name": "GDPPerCapita",
            "params": [{ "name": "country", "type": "String" }],
            "output": "Float",
            "description": "Find GDP per capita for the specified country and identify the year and GDP measure.",
            "validators": [
                { "kind": "min", "value": 0 },
                { "kind": "prose", "text": "compatible GDP/population basis" }
            ]
        },
        "args": { "country": country }
    })
}

/// `Divide(Ask(GDP(country)), Ask(Population(country)))`
fn divide_procedure() -> Value {
    json!({ "procedure": {
        "e": "op", "op": "div", "args": [
            { "e": "ask", "spec": gdp_spec(), "args": { "country": { "e": "var", "name": "country" } } },
            { "e": "ask", "spec": population_spec(), "args": { "country": { "e": "var", "name": "country" } } }
        ]
    }})
}

fn assess(ask: Value, answer: Value, resolved: Value) -> Value {
    call(json!({ "op": "assess", "ask": ask, "answer": answer, "resolved": resolved }))
}

#[test]
fn construction_is_not_satisfaction() {
    let report = assess(gdp_per_capita("Kenya"), divide_procedure(), json!({}));
    assert_eq!(report["state"], "constructed");
    assert!(report.get("value").is_none());
    // Both holes are reported at once, so they can be resolved in parallel.
    let open = report["open"].as_array().unwrap();
    assert_eq!(open.len(), 2);
    assert_eq!(open[0]["ask"]["spec"]["name"], "GDP");
    assert_eq!(open[0]["ask"]["args"]["country"], "Kenya");
    assert_eq!(open[1]["ask"]["spec"]["name"], "Population");
}

#[test]
fn resolving_the_holes_satisfies_the_ask() {
    let partial = assess(gdp_per_capita("Kenya"), divide_procedure(), json!({}));
    let gdp = partial["open"][0]["key"].as_str().unwrap().to_string();
    let population = partial["open"][1]["key"].as_str().unwrap().to_string();

    let half = assess(
        gdp_per_capita("Kenya"),
        divide_procedure(),
        json!({ gdp.clone(): 1.0e11 }),
    );
    assert_eq!(half["state"], "constructed");
    assert_eq!(half["open"].as_array().unwrap().len(), 1);

    let done = assess(
        gdp_per_capita("Kenya"),
        divide_procedure(),
        json!({ gdp: 1.0e11, population: 50_000_000 }),
    );
    assert_eq!(done["state"], "satisfied");
    // An integral float has the integer's encoding.
    assert_eq!(done["value"], json!(2000));
    let outcomes: Vec<&Value> = done["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| &c["outcome"]["result"])
        .collect();
    assert_eq!(
        outcomes,
        [&json!("pass"), &json!("pass"), &json!("unchecked")]
    );
}

#[test]
fn a_resolution_must_inhabit_its_holes_type() {
    let partial = assess(gdp_per_capita("Kenya"), divide_procedure(), json!({}));
    let population = partial["open"][1]["key"].as_str().unwrap().to_string();
    let report = assess(
        gdp_per_capita("Kenya"),
        divide_procedure(),
        json!({ population: "fifty million" }),
    );
    assert_eq!(report["state"], "failed");
    assert_eq!(report["error"]["kind"], "resolution");
}

#[test]
fn a_procedure_is_reusable_across_arguments() {
    let kenya = assess(gdp_per_capita("Kenya"), divide_procedure(), json!({}));
    let uganda = assess(gdp_per_capita("Uganda"), divide_procedure(), json!({}));
    assert_ne!(kenya["open"][0]["key"], uganda["open"][0]["key"]);
    assert_eq!(uganda["open"][0]["ask"]["args"]["country"], "Uganda");
}

#[test]
fn a_raw_value_is_an_assumption_by_its_responder() {
    let report = assess(
        gdp_per_capita("Kenya"),
        json!({ "value": 2099.3 }),
        json!({}),
    );
    assert_eq!(report["state"], "satisfied");
    assert_eq!(
        report["assumptions"],
        json!([{ "kind": "rawValue", "value": 2099.3 }])
    );
}

#[test]
fn assumptions_are_listed_and_do_not_block() {
    let answer = json!({ "procedure": {
        "e": "op", "op": "div", "args": [
            { "e": "assume", "v": 1.08e11, "type": "Float", "note": "nominal GDP 2023", "source": "World Bank WDI" },
            { "e": "ask", "spec": population_spec(), "args": { "country": { "e": "var", "name": "country" } } }
        ]
    }});
    let report = assess(gdp_per_capita("Kenya"), answer, json!({}));
    assert_eq!(report["state"], "constructed");
    assert_eq!(report["assumptions"][0]["kind"], "assumed");
    assert_eq!(report["assumptions"][0]["source"], "World Bank WDI");
    assert_eq!(report["holes"].as_array().unwrap().len(), 1);
}

#[test]
fn an_opaque_hole_blocks_evaluation_and_has_no_resolution() {
    let answer = json!({ "procedure": { "e": "opaque", "type": "Float", "note": "sorry" } });
    let report = assess(gdp_per_capita("Kenya"), answer, json!({}));
    assert_eq!(report["state"], "constructed");
    assert_eq!(
        report["open"],
        json!([{ "kind": "opaque", "at": "", "type": "Float" }])
    );
}

#[test]
fn a_derivation_rejects_opaque_parts() {
    let mut ask = gdp_per_capita("Kenya");
    ask["spec"]["form"] = json!("derivation");
    let raw = assess(ask.clone(), json!({ "value": 2000.0 }), json!({}));
    assert_eq!(raw["state"], "illFormed");
    assert_eq!(
        raw["problems"][0],
        json!({ "kind": "notDerived", "at": "", "reason": "rawValue" })
    );
    let derived = assess(ask, divide_procedure(), json!({}));
    assert_eq!(derived["state"], "constructed");
}

#[test]
fn ill_typed_procedures_are_not_constructions() {
    let answer = json!({ "procedure": {
        "e": "op", "op": "concat", "args": [{ "e": "var", "name": "country" }, { "e": "lit", "v": 3 }]
    }});
    let report = assess(gdp_per_capita("Kenya"), answer, json!({}));
    assert_eq!(report["state"], "illFormed");
    let kinds: Vec<&Value> = report["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| &p["kind"])
        .collect();
    assert_eq!(kinds, [&json!("operator")]);
}

#[test]
fn a_contract_violation_fails() {
    let report = assess(gdp_per_capita("Kenya"), json!({ "value": -5.0 }), json!({}));
    assert_eq!(report["state"], "failed");
    assert_eq!(
        report["checks"][1]["outcome"]["failure"]["kind"],
        "belowMin"
    );
}

#[test]
fn facts_are_environmental_assumptions_and_part_of_the_key() {
    let ask = json!({
        "spec": { "name": "Greeting", "output": "String", "description": "Greet the user by name." },
        "args": {}
    });
    let world =
        json!({ "facts": { "user": { "value": "Ada", "type": "String", "source": "profile" } } });
    let answer = json!({ "procedure": {
        "e": "op", "op": "concat", "args": [{ "e": "lit", "v": "Hello, " }, { "e": "fact", "name": "user" }]
    }});
    let report = call(json!({ "op": "assess", "ask": ask, "world": world, "answer": answer }));
    assert_eq!(report["state"], "satisfied");
    assert_eq!(report["value"], "Hello, Ada");
    assert_eq!(
        report["assumptions"],
        json!([{ "kind": "fact", "name": "user", "value": "Ada", "source": "profile" }])
    );
    let bare = call(json!({ "op": "key", "ask": ask }));
    assert_ne!(bare["key"], report["key"]);
}

#[test]
fn square_delegates_to_multiply() {
    let ask = json!({
        "spec": { "name": "Square", "params": [{ "name": "n", "type": "Int" }], "output": "Int" },
        "args": { "n": 3 }
    });
    let multiply = json!({
        "name": "Multiply",
        "params": [{ "name": "a", "type": "Int" }, { "name": "b", "type": "Int" }],
        "output": "Int"
    });
    let answer = json!({ "procedure": {
        "e": "ask", "spec": multiply, "args": { "a": { "e": "var", "name": "n" }, "b": { "e": "var", "name": "n" } }
    }});
    let partial = assess(ask.clone(), answer.clone(), json!({}));
    let hole = partial["open"][0].clone();
    assert_eq!(hole["ask"]["args"], json!({ "a": 3, "b": 3 }));

    // The hole is an Ask of its own: answered by a Procedure over its parameters, it evaluates to 9.
    let inner = assess(
        hole["ask"].clone(),
        json!({ "procedure": {
            "e": "op", "op": "mul", "args": [{ "e": "var", "name": "a" }, { "e": "var", "name": "b" }]
        }}),
        json!({}),
    );
    assert_eq!(inner["key"], hole["key"]);
    assert_eq!(inner["value"], json!(9));

    let done = assess(ask, answer, json!({ hole["key"].as_str().unwrap(): 9 }));
    assert_eq!(done["state"], "satisfied");
    assert_eq!(done["value"], json!(9));
}

#[test]
fn malformed_requests_are_reported_not_panicked() {
    let response = decode(&kernel::api::call(&encode(&json!({ "op": "nope" })))).unwrap();
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["error"]["kind"], "malformed");
    let response = decode(&kernel::api::call(b"{\"op\":\"key\"}")).unwrap();
    assert_eq!(response["error"]["kind"], "undecodable");
}
