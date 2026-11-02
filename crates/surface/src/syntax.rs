//! What the parser reads a file as, before any name is resolved.

use crate::lex::Span;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Name {
    pub text: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeExpr {
    /// `String`, `Int`, `Float`, `Bool`, `Any`, or a type declared with `type`.
    Named(Name),
    List(Box<TypeExpr>, Span),
    Record(Vec<(Name, TypeExpr)>, Span),
    /// Cases tagged by `kind`, each with the fields of its record, if any.
    Variant(Vec<(Name, Option<TypeExpr>)>, Span),
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named(n) => n.span,
            TypeExpr::List(_, s) | TypeExpr::Record(_, s) | TypeExpr::Variant(_, s) => *s,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unary {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Binary {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    /// `a implies b`: `not a or b`.
    Implies,
    /// `a since b`: only in properties.
    Since,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Lit(Value),
    Name(String),
    Field(Box<Expr>, Name),
    Record(Vec<(Name, Expr)>),
    List(Vec<Expr>),
    /// `f(a, b)` or `cap.op(field: e)`. Which one is decided when names are resolved.
    Call(Box<Expr>, Vec<Arg>),
    Unary(Unary, Box<Expr>),
    Binary(Binary, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Match(Box<Expr>, Vec<Arm>),
    Fold {
        over: Box<Expr>,
        init: Box<Expr>,
        acc: Name,
        item: Name,
        body: Box<Expr>,
    },
    /// `exists x in e: body` or `forall x in e: body`: only in properties, over the values `e` gave so far.
    Quantified {
        exists: bool,
        var: Name,
        over: Box<Expr>,
        body: Box<Expr>,
    },
    /// `for item in over: call`: one call per item. Only a node can be one.
    ForEach {
        item: Name,
        over: Box<Expr>,
        call: Box<Expr>,
    },
    /// `spawn child(arg) only a.b, c.d without e.f`: a child run against a narrowed interface. Only a node can be one.
    Spawn {
        harness: Name,
        arg: Box<Expr>,
        only: Option<Vec<Name>>,
        without: Vec<Name>,
    },
    /// `receive m from channel`: the first message on a channel. Only a node can be one.
    Receive {
        binder: Option<Name>,
        /// The channel's full name, `prefix.name`.
        channel: Name,
    },
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<Name>,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pattern: Pattern,
    pub body: Expr,
}

#[derive(Clone, Debug)]
pub enum Pattern {
    /// `_`.
    Wild(Span),
    /// `case` or `case(bind)`: a variant's case, or a call node's outcome.
    Case { case: Name, bind: Option<Name> },
    /// `"a" | "b"`: equal to one of these.
    Literals(Vec<Value>, Span),
}

#[derive(Clone, Debug)]
pub struct Use {
    pub name: Name,
    /// The capability's identifier, when it isn't the name itself.
    pub capability: Option<(String, Span)>,
    /// `uses name = harness "id"`: a registered harness to spawn, not a capability.
    pub harness: bool,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: Name,
    pub value: Expr,
    /// A call's guard, or a receive's filter.
    pub when: Option<Expr>,
    /// The node whose settling cancels this call.
    pub until: Option<Name>,
}

#[derive(Clone, Debug)]
pub struct File {
    pub name: Name,
    pub param: Name,
    pub param_type: TypeExpr,
    pub result_type: TypeExpr,
    pub uses: Vec<Use>,
    pub types: Vec<(Name, TypeExpr)>,
    pub nodes: Vec<Node>,
    /// Every `return`, so more than one can be reported.
    pub returns: Vec<Expr>,
    pub otherwise: Vec<Expr>,
    /// `property name: formula`, each `□formula` over the events of a run.
    pub properties: Vec<(Name, Expr)>,
    /// `invariant name: expr`, over the node slots of the state.
    pub invariants: Vec<(Name, Expr)>,
}
