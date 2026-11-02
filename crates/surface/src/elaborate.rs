//! From a parsed file to a harness.
//!
//! Names are resolved and the graph is checked first: every reference resolves, and no node depends on itself.
//! Each expression is then typed by the kernel's checker, against the interface the `uses` bindings install, and
//! the step function is generated.
//!
//! The generated state is `{arg, next, nodes}`: the start value, the identifier the interpreter will give the next
//! call, and one slot per call node. A slot is a record `{kind, call, value, error}` whose `kind` (`waiting`,
//! `running`, `ok`, `failed`, `refused` or `skipped`) says which of the other fields mean anything. It is a record
//! rather than a variant because the kernel has no expression that builds a value at a variant type. `value` is
//! carried from slot to slot, never rebuilt from a literal, so it keeps the operation's output type throughout.
//! `error` is the failure's error as text (`str`), or `"cancelled"`: a failure's error is `Any`, and a state that
//! holds `Any` has no SMT sort, so no induction obligation could be written for it.
//!
//! The generated step does three things, in order:
//!
//! 1. Records its input: `start` stores the start value, and a result settles the slot running its call.
//! 2. Halts through `otherwise` if a node whose failure the harness doesn't handle has failed or been refused.
//! 3. Schedules: each waiting call node whose dependencies have settled, in dependency order, is skipped or
//!    started. Each is a `let` over the state the one before produced, so one pass reaches every node that can
//!    move. Once every slot has settled, it halts with `return`.

use crate::claims::Invariant;
use crate::lex::{Span, lex};
use crate::parse::parse;
use crate::syntax::{self, Binary, ExprKind, File, Pattern, TypeExpr, Unary};
use crate::{Problem, What};
use kernel::check::infer;
use kernel::harness::{
    Harness, Installation, Interface, Mode, Narrowing, Signature, check_harness, install, narrow,
    spawn_op,
};
use kernel::ir::{Case, Expr as K, Type};
use kernel::property::Property;
use kernel::types::conforms;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// A capability the harness uses, and the prefix it is installed under.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Binding {
    pub name: String,
    pub capability: String,
}

/// A registered harness the harness spawns, by the name it uses for it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HarnessBinding {
    pub name: String,
    pub harness: String,
}

/// Where each named thing of the source is, so that what the kernel reports about the term (a counterexample's
/// slots, a refused call, a violated property) can be read back as source lines.
#[derive(Serialize, Clone, Debug, Default)]
pub struct SourceMap {
    pub nodes: BTreeMap<String, Span>,
    pub properties: BTreeMap<String, Span>,
    pub invariants: BTreeMap<String, Span>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Elaborated {
    pub harness: Harness,
    pub uses: Vec<Binding>,
    pub harnesses: Vec<HarnessBinding>,
    pub properties: Vec<Property>,
    pub invariants: Vec<Invariant>,
    pub source_map: SourceMap,
}

/// Functions an expression may call, each an operator of the kernel's but `label`.
const FUNCTIONS: [&str; 7] = ["len", "str", "round", "prefix", "concat", "append", "label"];

const OUTCOMES: [&str; 4] = ["ok", "failed", "refused", "skipped"];

/// What a name in an expression refers to, unless a local binding shadows it.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Bound {
    Param,
    Node(usize),
    Capability,
    /// A registered harness, to spawn.
    Harness,
}

/// How an expression reads a node: as its value, or through a `match` on its outcome.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Read {
    Value,
    /// Whether the `match` has arms for failure and refusal, explicitly or through `_`.
    Outcome {
        handles_failure: bool,
    },
}

pub(crate) type Reads = Vec<(usize, Read)>;

/// Everything a consumer needs bound: call nodes read as values, call nodes read through `match`, and pure nodes,
/// each through any pure node it reads.
#[derive(Default, Clone)]
struct Needs {
    values: BTreeSet<usize>,
    outcomes: BTreeSet<usize>,
    pures: BTreeSet<usize>,
}

impl Needs {
    fn add(&mut self, other: &Needs) {
        self.values.extend(&other.values);
        self.outcomes.extend(&other.outcomes);
        self.pures.extend(&other.pures);
    }

    fn calls(&self) -> BTreeSet<usize> {
        self.values.union(&self.outcomes).copied().collect()
    }
}

/// What fills a slot.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Slotted {
    /// A call that delivers one result.
    Single,
    /// A call that delivers items, then an end.
    Stream,
    /// A message on a channel.
    Receive,
    /// One call per item of a list, issued together.
    FanOut,
    /// A child run.
    Spawn,
}

/// An input about a call.
#[derive(Clone, Copy, PartialEq)]
enum Input {
    Ret,
    End,
    Failed,
    Refused,
}

/// A node with a slot in the state: a call of an operation, or a receive on a channel.
pub(crate) struct Call {
    pub(crate) kind: Slotted,
    /// The operation called, or the channel received on.
    target: String,
    input: Type,
    /// What the slot's `value` holds: the operation's output, a stream's items as a list, or the message.
    value: Type,
    /// What a receive names its message, for its filter, or a fan-out its item, for its arguments.
    binder: Option<String>,
    /// A spawn's child and narrowings, as its action carries them.
    child: Option<(Harness, Vec<Narrowing>)>,
    /// The node whose return cancels this call.
    until: Option<usize>,
}

pub(crate) struct Cx<'a> {
    pub(crate) file: &'a File,
    /// The harnesses the header binds, by name.
    children: BTreeMap<String, Harness>,
    pub(crate) problems: Vec<Problem>,
    pub(crate) names: BTreeMap<String, Bound>,
    /// The nodes with slots, by node index.
    pub(crate) calls: BTreeMap<usize, Call>,
    may_skip: BTreeSet<usize>,
    fresh: usize,
}

pub(crate) fn problem(span: Span, what: What) -> Problem {
    Problem { span, what }
}

// Builders for the kernel's terms.

pub(crate) fn lit(v: Value) -> K {
    K::Lit { v }
}

pub(crate) fn var(name: &str) -> K {
    K::Var {
        name: name.to_string(),
    }
}

pub(crate) fn get(of: K, field: &str) -> K {
    K::Get {
        of: Box::new(of),
        field: field.to_string(),
    }
}

pub(crate) fn op(name: &str, args: Vec<K>) -> K {
    K::Op {
        op: name.to_string(),
        args,
    }
}

pub(crate) fn let_(name: &str, be: K, body: K) -> K {
    K::Let {
        ty: None,
        name: name.to_string(),
        be: Box::new(be),
        body: Box::new(body),
    }
}

pub(crate) fn if_(cond: K, then: K, otherwise: K) -> K {
    K::If {
        cond: Box::new(cond),
        then: Box::new(then),
        otherwise: Box::new(otherwise),
    }
}

pub(crate) fn rec<S: Into<String>>(fields: Vec<(S, K)>) -> K {
    K::Record {
        fields: fields.into_iter().map(|(n, e)| (n.into(), e)).collect(),
    }
}

pub(crate) fn list(items: Vec<K>) -> K {
    K::List { items }
}

pub(crate) fn eq(a: K, b: K) -> K {
    op("eq", vec![a, b])
}

/// The strict conjunction: every operand here is total, so there is nothing to short-circuit.
pub(crate) fn all(items: Vec<K>) -> K {
    items
        .into_iter()
        .reduce(|a, b| op("and", vec![a, b]))
        .unwrap_or(lit(json!(true)))
}

pub(crate) fn any(items: Vec<K>) -> K {
    items
        .into_iter()
        .reduce(|a, b| op("or", vec![a, b]))
        .unwrap_or(lit(json!(false)))
}

pub(crate) fn one_of(kinds: &[&str], of: K) -> K {
    op("contains", vec![lit(json!(kinds)), of])
}

fn step(state: K, actions: K) -> K {
    rec(vec![("state", state), ("actions", actions)])
}

fn halt(value: K) -> K {
    rec(vec![("kind", lit(json!("halt"))), ("value", value)])
}

/// A value of the type, for slots and the start value before anything has filled them.
fn default_value(ty: &Type) -> Value {
    match ty {
        Type::Int => json!(0),
        Type::Float => json!(0.0),
        Type::String => json!(""),
        Type::Bool => json!(false),
        Type::Any => Value::Null,
        Type::List(_) => json!([]),
        Type::Record(fields) => Value::Object(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), default_value(t)))
                .collect(),
        ),
        Type::Variant { tag, cases } => match cases.iter().next() {
            Some((case, t)) => {
                let mut v = default_value(t);
                if let Value::Object(fields) = &mut v {
                    fields.insert(tag.clone(), json!(case));
                }
                v
            }
            None => Value::Null,
        },
    }
}

fn slot_type(call: &Call) -> Type {
    let mut fields = BTreeMap::from([
        ("kind".to_string(), Type::String),
        ("call".to_string(), Type::Int),
        ("value".to_string(), call.value.clone()),
        ("error".to_string(), Type::String),
    ]);
    // A fan-out's calls have consecutive identifiers from `call`; `items` holds each one's kind.
    if call.kind == Slotted::FanOut {
        fields.insert("items".to_string(), Type::List(Box::new(Type::String)));
    }
    Type::Record(fields)
}

