//! Types and expressions, as their JSON form.
//!
//! Values are plain JSON. Everything else is a small tagged structure, so that the TypeScript ports and the Gleam
//! frontend can produce the same objects without a shared code generator.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A structural type over JSON values. `Int` conforms to `Float`, a record with more fields conforms to one with fewer,
/// and `Any` conforms both ways: it is how shapes the interpreter checks when they arrive (a step's input, an event)
/// enter without being checked statically.
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

/// A named, typed parameter.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: Type,
}

/// A pure expression: what a harness's step function, its contracts and its properties are written in. It reads
/// variables and computes; it has no effects, holes or loops other than `fold`, so evaluating it always ends.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "e", rename_all = "camelCase")]
pub enum Expr {
    Lit {
        v: Value,
    },
    Var {
        name: String,
    },
    Op {
        op: String,
        args: Vec<Expr>,
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
    /// The integrity label of a value: the untrusted sources it may depend on, as a sorted list. Empty means trusted.
    Label {
        of: Box<Expr>,
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
