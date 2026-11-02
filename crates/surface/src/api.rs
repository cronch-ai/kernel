//! The `elaborate` request, next to the kernel's own: this is what the WebAssembly build calls, so one module answers
//! both. Every request but `elaborate` goes to the kernel unchanged.

use crate::elaborate::elaborate;
use kernel::cbor;
use kernel::harness::{Harness, Interface};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Request {
    /// A source elaborated against the contracts of the capabilities it may use, and the terms of the harnesses it
    /// may spawn, each by identifier.
    Elaborate {
        source: String,
        #[serde(default)]
        capabilities: BTreeMap<String, Interface>,
        #[serde(default)]
        harnesses: BTreeMap<String, Harness>,
    },
}

fn respond(request: Request) -> Value {
    match request {
        Request::Elaborate {
            source,
            capabilities,
            harnesses,
        } => match elaborate(&source, &capabilities, &harnesses) {
            Ok(e) => json!({
                "kind": "elaborated", "harness": e.harness, "uses": e.uses, "harnesses": e.harnesses,
            }),
            Err(problems) => json!({ "kind": "problems", "problems": problems }),
        },
    }
}

pub fn call(request: &[u8]) -> Vec<u8> {
    let Ok(value) = cbor::decode(request) else {
        return kernel::api::call(request);
    };
    if value["op"] != "elaborate" {
        return kernel::api::call(request);
    }
    let response = match serde_json::from_value::<Request>(value) {
        Ok(r) => json!({ "ok": true, "result": respond(r) }),
        Err(e) => json!({ "ok": false, "error": { "kind": "malformed", "detail": e.to_string() } }),
    };
    cbor::encode(&response)
}