fn initial_slot(call: &Call) -> Value {
    let mut slot =
        json!({ "kind": "waiting", "call": 0, "value": default_value(&call.value), "error": "" });
    if call.kind == Slotted::FanOut {
        slot["items"] = json!([]);
    }
    slot
}

/// `list` with its item at `index` replaced by `value`. The result is typed by the fold's empty start, so it is a
/// list of `Any`: the kernel has no typed empty list.
fn replaced(items: K, index: K, value: K) -> K {
    let acc = || var("@b");
    let body = rec(vec![
        ("i", op("add", vec![get(acc(), "i"), lit(json!(1))])),
        (
            "out",
            op(
                "append",
                vec![
                    get(acc(), "out"),
                    if_(eq(get(acc(), "i"), index), value, var("@y")),
                ],
            ),
        ),
    ]);
    let fold = K::Fold {
        over: Box::new(items),
        init: Box::new(rec(vec![("i", lit(json!(0))), ("out", list(vec![]))])),
        acc: "@b".to_string(),
        item: "@y".to_string(),
        body: Box::new(body),
        ty: None,
    };
    get(fold, "out")
}

/// `body` folded over `over` from `init`, with the accumulator as `@a` and each item as `item`.
fn fold_into(over: K, init: K, item: &str, body: K) -> K {
    K::Fold {
        over: Box::new(over),
        init: Box::new(init),
        acc: "@a".to_string(),
        item: item.to_string(),
        body: Box::new(body),
        ty: None,
    }
}

impl<'a> Cx<'a> {
    pub(crate) fn report(&mut self, span: Span, what: What) {
        self.problems.push(problem(span, what));
    }

    fn fresh(&mut self) -> String {
        self.fresh += 1;
        format!("@{}", self.fresh)
    }

    pub(crate) fn node_name(&self, i: usize) -> &str {
        &self.file.nodes[i].name.text
    }

    fn types(&mut self) -> (Type, Type) {
        let file = self.file;
        let mut declared: BTreeMap<String, &TypeExpr> = BTreeMap::new();
        for (name, ty) in &file.types {
            let builtin =
                ["String", "Int", "Float", "Bool", "Any", "List"].contains(&name.text.as_str());
            if builtin || declared.insert(name.text.clone(), ty).is_some() {
                self.report(
                    name.span,
                    What::Duplicate {
                        name: name.text.clone(),
                    },
                );
            }
        }
        let mut resolved = BTreeMap::new();
        for (name, _) in &file.types {
            self.named_type(
                &name.text,
                name.span,
                &declared,
                &mut resolved,
                &mut Vec::new(),
            );
        }
        let arg = self.resolve_type(&file.param_type, &declared, &mut resolved, &mut Vec::new());
        let result =
            self.resolve_type(&file.result_type, &declared, &mut resolved, &mut Vec::new());
        (arg, result)
    }

    fn named_type(
        &mut self,
        name: &str,
        span: Span,
        declared: &BTreeMap<String, &TypeExpr>,
        resolved: &mut BTreeMap<String, Type>,
        visiting: &mut Vec<String>,
    ) -> Type {
        match name {
            "String" => return Type::String,
            "Int" => return Type::Int,
            "Float" => return Type::Float,
            "Bool" => return Type::Bool,
            "Any" => return Type::Any,
            _ => {}
        }
        if let Some(t) = resolved.get(name) {
            return t.clone();
        }
        let Some(te) = declared.get(name) else {
            self.report(
                span,
                What::UnknownType {
                    name: name.to_string(),
                },
            );
            return Type::Any;
        };
        if visiting.iter().any(|v| v == name) {
            self.report(
                span,
                What::RecursiveType {
                    name: name.to_string(),
                },
            );
            return Type::Any;
        }
        visiting.push(name.to_string());
        let t = self.resolve_type(te, declared, resolved, visiting);
        visiting.pop();
        resolved.insert(name.to_string(), t.clone());
        t
    }

    fn resolve_type(
        &mut self,
        te: &TypeExpr,
        declared: &BTreeMap<String, &TypeExpr>,
        resolved: &mut BTreeMap<String, Type>,
        visiting: &mut Vec<String>,
    ) -> Type {
        match te {
            TypeExpr::Named(n) => self.named_type(&n.text, n.span, declared, resolved, visiting),
            TypeExpr::List(item, _) => Type::List(Box::new(
                self.resolve_type(item, declared, resolved, visiting),
            )),
            TypeExpr::Record(fields, _) => {
                let mut out = BTreeMap::new();
                for (name, t) in fields {
                    let t = self.resolve_type(t, declared, resolved, visiting);
                    if out.insert(name.text.clone(), t).is_some() {
                        self.report(
                            name.span,
                            What::Duplicate {
                                name: name.text.clone(),
                            },
                        );
                    }
                }
                Type::Record(out)
            }
            TypeExpr::Variant(cases, _) => {
                let mut out = BTreeMap::new();
                for (name, fields) in cases {
                    let t = match fields {
                        Some(f) => self.resolve_type(f, declared, resolved, visiting),
                        None => Type::Record(BTreeMap::new()),
                    };
                    if out.insert(name.text.clone(), t).is_some() {
                        self.report(
                            name.span,
                            What::Duplicate {
                                name: name.text.clone(),
                            },
                        );
                    }
                }
                Type::Variant {
                    tag: "kind".to_string(),
                    cases: out,
                }
            }
        }
    }

    /// The interface the `uses` bindings install, and the bindings.
    fn install(
        &mut self,
        capabilities: &BTreeMap<String, Interface>,
        harnesses: &BTreeMap<String, Harness>,
        spawned: &mut Vec<HarnessBinding>,
    ) -> (Interface, Vec<Binding>) {
        let mut installations = Vec::new();
        let mut bindings = Vec::new();
        for u in &self.file.uses.clone() {
            let (id, span) = match &u.capability {
                Some((id, span)) => (id.clone(), *span),
                None => (u.name.text.clone(), u.name.span),
            };
            if u.harness {
                if !self.bind(&u.name, Bound::Harness) {
                    continue;
                }
                match harnesses.get(&id) {
                    Some(h) => {
                        self.children.insert(u.name.text.clone(), h.clone());
                        spawned.push(HarnessBinding {
                            name: u.name.text.clone(),
                            harness: id,
                        });
                    }
                    None => self.report(
                        span,
                        What::UnknownHarness {
                            name: u.name.text.clone(),
                            harness: id,
                        },
                    ),
                }
                continue;
            }
            if !self.bind(&u.name, Bound::Capability) {
                continue;
            }
            let Some(capability) = capabilities.get(&id) else {
                self.report(
                    span,
                    What::UnknownCapability {
                        name: u.name.text.clone(),
                        capability: id,
                    },
                );
                continue;
            };
            installations.push(Installation {
                prefix: u.name.text.clone(),
                capability: capability.clone(),
            });
            bindings.push(Binding {
                name: u.name.text.clone(),
                capability: id,
            });
        }
        match install(&installations) {
            Ok(interface) => (interface, bindings),
            Err(kernel::harness::InstallError::Clash { name }) => {
                self.report(self.file.name.span, What::Duplicate { name });
                (Interface::default(), bindings)
            }
        }
    }

    fn bind(&mut self, name: &syntax::Name, bound: Bound) -> bool {
        if self.names.contains_key(&name.text) {
            self.report(
                name.span,
                What::Duplicate {
                    name: name.text.clone(),
                },
            );
            return false;
        }
        self.names.insert(name.text.clone(), bound);
        true
    }

    /// `cap.a.b` as the operation `cap.a.b`, when `cap` is a capability no local shadows.
    pub(crate) fn operation(&self, e: &syntax::Expr, locals: &[String]) -> Option<String> {
        let mut path = Vec::new();
        let mut at = e;
        while let ExprKind::Field(of, field) = &at.kind {
            path.push(field.text.as_str());
            at = of;
        }
        let ExprKind::Name(root) = &at.kind else {
            return None;
        };
        let is_capability =
            !locals.contains(root) && self.names.get(root) == Some(&Bound::Capability);
        if !is_capability || path.is_empty() {
            return None;
        }
        path.push(root);
        path.reverse();
        Some(path.join("."))
    }

    /// The call node a `match` reads the outcome of, if that's what it does.
    fn outcome_of(&self, scrutinee: &syntax::Expr, locals: &[String]) -> Option<usize> {
        let ExprKind::Name(n) = &scrutinee.kind else {
            return None;
        };
        match self.names.get(n) {
            Some(Bound::Node(i)) if !locals.contains(n) && self.calls.contains_key(i) => Some(*i),
            _ => None,
        }
    }

