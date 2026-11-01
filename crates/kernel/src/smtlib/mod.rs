//! SMT-LIB 2: typed functions over Cronch values, as a theory any SMT solver reads without a Cronch-specific reader.
//!
//! Beside deterministic CBOR (`cbor`) and JSON, this is a third serialization, and an export only: identities stay
//! the hash of the CBOR, and nothing reads SMT-LIB back except values a solver gives (`values`). What is exported is a
//! list of definitions, each a pure expression over typed parameters: a contract's precondition, an invariant, a part
//! of a step function. Each becomes a `define-fun`, so a solver can be asked about it: whether one precondition implies
//! another, whether an invariant is preserved. It is built the way the other serializations are: the objects become
//! data (`script::Script`), read through declared registries (`readings`), and one printer writes the text. Only a
//! well-typed definition is exported, and only when every part of it has a faithful reading; anything else is refused
//! with where and why, never approximated.
//!
//! | Cronch                   | SMT-LIB                                                           |
//! |--------------------------|-------------------------------------------------------------------|
//! | `Int`, `Bool`, `String`  | `Int`, `Bool`, `String`                                           |
//! | `Float`                  | `Float64`, rounding to nearest, ties to even, as the kernel does  |
//! | `List(t)`                | `(Seq t)`                                                         |
//! | a record type            | a one-constructor `declare-datatype`, named by its shape's hash   |
//! | a definition             | `(define-fun def:name (params) sort body)`                        |
//! | where it is defined      | `(define-fun defined:name (params) Bool ...)`                     |
//!
//! Where the kernel stops with an error (division by zero, integer overflow, a non-finite float), SMT-LIB's functions
//! are still total and give some value: `defined:name` says where the reading is faithful. A solver's verdict about
//! an export is never the kernel's: the kernel only accepts what it can check itself.

mod encode;
pub mod readings;
pub mod script;
pub mod values;

pub use encode::Floats;

use crate::check::{Path, Problem, infer};
use crate::ir::{Expr, Param, Type};
use encode::Encoder;
use script::Script;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A named function of typed parameters.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Definition {
    pub name: String,
    #[serde(default)]
    pub params: Vec<Param>,
    pub body: Expr,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// Where a part with no SMT-LIB reading is.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "in", rename_all = "camelCase")]
pub enum Place {
    Param { name: String },
    Body { at: Path },
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
    /// A fold: it runs once per item of a list whose length a solver doesn't know.
    Fold,
    /// A label: an export reads values, not their provenance.
    Label,
    /// A variant with no cases: SMT-LIB has no datatype without constructors.
    EmptyVariant,
    /// `round` to a number of digits that isn't a literal from 0 to 22, whose scale is exact as a float.
    RoundingDigits,
    /// A record's size counts fields its type doesn't declare.
    RecordSize,
    /// A record read at a type with a field it doesn't declare.
    UndeclaredField { field: String },
    /// A name with a character a quoted SMT-LIB symbol can't hold (`|`, `\`, a control character).
    Name { name: String },
    /// A string with a character beyond SMT-LIB's range of code points (up to U+2FFFF).
    Character { code: u32 },
    /// A value that doesn't inhabit the type it is read at.
    Value { expected: Type, got: Value },
}

/// Why a definition wasn't exported.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportError {
    /// It doesn't type-check: only well-typed definitions are exported.
    IllFormed { problems: Vec<Problem> },
    /// A part has no faithful reading.
    NoReading {
        place: Place,
        reason: Box<NoReading>,
    },
}

/// The definition an export stopped at, and why.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Refusal {
    pub definition: String,
    #[serde(flatten)]
    pub error: Box<ExportError>,
}

/// Definitions as an SMT-LIB 2 script, or why not.
pub fn export(definitions: &[Definition], floats: Floats) -> Result<String, Refusal> {
    let mut encoder = Encoder::new().reading(floats);
    let mut entries = Vec::new();
    for def in definitions {
        let refused = |error| Refusal {
            definition: def.name.clone(),
            error: Box::new(error),
        };
        let scope = def
            .params
            .iter()
            .map(|p| (p.name.clone(), p.ty.clone()))
            .collect();
        let (_, problems) = infer(&def.body, scope);
        if !problems.is_empty() {
            return Err(refused(ExportError::IllFormed { problems }));
        }
        entries.extend(encoder.definition(def).map_err(refused)?);
    }
    let records = encoder.records().map_err(|error| Refusal {
        definition: String::new(),
        error: Box::new(error),
    })?;
    let script = Script {
        preamble: vec![
            "Exported from Cronch. Each definition's identity is the content key of its deterministic CBOR, not this text."
                .into(),
        ],
        options: vec![],
        logic: "ALL",
        sections: vec![
            records,
            script::Section {
                heading: "Definitions.",
                entries,
            },
        ],
    };
    Ok(script.to_string())
}

