//! Types and expressions, as their JSON form.
//!
//! Values are plain JSON. Everything else is a small tagged structure, so that the TypeScript ports and the Gleam
//! frontend can produce the same objects without a shared code generator.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A structural type over JSON values. `Int` conforms to `Float`, a record with more fields conforms to one with fewer,
/// and every type conforms to `Json`, which is read only after `is` has tested it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Type {
    Int,
    Float,
    String,
    Bool,
    /// Any JSON value: null, a boolean, a number, a string, a list of them or a record of them. Every value is one, so
    /// every type conforms to it, and it conforms to nothing else: what it holds is read only by testing it with `is`.
    Json,
    /// An action the harness may take against the interface it runs against: its interface's action type, resolved
    /// where the step is checked, run or read in SMT-LIB. So one term tests a choice made at run time (a model's tool
    /// call) against whatever its interface allows, and a narrowed child against what its narrowing leaves.
    Action,
    /// A call of one of the operations of the interface the harness runs against: `Action` with only its `call` case,
    /// so a variant over `kind` whose one case is a variant over `op`, each holding that operation's `args`. It
    /// conforms to `Action`, so a `Call` is a call action wherever one is wanted. Resolved where `Action` is, and where
    /// an operation's output is read, so a model's chosen call is typed by whatever its interface allows.
    Call,
    List(Box<Type>),
    Record(BTreeMap<String, Type>),
    /// A tagged union: a record whose `tag` field names its case, and which has that case's type. A case is a record
    /// type, or another variant with a different tag.
    Variant {
        tag: String,
        cases: BTreeMap<String, Type>,
    },
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
    /// `name` is `be` in `body`. With `type`, `be` is checked against it, and `name` has it.
    Let {
        name: String,
        be: Box<Expr>,
        #[serde(rename = "in")]
        body: Box<Expr>,
        #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
        ty: Option<Type>,
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
    /// The case of `of`, a variant, read off its `on` field (`kind` by default): the body of that case, with `bind`
    /// holding the value at the case's type. Cases must cover the variant unless there is an `otherwise`.
    Match {
        of: Box<Expr>,
        #[serde(default = "default_tag", skip_serializing_if = "is_default_tag")]
        on: String,
        cases: BTreeMap<String, Case>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        otherwise: Option<Box<Expr>>,
    },
    /// Whether `of` has type `type`: if it does, `then`, with `bind` holding it at that type; if not, `else`. How a
    /// `Json` value is read, such as a tool call a model chose, checked against the operations it may name.
    Is {
        of: Box<Expr>,
        #[serde(rename = "type")]
        ty: Type,
        bind: String,
        then: Box<Expr>,
        #[serde(rename = "else")]
        otherwise: Box<Expr>,
    },
    /// `acc` starts as `init` and becomes `body` once per item of `over`, in order, with `item` bound to that item.
    /// The only iteration there is: it runs once per item of a list it is given, so evaluation always terminates.
    /// With `type`, the accumulator has it, and `init` and `body` are checked against it.
    Fold {
        over: Box<Expr>,
        init: Box<Expr>,
        acc: String,
        item: String,
        body: Box<Expr>,
        #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
        ty: Option<Type>,
    },
}

/// One case of a `match`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Case {
    pub bind: String,
    pub body: Expr,
}

pub fn default_tag() -> String {
    "kind".to_string()
}

fn is_default_tag(tag: &str) -> bool {
    tag == "kind"
}

impl Type {
    /// This type with `Action` as `action`, and `Call` as `action` with only its `call` case.
    pub fn resolved(&self, action: &Type) -> Type {
        match self {
            Type::Action => action.clone(),
            Type::Call => match action {
                Type::Variant { tag, cases } => Type::Variant {
                    tag: tag.clone(),
                    cases: cases
                        .iter()
                        .filter(|(case, _)| *case == "call")
                        .map(|(case, t)| (case.clone(), t.clone()))
                        .collect(),
                },
                _ => Type::Call,
            },
            Type::List(t) => Type::List(Box::new(t.resolved(action))),
            Type::Record(fields) => Type::Record(
                fields
                    .iter()
                    .map(|(n, t)| (n.clone(), t.resolved(action)))
                    .collect(),
            ),
            Type::Variant { tag, cases } => Type::Variant {
                tag: tag.clone(),
                cases: cases
                    .iter()
                    .map(|(n, t)| (n.clone(), t.resolved(action)))
                    .collect(),
            },
            other => other.clone(),
        }
    }
}

impl Expr {
    /// This expression with `Action` in its types as `action`.
    pub fn resolved(&self, action: &Type) -> Expr {
        let again = |e: &Expr| Box::new(e.resolved(action));
        match self {
            Expr::Lit { .. } | Expr::Var { .. } => self.clone(),
            Expr::Op { op, args } => Expr::Op {
                op: op.clone(),
                args: args.iter().map(|a| a.resolved(action)).collect(),
            },
            Expr::Let { name, be, body, ty } => Expr::Let {
                name: name.clone(),
                be: again(be),
                body: again(body),
                ty: ty.as_ref().map(|t| t.resolved(action)),
            },
            Expr::If {
                cond,
                then,
                otherwise,
            } => Expr::If {
                cond: again(cond),
                then: again(then),
                otherwise: again(otherwise),
            },
            Expr::Record { fields } => Expr::Record {
                fields: fields
                    .iter()
                    .map(|(n, e)| (n.clone(), e.resolved(action)))
                    .collect(),
            },
            Expr::List { items } => Expr::List {
                items: items.iter().map(|e| e.resolved(action)).collect(),
            },
            Expr::Get { of, field } => Expr::Get {
                of: again(of),
                field: field.clone(),
            },
            Expr::Label { of } => Expr::Label { of: again(of) },
            Expr::Is {
                of,
                ty,
                bind,
                then,
                otherwise,
            } => Expr::Is {
                of: again(of),
                ty: ty.resolved(action),
                bind: bind.clone(),
                then: again(then),
                otherwise: again(otherwise),
            },
            Expr::Match {
                of,
                on,
                cases,
                otherwise,
            } => Expr::Match {
                of: again(of),
                on: on.clone(),
                cases: cases
                    .iter()
                    .map(|(n, c)| {
                        let case = Case {
                            bind: c.bind.clone(),
                            body: c.body.resolved(action),
                        };
                        (n.clone(), case)
                    })
                    .collect(),
                otherwise: otherwise.as_ref().map(|o| again(o)),
            },
            Expr::Fold {
                over,
                init,
                acc,
                item,
                body,
                ty,
            } => Expr::Fold {
                over: again(over),
                init: again(init),
                acc: acc.clone(),
                item: item.clone(),
                body: again(body),
                ty: ty.as_ref().map(|t| t.resolved(action)),
            },
        }
    }
}