    /// Checks the names an expression uses, and records which nodes it reads and how.
    pub(crate) fn walk(&mut self, e: &syntax::Expr, locals: &mut Vec<String>, reads: &mut Reads) {
        match &e.kind {
            ExprKind::Lit(_) => {}
            ExprKind::Name(n) => {
                if locals.contains(n) {
                    return;
                }
                match self.names.get(n) {
                    None => self.report(e.span, What::UnknownName { name: n.clone() }),
                    Some(Bound::Capability | Bound::Harness) => {
                        self.report(e.span, What::NotAValue { name: n.clone() })
                    }
                    Some(Bound::Param) => {}
                    Some(Bound::Node(i)) => reads.push((*i, Read::Value)),
                }
            }
            ExprKind::Field(of, _) => self.walk(of, locals, reads),
            ExprKind::Record(fields) => {
                for (_, v) in fields {
                    self.walk(v, locals, reads);
                }
            }
            ExprKind::List(items) => {
                for v in items {
                    self.walk(v, locals, reads);
                }
            }
            ExprKind::Call(callee, args) => {
                if let Some(op) = self.operation(callee, locals) {
                    self.report(e.span, What::NestedCall { op });
                } else {
                    match &callee.kind {
                        ExprKind::Name(f) if FUNCTIONS.contains(&f.as_str()) => {
                            if let Some(named) = args.iter().find_map(|a| a.name.as_ref()) {
                                self.report(
                                    named.span,
                                    What::Argument {
                                        expected: format!("positional arguments to `{f}`"),
                                    },
                                );
                            }
                        }
                        ExprKind::Name(f) => {
                            self.report(callee.span, What::UnknownFunction { name: f.clone() })
                        }
                        _ => self.report(
                            callee.span,
                            What::UnknownFunction {
                                name: String::new(),
                            },
                        ),
                    }
                }
                for a in args {
                    self.walk(&a.value, locals, reads);
                }
            }
            ExprKind::Unary(_, a) => self.walk(a, locals, reads),
            ExprKind::Binary(b, x, y) => {
                if *b == Binary::Since {
                    self.report(
                        e.span,
                        What::PropertyOnly {
                            construct: "since".to_string(),
                        },
                    );
                }
                self.walk(x, locals, reads);
                self.walk(y, locals, reads);
            }
            ExprKind::Quantified { exists, .. } => {
                let construct = if *exists { "exists" } else { "forall" };
                self.report(
                    e.span,
                    What::PropertyOnly {
                        construct: construct.to_string(),
                    },
                );
            }
            ExprKind::If(c, t, o) => {
                self.walk(c, locals, reads);
                self.walk(t, locals, reads);
                self.walk(o, locals, reads);
            }
            ExprKind::Match(scrutinee, arms) => {
                match self.outcome_of(scrutinee, locals) {
                    Some(i) => {
                        let covers = |outcome: &str| {
                            arms.iter().any(|a| match &a.pattern {
                                Pattern::Wild(_) => true,
                                Pattern::Case { case, .. } => case.text == outcome,
                                Pattern::Literals(..) => false,
                            })
                        };
                        let handles_failure = covers("failed") && covers("refused");
                        reads.push((i, Read::Outcome { handles_failure }));
                    }
                    None => self.walk(scrutinee, locals, reads),
                }
                for arm in arms {
                    let bound = match &arm.pattern {
                        Pattern::Case { bind: Some(b), .. } => Some(b.text.clone()),
                        _ => None,
                    };
                    locals.extend(bound.clone());
                    self.walk(&arm.body, locals, reads);
                    if bound.is_some() {
                        locals.pop();
                    }
                }
            }
            ExprKind::Receive { .. } | ExprKind::ForEach { .. } | ExprKind::Spawn { .. } => {
                let op = match &e.kind {
                    ExprKind::Receive { .. } => "receive",
                    ExprKind::ForEach { .. } => "for",
                    _ => "spawn",
                };
                self.report(e.span, What::NestedCall { op: op.to_string() })
            }
            ExprKind::Fold {
                over,
                init,
                acc,
                item,
                body,
            } => {
                self.walk(over, locals, reads);
                self.walk(init, locals, reads);
                locals.push(acc.text.clone());
                locals.push(item.text.clone());
                self.walk(body, locals, reads);
                locals.truncate(locals.len() - 2);
            }
        }
    }

    fn reads(&mut self, e: &syntax::Expr) -> Reads {
        let mut reads = Vec::new();
        self.walk(e, &mut Vec::new(), &mut reads);
        reads
    }

    /// A dependency order of the nodes, or `None` after reporting a cycle.
    fn order(&mut self, deps: &[BTreeSet<usize>]) -> Option<Vec<usize>> {
        fn visit(
            i: usize,
            deps: &[BTreeSet<usize>],
            state: &mut [u8],
            path: &mut Vec<usize>,
            order: &mut Vec<usize>,
        ) -> Result<(), Vec<usize>> {
            state[i] = 1;
            path.push(i);
            for &d in &deps[i] {
                match state[d] {
                    1 => {
                        let from = path.iter().position(|&p| p == d).unwrap_or(0);
                        return Err(path[from..].to_vec());
                    }
                    0 => visit(d, deps, state, path, order)?,
                    _ => {}
                }
            }
            path.pop();
            state[i] = 2;
            order.push(i);
            Ok(())
        }
        let mut state = vec![0u8; deps.len()];
        let mut order = Vec::new();
        for i in 0..deps.len() {
            if state[i] == 0
                && let Err(cycle) = visit(i, deps, &mut state, &mut Vec::new(), &mut order)
            {
                let nodes = cycle
                    .iter()
                    .map(|&c| self.node_name(c).to_string())
                    .collect();
                self.report(self.file.nodes[cycle[0]].name.span, What::Cycle { nodes });
                return None;
            }
        }
        Some(order)
    }

    fn needs(&self, reads: &Reads, of_pure: &BTreeMap<usize, Needs>) -> Needs {
        let mut needs = Needs::default();
        for &(i, read) in reads {
            if self.calls.contains_key(&i) {
                match read {
                    Read::Value => needs.values.insert(i),
                    Read::Outcome { .. } => needs.outcomes.insert(i),
                };
            } else {
                needs.pures.insert(i);
                if let Some(n) = of_pure.get(&i) {
                    needs.add(n);
                }
            }
        }
        needs
    }

    // Lowering source expressions onto the kernel's.

    pub(crate) fn lower(&mut self, e: &syntax::Expr, locals: &mut Vec<String>) -> K {
        match &e.kind {
            ExprKind::Lit(v) => lit(v.clone()),
            ExprKind::Name(n) => var(n),
            ExprKind::Field(of, field) => get(self.lower(of, locals), &field.text),
            ExprKind::Record(fields) => {
                let mut out = BTreeMap::new();
                for (name, v) in fields {
                    let v = self.lower(v, locals);
                    if out.insert(name.text.clone(), v).is_some() {
                        self.report(
                            name.span,
                            What::Duplicate {
                                name: name.text.clone(),
                            },
                        );
                    }
                }
                K::Record { fields: out }
            }
            ExprKind::List(items) => list(items.iter().map(|i| self.lower(i, locals)).collect()),
            ExprKind::Call(callee, args) => {
                let args: Vec<K> = args.iter().map(|a| self.lower(&a.value, locals)).collect();
                match &callee.kind {
                    ExprKind::Name(f) if f == "label" => match <[K; 1]>::try_from(args) {
                        Ok([of]) => K::Label { of: Box::new(of) },
                        Err(_) => {
                            self.report(
                                e.span,
                                What::Argument {
                                    expected: "one argument to `label`".to_string(),
                                },
                            );
                            lit(Value::Null)
                        }
                    },
                    ExprKind::Name(f) => op(f, args),
                    _ => lit(Value::Null),
                }
            }
            ExprKind::Unary(u, a) => {
                let a = self.lower(a, locals);
                match u {
                    Unary::Neg => op("neg", vec![a]),
                    Unary::Not => op("not", vec![a]),
                }
            }
            ExprKind::Binary(b, x, y) => {
                let (x, y) = (self.lower(x, locals), self.lower(y, locals));
                let name = match b {
                    // `and` and `or` short-circuit, as a reader expects.
                    Binary::And => return if_(x, y, lit(json!(false))),
                    Binary::Or => return if_(x, lit(json!(true)), y),
                    Binary::In => return op("contains", vec![y, x]),
                    Binary::NotIn => return op("not", vec![op("contains", vec![y, x])]),
                    Binary::Implies => return if_(x, y, lit(json!(true))),
                    // Reported by `walk`: only a property can say these.
                    Binary::Since => return lit(Value::Null),
                    Binary::Eq => "eq",
                    Binary::Ne => "ne",
                    Binary::Lt => "lt",
                    Binary::Le => "le",
                    Binary::Gt => "gt",
                    Binary::Ge => "ge",
                    Binary::Add => "add",
                    Binary::Sub => "sub",
                    Binary::Mul => "mul",
                    Binary::Div => "div",
                    Binary::IDiv => "idiv",
                    Binary::Mod => "mod",
                };
                op(name, vec![x, y])
            }
            ExprKind::If(c, t, o) => if_(
                self.lower(c, locals),
                self.lower(t, locals),
                self.lower(o, locals),
            ),
            ExprKind::Match(scrutinee, arms) => match self.outcome_of(scrutinee, locals) {
                Some(i) => self.lower_outcome(e.span, i, arms, locals),
                None => self.lower_match(e.span, scrutinee, arms, locals),
            },
            // Reported by `walk`: these are only ever nodes, or only ever in properties.
            ExprKind::Receive { .. }
            | ExprKind::ForEach { .. }
            | ExprKind::Spawn { .. }
            | ExprKind::Quantified { .. } => lit(Value::Null),
            ExprKind::Fold {
                over,
                init,
                acc,
                item,
                body,
            } => {
                let over = self.lower(over, locals);
                let init = self.lower(init, locals);
                locals.push(acc.text.clone());
                locals.push(item.text.clone());
                let body = self.lower(body, locals);
                locals.truncate(locals.len() - 2);
                K::Fold {
                    over: Box::new(over),
                    init: Box::new(init),
                    acc: acc.text.clone(),
                    item: item.text.clone(),
                    body: Box::new(body),
                    ty: None,
                }
            }
        }
    }