/// Consecution for an invariant of a step's state, as a query: `sat` exactly where some state satisfies the
/// invariant, `next` is defined on it and some input, and the invariant fails of the result. `get-value` asks for
/// that state and input, which the kernel can then check (`induction::check_step`).
pub fn induction(
    state: &Type,
    input: &Type,
    invariant: &Expr,
    next: &Expr,
    excluded: &[String],
    floats: Floats,
) -> Result<String, Refusal> {
    let param = |name: &str, ty: &Type| Param {
        name: name.to_string(),
        ty: ty.clone(),
    };
    let definitions = [
        Definition {
            name: "invariant".into(),
            params: vec![param("state", state)],
            body: invariant.clone(),
            description: "The invariant, of a state.".into(),
        },
        Definition {
            name: "next".into(),
            params: vec![param("state", state), param("input", input)],
            body: next.clone(),
            description: "The state the step gives.".into(),
        },
    ];
    let mut encoder = Encoder::new().reading(floats);
    let mut entries = Vec::new();
    let mut partial = Vec::new();
    for def in &definitions {
        let refused = |error| Refusal {
            definition: def.name.clone(),
            error: Box::new(error),
        };
        let scope = def
            .params
            .iter()
            .map(|p| (p.name.clone(), p.ty.clone()))
            .collect();
        let (_, problems) = infer(&def.body, scope);
        if !problems.is_empty() {
            return Err(refused(ExportError::IllFormed { problems }));
        }
        let encoded = encoder.definition(def).map_err(refused)?;
        partial.push(encoded.len() > 1);
        entries.extend(encoded);
    }
    let refused = |error| Refusal {
        definition: String::new(),
        error: Box::new(error),
    };
    let at = |reason| {
        refused(ExportError::NoReading {
            place: Place::Body { at: String::new() },
            reason: Box::new(reason),
        })
    };
    let state_sort = encoder.sort(state).map_err(at)?;
    let input_sort = encoder.sort(input).map_err(at)?;
    let records = encoder.records().map_err(refused)?;
    let name = |prefix: &str, n: &str| script::Term::symbol(prefix, n).map_err(at);
    let (s, i) = (name("q:", "state")?, name("q:", "input")?);
    // Where a function errs in the kernel, it holds of nothing: the invariant is false, the step doesn't happen.
    let holds_of = |of: script::Term| -> Result<script::Term, Refusal> {
        let held = script::Term::app(name("def:", "invariant")?, vec![of.clone()]);
        Ok(if partial[0] {
            let defined = script::Term::app(name("defined:", "invariant")?, vec![of]);
            script::Term::call("and", vec![defined, held])
        } else {
            held
        })
    };
    let after = script::Term::app(name("def:", "next")?, vec![s.clone(), i.clone()]);
    let mut asserts = vec![holds_of(s.clone())?];
    if partial[1] {
        asserts.push(script::Term::app(
            name("defined:", "next")?,
            vec![s.clone(), i.clone()],
        ));
    }
    asserts.push(script::Term::call("not", vec![holds_of(after)?]));
    let command = |c| script::Entry {
        comments: vec![],
        command: Some(c),
    };
    let mut query = vec![
        command(script::Command::DeclareConst {
            name: s.clone(),
            sort: state_sort,
        }),
        command(script::Command::DeclareConst {
            name: i.clone(),
            sort: input_sort,
        }),
    ];
    query.extend(
        asserts
            .into_iter()
            .map(|a| command(script::Command::Assert(a))),
    );
    query.push(command(script::Command::CheckSat));
    query.push(command(script::Command::GetValue(vec![s, i])));
    let mut preamble = vec![
        "Consecution: sat exactly where the invariant holds of a state and fails after one step."
            .into(),
        "A model is a counterexample to induction for the kernel to check; unsat is not a verdict."
            .into(),
    ];
    if !excluded.is_empty() {
        preamble.push(format!(
            "Inputs left out, whose types have no sort: {}.",
            excluded.join(", ")
        ));
    }
    let script = Script {
        preamble,
        options: vec![(":produce-models", "true")],
        logic: "ALL",
        sections: vec![
            records,
            script::Section {
                heading: "Definitions.",
                entries,
            },
            script::Section {
                heading: "A counterexample to induction.",
                entries: query,
            },
        ],
    };
    Ok(script.to_string())
}

