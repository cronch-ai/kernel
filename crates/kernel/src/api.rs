//! The kernel's interface: one request in, one response out, each in deterministic CBOR.

use crate::assess::assess;
use crate::cbor::{self, DecodeError};
use crate::check::check_ask;
use crate::claim::{
    Assessment, Claim, Verdict as ClaimVerdict, Witness, assess_claim, check_claim, claim_key,
};
use crate::ir::{Answer, Ask, AskSpec, World};
use crate::key::{ask_key, spec_key};
use crate::smtlib;
use crate::smtlib::claim::{Floats, Read};
use crate::validate::Verdict;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

// A request is decoded once per call, so its size doesn't matter.
#[allow(clippy::large_enum_variant)]
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    /// The Ask's content key in this world.
    Key {
        ask: Ask,
        #[serde(default)]
        world: World,
    },
    /// The key of a spec alone, under which a reusable Procedure for it is kept.
    SpecKey { spec: AskSpec },
    /// Problems with a closed Ask.
    CheckAsk { ask: Ask },
    /// Whether `answer` is an Answer the kernel can read, and if not, why.
    ReadAnswer { answer: Value },
    /// Check an Answer against its Ask, evaluate it as far as `resolved` allows, and check the contract.
    Assess {
        ask: Ask,
        #[serde(default)]
        world: World,
        answer: Answer,
        #[serde(default)]
        resolved: BTreeMap<String, Value>,
        #[serde(default)]
        evidence: Vec<Verdict>,
    },
    /// The Ask, its world and an Answer to it as an SMT-LIB 2 script, or why they have no faithful reading.
    Smtlib {
        ask: Ask,
        #[serde(default)]
        world: World,
        answer: Answer,
    },
    /// A claim's content key.
    ClaimKey { claim: Claim },
    /// Problems with a claim.
    CheckClaim { claim: Claim },
    /// Re-check a counterexample to a claim about an Answer: a verdict, or the values it still lacks.
    AssessClaim {
        claim: Claim,
        answer: Answer,
        #[serde(default)]
        witness: Witness,
    },
    /// The kernel's verdict if the witness is complete; otherwise the SMT-LIB query that asks a solver for what it lacks.
    ClaimQuery {
        claim: Claim,
        answer: Answer,
        #[serde(default)]
        witness: Witness,
        /// `exact` by default; `real` to search fast, with every candidate re-checked as always.
        #[serde(default)]
        floats: Floats,
    },
    /// A solver's `get-value` reply to a claim query, as the values its reads ask for.
    ReadValues { reply: String, reads: Vec<Read> },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RequestError {
    /// The request isn't a deterministic CBOR encoding.
    Undecodable { error: DecodeError },
    /// The request decodes, but isn't one the kernel understands. `detail` is the deserializer's explanation.
    Malformed { detail: String },
}

/// The kernel's assessment of a counterexample, with a validation script any solver confirms when it is one.
fn assessed(claim: &Claim, answer: &Answer, witness: &Witness) -> Value {
    match assess_claim(claim, answer, witness) {
        Assessment::Assessed { verdict } => {
            let refuted = matches!(verdict, ClaimVerdict::Refutes | ClaimVerdict::Inconsistent);
            let validation = refuted
                .then(|| smtlib::claim::validation(claim, answer, witness).ok())
                .flatten();
            json!({ "kind": "assessed", "claim": claim_key(claim), "verdict": verdict, "validation": validation })
        }
        incomplete => json!(incomplete),
    }
}

fn respond(request: Request) -> Value {
    match request {
        Request::Key { ask, world } => json!({ "key": ask_key(&ask, &world.facts) }),
        Request::SpecKey { spec } => json!({ "key": spec_key(&spec) }),
        Request::CheckAsk { ask } => json!({ "problems": check_ask(&ask) }),
        Request::ReadAnswer { answer } => match serde_json::from_value::<Answer>(answer) {
            Ok(_) => json!({ "readable": true }),
            Err(e) => json!({ "readable": false, "detail": e.to_string() }),
        },
        Request::Assess {
            ask,
            world,
            answer,
            resolved,
            evidence,
        } => {
            json!(assess(&ask, &world, &answer, &resolved, &evidence))
        }
        Request::Smtlib { ask, world, answer } => match smtlib::export(&ask, &world, &answer) {
            Ok(text) => json!({ "kind": "exported", "text": text }),
            Err(error) => json!({ "kind": "refused", "error": error }),
        },
        Request::ClaimKey { claim } => json!({ "key": claim_key(&claim) }),
        Request::CheckClaim { claim } => json!({ "problems": check_claim(&claim) }),
        Request::AssessClaim {
            claim,
            answer,
            witness,
        } => assessed(&claim, &answer, &witness),
        Request::ClaimQuery {
            claim,
            answer,
            witness,
            floats,
        } => match assess_claim(&claim, &answer, &witness) {
            Assessment::Incomplete { vars, holes } => {
                match smtlib::claim::query(&claim, &answer, &witness, &vars, &holes, floats) {
                    Ok(query) => {
                        json!({ "kind": "query", "script": query.script, "reads": query.reads })
                    }
                    Err(error) => json!({ "kind": "refused", "error": error }),
                }
            }
            Assessment::Assessed { .. } => assessed(&claim, &answer, &witness),
        },
        Request::ReadValues { reply, reads } => {
            match smtlib::values::read(&reply, &reads.iter().map(Read::ty).collect::<Vec<_>>()) {
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