    fn lower_arm(
        &mut self,
        bind: Option<(&str, K)>,
        body: &syntax::Expr,
        locals: &mut Vec<String>,
    ) -> K {
        match bind {
            Some((name, value)) => {
                locals.push(name.to_string());
                let body = self.lower(body, locals);
                locals.pop();
                let_(name, value, body)
            }
            None => self.lower(body, locals),
        }
    }

    /// A `match` on a call node's outcome: a chain of `if`s on its slot's `kind`. The last arm takes every case the
    /// arms before it don't, including those that can't happen once the node has settled.
    fn lower_outcome(
        &mut self,
        span: Span,
        node: usize,
        arms: &[syntax::Arm],
        locals: &mut Vec<String>,
    ) -> K {
        let name = self.node_name(node).to_string();
        let slot = self.fresh();
        let mut seen = BTreeSet::new();
        let mut wild = None;
        let mut chain: Vec<(&str, Option<String>, &syntax::Expr)> = Vec::new();
        for arm in arms {
            match &arm.pattern {
                Pattern::Wild(_) => {
                    wild.get_or_insert(&arm.body);
                }
                Pattern::Literals(_, at) => self.report(*at, What::MixedPatterns),
                Pattern::Case { case, bind } => {
                    let Some(outcome) = OUTCOMES.iter().find(|o| **o == case.text) else {
                        self.report(
                            case.span,
                            What::NoSuchOutcome {
                                outcome: case.text.clone(),
                            },
                        );
                        continue;
                    };
                    if !seen.insert(*outcome) {
                        self.report(
                            case.span,
                            What::Duplicate {
                                name: case.text.clone(),
                            },
                        );
                        continue;
                    }
                    if bind.is_some() && !["ok", "failed"].contains(outcome) {
                        self.report(
                            case.span,
                            What::Argument {
                                expected: format!("no binding: `{outcome}` carries nothing"),
                            },
                        );
                    }
                    chain.push((outcome, bind.as_ref().map(|b| b.text.clone()), &arm.body));
                }
            }
        }
        if wild.is_none() {
            let mut required = vec!["ok"];
            if self.may_skip.contains(&node) {
                required.push("skipped");
            }
            for outcome in required.into_iter().filter(|o| !seen.contains(o)) {
                self.report(
                    span,
                    What::Uncovered {
                        node: name.clone(),
                        outcome: outcome.to_string(),
                    },
                );
            }
        }
        let field = |outcome: &str| {
            if outcome == "failed" {
                "error"
            } else {
                "value"
            }
        };
        let mut lowered = match wild {
            Some(body) => self.lower(body, locals),
            None => match chain.pop() {
                Some((outcome, bind, body)) => {
                    let value = get(var(&slot), field(outcome));
                    self.lower_arm(bind.as_deref().map(|b| (b, value)), body, locals)
                }
                None => lit(Value::Null),
            },
        };
        for (outcome, bind, body) in chain.into_iter().rev() {
            let value = get(var(&slot), field(outcome));
            let then = self.lower_arm(bind.as_deref().map(|b| (b, value)), body, locals);
            lowered = if_(
                eq(get(var(&slot), "kind"), lit(json!(outcome))),
                then,
                lowered,
            );
        }
        let_(&slot, get(var("@nodes"), &name), lowered)
    }

    /// A `match` on a value: onto `Match` for case patterns, or a chain of `if`s for literal ones.
    fn lower_match(
        &mut self,
        span: Span,
        scrutinee: &syntax::Expr,
        arms: &[syntax::Arm],
        locals: &mut Vec<String>,
    ) -> K {
        let of = self.lower(scrutinee, locals);
        let wild = arms.iter().find_map(|a| match a.pattern {
            Pattern::Wild(_) => Some(&a.body),
            _ => None,
        });
        let literal = arms
            .iter()
            .any(|a| matches!(a.pattern, Pattern::Literals(..)));
        let case = arms
            .iter()
            .any(|a| matches!(a.pattern, Pattern::Case { .. }));
        if literal && case {
            self.report(span, What::MixedPatterns);
            return lit(Value::Null);
        }
        if literal {
            let Some(wild) = wild else {
                self.report(span, What::NoDefault);
                return lit(Value::Null);
            };
            let subject = self.fresh();
            let mut lowered = self.lower(wild, locals);
            for arm in arms.iter().rev() {
                if let Pattern::Literals(values, _) = &arm.pattern {
                    let cond = any(values
                        .iter()
                        .map(|v| eq(var(&subject), lit(v.clone())))
                        .collect());
                    lowered = if_(cond, self.lower(&arm.body, locals), lowered);
                }
            }
            return let_(&subject, of, lowered);
        }
        let mut cases = BTreeMap::new();
        for arm in arms {
            if let Pattern::Case { case, bind } = &arm.pattern {
                let bind = match bind {
                    Some(b) => b.text.clone(),
                    None => self.fresh(),
                };
                locals.push(bind.clone());
                let body = self.lower(&arm.body, locals);
                locals.pop();
                if cases
                    .insert(case.text.clone(), Case { bind, body })
                    .is_some()
                {
                    self.report(
                        case.span,
                        What::Duplicate {
                            name: case.text.clone(),
                        },
                    );
                }
            }
        }
        let otherwise = wild.map(|w| Box::new(self.lower(w, locals)));
        K::Match {
            of: Box::new(of),
            on: "kind".to_string(),
            cases,
            otherwise,
        }
    }

    /// The kernel's type of a lowered expression, with its problems reported at the source's span.
    pub(crate) fn type_of(
        &mut self,
        lowered: &K,
        span: Span,
        scope: &[(String, Type)],
    ) -> Option<Type> {
        let (ty, problems) = infer(lowered, scope.to_vec());
        self.problems.extend(problems.into_iter().map(|p| {
            problem(
                span,
                What::Type {
                    problem: Box::new(p),
                },
            )
        }));
        ty
    }

    pub(crate) fn expect(&mut self, got: Option<Type>, expected: &Type, span: Span) {
        if let Some(got) = got
            && !conforms(&got, expected)
        {
            self.report(
                span,
                What::Mismatch {
                    expected: Box::new(expected.clone()),
                    got: Box::new(got),
                },
            );
        }
    }
}

/// What every expression of the file lowers to, typed.
struct Lowered {
    /// Each call node's arguments, as a record.
    args: BTreeMap<usize, K>,
    guards: BTreeMap<usize, K>,
    /// Each fan-out's list.
    overs: BTreeMap<usize, K>,
    /// Each receive's filter, over its message and the parameter.
    filters: BTreeMap<usize, K>,
    pures: BTreeMap<usize, K>,
    ret: K,
    otherwise: Option<K>,
}

struct Plan {
    arg: Type,
    result: Type,
    param: String,
    order: Vec<usize>,
    needs: Vec<Needs>,
    return_needs: Needs,
    /// Call nodes whose failure ends the run.
    unhandled: BTreeSet<usize>,
}

