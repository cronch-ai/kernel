//! The kernel's interface: one request in, one response out, each in deterministic CBOR.

use crate::cbor::{self, DecodeError};
use crate::check::infer;
use crate::evidence::{Evidence, Method, Subject, prove, verify};
use crate::harness::{
    Harness, Installation, Interface, Narrowing, check_harness, check_interface, install, narrow,
};
use crate::ir::{Expr, Type};
use crate::key::{Kind, key};
use crate::labeled::{self, LValue};
use crate::property::{Property, check_property, monitor};
use crate::run::{Delivery, System};
use crate::smtlib::{self, Definition, Floats};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

// A request is decoded once per call, so its size doesn't matter.
#[allow(clippy::large_enum_variant)]
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    /// The type of an expression, given its variables' types, and what is wrong with it.
    Infer {
        expr: Expr,
        #[serde(default)]
        vars: BTreeMap<String, Type>,
    },
    /// The value of an expression, given its variables' values, all trusted.
    Evaluate {
        expr: Expr,
        #[serde(default)]
        vars: BTreeMap<String, Value>,
    },
    /// Problems with a harness.
    CheckHarness { harness: Harness },
    /// Problems with an interface's contracts.
    CheckInterface { interface: Interface },
    /// The interface of several capabilities, each under its prefix.
    Install { capabilities: Vec<Installation> },
    /// An interface narrowed for a child.
    Narrow {
        interface: Interface,
        narrowings: Vec<Narrowing>,
    },
    /// A system that hasn't started: the harness at its initial state, against the interface.
    Init {
        harness: Harness,
        #[serde(default)]
        interface: Interface,
        /// The grant state the root principal starts the run with.
        #[serde(default)]
        grant: Value,
    },
    /// Deliver one input to a system: the next configuration, and what happened.
    Deliver { system: System, delivery: Delivery },
    /// Problems with a property.
    CheckProperty { property: Property },
    /// Whether a trace satisfies a property, and if not, where it first fails.
    Monitor {
        property: Property,
        trace: Vec<Value>,
    },
    /// The kernel's verdict on a property of a harness, by a method, as evidence.
    Prove {
        #[serde(flatten)]
        subject: Subject,
        method: Method,
    },
    /// Whether evidence is about this subject, and the kernel reaches its verdict.
    Verify {
        #[serde(flatten)]
        subject: Subject,
        evidence: Evidence,
    },
    /// The content key of a harness object.
    ObjectKey { kind: Kind, object: Value },
    /// Definitions as an SMT-LIB 2 script, or why they have no faithful reading.
    Smtlib {
        definitions: Vec<Definition>,
        #[serde(default)]
        floats: Floats,
    },
    /// A solver's `get-value` reply, as values of these types.
    ReadValues { reply: String, types: Vec<Type> },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RequestError {
    /// The request isn't a deterministic CBOR encoding.
    Undecodable { error: DecodeError },
    /// The request decodes, but isn't one the kernel understands. `detail` is the deserializer's explanation.
    Malformed { detail: String },
}

fn refused_or<T: Serialize, E: Serialize>(kind: &str, field: &str, result: Result<T, E>) -> Value {
    match result {
        Ok(v) => json!({ "kind": kind, field: v }),
        Err(error) => json!({ "kind": "refused", "error": error }),
    }
}

fn respond(request: Request) -> Value {
    match request {
        Request::Infer { expr, vars } => {
            let (ty, problems) = infer(&expr, vars.into_iter().collect());
            json!({ "type": ty, "problems": problems })
        }
        Request::Evaluate { expr, vars } => {
            let scope = vars
                .iter()
                .map(|(n, v)| (n.clone(), LValue::trusted(v)))
                .collect();
            match labeled::eval(&expr, scope) {
                Ok(v) => json!({ "kind": "value", "value": v.value() }),
                Err(error) => json!({ "kind": "failed", "error": error }),
            }
        }
        Request::CheckHarness { harness } => json!({ "problems": check_harness(&harness) }),
        Request::CheckInterface { interface } => {
            json!({ "problems": check_interface(&interface) })
        }
        Request::Install { capabilities } => {
            refused_or("installed", "interface", install(&capabilities))
        }
        Request::Narrow {
            interface,
            narrowings,
        } => refused_or("narrowed", "interface", narrow(&interface, &narrowings)),
        Request::Init {
            harness,
            interface,
            grant,
        } => refused_or("ready", "system", System::new(harness, interface, grant)),
        Request::Deliver {
            mut system,
            delivery,
        } => {
            let events = system.deliver(&delivery);
            json!({ "system": system, "events": events })
        }
        Request::CheckProperty { property } => json!({ "problems": check_property(&property) }),
        Request::Monitor { property, trace } => json!(monitor(&property, &trace)),
        Request::Prove { subject, method } => match prove(&subject, &method) {
            Ok(evidence) => {
                json!({ "kind": "evidence", "key": key(Kind::Evidence, &evidence), "evidence": evidence })
            }
            Err(error) => json!({ "kind": "refused", "error": error }),
        },
        Request::Verify { subject, evidence } => match verify(&subject, &evidence) {
            Ok(()) => json!({ "valid": true }),
            Err(error) => json!({ "valid": false, "error": error }),
        },
        Request::ObjectKey { kind, object } => json!({ "key": key(kind, &object) }),
        Request::Smtlib {
            definitions,
            floats,
        } => refused_or("exported", "text", smtlib::export(&definitions, floats)),
        Request::ReadValues { reply, types } => {
            match smtlib::values::read(&reply, &types.iter().collect::<Vec<_>>()) {
                Ok(values) => json!({ "kind": "values", "values": values }),
                Err(error) => json!({ "kind": "unreadable", "error": error }),
            }
        }
    }
}

fn handle(request: &[u8]) -> Result<Value, RequestError> {
    let value = cbor::decode(request).map_err(|error| RequestError::Undecodable { error })?;
    let request =
        serde_json::from_value::<Request>(value).map_err(|e| RequestError::Malformed {
            detail: e.to_string(),
        })?;
    Ok(respond(request))
}

pub fn call(request: &[u8]) -> Vec<u8> {
    let response = match handle(request) {
        Ok(result) => json!({ "ok": true, "result": result }),
        Err(error) => json!({ "ok": false, "error": error }),
    };
    cbor::encode(&response)
}
