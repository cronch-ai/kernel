//! The kernel's objects: Asks, Answers and Procedures, as their JSON form.
//!
//! Values are plain JSON. Everything else is a small tagged structure, so that the TypeScript ports and the Gleam
//! frontend can produce the same objects without a shared code generator.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A structural type over JSON values. `Int` conforms to `Float`, a record with more fields conforms to one with fewer,
/// and `Any` conforms both ways (it is how prose and unknown shapes enter without being checked).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Type {
    Int,
    Float,
    String,
    Bool,
    Any,
    List(Box<Type>),
    Record(BTreeMap<String, Type>),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: Type,
}

/// Whether a raw value can answer an Ask, or the Answer must be a derivation (a Procedure with no assumptions).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Form {
    #[default]
    Any,
    Derivation,
}

impl Form {
    fn is_any(&self) -> bool {
        *self == Form::Any
    }
}

/// A check on the value an Ask's Answer evaluates to. `at` navigates record fields first.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Validator {
    Min {
        value: f64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        at: Vec<String>,
    },
    Max {
        value: f64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        at: Vec<String>,
    },
    NonEmpty {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        at: Vec<String>,
    },
    OneOf {
        values: Vec<Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        at: Vec<String>,
    },
    MinLength {
        value: u64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        at: Vec<String>,
    },
    MaxLength {
        value: u64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        at: Vec<String>,
    },
    /// Delegated to a Checker Actor. The kernel records it as `delegated` until Evidence for it is supplied.
    Checker {
        actor: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        note: String,
    },
    /// An acceptance criterion stated for the answering Actor. Never checked, never reported as passed.
    Prose { text: String },
}

/// An Ask's signature and contract: a typed hole.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AskSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<Param>,
    pub output: Type,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validators: Vec<Validator>,
    #[serde(default, skip_serializing_if = "Form::is_any")]
    pub form: Form,
    /// An effect Ask changes the world: it needs authority, and is never answered from the store.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub effect: bool,
}

/// A closed Ask: a spec with a value for each parameter.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Ask {
    pub spec: AskSpec,
    #[serde(default)]
    pub args: BTreeMap<String, Value>,
}

/// A named input the requester declares, with where it came from. Using one is an environmental assumption.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Fact {
    pub value: Value,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<Type>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// The context an Ask is answered in: information (facts) and capabilities (Actors the responder may route to).
/// Facts are part of the Ask's content key; capabilities are not, since they don't change what a correct Answer is.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct World {
    #[serde(default)]
    pub facts: BTreeMap<String, Fact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<String>,
}

/// A Procedure's body. `ask` is a computational hole; `assume`, `opaque` and `fact` are epistemic ones.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "e", rename_all = "camelCase")]
pub enum Expr {
    Lit {
        v: Value,
    },
    Var {
        name: String,
    },
    Fact {
        name: String,
    },
    Op {
        op: String,
        args: Vec<Expr>,
    },
    Ask {
        spec: AskSpec,
        #[serde(default)]
        args: BTreeMap<String, Expr>,
    },
    Assume {
        v: Value,
        #[serde(rename = "type")]
        ty: Type,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        note: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        source: String,
    },
    Opaque {
        #[serde(rename = "type")]
        ty: Type,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        note: String,
    },
    Let {
        name: String,
        be: Box<Expr>,
        #[serde(rename = "in")]
        body: Box<Expr>,
    },
    If {
        cond: Box<Expr>,
        then: Box<Expr>,
        #[serde(rename = "else")]
        otherwise: Box<Expr>,
    },
    Record {
        fields: BTreeMap<String, Expr>,
    },
    List {
        items: Vec<Expr>,
    },
    Get {
        of: Box<Expr>,
        field: String,
    },
    /// `acc` starts as `init` and becomes `body` once per item of `over`, in order, with `item` bound to that item.
    /// The only iteration there is: it runs once per item of a list it is given, so evaluation always terminates.
    Fold {
        over: Box<Expr>,
        init: Box<Expr>,
        acc: String,
        item: String,
        body: Box<Expr>,
    },
}

/// What an Actor returns: a raw value, or a Procedure over the Ask's parameters.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    Value(Value),
    Procedure(Expr),
}