impl Cx<'_> {
    /// The slot of `node` in the state held by `state`.
    fn slot(&self, state: &str, node: usize) -> K {
        get(get(var(state), "nodes"), self.node_name(node))
    }

    /// The state held by `state`, with `next` and `node`'s slot replaced.
    fn with_slot(&self, state: &str, next: K, node: usize, slot: K) -> K {
        let mut slot = Some(slot);
        let nodes = self
            .calls
            .keys()
            .map(|&i| {
                let s = if i == node {
                    slot.take().unwrap_or(lit(Value::Null))
                } else {
                    self.slot(state, i)
                };
                (self.node_name(i).to_string(), s)
            })
            .collect();
        rec(vec![
            ("arg", get(var(state), "arg")),
            ("next", next),
            ("nodes", rec(nodes)),
        ])
    }

    /// `node`'s slot in `state`, at another `kind`, with `call` and `error` given and `value` carried over.
    fn moved(
        &self,
        state: &str,
        node: usize,
        kind: &str,
        call: K,
        value: Option<K>,
        error: K,
    ) -> K {
        let mut fields = vec![
            ("kind", lit(json!(kind))),
            ("call", call),
            (
                "value",
                value.unwrap_or(get(self.slot(state, node), "value")),
            ),
            ("error", error),
        ];
        if self.calls[&node].kind == Slotted::FanOut {
            fields.push(("items", get(self.slot(state, node), "items")));
        }
        rec(fields)
    }

    /// `body`, with the parameter, every call node `needs` reads as a value, and every pure node it reads bound.
    fn scope(&self, plan: &Plan, lowered: &Lowered, needs: &Needs, body: K) -> K {
        let mut out = body;
        for &p in plan.order.iter().rev().filter(|p| needs.pures.contains(p)) {
            out = let_(self.node_name(p), lowered.pures[&p].clone(), out);
        }
        for &c in needs.values.iter().rev() {
            out = let_(self.node_name(c), get(self.slot("@state", c), "value"), out);
        }
        let_(&plan.param, get(var("@state"), "arg"), out)
    }

    fn settled(&self, state: &str, node: usize) -> K {
        one_of(&OUTCOMES, get(self.slot(state, node), "kind"))
    }

    /// The slotted nodes in dependency order, or only those of these kinds.
    fn slotted(&self, plan: &Plan, kinds: &[Slotted]) -> Vec<usize> {
        plan.order
            .iter()
            .copied()
            .filter(|i| self.calls.get(i).is_some_and(|c| kinds.contains(&c.kind)))
            .collect()
    }

    /// The accumulator `@acc` once node `n` starts, from the state in `@state`, with the next identifier `next`.
    fn started(&self, n: usize, lowered: &Lowered, next: K, call: K) -> K {
        let c = &self.calls[&n];
        let actions = get(var("@acc"), "actions");
        let issued = |count: K, slot: K, actions: K| {
            step(
                self.with_slot("@state", op("add", vec![next.clone(), count]), n, slot),
                actions,
            )
        };
        let running = || self.moved("@state", n, "running", next.clone(), None, lit(json!("")));
        let one = || lit(json!(1));
        match c.kind {
            // A receive starts listening; there is nothing to ask the world.
            Slotted::Receive => step(
                self.with_slot(
                    "@state",
                    next.clone(),
                    n,
                    self.moved("@state", n, "running", call, None, lit(json!(""))),
                ),
                actions,
            ),
            Slotted::Single | Slotted::Stream => {
                let action = rec(vec![
                    ("kind", lit(json!("call"))),
                    ("op", lit(json!(c.target))),
                    ("args", lowered.args[&n].clone()),
                ]);
                issued(one(), running(), op("append", vec![actions, action]))
            }
            Slotted::Spawn => {
                let (child, narrowings) = c
                    .child
                    .clone()
                    .unwrap_or_else(|| unreachable!("a spawn node always has its child"));
                let name = c.target.strip_prefix("spawn.").unwrap_or_default();
                let action = rec(vec![
                    ("kind", lit(json!("spawn"))),
                    ("name", lit(json!(name))),
                    ("harness", lit(json!(child))),
                    ("narrow", lit(json!(narrowings))),
                    ("arg", lowered.args[&n].clone()),
                ]);
                issued(one(), running(), op("append", vec![actions, action]))
            }
            // One call per item, with consecutive identifiers from `next`. An empty list has nothing to wait for.
            Slotted::FanOut => {
                let items = || var("@items");
                let count = op("len", vec![items()]);
                let item_default = match &c.value {
                    Type::List(t) => default_value(t),
                    _ => Value::Null,
                };
                let defaults = fold_into(
                    items(),
                    list(vec![]),
                    "@x",
                    op("append", vec![var("@a"), lit(item_default)]),
                );
                let kinds = fold_into(
                    items(),
                    list(vec![]),
                    "@x",
                    op("append", vec![var("@a"), lit(json!("running"))]),
                );
                let slot = rec(vec![
                    (
                        "kind",
                        if_(
                            eq(count.clone(), lit(json!(0))),
                            lit(json!("ok")),
                            lit(json!("running")),
                        ),
                    ),
                    ("call", next.clone()),
                    ("value", defaults),
                    ("error", lit(json!(""))),
                    ("items", kinds),
                ]);
                let item = c.binder.clone().unwrap_or_default();
                let action = rec(vec![
                    ("kind", lit(json!("call"))),
                    ("op", lit(json!(c.target))),
                    ("args", lowered.args[&n].clone()),
                ]);
                let calls = fold_into(
                    items(),
                    actions,
                    &item,
                    op("append", vec![var("@a"), action]),
                );
                let_(
                    "@items",
                    lowered.overs[&n].clone(),
                    issued(count, slot, calls),
                )
            }
        }
    }

    /// One pass over the slotted nodes, from the state in `@state` and the actions in `@pre`, then a halt once every
    /// call, and everything `return` reads, has settled. A receive nothing reads doesn't keep the run going.
    fn schedule(&self, plan: &Plan, lowered: &Lowered) -> K {
        let mut must_settle: BTreeSet<usize> = self
            .slotted(
                plan,
                &[
                    Slotted::Single,
                    Slotted::Stream,
                    Slotted::FanOut,
                    Slotted::Spawn,
                ],
            )
            .into_iter()
            .collect();
        must_settle.extend(plan.return_needs.calls());
        let finish = {
            let all_settled = all(must_settle
                .iter()
                .map(|&c| self.settled("@state", c))
                .collect());
            let halted = step(
                var("@state"),
                op(
                    "append",
                    vec![get(var("@acc"), "actions"), halt(lowered.ret.clone())],
                ),
            );
            if_(
                all_settled,
                self.scope(plan, lowered, &plan.return_needs, halted),
                var("@acc"),
            )
        };
        let mut out = enter(finish);
        for n in self
            .slotted(
                plan,
                &[
                    Slotted::Single,
                    Slotted::Stream,
                    Slotted::Receive,
                    Slotted::FanOut,
                    Slotted::Spawn,
                ],
            )
            .into_iter()
            .rev()
        {
            let needs = &plan.needs[n];
            let next = get(var("@state"), "next");
            let call = get(self.slot("@state", n), "call");
            let skip = step(
                self.with_slot(
                    "@state",
                    next.clone(),
                    n,
                    self.moved("@state", n, "skipped", call.clone(), None, lit(json!(""))),
                ),
                get(var("@acc"), "actions"),
            );
            let started = self.started(n, lowered, next, call);
            let guarded = match lowered.guards.get(&n) {
                Some(g) => if_(g.clone(), started, skip.clone()),
                None => started,
            };
            let skipped_dep = any(needs
                .values
                .iter()
                .map(|&d| eq(get(self.slot("@state", d), "kind"), lit(json!("skipped"))))
                .collect());
            let body = if_(skipped_dep, skip, self.scope(plan, lowered, needs, guarded));
            let waiting = eq(get(self.slot("@state", n), "kind"), lit(json!("waiting")));
            let ready = all(needs
                .calls()
                .iter()
                .map(|&d| self.settled("@state", d))
                .collect());
            let moved = enter(if_(all(vec![waiting, ready]), body, var("@acc")));
            out = let_("@acc", moved, out);
        }
        let_(
            "@acc",
            step(var("@state"), get(var("@pre"), "actions")),
            out,
        )
    }

    /// After the input is recorded and calls are cancelled: halt through `otherwise` if an unhandled node has failed,
    /// or else schedule.
    fn after_result(&self, plan: &Plan, lowered: &Lowered, scheduled: K) -> K {
        let mut out = scheduled;
        for &n in plan.unhandled.iter().rev() {
            let value = match &lowered.otherwise {
                Some(o) => let_(&plan.param, get(var("@state"), "arg"), o.clone()),
                None => lit(json!({ "failed": self.node_name(n) })),
            };
            let failed = one_of(&["failed", "refused"], get(self.slot("@state", n), "kind"));
            let halted = op("append", vec![get(var("@pre"), "actions"), halt(value)]);
            out = if_(failed, step(var("@state"), halted), out);
        }
        out
    }

    /// `@pre` after cancelling every call whose `until` node has returned: its slot fails with the error `"cancelled"`,
    /// and if it was running, the world is asked to stop it. Its results, if any still come, find no slot running it.
    fn cancellations(&self, plan: &Plan, body: K) -> K {
        let mut out = let_("@state", get(var("@pre"), "state"), body);
        for &n in plan.order.iter().rev() {
            let Some(stop) = self.calls.get(&n).and_then(|c| c.until) else {
                continue;
            };
            let kind = get(self.slot("@state", n), "kind");
            let call = get(self.slot("@state", n), "call");
            let stopped = all(vec![
                eq(get(self.slot("@state", stop), "kind"), lit(json!("ok"))),
                one_of(&["waiting", "running"], kind.clone()),
            ]);
            let cancelled = self.moved(
                "@state",
                n,
                "failed",
                call.clone(),
                None,
                lit(json!("cancelled")),
            );
            let actions = if_(
                eq(kind, lit(json!("running"))),
                op(
                    "append",
                    vec![
                        get(var("@pre"), "actions"),
                        rec(vec![("kind", lit(json!("cancel"))), ("call", call)]),
                    ],
                ),
                get(var("@pre"), "actions"),
            );
            let next = get(var("@state"), "next");
            let moved = step(self.with_slot("@state", next, n, cancelled), actions);
            let checked = let_(
                "@state",
                get(var("@pre"), "state"),
                if_(stopped, moved, var("@pre")),
            );
            out = let_("@pre", checked, out);
        }
        let_("@pre", step(var("@state"), list(vec![])), out)
    }

    /// The state in `@r` after `then` is applied to each of `nodes` the input `hit`s, in turn.
    fn each(&self, nodes: &[usize], hit: impl Fn(usize) -> K, then: impl Fn(usize) -> K) -> K {
        let mut out = var("@r");
        for &n in nodes.iter().rev() {
            out = let_("@r", if_(hit(n), then(n), var("@r")), out);
        }
        let_("@r", var("state"), out)
    }

    /// The state after an input about a call of one of `nodes`: the slot running it settles. A stream's item is
    /// appended to its slot, which stays running until the stream ends.
    fn record(&self, nodes: &[usize], input: Input) -> K {
        let call = get(var("@in"), "call");
        let hit = |n| {
            let slot = || self.slot("@r", n);
            let running = eq(get(slot(), "kind"), lit(json!("running")));
            if self.calls[&n].kind == Slotted::FanOut {
                let first = get(slot(), "call");
                let after = op(
                    "add",
                    vec![first.clone(), op("len", vec![get(slot(), "items")])],
                );
                all(vec![
                    running,
                    op("ge", vec![call.clone(), first]),
                    op("lt", vec![call.clone(), after]),
                ])
            } else {
                all(vec![running, eq(get(slot(), "call"), call.clone())])
            }
        };
        let then = |n| {
            if self.calls[&n].kind == Slotted::FanOut {
                return self.record_item(n, input);
            }
            let slot = get(self.slot("@r", n), "value");
            let item = get(var("@in"), "value");
            let (kind, value) = match (input, self.calls[&n].kind) {
                (Input::Ret, Slotted::Stream) => ("running", Some(op("append", vec![slot, item]))),
                (Input::Ret, _) => ("ok", Some(item)),
                (Input::End, _) => ("ok", None),
                (Input::Failed, _) => ("failed", None),
                (Input::Refused, _) => ("refused", None),
            };
            let error = if input == Input::Failed {
                op("str", vec![get(var("@in"), "error")])
            } else {
                lit(json!(""))
            };
            let settled = self.moved("@r", n, kind, call.clone(), value, error);
            self.with_slot("@r", get(var("@r"), "next"), n, settled)
        };
        self.each(nodes, hit, then)
    }

    /// The state in `@r` after an input for one of a fan-out's calls: its item settles, and once none is running,
    /// so does the fan-out, failing if any item did not return.
    fn record_item(&self, n: usize, input: Input) -> K {
        let slot = || self.slot("@r", n);
        let index = op("sub", vec![get(var("@in"), "call"), get(slot(), "call")]);
        let kind = match input {
            Input::Ret => "ok",
            Input::Failed => "failed",
            Input::Refused | Input::End => "refused",
        };
        let values = match input {
            Input::Ret => replaced(get(slot(), "value"), var("@i"), get(var("@in"), "value")),
            _ => get(slot(), "value"),
        };
        let error = match input {
            Input::Failed => op("str", vec![get(var("@in"), "error")]),
            _ => get(slot(), "error"),
        };
        let has = |k: &str| op("contains", vec![var("@kinds"), lit(json!(k))]);
        let settled = if_(
            has("running"),
            lit(json!("running")),
            if_(
                any(vec![has("failed"), has("refused")]),
                lit(json!("failed")),
                lit(json!("ok")),
            ),
        );
        let updated = rec(vec![
            ("kind", settled),
            ("call", get(slot(), "call")),
            ("value", values),
            ("error", error),
            ("items", var("@kinds")),
        ]);
        let_(
            "@i",
            index,
            let_(
                "@kinds",
                replaced(get(slot(), "items"), var("@i"), lit(json!(kind))),
                self.with_slot("@r", get(var("@r"), "next"), n, updated),
            ),
        )
    }

    /// The state after a message on a channel: every receive listening there whose filter it passes returns it.
    fn record_message(&self, nodes: &[usize], lowered: &Lowered, param: &str) -> K {
        let hit = |n| {
            let listening = eq(get(self.slot("@r", n), "kind"), lit(json!("running")));
            match (&lowered.filters.get(&n), &self.calls[&n].binder) {
                (Some(filter), binder) => {
                    let mut f = let_(param, get(var("@r"), "arg"), (*filter).clone());
                    if let Some(b) = binder {
                        f = let_(b, get(var("@in"), "value"), f);
                    }
                    // A filter is only read of a message a receive is listening for.
                    if_(listening, f, lit(json!(false)))
                }
                (None, _) => listening,
            }
        };
        let then = |n| {
            let call = get(self.slot("@r", n), "call");
            let value = Some(get(var("@in"), "value"));
            let settled = self.moved("@r", n, "ok", call, value, lit(json!("")));
            self.with_slot("@r", get(var("@r"), "next"), n, settled)
        };
        self.each(nodes, hit, then)
    }

    /// A `match` of `@in` on `on`, with one case per key, each binding `@in` again.
    fn dispatch(&self, on: &str, cases: BTreeMap<String, K>) -> K {
        K::Match {
            of: Box::new(var("@in")),
            on: on.to_string(),
            cases: cases
                .into_iter()
                .map(|(name, body)| {
                    let case = Case {
                        bind: "@in".to_string(),
                        body,
                    };
                    (name, case)
                })
                .collect(),
            otherwise: Some(Box::new(var("state"))),
        }
    }

    fn generate(&self, plan: &Plan, lowered: &Lowered) -> Harness {
        let ops = self.slotted(
            plan,
            &[
                Slotted::Single,
                Slotted::Stream,
                Slotted::FanOut,
                Slotted::Spawn,
            ],
        );
        let streams = self.slotted(plan, &[Slotted::Stream]);
        let receives = self.slotted(plan, &[Slotted::Receive]);
        let grouped = |nodes: &[usize]| {
            let mut by: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            for &n in nodes {
                by.entry(self.calls[&n].target.clone()).or_default().push(n);
            }
            by
        };
        let results = self.dispatch(
            "op",
            grouped(&ops)
                .into_iter()
                .map(|(op, nodes)| (op, self.record(&nodes, Input::Ret)))
                .collect(),
        );
        let ends = self.dispatch(
            "op",
            grouped(&streams)
                .into_iter()
                .map(|(op, nodes)| (op, self.record(&nodes, Input::End)))
                .collect(),
        );
        let messages = self.dispatch(
            "channel",
            grouped(&receives)
                .into_iter()
                .map(|(channel, nodes)| {
                    (channel, self.record_message(&nodes, lowered, &plan.param))
                })
                .collect(),
        );
        let started = rec(vec![
            ("arg", get(var("@in"), "value")),
            ("next", get(var("state"), "next")),
            ("nodes", get(var("state"), "nodes")),
        ]);
        let case = |body: K| Case {
            bind: "@in".to_string(),
            body,
        };
        let mut cases = BTreeMap::from([
            ("start".to_string(), case(started)),
            ("ret".to_string(), case(results)),
            (
                "refused".to_string(),
                case(self.record(&ops, Input::Refused)),
            ),
        ]);
        // Only the inputs the harness can use are named. An induction obligation drops the cases of the input that
        // have no SMT sort, a failure's among them (its error is `Any`), and a step that named one would be refused.
        if !streams.is_empty() {
            cases.insert("end".to_string(), case(ends));
        }
        if !receives.is_empty() {
            cases.insert("msg".to_string(), case(messages));
        }
        let failures = let_(
            "@in",
            var("input"),
            if_(
                eq(get(var("input"), "kind"), lit(json!("failed"))),
                self.record(&ops, Input::Failed),
                var("state"),
            ),
        );
        let recorded = K::Match {
            of: Box::new(var("input")),
            on: "kind".to_string(),
            cases,
            otherwise: Some(Box::new(failures)),
        };
        let step_fn = let_(
            "@state",
            recorded,
            self.cancellations(
                plan,
                self.after_result(plan, lowered, self.schedule(plan, lowered)),
            ),
        );
        let slots = self
            .calls
            .iter()
            .map(|(&i, c)| (self.node_name(i).to_string(), slot_type(c)))
            .collect();
        let state = Type::Record(BTreeMap::from([
            ("arg".to_string(), plan.arg.clone()),
            ("next".to_string(), Type::Int),
            ("nodes".to_string(), Type::Record(slots)),
        ]));
        let waiting: serde_json::Map<String, Value> = self
            .calls
            .iter()
            .map(|(&i, c)| (self.node_name(i).to_string(), initial_slot(c)))
            .collect();
        // Each child the harness spawns, by the name its header binds it to.
        let spawns = self
            .calls
            .values()
            .filter_map(|c| {
                let (child, _) = c.child.as_ref()?;
                let name = c.target.strip_prefix("spawn.")?.to_string();
                let signature = Signature {
                    arg: child.arg.clone(),
                    result: child.result.clone(),
                };
                Some((name, signature))
            })
            .collect();
        Harness {
            arg: plan.arg.clone(),
            result: plan.result.clone(),
            spawns,
            state,
            init: json!({ "arg": default_value(&plan.arg), "next": 0, "nodes": waiting }),
            step: step_fn,
        }
    }
}

