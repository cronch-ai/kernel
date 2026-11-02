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
//!
//! The generated step does three things, in order:
//!
//! 1. Records its input: `start` stores the start value, and a result settles the slot running its call.
//! 2. Halts through `otherwise` if a node whose failure the harness doesn't handle has failed or been refused.
//! 3. Schedules: each waiting call node whose dependencies have settled, in dependency order, is skipped or
//!    started. Each is a `let` over the state the one before produced, so one pass reaches every node that can
//!    move. Once every slot has settled, it halts with `return`.

use crate::lex::{Span, lex};
use crate::parse::parse;
use crate::syntax::{self, Binary, ExprKind, File, Pattern, TypeExpr, Unary};
use crate::{Problem, What};
use kernel::check::infer;
use kernel::harness::{Harness, Installation, Interface, Mode, check_harness, install};
use kernel::ir::{Case, Expr as K, Type};
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

#[derive(Serialize, Clone, Debug)]
pub struct Elaborated {
    pub harness: Harness,
    pub uses: Vec<Binding>,
}

/// Functions an expression may call, each an operator of the kernel's but `label`.
const FUNCTIONS: [&str; 7] = ["len", "str", "round", "prefix", "concat", "append", "label"];

const OUTCOMES: [&str; 4] = ["ok", "failed", "refused", "skipped"];

/// What a name in an expression refers to, unless a local binding shadows it.
#[derive(Clone, Copy, PartialEq)]
enum Bound {
    Param,
    Node(usize),
    Capability,
}

/// How an expression reads a node: as its value, or through a `match` on its outcome.
#[derive(Clone, Copy, PartialEq)]
enum Read {
    Value,
    /// Whether the `match` has arms for failure and refusal, explicitly or through `_`.
    Outcome {
        handles_failure: bool,
    },
}

type Reads = Vec<(usize, Read)>;

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

struct Call {
    op: String,
    input: Type,
    output: Type,
}

struct Cx<'a> {
    file: &'a File,
    problems: Vec<Problem>,
    names: BTreeMap<String, Bound>,
    /// The call nodes, by node index.
    calls: BTreeMap<usize, Call>,
    may_skip: BTreeSet<usize>,
    fresh: usize,
}

fn problem(span: Span, what: What) -> Problem {
    Problem { span, what }
}

// Builders for the kernel's terms.

fn lit(v: Value) -> K {
    K::Lit { v }
}

fn var(name: &str) -> K {
    K::Var {
        name: name.to_string(),
    }
}

fn get(of: K, field: &str) -> K {
    K::Get {
        of: Box::new(of),
        field: field.to_string(),
    }
}

fn op(name: &str, args: Vec<K>) -> K {
    K::Op {
        op: name.to_string(),
        args,
    }
}

fn let_(name: &str, be: K, body: K) -> K {
    K::Let {
        name: name.to_string(),
        be: Box::new(be),
        body: Box::new(body),
    }
}

fn if_(cond: K, then: K, otherwise: K) -> K {
    K::If {
        cond: Box::new(cond),
        then: Box::new(then),
        otherwise: Box::new(otherwise),
    }
}

fn rec<S: Into<String>>(fields: Vec<(S, K)>) -> K {
    K::Record {
        fields: fields.into_iter().map(|(n, e)| (n.into(), e)).collect(),
    }
}

fn list(items: Vec<K>) -> K {
    K::List { items }
}

fn eq(a: K, b: K) -> K {
    op("eq", vec![a, b])
}

/// The strict conjunction: every operand here is total, so there is nothing to short-circuit.
fn all(items: Vec<K>) -> K {
    items
        .into_iter()
        .reduce(|a, b| op("and", vec![a, b]))
        .unwrap_or(lit(json!(true)))
}

fn any(items: Vec<K>) -> K {
    items
        .into_iter()
        .reduce(|a, b| op("or", vec![a, b]))
        .unwrap_or(lit(json!(false)))
}

