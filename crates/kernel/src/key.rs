//! Content keys: the identity of what is asked, independent of who asks.
//!
//! An Ask's key hashes its spec (goal and contract), its arguments and the facts of its declared world. A spec's key
//! hashes the spec alone, and identifies a reusable Procedure for every Ask with that spec. Each preimage starts with a
//! kind and version, so two kinds of object never share a key.

use crate::cbor;
use crate::ir::{Ask, AskSpec, Fact};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const ASK: &str = "cronch.ask/1";
const SPEC: &str = "cronch.spec/1";

/// `sha256:<hex>` of the deterministic CBOR of a preimage, which starts with its kind and version.
pub(crate) fn digest(preimage: &Value) -> String {
    let hash = Sha256::digest(cbor::encode(preimage));
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

pub fn ask_key(ask: &Ask, facts: &BTreeMap<String, Fact>) -> String {
    digest(&json!({ "kind": ASK, "spec": ask.spec, "args": ask.args, "facts": facts }))
}

pub fn spec_key(spec: &AskSpec) -> String {
    digest(&json!({ "kind": SPEC, "spec": spec }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Type;

    fn square(n: i64) -> Ask {
        serde_json::from_value(json!({
            "spec": { "name": "Square", "params": [{ "name": "n", "type": "Int" }], "output": "Int" },
            "args": { "n": n }
        }))
        .unwrap()
    }

    #[test]
    fn same_content_same_key() {
        assert_eq!(
            ask_key(&square(3), &BTreeMap::new()),
            ask_key(&square(3), &BTreeMap::new())
        );
        assert_ne!(
            ask_key(&square(3), &BTreeMap::new()),
            ask_key(&square(4), &BTreeMap::new())
        );
    }

    #[test]
    fn facts_are_part_of_the_key() {
        let facts = BTreeMap::from([(
            "user".to_string(),
            Fact {
                value: json!("alice"),
                ty: Some(Type::String),
                source: String::new(),
            },
        )]);
        assert_ne!(
            ask_key(&square(3), &BTreeMap::new()),
            ask_key(&square(3), &facts)
        );
    }

    #[test]
    fn spec_key_ignores_arguments() {
        assert_eq!(spec_key(&square(3).spec), spec_key(&square(4).spec));
        assert_ne!(
            spec_key(&square(3).spec),
            ask_key(&square(3), &BTreeMap::new())
        );
    }
}