/// `body` with `@state` and `@nodes` bound from the accumulator `@acc`.
fn enter(body: K) -> K {
    let_(
        "@state",
        get(var("@acc"), "state"),
        let_("@nodes", get(var("@state"), "nodes"), body),
    )
}

/// Elaborate a source against the contracts of the capabilities it may use, and the terms of the harnesses it may
/// spawn, each by identifier.
pub fn elaborate(
    source: &str,
    capabilities: &BTreeMap<String, Interface>,
    harnesses: &BTreeMap<String, Harness>,
) -> Result<Elaborated, Vec<Problem>> {
    let file = parse(lex(source).map_err(|p| vec![p])?).map_err(|p| vec![p])?;
    let mut cx = Cx {
        file: &file,
        children: BTreeMap::new(),
        problems: Vec::new(),
        names: BTreeMap::new(),
        calls: BTreeMap::new(),
        may_skip: BTreeSet::new(),
        fresh: 0,
    };
    let (arg, result) = cx.types();
    let mut spawned = Vec::new();
    let (interface, uses) = cx.install(capabilities, harnesses, &mut spawned);
    cx.bind(&file.param, Bound::Param);
    for (i, node) in file.nodes.iter().enumerate() {
        cx.bind(&node.name, Bound::Node(i));
    }
    match file.returns.as_slice() {
        [] => cx.report(file.name.span, What::NoReturn),
        [_, rest @ ..] => {
            for r in rest {
                cx.report(
                    r.span,
                    What::Duplicate {
                        name: "return".to_string(),
                    },
                );
            }
        }
    }
    for o in file.otherwise.iter().skip(1) {
        cx.report(
            o.span,
            What::Duplicate {
                name: "otherwise".to_string(),
            },
        );
    }

    // Which nodes are calls or receives, and of what.
    for (i, node) in file.nodes.iter().enumerate() {
        if let ExprKind::Receive { binder, channel } = &node.value.kind {
            let Some(c) = interface.channels.get(&channel.text) else {
                cx.report(
                    channel.span,
                    What::UnknownChannel {
                        channel: channel.text.clone(),
                    },
                );
                continue;
            };
            if let Some(u) = &node.until {
                cx.report(
                    u.span,
                    What::Until {
                        name: u.text.clone(),
                    },
                );
            }
            cx.calls.insert(
                i,
                Call {
                    kind: Slotted::Receive,
                    target: channel.text.clone(),
                    input: Type::Any,
                    value: c.ty.clone(),
                    binder: binder.as_ref().map(|b| b.text.clone()),
                    child: None,
                    until: None,
                },
            );
            continue;
        }
        let call = match &node.value.kind {
            ExprKind::ForEach { item, call, .. } => {
                let ExprKind::Call(callee, args) = &call.kind else {
                    cx.report(
                        call.span,
                        What::Argument {
                            expected: "a call of an operation after `:`".to_string(),
                        },
                    );
                    continue;
                };
                let Some(mut c) = classify_call(&mut cx, &interface, callee, args) else {
                    continue;
                };
                if c.kind != Slotted::Single {
                    cx.report(
                        callee.span,
                        What::Argument {
                            expected: "an operation with one result to fan out over".to_string(),
                        },
                    );
                    continue;
                }
                c.kind = Slotted::FanOut;
                c.value = Type::List(Box::new(c.value));
                c.binder = Some(item.text.clone());
                c
            }
            ExprKind::Spawn {
                harness,
                only,
                without,
                ..
            } => {
                let Some(child) = cx.children.get(&harness.text).cloned() else {
                    if cx.names.get(&harness.text) != Some(&Bound::Harness) {
                        cx.report(
                            harness.span,
                            What::NotAHarness {
                                name: harness.text.clone(),
                            },
                        );
                    }
                    continue;
                };
                let mut narrowings = Vec::new();
                if let Some(only) = only {
                    narrowings.push(Narrowing::Only {
                        ops: only.iter().map(|o| o.text.clone()).collect(),
                    });
                }
                for w in without {
                    narrowings.push(Narrowing::Drop { op: w.text.clone() });
                }
                let narrowed = match narrow(&interface, &narrowings) {
                    Ok(n) => n,
                    Err(e) => {
                        let op = match e {
                            kernel::harness::NarrowError::NoSuchOperation { op } => op,
                            kernel::harness::NarrowError::NoSuchChannel { channel } => channel,
                        };
                        let at = only
                            .iter()
                            .flatten()
                            .chain(without)
                            .find(|n| n.text == op)
                            .map_or(node.value.span, |n| n.span);
                        cx.report(at, What::UnknownOperation { op });
                        continue;
                    }
                };
                let problems = check_harness(&child, Some(&narrowed));
                if !problems.is_empty() {
                    cx.report(
                        harness.span,
                        What::Child {
                            name: harness.text.clone(),
                            problems,
                        },
                    );
                    continue;
                }
                Call {
                    kind: Slotted::Spawn,
                    // A declared child: its results arrive on its own operation, at its result type.
                    target: spawn_op(&harness.text),
                    input: child.arg.clone(),
                    value: child.result.clone(),
                    binder: None,
                    child: Some((child, narrowings)),
                    until: None,
                }
            }
            ExprKind::Call(callee, args) if cx.operation(callee, &[]).is_some() => {
                match classify_call(&mut cx, &interface, callee, args) {
                    Some(c) => c,
                    None => continue,
                }
            }
            _ => {
                if let Some(w) = &node.when {
                    cx.report(
                        w.span,
                        What::GuardOnValue {
                            node: node.name.text.clone(),
                        },
                    );
                }
                continue;
            }
        };
        cx.calls.insert(i, call);
    }
    for (i, node) in file.nodes.iter().enumerate() {
        let Some(u) = &node.until else { continue };
        if cx
            .calls
            .get(&i)
            .is_none_or(|c| matches!(c.kind, Slotted::Receive | Slotted::FanOut))
        {
            cx.report(
                u.span,
                What::Until {
                    name: u.text.clone(),
                },
            );
            continue;
        }
        match cx.names.get(&u.text) {
            Some(Bound::Node(j)) if *j != i && cx.calls.contains_key(j) => {
                let j = *j;
                if let Some(c) = cx.calls.get_mut(&i) {
                    c.until = Some(j);
                }
            }
            _ => cx.report(
                u.span,
                What::Until {
                    name: u.text.clone(),
                },
            ),
        }
    }

    // What each expression reads.
    let mut node_reads: Vec<Reads> = Vec::new();
    for (i, node) in file.nodes.iter().enumerate() {
        if let ExprKind::Receive { binder, .. } = &node.value.kind {
            // A filter reads the message and the parameter, never a node: it runs when the message arrives,
            // whatever else has settled.
            if let Some(w) = &node.when {
                let mut reads = Vec::new();
                cx.walk(
                    w,
                    &mut binder.iter().map(|b| b.text.clone()).collect(),
                    &mut reads,
                );
                for (j, _) in reads {
                    let read = cx.node_name(j).to_string();
                    cx.report(w.span, What::FilterReadsNode { node: read });
                }
            }
            node_reads.push(Vec::new());
            continue;
        }
        let mut reads = match (&node.value.kind, cx.calls.contains_key(&i)) {
            (ExprKind::Call(_, args), true) => {
                args.iter().flat_map(|a| cx.reads(&a.value)).collect()
            }
            (ExprKind::ForEach { item, over, call }, true) => {
                let mut reads = cx.reads(over);
                if let ExprKind::Call(_, args) = &call.kind {
                    for a in args {
                        cx.walk(&a.value, &mut vec![item.text.clone()], &mut reads);
                    }
                }
                reads
            }
            (ExprKind::Spawn { arg, .. }, true) => cx.reads(arg),
            _ => cx.reads(&node.value),
        };
        if let Some(w) = &node.when {
            reads.extend(cx.reads(w));
        }
        node_reads.push(reads);
    }
    let return_reads = file
        .returns
        .first()
        .map(|r| cx.reads(r))
        .unwrap_or_default();
    if let Some(o) = file.otherwise.first() {
        for (i, _) in cx.reads(o) {
            let node = cx.node_name(i).to_string();
            cx.report(o.span, What::OtherwiseReadsNode { node });
        }
    }
    if !cx.problems.is_empty() {
        return Err(cx.problems);
    }

    let deps: Vec<BTreeSet<usize>> = node_reads
        .iter()
        .map(|r| r.iter().map(|(i, _)| *i).collect())
        .collect();
    let Some(order) = cx.order(&deps) else {
        return Err(cx.problems);
    };

    // What each node needs, which nodes may be skipped, and which failures end the run.
    let mut needs = vec![Needs::default(); file.nodes.len()];
    let mut of_pure = BTreeMap::new();
    for &i in &order {
        let n = cx.needs(&node_reads[i], &of_pure);
        let guarded = cx.calls.get(&i).is_some_and(|c| c.kind != Slotted::Receive);
        let skippable = (file.nodes[i].when.is_some() && guarded)
            || n.values.iter().any(|d| cx.may_skip.contains(d));
        if skippable {
            cx.may_skip.insert(i);
        }
        if !cx.calls.contains_key(&i) {
            of_pure.insert(i, n.clone());
        }
        needs[i] = n;
    }
    let return_needs = cx.needs(&return_reads, &of_pure);
    if let Some(r) = file.returns.first() {
        for &(i, read) in &return_reads {
            if read != Read::Value {
                continue;
            }
            let via = cx.node_name(i).to_string();
            let culprit = if cx.calls.contains_key(&i) {
                cx.may_skip.contains(&i).then_some(i)
            } else {
                needs[i]
                    .values
                    .iter()
                    .copied()
                    .find(|d| cx.may_skip.contains(d))
            };
            if let Some(c) = culprit {
                let node = cx.node_name(c).to_string();
                cx.report(r.span, What::MayBeSkipped { node, via });
            }
        }
    }
    let every_read: Vec<&(usize, Read)> =
        node_reads.iter().flatten().chain(&return_reads).collect();
    // A receive can't fail, but a cancelled call does.
    let unhandled: BTreeSet<usize> = cx
        .calls
        .iter()
        .filter(|(_, c)| c.kind != Slotted::Receive)
        .map(|(&i, _)| i)
        .filter(|&c| {
            let reads: Vec<_> = every_read.iter().filter(|(i, _)| *i == c).collect();
            reads.is_empty()
                || reads.iter().any(|(_, r)| {
                    *r != Read::Outcome {
                        handles_failure: true,
                    }
                })
        })
        .collect();

    // Without `otherwise`, an unhandled failure halts with `{"failed": node}`, which the result type must admit.
    let failed = Type::Record(BTreeMap::from([("failed".to_string(), Type::String)]));
    if file.otherwise.is_empty()
        && !conforms(&failed, &result)
        && let Some(&n) = unhandled.iter().next()
    {
        let node = cx.node_name(n).to_string();
        let at = file.returns.first().map_or(file.name.span, |r| r.span);
        cx.report(at, What::NeedsOtherwise { node });
    }

    // Lowering and typing.
    let nodes_type = Type::Record(
        cx.calls
            .iter()
            .map(|(&i, c)| (cx.node_name(i).to_string(), slot_type(c)))
            .collect(),
    );
    let mut scope = vec![
        ("@nodes".to_string(), nodes_type),
        (file.param.text.clone(), arg.clone()),
    ];
    for (&i, c) in &cx.calls {
        scope.push((cx.node_name(i).to_string(), c.value.clone()));
    }
    let mut lowered = Lowered {
        args: BTreeMap::new(),
        guards: BTreeMap::new(),
        overs: BTreeMap::new(),
        filters: BTreeMap::new(),
        pures: BTreeMap::new(),
        ret: lit(Value::Null),
        otherwise: None,
    };
    for &i in &order {
        let node = &file.nodes[i];
        if let ExprKind::Receive { binder, .. } = &node.value.kind {
            let (Some(w), Some(call)) = (&node.when, cx.calls.get(&i)) else {
                continue;
            };
            let mut filter_scope = vec![(file.param.text.clone(), arg.clone())];
            let mut locals = Vec::new();
            if let Some(b) = binder {
                filter_scope.push((b.text.clone(), call.value.clone()));
                locals.push(b.text.clone());
            }
            let f = cx.lower(w, &mut locals);
            let t = cx.type_of(&f, w.span, &filter_scope);
            cx.expect(t, &Type::Bool, w.span);
            lowered.filters.insert(i, f);
            continue;
        }
        let Some(call) = cx.calls.get(&i) else {
            let e = cx.lower(&node.value, &mut Vec::new());
            let t = cx.type_of(&e, node.value.span, &scope);
            scope.push((node.name.text.clone(), t.unwrap_or(Type::Any)));
            lowered.pures.insert(i, e);
            continue;
        };
        let input = call.input.clone();
        match &node.value.kind {
            ExprKind::Call(_, args) => {
                let args = lower_args(&mut cx, i, args, &input, node.value.span, &scope, &[]);
                lowered.args.insert(i, args);
            }
            ExprKind::ForEach { item, over, call } => {
                let o = cx.lower(over, &mut Vec::new());
                let item_type = match cx.type_of(&o, over.span, &scope) {
                    Some(Type::List(t)) => *t,
                    Some(Type::Any) | None => Type::Any,
                    Some(got) => {
                        cx.expect(Some(got), &Type::List(Box::new(Type::Any)), over.span);
                        Type::Any
                    }
                };
                let ExprKind::Call(_, args) = &call.kind else {
                    continue;
                };
                let mut item_scope = scope.clone();
                item_scope.push((item.text.clone(), item_type));
                let locals = [item.text.clone()];
                let args = lower_args(&mut cx, i, args, &input, call.span, &item_scope, &locals);
                lowered.args.insert(i, args);
                lowered.overs.insert(i, o);
            }
            ExprKind::Spawn { arg, .. } => {
                let e = cx.lower(arg, &mut Vec::new());
                let t = cx.type_of(&e, arg.span, &scope);
                cx.expect(t, &input, arg.span);
                lowered.args.insert(i, e);
            }
            _ => {}
        }
        if let Some(w) = node
            .when
            .as_ref()
            .filter(|_| !matches!(node.value.kind, ExprKind::Receive { .. }))
        {
            let g = cx.lower(w, &mut Vec::new());
            let t = cx.type_of(&g, w.span, &scope);
            cx.expect(t, &Type::Bool, w.span);
            lowered.guards.insert(i, g);
        }
    }
    if let Some(r) = file.returns.first() {
        let e = cx.lower(r, &mut Vec::new());
        let t = cx.type_of(&e, r.span, &scope);
        cx.expect(t, &result, r.span);
        lowered.ret = e;
    }
    if let Some(o) = file.otherwise.first() {
        let e = cx.lower(o, &mut Vec::new());
        let t = cx.type_of(&e, o.span, &[(file.param.text.clone(), arg.clone())]);
        cx.expect(t, &result, o.span);
        lowered.otherwise = Some(e);
    }
    if !cx.problems.is_empty() {
        return Err(cx.problems);
    }

    let plan = Plan {
        arg,
        result: result.clone(),
        param: file.param.text.clone(),
        order,
        needs,
        return_needs,
        unhandled,
    };
    let harness = cx.generate(&plan, &lowered);
    let problems = check_harness(&harness, Some(&interface));
    if !problems.is_empty() {
        return Err(vec![problem(file.name.span, What::Internal { problems })]);
    }

    // What is claimed about it.
    let mut source_map = SourceMap::default();
    for node in &file.nodes {
        source_map
            .nodes
            .insert(node.name.text.clone(), node.name.span);
    }
    let mut properties = Vec::new();
    for (name, body) in &file.properties {
        if source_map
            .properties
            .insert(name.text.clone(), name.span)
            .is_some()
        {
            cx.report(
                name.span,
                What::Duplicate {
                    name: name.text.clone(),
                },
            );
        } else if let Some(p) = cx.property(name, body, &interface) {
            properties.push(p);
        }
    }
    let mut invariants = Vec::new();
    for (name, body) in &file.invariants {
        if source_map
            .invariants
            .insert(name.text.clone(), name.span)
            .is_some()
        {
            cx.report(
                name.span,
                What::Duplicate {
                    name: name.text.clone(),
                },
            );
        } else if let Some(i) = cx.invariant(name, body, &harness.state) {
            invariants.push(i);
        }
    }
    if !cx.problems.is_empty() {
        return Err(cx.problems);
    }
    Ok(Elaborated {
        harness,
        uses,
        harnesses: spawned,
        properties,
        invariants,
        source_map,
    })
}

