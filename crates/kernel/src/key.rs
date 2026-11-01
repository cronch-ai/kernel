//! Content keys: an object's identity is the SHA-256 of its deterministic CBOR, so the same content has the same key
//! wherever it is produced. Each preimage starts with the object's kind and version, so two kinds of object never
//! share a key. Signatures sit beside objects, never inside them, so signing doesn't change a key.

use crate::cbor;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// `sha256:<hex>` of the deterministic CBOR of a preimage, which starts with its kind and version.
pub(crate) fn digest(preimage: &Value) -> String {
    let hash = Sha256::digest(cbor::encode(preimage));
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// The kinds of harness object, each keyed under its own name and version.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Harness,
    Interface,
    Property,
    Grant,
    Alphabet,
    Evidence,
    /// An entry of a run's log: `{ seq, prev, delivery, events }`, where `prev` is the previous entry's key.
    LogEntry,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Harness => "cronch.harness/1",
            Kind::Interface => "cronch.interface/1",
            Kind::Property => "cronch.property/1",
            Kind::Grant => "cronch.grant/1",
            Kind::Alphabet => "cronch.alphabet/1",
            Kind::Evidence => "cronch.evidence/1",
            Kind::LogEntry => "cronch.logEntry/1",
        }
    }
}

/// The key of a harness object of this kind.
pub fn key(kind: Kind, object: &impl serde::Serialize) -> String {
    digest(&json!({ "kind": kind.name(), "object": object }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_content_same_key() {
        let a = json!({ "arg": "Int", "n": [1, 2] });
        let b = json!({ "n": [1, 2], "arg": "Int" });
        assert_eq!(key(Kind::Harness, &a), key(Kind::Harness, &b));
        assert_ne!(
            key(Kind::Harness, &a),
            key(Kind::Harness, &json!({ "arg": "Int" }))
        );
        assert!(key(Kind::Harness, &a).starts_with("sha256:"));
    }

    #[test]
    fn kinds_never_share_a_key() {
        let object = json!({ "arg": "Int" });
        assert_ne!(key(Kind::Harness, &object), key(Kind::Property, &object));
    }
}