fn one_of(kinds: &[&str], of: K) -> K {
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

fn slot_type(value: &Type) -> Type {
    Type::Record(BTreeMap::from([
        ("kind".to_string(), Type::String),
        ("call".to_string(), Type::Int),
        ("value".to_string(), value.clone()),
        ("error".to_string(), Type::Any),
    ]))
}

impl<'a> Cx<'a> {
    fn report(&mut self, span: Span, what: What) {
        self.problems.push(problem(span, what));
    }

    fn fresh(&mut self) -> String {
        self.fresh += 1;
        format!("@{}", self.fresh)
    }

    fn node_name(&self, i: usize) -> &str {
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
    fn install(&mut self, capabilities: &BTreeMap<String, Interface>) -> (Interface, Vec<Binding>) {
        let mut installations = Vec::new();
        let mut bindings = Vec::new();
        for u in &self.file.uses.clone() {
            let (id, span) = match &u.capability {
                Some((id, span)) => (id.clone(), *span),
                None => (u.name.text.clone(), u.name.span),
            };
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
    fn operation(&self, e: &syntax::Expr, locals: &[String]) -> Option<String> {
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
    fn walk(&mut self, e: &syntax::Expr, locals: &mut Vec<String>, reads: &mut Reads) {
        match &e.kind {
            ExprKind::Lit(_) => {}
            ExprKind::Name(n) => {
                if locals.contains(n) {
                    return;
                }
                match self.names.get(n) {
                    None => self.report(e.span, What::UnknownName { name: n.clone() }),
                    Some(Bound::Capability) => {
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
            ExprKind::Binary(_, a, b) => {
                self.walk(a, locals, reads);
                self.walk(b, locals, reads);
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

    fn lower(&mut self, e: &syntax::Expr, locals: &mut Vec<String>) -> K {
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
    fn type_of(&mut self, lowered: &K, span: Span, scope: &[(String, Type)]) -> Option<Type> {
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

    fn expect(&mut self, got: Option<Type>, expected: &Type, span: Span) {
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
    pures: BTreeMap<usize, K>,
    ret: K,
    otherwise: Option<K>,
}

struct Plan {
    arg: Type,
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
        rec(vec![
            ("kind", lit(json!(kind))),
            ("call", call),
            (
                "value",
                value.unwrap_or(get(self.slot(state, node), "value")),
            ),
            ("error", error),
        ])
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

    /// One pass over the call nodes, from the state in `@state`, then a halt if every slot has settled.
    fn schedule(&self, plan: &Plan, lowered: &Lowered) -> K {
        let calls: Vec<usize> = plan
            .order
            .iter()
            .copied()
            .filter(|i| self.calls.contains_key(i))
            .collect();
        let finish = {
            let all_settled = all(calls.iter().map(|&c| self.settled("@state", c)).collect());
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
        for &n in calls.iter().rev() {
            let needs = &plan.needs[n];
            let next = get(var("@state"), "next");
            let skip = step(
                self.with_slot(
                    "@state",
                    next.clone(),
                    n,
                    self.moved(
                        "@state",
                        n,
                        "skipped",
                        get(self.slot("@state", n), "call"),
                        None,
                        lit(Value::Null),
                    ),
                ),
                get(var("@acc"), "actions"),
            );
            let call = step(
                self.with_slot(
                    "@state",
                    op("add", vec![next.clone(), lit(json!(1))]),
                    n,
                    self.moved("@state", n, "running", next, None, lit(Value::Null)),
                ),
                op(
                    "append",
                    vec![
                        get(var("@acc"), "actions"),
                        rec(vec![
                            ("kind", lit(json!("call"))),
                            ("op", lit(json!(self.calls[&n].op))),
                            ("args", lowered.args[&n].clone()),
                        ]),
                    ],
                ),
            );
            let guarded = match lowered.guards.get(&n) {
                Some(g) => if_(g.clone(), call, skip.clone()),
                None => call,
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
        let_("@acc", step(var("@state"), list(vec![])), out)
    }

    /// After a result: halt through `otherwise` if an unhandled node has failed, or else schedule.
    fn after_result(&self, plan: &Plan, lowered: &Lowered, scheduled: K) -> K {
        let mut out = scheduled;
        for &n in plan.unhandled.iter().rev() {
            let value = match &lowered.otherwise {
                Some(o) => let_(&plan.param, get(var("@state"), "arg"), o.clone()),
                None => lit(json!({ "failed": self.node_name(n) })),
            };
            let failed = one_of(&["failed", "refused"], get(self.slot("@state", n), "kind"));
            out = if_(failed, step(var("@state"), list(vec![halt(value)])), out);
        }
        out
    }

    /// The state after a result for one of `nodes`: the slot running its call settles at `kind`.
    fn record(&self, nodes: &[usize], kind: &str) -> K {
        let call = get(var("@in"), "call");
        let mut out = var("state");
        for &n in nodes.iter().rev() {
            let hit = all(vec![
                eq(get(self.slot("state", n), "kind"), lit(json!("running"))),
                eq(get(self.slot("state", n), "call"), call.clone()),
            ]);
            let value = (kind == "ok").then(|| get(var("@in"), "value"));
            let error = if kind == "failed" {
                get(var("@in"), "error")
            } else {
                lit(Value::Null)
            };
            let settled = self.moved("state", n, kind, call.clone(), value, error);
            out = if_(
                hit,
                self.with_slot("state", get(var("state"), "next"), n, settled),
                out,
            );
        }
        out
    }

    fn generate(&self, plan: &Plan, lowered: &Lowered) -> Harness {
        let calls: Vec<usize> = plan
            .order
            .iter()
            .copied()
            .filter(|i| self.calls.contains_key(i))
            .collect();
        let mut by_op: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &n in &calls {
            by_op.entry(self.calls[&n].op.as_str()).or_default().push(n);
        }
        let results = K::Match {
            of: Box::new(var("@in")),
            on: "op".to_string(),
            cases: by_op
                .iter()
                .map(|(op, nodes)| {
                    let case = Case {
                        bind: "@in".to_string(),
                        body: self.record(nodes, "ok"),
                    };
                    (op.to_string(), case)
                })
                .collect(),
            otherwise: Some(Box::new(var("state"))),
        };
        let started = rec(vec![
            ("arg", get(var("@in"), "value")),
            ("next", get(var("state"), "next")),
            ("nodes", get(var("state"), "nodes")),
        ]);
        let case = |body: K| Case {
            bind: "@in".to_string(),
            body,
        };
        let cases = BTreeMap::from([
            ("start".to_string(), case(started)),
            ("ret".to_string(), case(results)),
            ("failed".to_string(), case(self.record(&calls, "failed"))),
            ("refused".to_string(), case(self.record(&calls, "refused"))),
        ]);
        let recorded = K::Match {
            of: Box::new(var("input")),
            on: "kind".to_string(),
            cases,
            otherwise: Some(Box::new(var("state"))),
        };
        // A message, or the end of a stream nobody reads, changes nothing; nor may anything run before `start`.
        let ignored = one_of(&["msg", "end"], get(var("input"), "kind"));
        let step_fn = if_(
            ignored,
            step(var("state"), list(vec![])),
            let_(
                "@state",
                recorded,
                self.after_result(plan, lowered, self.schedule(plan, lowered)),
            ),
        );
        let slots = self
            .calls
            .iter()
            .map(|(&i, c)| (self.node_name(i).to_string(), slot_type(&c.output)))
            .collect();
        let state = Type::Record(BTreeMap::from([
            ("arg".to_string(), plan.arg.clone()),
            ("next".to_string(), Type::Int),
            ("nodes".to_string(), Type::Record(slots)),
        ]));
        let waiting: serde_json::Map<String, Value> = self
            .calls
            .iter()
            .map(|(&i, c)| {
                let slot = json!({ "kind": "waiting", "call": 0, "value": default_value(&c.output), "error": null });
                (self.node_name(i).to_string(), slot)
            })
            .collect();
        Harness {
            arg: plan.arg.clone(),
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

/// Elaborate a source against the contracts of the capabilities it may use, by identifier.
pub fn elaborate(
    source: &str,
    capabilities: &BTreeMap<String, Interface>,
) -> Result<Elaborated, Vec<Problem>> {
    let file = parse(lex(source).map_err(|p| vec![p])?).map_err(|p| vec![p])?;
    let mut cx = Cx {
        file: &file,
        problems: Vec::new(),
        names: BTreeMap::new(),
        calls: BTreeMap::new(),
        may_skip: BTreeSet::new(),
        fresh: 0,
    };
    let (arg, result) = cx.types();
    let (interface, uses) = cx.install(capabilities);
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

    // Which nodes are calls, and of what.
    for (i, node) in file.nodes.iter().enumerate() {
        let ExprKind::Call(callee, args) = &node.value.kind else {
            if let Some(w) = &node.when {
                cx.report(
                    w.span,
                    What::GuardOnValue {
                        node: node.name.text.clone(),
                    },
                );
            }
            continue;
        };
        let Some(name) = cx.operation(callee, &[]) else {
            if let Some(w) = &node.when {
                cx.report(
                    w.span,
                    What::GuardOnValue {
                        node: node.name.text.clone(),
                    },
                );
            }
            continue;
        };
        let Some(operation) = interface.operations.get(&name) else {
            cx.report(callee.span, What::UnknownOperation { op: name });
            continue;
        };
        if operation.mode == Mode::Stream {
            cx.report(
                callee.span,
                What::NotYet {
                    feature: "streams".to_string(),
                },
            );
        }
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
        cx.calls.insert(
            i,
            Call {
                op: name,
                input: operation.input.clone(),
                output: operation.output.clone(),
            },
        );
    }

    // What each expression reads.
    let mut node_reads: Vec<Reads> = Vec::new();
    for (i, node) in file.nodes.iter().enumerate() {
        let mut reads = match (&node.value.kind, cx.calls.contains_key(&i)) {
            (ExprKind::Call(_, args), true) => {
                args.iter().flat_map(|a| cx.reads(&a.value)).collect()
            }
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
        let skippable = (file.nodes[i].when.is_some() && cx.calls.contains_key(&i))
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
    let unhandled = cx
        .calls
        .keys()
        .copied()
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

    // Lowering and typing.
    let nodes_type = Type::Record(
        cx.calls
            .iter()
            .map(|(&i, c)| (cx.node_name(i).to_string(), slot_type(&c.output)))
            .collect(),
    );
    let mut scope = vec![
        ("@nodes".to_string(), nodes_type),
        (file.param.text.clone(), arg.clone()),
    ];
    for (&i, c) in &cx.calls {
        scope.push((cx.node_name(i).to_string(), c.output.clone()));
    }
    let mut lowered = Lowered {
        args: BTreeMap::new(),
        guards: BTreeMap::new(),
        pures: BTreeMap::new(),
        ret: lit(Value::Null),
        otherwise: None,
    };
    for &i in &order {
        let node = &file.nodes[i];
        if let (Some(call), ExprKind::Call(_, args)) = (cx.calls.get(&i), &node.value.kind) {
            let input = call.input.clone();
            let mut fields = BTreeMap::new();
            for a in args {
                let Some(name) = &a.name else { continue };
                let e = cx.lower(&a.value, &mut Vec::new());
                let t = cx.type_of(&e, a.value.span, &scope);
                if let Type::Record(declared) = &input {
                    match declared.get(&name.text) {
                        Some(expected) => cx.expect(t, expected, a.value.span),
                        None => cx.report(
                            name.span,
                            What::Argument {
                                expected: format!(
                                    "one of the arguments of `{}`",
                                    call_name(&cx, i)
                                ),
                            },
                        ),
                    }
                }
                fields.insert(name.text.clone(), e);
            }
            if let Type::Record(declared) = &input {
                for missing in declared.keys().filter(|k| !fields.contains_key(*k)) {
                    cx.report(
                        node.value.span,
                        What::Argument {
                            expected: format!("the argument `{missing}`"),
                        },
                    );
                }
            }
            lowered.args.insert(i, K::Record { fields });
            if let Some(w) = &node.when {
                let g = cx.lower(w, &mut Vec::new());
                let t = cx.type_of(&g, w.span, &scope);
                cx.expect(t, &Type::Bool, w.span);
                lowered.guards.insert(i, g);
            }
        } else {
            let e = cx.lower(&node.value, &mut Vec::new());
            let t = cx.type_of(&e, node.value.span, &scope);
            scope.push((node.name.text.clone(), t.unwrap_or(Type::Any)));
            lowered.pures.insert(i, e);
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
    Ok(Elaborated { harness, uses })
}

fn call_name(cx: &Cx, i: usize) -> String {
    cx.calls[&i].op.clone()
}