fn call_name(cx: &Cx, i: usize) -> String {
    cx.calls[&i].target.clone()
}

/// A node that calls an operation: its slot, with what is wrong with its arguments reported.
fn classify_call(
    cx: &mut Cx,
    interface: &Interface,
    callee: &syntax::Expr,
    args: &[syntax::Arg],
) -> Option<Call> {
    let name = cx.operation(callee, &[])?;
    let Some(operation) = interface.operations.get(&name) else {
        cx.report(callee.span, What::UnknownOperation { op: name });
        return None;
    };
    let mut seen = BTreeSet::new();
    for a in args {
        match &a.name {
            None => cx.report(
                a.value.span,
                What::Argument {
                    expected: format!("named arguments to `{name}`, as `field: value`"),
                },
            ),
            Some(n) if !seen.insert(n.text.clone()) => cx.report(
                n.span,
                What::Duplicate {
                    name: n.text.clone(),
                },
            ),
            Some(_) => {}
        }
    }
    let (kind, value) = match operation.mode {
        Mode::Single => (Slotted::Single, operation.output.clone()),
        Mode::Stream => (
            Slotted::Stream,
            Type::List(Box::new(operation.output.clone())),
        ),
    };
    Some(Call {
        kind,
        target: name,
        input: operation.input.clone(),
        value,
        binder: None,
        child: None,
        until: None,
    })
}

/// A call's named arguments, lowered and checked against the operation's input, as a record.
fn lower_args(
    cx: &mut Cx,
    node: usize,
    args: &[syntax::Arg],
    input: &Type,
    span: Span,
    scope: &[(String, Type)],
    locals: &[String],
) -> K {
    let mut fields = BTreeMap::new();
    for a in args {
        let Some(name) = &a.name else { continue };
        let e = cx.lower(&a.value, &mut locals.to_vec());
        let t = cx.type_of(&e, a.value.span, scope);
        if let Type::Record(declared) = input {
            match declared.get(&name.text) {
                Some(expected) => cx.expect(t, expected, a.value.span),
                None => cx.report(
                    name.span,
                    What::Argument {
                        expected: format!("one of the arguments of `{}`", call_name(cx, node)),
                    },
                ),
            }
        }
        fields.insert(name.text.clone(), e);
    }
    if let Type::Record(declared) = input {
        for missing in declared.keys().filter(|k| !fields.contains_key(*k)) {
            cx.report(
                span,
                What::Argument {
                    expected: format!("the argument `{missing}`"),
                },
            );
        }
    }
    K::Record { fields }
}