/// A value as the term an export writes for it at this type.
pub fn value(v: &Value, ty: &Type) -> Result<String, NoReading> {
    Ok(Encoder::new().value(v, ty)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(name: &str, params: Value, body: Value) -> Definition {
        serde_json::from_value(json!({ "name": name, "params": params, "body": body })).unwrap()
    }

    fn n_int() -> Value {
        json!([{ "name": "n", "type": "Int" }])
    }

    #[test]
    fn integers_compare_as_integers_as_the_kernel_compares_them() {
        let body = json!({ "e": "op", "op": "lt", "args": [{ "e": "var", "name": "n" }, { "e": "lit", "v": 2 }] });
        let text = export(&[def("small", n_int(), body)], Floats::Exact).unwrap();
        assert!(
            text.contains("(define-fun |def:small| ((|v:n| Int)) Bool (< |v:n| 2))"),
            "{text}"
        );
    }

    #[test]
    fn where_evaluation_can_fail_is_stated() {
        let body = json!({ "e": "op", "op": "idiv", "args": [{ "e": "lit", "v": 10 }, { "e": "var", "name": "n" }] });
        let text = export(&[def("tenth", n_int(), body)], Floats::Exact).unwrap();
        assert!(
            text.contains("(define-fun |defined:tenth| ((|v:n| Int)) Bool"),
            "{text}"
        );
    }

    #[test]
    fn only_well_typed_definitions_are_exported() {
        let body = json!({ "e": "op", "op": "add", "args": [{ "e": "var", "name": "n" }, { "e": "lit", "v": "one" }] });
        let refused = export(&[def("bad", n_int(), body)], Floats::Exact).unwrap_err();
        assert_eq!(refused.definition, "bad");
        assert!(matches!(*refused.error, ExportError::IllFormed { .. }));
    }

    #[test]
    fn what_has_no_reading_is_refused_where_it_is() {
        let record = json!({ "e": "record", "fields": { "a": { "e": "lit", "v": 1 } } });
        let body = json!({ "e": "op", "op": "eq", "args": [record, record] });
        let a = Type::Record(std::collections::BTreeMap::from([("a".into(), Type::Int)]));
        assert_eq!(
            *export(&[def("records", json!([]), body)], Floats::Exact)
                .unwrap_err()
                .error,
            ExportError::NoReading {
                place: Place::Body { at: String::new() },
                reason: Box::new(NoReading::Equality {
                    left: a.clone(),
                    right: a
                }),
            }
        );
        let fold = json!({ "e": "fold", "over": { "e": "lit", "v": [1] }, "init": { "e": "lit", "v": 0 }, "acc": "a", "item": "x", "body": { "e": "var", "name": "a" } });
        assert_eq!(
            *export(&[def("folded", json!([]), fold)], Floats::Exact)
                .unwrap_err()
                .error,
            ExportError::NoReading {
                place: Place::Body { at: String::new() },
                reason: Box::new(NoReading::Fold)
            }
        );
    }

    #[test]
    fn a_precondition_exports_with_its_records() {
        // A workspace rule over a record of arguments, ready for a solver to compare with a narrower one.
        let args = json!([{ "name": "args", "type": { "Record": { "path": "String" } } }]);
        let within = json!({ "e": "op", "op": "prefix", "args": [{ "e": "lit", "v": "/workspace/" }, { "e": "get", "of": { "e": "var", "name": "args" }, "field": "path" }] });
        let text = export(&[def("pre", args, within)], Floats::Exact).unwrap();
        assert!(text.contains("(declare-datatype |record:"), "{text}");
        assert!(text.contains("str.prefixof \"/workspace/\""), "{text}");
    }
}
