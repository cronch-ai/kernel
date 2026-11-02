//! Types and expressions, as their JSON form.
//!
//! Values are plain JSON. Everything else is a small tagged structure, so that the TypeScript ports and the Gleam
//! frontend can produce the same objects without a shared code generator.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A structural type over JSON values. `Int` conforms to `Float`, and a record with more fields conforms to one with
/// fewer. There is no type of any value: what no contract has typed is text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Type {
    Int,
    Float,
    String,
    Bool,
    /// An action the harness may take against the interface it runs against: its interface's action type, resolved
    /// where the step is checked, run or read in SMT-LIB. So one term tests a choice made at run time (a model's tool
    /// call) against whatever its interface allows, and a narrowed child against what its narrowing leaves.
    Action,
    /// A call of one of the operations of the interface the harness runs against: `Action` with only its `call` case,
    /// so a variant over `kind` whose one case is a variant over `op`, each holding that operation's `args`. It
    /// conforms to `Action`, so a `Call` is a call action wherever one is wanted. Resolved where `Action` is, and where
    /// an operation's output is read, so a model's chosen call is typed by whatever its interface allows.
    Call,
    /// What a call chosen at run time returned: a variant over `op` with a case for each thing that can answer a call in
    /// the interface the harness runs against, holding its `call` and `value`, and the case `@none`, for a call that
    /// hasn't returned. Resolved where `Call` is, so one term records a result whichever operation gave it.
    Returned,
    List(Box<Type>),
    Record(BTreeMap<String, Type>),
    /// A tagged union: a record whose `tag` field names its case, and which has that case's type. A case is a record
    /// type, or another variant with a different tag.
    Variant {
        tag: String,
        cases: BTreeMap<String, Type>,
    },
}

/// What the types an interface decides are, against one interface.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolution {
    pub action: Type,
    pub returned: Type,
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
    /// This type with `Action`, `Call` and `Returned` as they are against one interface. `Call` is `Action` with only
    /// its `call` case.
    pub fn resolved(&self, with: &Resolution) -> Type {
        match self {
            Type::Action => with.action.clone(),
            Type::Returned => with.returned.clone(),
            Type::Call => match &with.action {
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
            Type::List(t) => Type::List(Box::new(t.resolved(with))),
            Type::Record(fields) => Type::Record(
                fields
                    .iter()
                    .map(|(n, t)| (n.clone(), t.resolved(with)))
                    .collect(),
            ),
            Type::Variant { tag, cases } => Type::Variant {
                tag: tag.clone(),
                cases: cases
                    .iter()
                    .map(|(n, t)| (n.clone(), t.resolved(with)))
                    .collect(),
            },
            other => other.clone(),
        }
    }
}

impl Expr {
    /// This expression with the types an interface decides as they are against one interface.
    pub fn resolved(&self, with: &Resolution) -> Expr {
        let again = |e: &Expr| Box::new(e.resolved(with));
        match self {
            Expr::Lit { .. } | Expr::Var { .. } => self.clone(),
            Expr::Op { op, args } => Expr::Op {
                op: op.clone(),
                args: args.iter().map(|a| a.resolved(with)).collect(),
            },
            Expr::Let { name, be, body, ty } => Expr::Let {
                name: name.clone(),
                be: again(be),
                body: again(body),
                ty: ty.as_ref().map(|t| t.resolved(with)),
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
                    .map(|(n, e)| (n.clone(), e.resolved(with)))
                    .collect(),
            },
            Expr::List { items } => Expr::List {
                items: items.iter().map(|e| e.resolved(with)).collect(),
            },
            Expr::Get { of, field } => Expr::Get {
                of: again(of),
                field: field.clone(),
            },
            Expr::Label { of } => Expr::Label { of: again(of) },
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
                            body: c.body.resolved(with),
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
                ty: ty.as_ref().map(|t| t.resolved(with)),
            },
        }
    }
}
