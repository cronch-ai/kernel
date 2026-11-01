//! SMT-LIB 2: an Ask and its Answer as a theory any SMT solver reads without a Cronch-specific reader.
//!
//! Beside deterministic CBOR (`cbor`) and JSON, this is a third serialization of Cronch objects, and an export only:
//! identities stay the hash of the CBOR, and nothing reads SMT-LIB back. It is built the way the others are: the
//! objects become data (`script::Script`), read through declared registries (`readings`), and one printer writes the
//! text. Only a checked construction is exported, and only when every part of it has a faithful reading; anything
//! else is refused with where and why, never approximated.
//!
//! | Cronch                                  | SMT-LIB                                                           |
//! |-----------------------------------------|-------------------------------------------------------------------|
//! | `Int`, `Bool`, `String`                 | `Int`, `Bool`, `String`                                           |
//! | `Float`                                 | `Float64`, rounding to nearest, ties to even, as the kernel does  |
//! | `List(t)`                               | `(Seq t)`                                                         |
//! | a record type                           | a one-constructor `declare-datatype`, named by its shape's hash   |
//! | the Answer                              | `(define-fun answer (params) output body)`                        |
//! | a nested Ask                            | an uninterpreted `declare-fun`, one per distinct spec             |
//! | `opaque`                                | an uninterpreted `declare-const`                                  |
//! | a world fact                            | a `define-fun` with its value                                     |
//! | an assumption                           | its value, listed in a comment                                    |
//! | the replayable validators               | `(define-fun contract (params result) Bool ...)`                  |
//! | the closed Ask                          | `(define-fun value () output (answer args))`                      |
//! | whether it meets the contract           | `(define-fun holds () Bool (contract args value))`                |
//! | Checker and prose validators, effects   | comments                                                          |
//!
//! A nested Ask stays uninterpreted even when its spec is the Ask's own: this Answer doesn't decide how it is answered.
//!
//! Wherever the kernel evaluates the closed Ask to a value, the solver's `value` is that value, read at the output type
//! (a record keeps only the fields its type declares), and `contract` holds exactly when every replayable validator
//! passes. Where the kernel stops with an error (division by zero, integer overflow, a non-finite float), SMT-LIB's
//! functions are still total and give some value: the reading says nothing there.
//!
//! The same encoding writes claims as queries (`claim`): the Answer as above, the claim negated, and what the kernel's
//! evaluation needs stated outright, so a solver's model is a counterexample the kernel can re-check (`values` reads
//! it back). Those queries may read floats as reals, to search faster; the export and validation scripts never do.

pub mod claim;
mod encode;
pub mod readings;
pub mod script;
pub mod values;

use crate::check::{Path, Problem, check, check_ask};
use crate::ir::{Answer, Ask, Type, World};
use encode::Encoder;
use serde::Serialize;
use serde_json::Value;

/// Where a part with no SMT-LIB reading is.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "in", rename_all = "camelCase")]
pub enum Place {
    /// The Ask's parameters or output type, or a nested Ask's.
    Spec,
    /// The closed Ask's value for a parameter.
    Argument { param: String },
    /// A fact of the declared world.
    Fact { name: String },
    /// The Answer: its raw value, or a node of its Procedure.
    Body { at: Path },
    /// One of the Ask's validators.
    Validator { index: usize },
    /// A node of a claim's `holds`.
    Claim { at: Path },
    /// A claim's variable, or the counterexample's value for it.
    Var { name: String },
    /// The counterexample's value for a nested Ask of the Answer.
    Hole { ask: String },
}

/// Why a part has no faithful SMT-LIB reading.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NoReading {
    /// `Any` has no sort: an unchecked shape can't be declared. Empty list literals have this type.
    Any,
    /// An operator with no registered reading.
    Operator { op: String },
    /// A list used as a list of another element type: SMT-LIB has no map over sequences.
    ListConversion { from: Type, to: Type },
    /// Equality the kernel decides on JSON values, which may hold more than their type says (a record's undeclared
    /// fields, a float's integer encoding).
    Equality { left: Type, right: Type },
    /// `str` of a value the kernel writes as JSON text.
    Text { of: Type },
    /// `round` to a number of digits that isn't a literal from 0 to 22, whose scale is exact as a float.
    RoundingDigits,
    /// A record's size counts fields its type doesn't declare.
    RecordSize,
    /// A validator's path goes through a field the type doesn't declare.
    UndeclaredField { field: String },
    /// A name with a character a quoted SMT-LIB symbol can't hold (`|`, `\`, a control character).
    Name { name: String },
    /// A string with a character beyond SMT-LIB's range of code points (up to U+2FFFF).
    Character { code: u32 },
    /// A value that doesn't inhabit the type it is read at.
    Value { expected: Type, got: Value },
}

/// Why an Ask and its Answer weren't exported.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportError {
    /// The Ask or its Answer is ill-formed: only checked constructions are exported.
    IllFormed { problems: Vec<Problem> },
    /// A part has no faithful reading.
    NoReading { place: Place, reason: NoReading },
}

/// A closed Ask, its declared world and an Answer to it as an SMT-LIB 2 script, or why not.
pub fn export(ask: &Ask, world: &World, answer: &Answer) -> Result<String, ExportError> {
    let problems: Vec<Problem> = check_ask(ask)
        .into_iter()
        .chain(check(ask, world, answer).problems)
        .collect();
    if !problems.is_empty() {
        return Err(ExportError::IllFormed { problems });
    }
    Ok(Encoder::new(world).script(ask, answer)?.to_string())
}

/// A value as the term `export` writes for it at this type.
pub fn value(v: &Value, ty: &Type) -> Result<String, NoReading> {
    Ok(Encoder::new(&World::default()).value(v, ty)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn ask(spec: Value, args: Value) -> Ask {
        serde_json::from_value(json!({ "spec": spec, "args": args })).unwrap()
    }

    fn procedure(body: Value) -> Answer {
        serde_json::from_value(json!({ "procedure": body })).unwrap()
    }

    fn square() -> Ask {
        ask(
            json!({ "name": "Square", "params": [{ "name": "n", "type": "Int" }], "output": "Int" }),
            json!({ "n": 3 }),
        )
    }

    #[test]
    fn integers_compare_as_integers_as_the_kernel_compares_them() {
        let body = json!({ "e": "op", "op": "lt", "args": [{ "e": "var", "name": "n" }, { "e": "lit", "v": 2 }] });
        let spec = json!({ "name": "Small", "params": [{ "name": "n", "type": "Int" }], "output": "Bool" });
        let text = export(
            &ask(spec, json!({ "n": 1 })),
            &World::default(),
            &procedure(body),
        )
        .unwrap();
        assert!(text.contains("(< |v:n| 2)"));
    }

    #[test]
    fn only_checked_constructions_are_exported() {
        let body = json!({ "e": "lit", "v": "nine" });
        assert!(matches!(
            export(&square(), &World::default(), &procedure(body)),
            Err(ExportError::IllFormed { .. })
        ));
    }

    #[test]
    fn what_has_no_reading_is_refused_where_it_is() {
        let record = json!({ "e": "record", "fields": { "a": { "e": "lit", "v": 1 } } });
        let body = json!({ "e": "if", "cond": { "e": "op", "op": "eq", "args": [record, record] },
                           "then": { "e": "lit", "v": 1 }, "else": { "e": "lit", "v": 0 } });
        let a = Type::Record(BTreeMap::from([("a".into(), Type::Int)]));
        assert_eq!(
            export(&square(), &World::default(), &procedure(body)),
            Err(ExportError::NoReading {
                place: Place::Body { at: "/cond".into() },
                reason: NoReading::Equality {
                    left: a.clone(),
                    right: a
                },
            })
        );
        let empty = json!({ "e": "op", "op": "len", "args": [{ "e": "list", "items": [] }] });
        assert_eq!(
            export(&square(), &World::default(), &procedure(empty)),
            Err(ExportError::NoReading {
                place: Place::Body {
                    at: "/args/0".into()
                },
                reason: NoReading::Any
            })
        );
    }

    #[test]
    fn a_nested_ask_stays_uninterpreted_even_when_it_is_the_asks_own() {
        let spec = json!({ "name": "Square", "params": [{ "name": "n", "type": "Int" }], "output": "Int" });
        let body =
            json!({ "e": "ask", "spec": spec, "args": { "n": { "e": "var", "name": "n" } } });
        let text = export(&square(), &World::default(), &procedure(body)).unwrap();
        assert!(text.contains("(declare-fun |ask:Square:"));
        assert!(text.contains("(define-fun answer ((|v:n| Int)) Int (|ask:Square:"));
    }
}
