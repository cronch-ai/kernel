//! From Cronch objects to a `Script`: the walk over types, values and Procedures, reading each through the registries.

use super::claim::Read as ValueRead;
use super::readings::{self, Condition, Reading, SORTS};
use super::script::{Command, Entry, Script, Section, Term};
use super::{ExportError, NoReading, Place};
use crate::cbor;
use crate::check::{Path, Problem};
use crate::claim::{Claim, Witness, claim_key};
use crate::ir::{Answer, Ask, AskSpec, Expr, Type, World};
use crate::key::{ask_key, spec_key};
use crate::ops::{self, Op};
use crate::types::{conforms, inhabits, of_value};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

type Read<T> = Result<T, NoReading>;

pub(super) fn at_place(place: Place) -> impl Fn(NoReading) -> ExportError {
    move |reason| ExportError::NoReading {
        place: place.clone(),
        reason,
    }
}

fn at_body(at: &str) -> impl Fn(NoReading) -> ExportError {
    at_place(Place::Body { at: at.to_string() })
}

fn ill_formed(problem: Problem) -> ExportError {
    ExportError::IllFormed {
        problems: vec![problem],
    }
}

fn child(at: &str, segment: &str) -> Path {
    format!("{at}/{segment}")
}

/// A Procedure path as names write it: the root is `/`.
fn shown(at: &str) -> &str {
    if at.is_empty() { "/" } else { at }
}

pub(super) fn local(name: &str) -> Read<Term> {
    Term::symbol("v:", name)
}

/// The first eight hex digits of a SHA-256 over the deterministic CBOR of `value`.
fn short_hash(value: &Value) -> String {
    Sha256::digest(cbor::encode(value))
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A type as the comments write it.
fn describe(ty: &Type) -> String {
    match ty {
        Type::Int => "Int".into(),
        Type::Float => "Float".into(),
        Type::String => "String".into(),
        Type::Bool => "Bool".into(),
        Type::Any => "Any".into(),
        Type::List(t) => format!("List({})", describe(t)),
        Type::Record(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(f, t)| format!("{f}: {}", describe(t)))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
    }
}

fn int_literal(n: &Number) -> Option<Term> {
    if let Some(i) = n.as_i64() {
        return Some(Term::int(i < 0, i.unsigned_abs().to_string()));
    }
    if let Some(u) = n.as_u64() {
        return Some(Term::int(false, u.to_string()));
    }
    let f = n.as_f64().filter(|f| f.is_finite() && f.fract() == 0.0)?;
    Some(Term::int(f < 0.0, format!("{:.0}", f.abs())))
}

/// How Floats are read: as IEEE 754 doubles, exactly as the kernel computes, or as reals, to search fast.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Floats {
    #[default]
    Exact,
    /// No rounding, overflow or NaN: what it finds is a candidate the kernel re-checks, and its `unsat` says nothing.
    Real,
}

/// A number as the kernel computes with it: a float, an integer converted to nearest.
fn as_float(floats: Floats, term: Term, ty: &Type) -> Term {
    match (ty, floats) {
        (Type::Int, Floats::Exact) => Term::to_float(Term::call("to_real", vec![term])),
        (Type::Int, Floats::Real) => Term::call("to_real", vec![term]),
        _ => term,
    }
}

/// A float literal, in the reading.
fn float_literal(floats: Floats, f: f64) -> Term {
    match floats {
        Floats::Exact => Term::float(f),
        Floats::Real => Term::real(f),
    }
}

/// The function a float operation reads as over the reals.
fn over_reals(name: &str) -> &str {
    match name {
        "fp.add" => "+",
        "fp.sub" | "fp.neg" => "-",
        "fp.mul" => "*",
        "fp.div" => "/",
        "fp.lt" => "<",
        "fp.leq" => "<=",
        "fp.gt" => ">",
        "fp.geq" => ">=",
        "fp.eq" => "=",
        other => other,
    }
}

/// The integer is one every host carries exactly: JSON's and JavaScript's safe integers, within ±(2^53 - 1). Both
/// what a counterexample is made of and what the kernel computes from it stay within it.
fn carried(term: Term) -> Term {
    const SAFE: i64 = crate::ops::MAX_SAFE_INT;
    Term::call(
        "and",
        vec![
            Term::call("<=", vec![Term::int(true, SAFE.to_string()), term.clone()]),
            Term::call("<=", vec![term, Term::int(false, SAFE.to_string())]),
        ],
    )
}

/// The float is neither infinite nor NaN.
fn finite(term: Term) -> Term {
    Term::call(
        "not",
        vec![Term::call(
            "or",
            vec![
                Term::call("fp.isNaN", vec![term.clone()]),
                Term::call("fp.isInfinite", vec![term]),
            ],
        )],
    )
}

/// A comparison of numbers by exact value: of integers as integers, of floats as floats, and otherwise of both read
/// as reals, which a finite float and an integer both are exactly.
fn exactly(floats: Floats, int: &str, float: &str, terms: Vec<Term>, types: &[Type]) -> Term {
    match types {
        [Type::Int, Type::Int] => Term::call(int, terms),
        [Type::Float, Type::Float] if floats == Floats::Exact => Term::call(float, terms),
        _ => Term::call(
            int,
            terms
                .into_iter()
                .zip(types)
                .map(|(t, ty)| match (ty, floats) {
                    (Type::Int, _) => Term::call("to_real", vec![t]),
                    (_, Floats::Exact) => Term::call("fp.to_real", vec![t]),
                    (_, Floats::Real) => t,
                })
                .collect(),
        ),
    }
}

fn numeric(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::Float)
}

/// Equality on values of this type is SMT-LIB's: the type says everything the kernel's JSON equality looks at.
fn plain_equality(ty: &Type) -> bool {
    match ty {
        Type::Int | Type::Bool | Type::String => true,
        Type::List(t) => plain_equality(t),
        Type::Float | Type::Any | Type::Record(_) => false,
    }
}

/// The record sort's name and its selectors' names.
pub(super) fn record_names(fields: &BTreeMap<String, Type>) -> String {
    format!(
        "record:{}",
        short_hash(&json!(Type::Record(fields.clone())))
    )
}

fn selector(record: &str, field: &str) -> Read<Term> {
    Term::symbol(&format!("{record}."), field)
}

/// A nested Ask's uninterpreted function: its spec's name and the start of its spec's key.
fn hole_name(spec: &AskSpec) -> Read<Term> {
    let key = spec_key(spec);
    let hash = key.strip_prefix("sha256:").unwrap_or(&key);
    Term::symbol("ask:", &format!("{}:{}", spec.name, &hash[..8]))
}

struct Hole<'a> {
    name: Term,
    spec: &'a AskSpec,
}

pub struct Encoder<'w> {
    world: &'w World,
    /// Record sorts, each after the records its fields use.
    records: Vec<(String, BTreeMap<String, Type>)>,
    /// Nested Asks, in order of first appearance.
    holes: Vec<Hole<'w>>,
    /// Inputs with no value: name, type and note.
    opaque: Vec<(Term, Type, String)>,
    /// The world facts the Procedure reads.
    facts: BTreeMap<String, Type>,
    /// What the responder assumes, one line each.
    assumed: Vec<String>,
    /// Variables in scope, innermost last.
    scope: Vec<(String, Type)>,
    fresh: usize,
    /// While a claim is encoded, the key of the spec it is about: an Ask of it is the Answer, not a hole.
    about: Option<String>,
    /// The branch conditions on the way to the node being encoded.
    guards: Vec<Term>,
    /// Where the kernel's evaluation of what is encoded so far is defined: no overflow, no division by zero, no
    /// infinite or NaN result, each under the branch it is on. SMT-LIB's functions are total, so these are stated.
    defined: Vec<Term>,
    /// Where the Answer's own evaluation is defined, over its parameters: empty when it is total.
    answer_defined: Vec<Term>,
    floats: Floats,
}

impl<'w> Encoder<'w> {
    pub fn new(world: &'w World) -> Self {
        Encoder {
            world,
            records: Vec::new(),
            holes: Vec::new(),
            opaque: Vec::new(),
            facts: BTreeMap::new(),
            assumed: Vec::new(),
            scope: Vec::new(),
            fresh: 0,
            about: None,
            guards: Vec::new(),
            defined: Vec::new(),
            answer_defined: Vec::new(),
            floats: Floats::Exact,
        }
    }

    /// The same encoder, reading Floats as `floats` says.
    pub fn reading(mut self, floats: Floats) -> Self {
        self.floats = floats;
        self
    }

    /// That evaluation is defined here, wherever the branches on the way here are taken.
    fn require(&mut self, condition: Term) {
        let guarded = match self.guards.len() {
            0 => condition,
            _ => Term::call(
                "=>",
                vec![
                    Term::fold("and", Term::atom("true"), self.guards.clone()),
                    condition,
                ],
            ),
        };
        self.defined.push(guarded);
    }

    /// Where the kernel stops with an error applying `op`, as `ops::apply` does.
    fn require_for(&mut self, op: Op, result: &Term, operands: &[Term], types: &[Type], ty: &Type) {
        let floats = self.floats;
        let as_floats = |i: usize| as_float(floats, operands[i].clone(), &types[i]);
        let exact = floats == Floats::Exact;
        match (op, ty) {
            (Op::Add | Op::Sub | Op::Mul | Op::Neg | Op::Round, Type::Int) => {
                self.require(carried(result.clone()))
            }
            (Op::Add | Op::Sub | Op::Mul | Op::Round, Type::Float) if exact => {
                self.require(finite(result.clone()))
            }
            (Op::Div, _) if exact => {
                self.require(Term::call(
                    "not",
                    vec![Term::call("fp.isZero", vec![as_floats(1)])],
                ));
                self.require(finite(result.clone()));
            }
            (Op::Div, _) => self.require(Term::call(
                "not",
                vec![Term::call("=", vec![as_floats(1), Term::real(0.0)])],
            )),
            (Op::IDiv | Op::Mod, _) => {
                self.require(Term::call(
                    "not",
                    vec![Term::call("=", vec![operands[1].clone(), Term::atom("0")])],
                ));
                self.require(carried(result.clone()));
            }
            _ => {}
        }
    }

    fn fresh(&mut self) -> Term {
        self.fresh += 1;
        Term::atom(format!("|c:{}|", self.fresh))
    }

    /// The record sort's name, unquoted, declaring it (after the records its fields use) on first sight.
    fn record(&mut self, fields: &BTreeMap<String, Type>) -> Read<String> {
        let name = record_names(fields);
        if !self.records.iter().any(|(n, _)| *n == name) {
            for (field, ty) in fields {
                selector(&name, field)?;
                self.sort(ty)?;
            }
            self.records.push((name.clone(), fields.clone()));
        }
        Ok(name)
    }

    fn constructor(&mut self, fields: &BTreeMap<String, Type>) -> Read<Term> {
        let name = self.record(fields)?;
        Term::symbol("", &name)
    }

    fn select(&mut self, fields: &BTreeMap<String, Type>, field: &str, of: Term) -> Read<Term> {
        let name = self.record(fields)?;
        Ok(Term::app(selector(&name, field)?, vec![of]))
    }

    pub fn sort(&mut self, ty: &Type) -> Read<Term> {
        if *ty == Type::Float && self.floats == Floats::Real {
            return Ok(Term::atom("Real"));
        }
        if let Some((_, sort)) = SORTS.iter().find(|(t, _)| t == ty) {
            return Ok(Term::atom(*sort));
        }
        match ty {
            Type::List(t) => Ok(Term::call("Seq", vec![self.sort(t)?])),
            Type::Record(fields) => self.constructor(fields),
            _ => Err(NoReading::Any),
        }
    }

    pub fn value(&mut self, v: &Value, ty: &Type) -> Read<Term> {
        let mismatch = || NoReading::Value {
            expected: ty.clone(),
            got: v.clone(),
        };
        match (ty, v) {
            (Type::Any, _) => Err(NoReading::Any),
            (Type::Int, Value::Number(n)) => int_literal(n).ok_or_else(mismatch),
            (Type::Float, Value::Number(n)) => n
                .as_f64()
                .map(|f| float_literal(self.floats, f))
                .ok_or_else(mismatch),
            (Type::String, Value::String(s)) => Term::string(s),
            (Type::Bool, Value::Bool(b)) => Ok(Term::atom(b.to_string())),
            (Type::List(t), Value::Array(items)) => {
                let sort = self.sort(ty)?;
                let empty = Term::List(vec![Term::atom("as"), Term::atom("seq.empty"), sort]);
                let mut units = Vec::new();
                for item in items {
                    units.push(Term::call("seq.unit", vec![self.value(item, t)?]));
                }
                Ok(Term::fold("seq.++", empty, units))
            }
            (Type::Record(fields), Value::Object(map)) => {
                let constructor = self.constructor(fields)?;
                let mut args = Vec::new();
                for (field, t) in fields {
                    args.push(self.value(map.get(field).ok_or_else(mismatch)?, t)?);
                }
                Ok(Term::app(constructor, args))
            }
            _ => Err(mismatch()),
        }
    }

    /// A term of type `from` where `to`, which it conforms to, is required.
    pub(super) fn coerce(&mut self, term: Term, from: &Type, to: &Type) -> Read<Term> {
        match (from, to) {
            _ if from == to => Ok(term),
            (Type::Int, Type::Float) => Ok(as_float(self.floats, term, from)),
            (Type::Record(have), Type::Record(want)) => {
                let r = self.fresh();
                let mut fields = Vec::new();
                for (field, t) in want {
                    let have_t = have.get(field).ok_or(NoReading::UndeclaredField {
                        field: field.clone(),
                    })?;
                    let selected = self.select(have, field, r.clone())?;
                    fields.push(self.coerce(selected, have_t, t)?);
                }
                let constructor = self.constructor(want)?;
                Ok(Term::bind(r, term, Term::app(constructor, fields)))
            }
            (Type::List(_), Type::List(_)) => Err(NoReading::ListConversion {
                from: from.clone(),
                to: to.clone(),
            }),
            _ => Err(NoReading::Any),
        }
    }

    fn hole(&mut self, spec: &'w AskSpec, at: &str) -> Result<Term, ExportError> {
        let name = hole_name(spec).map_err(at_body(at))?;
        if !self.holes.iter().any(|h| h.name == name) {
            for ty in spec.params.iter().map(|p| &p.ty).chain([&spec.output]) {
                self.sort(ty).map_err(at_body(at))?;
            }
            self.holes.push(Hole {
                name: name.clone(),
                spec,
            });
        }
        Ok(name)
    }

    pub fn expr(&mut self, expr: &'w Expr, at: &str) -> Result<(Term, Type), ExportError> {
        let here = at_body(at);
        match expr {
            Expr::Lit { v } => {
                let ty = of_value(v);
                Ok((self.value(v, &ty).map_err(here)?, ty))
            }
            Expr::Var { name } => match self.scope.iter().rev().find(|(n, _)| n == name) {
                Some((_, ty)) => Ok((local(name).map_err(here)?, ty.clone())),
                None => Err(ill_formed(Problem::UnknownVariable {
                    at: at.to_string(),
                    name: name.clone(),
                })),
            },
            Expr::Fact { name } => {
                let fact = self.world.facts.get(name).ok_or_else(|| {
                    ill_formed(Problem::UnknownFact {
                        at: at.to_string(),
                        name: name.clone(),
                    })
                })?;
                let ty = fact.ty.clone().unwrap_or_else(|| of_value(&fact.value));
                self.facts.insert(name.clone(), ty.clone());
                Ok((Term::symbol("fact:", name).map_err(here)?, ty))
            }
            Expr::Op { op, args } => {
                let mut terms = Vec::new();
                let mut types = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    let (t, ty) = self.expr(a, &child(at, &format!("args/{i}")))?;
                    terms.push(t);
                    types.push(ty);
                }
                let parsed = Op::parse(op).ok_or_else(|| {
                    ill_formed(Problem::UnknownOperator {
                        at: at.to_string(),
                        op: op.clone(),
                    })
                })?;
                let ty = ops::result_type(parsed, &types).map_err(|error| {
                    ill_formed(Problem::Operator {
                        at: at.to_string(),
                        op: op.clone(),
                        error,
                    })
                })?;
                if types.contains(&Type::Any) {
                    return Err(here(NoReading::Any));
                }
                let reading = readings::operator(parsed)
                    .ok_or_else(|| NoReading::Operator { op: op.clone() });
                let operands = terms.clone();
                let term = reading
                    .and_then(|r| self.apply(r, terms, &types, &ty, args))
                    .map_err(here)?;
                self.require_for(parsed, &term, &operands, &types, &ty);
                Ok((term, ty))
            }
            Expr::Ask { spec, args } => {
                let name = if self.about.as_deref() == Some(spec_key(spec).as_str()) {
                    Term::atom("answer")
                } else {
                    self.hole(spec, at)?
                };
                let mut terms = Vec::new();
                for p in &spec.params {
                    let arg_at = child(at, &format!("args/{}", p.name));
                    let arg = args.get(&p.name).ok_or_else(|| {
                        ill_formed(Problem::MissingHoleArgument {
                            at: at.to_string(),
                            ask: spec.name.clone(),
                            param: p.name.clone(),
                        })
                    })?;
                    let (t, ty) = self.expr(arg, &arg_at)?;
                    terms.push(self.coerce(t, &ty, &p.ty).map_err(at_body(&arg_at))?);
                }
                let applied = Term::app(name.clone(), terms.clone());
                if name == Term::atom("answer") {
                    if !self.answer_defined.is_empty() {
                        self.require(Term::app(Term::atom("defined"), terms));
                    }
                } else if let Some(value) = self
                    .representable(applied.clone(), &spec.output)
                    .map_err(here)?
                {
                    // A nested Ask is answered with a value Cronch has, wherever it is asked.
                    self.require(value);
                }
                Ok((applied, spec.output.clone()))
            }
            Expr::Assume {
                v,
                ty,
                note,
                source,
            } => {
                let mut line = format!("{}: {v} : {}", shown(at), describe(ty));
                if !note.is_empty() {
                    line.push_str(&format!(" ({note})"));
                }
                if !source.is_empty() {
                    line.push_str(&format!(", from {source}"));
                }
                self.assumed.push(line);
                Ok((self.value(v, ty).map_err(here)?, ty.clone()))
            }
            Expr::Opaque { ty, note } => {
                let name = Term::symbol("opaque:", shown(at)).map_err(&here)?;
                self.sort(ty).map_err(here)?;
                self.opaque.push((name.clone(), ty.clone(), note.clone()));
                Ok((name, ty.clone()))
            }
            Expr::Let { name, be, body } => {
                let (bound, bound_ty) = self.expr(be, &child(at, "be"))?;
                let var = local(name).map_err(here)?;
                self.scope.push((name.clone(), bound_ty));
                let from = self.defined.len();
                let result = self.expr(body, &child(at, "in"));
                self.scope.pop();
                let (term, ty) = result?;
                // What the body requires mentions the bound variable, so it is stated under the same binding.
                for condition in &mut self.defined[from..] {
                    *condition = Term::bind(var.clone(), bound.clone(), condition.clone());
                }
                Ok((Term::bind(var, bound, term), ty))
            }
            Expr::If {
                cond,
                then,
                otherwise,
            } => {
                let (c, _) = self.expr(cond, &child(at, "cond"))?;
                // Only the branch taken is evaluated, so each requires only where it is taken.
                self.guards.push(c.clone());
                let then = self.expr(then, &child(at, "then"));
                self.guards.pop();
                self.guards.push(Term::call("not", vec![c.clone()]));
                let otherwise = self.expr(otherwise, &child(at, "else"));
                self.guards.pop();
                let ((t, then_ty), (e, else_ty)) = (then?, otherwise?);
                let ty = if conforms(&else_ty, &then_ty) {
                    then_ty.clone()
                } else {
                    else_ty.clone()
                };
                let t = self
                    .coerce(t, &then_ty, &ty)
                    .map_err(at_body(&child(at, "then")))?;
                let e = self
                    .coerce(e, &else_ty, &ty)
                    .map_err(at_body(&child(at, "else")))?;
                Ok((Term::call("ite", vec![c, t, e]), ty))
            }
            Expr::Record { fields } => {
                let mut terms = Vec::new();
                let mut types = BTreeMap::new();
                for (field, e) in fields {
                    let (t, ty) = self.expr(e, &child(at, &format!("fields/{field}")))?;
                    terms.push(t);
                    types.insert(field.clone(), ty);
                }
                let constructor = self.constructor(&types).map_err(here)?;
                Ok((Term::app(constructor, terms), Type::Record(types)))
            }
            Expr::List { items } => {
                let mut encoded = Vec::new();
                for (i, e) in items.iter().enumerate() {
                    let item_at = child(at, &format!("items/{i}"));
                    encoded.push((self.expr(e, &item_at)?, item_at));
                }
                // An empty list literal is a `List(Any)`, which has no sort.
                let element = encoded
                    .first()
                    .map(|((_, ty), _)| ty.clone())
                    .ok_or_else(|| here(NoReading::Any))?;
                let sort = self
                    .sort(&Type::List(Box::new(element.clone())))
                    .map_err(&here)?;
                let mut units = Vec::new();
                for ((t, ty), item_at) in encoded {
                    units.push(Term::call(
                        "seq.unit",
                        vec![self.coerce(t, &ty, &element).map_err(at_body(&item_at))?],
                    ));
                }
                let empty = Term::List(vec![Term::atom("as"), Term::atom("seq.empty"), sort]);
                Ok((
                    Term::fold("seq.++", empty, units),
                    Type::List(Box::new(element)),
                ))
            }
            Expr::Get { of, field } => {
                let (term, ty) = self.expr(of, &child(at, "of"))?;
                match ty {
                    Type::Record(fields) => {
                        let field_ty = fields.get(field).cloned().ok_or_else(|| {
                            ill_formed(Problem::NoSuchField {
                                at: at.to_string(),
                                field: field.clone(),
                                of: Type::Record(fields.clone()),
                            })
                        })?;
                        Ok((self.select(&fields, field, term).map_err(here)?, field_ty))
                    }
                    _ => Err(here(NoReading::Any)),
                }
            }
            Expr::Fold { .. } => Err(here(NoReading::Fold)),
        }
    }

    /// An operator's registered reading, applied to its operands.
    fn apply(
        &mut self,
        reading: Reading,
        terms: Vec<Term>,
        types: &[Type],
        ty: &Type,
        args: &[Expr],
    ) -> Read<Term> {
        let mode = self.floats;
        let floats = || {
            terms
                .iter()
                .cloned()
                .zip(types)
                .map(|(t, ty)| as_float(mode, t, ty))
                .collect::<Vec<_>>()
        };
        let float = |name: &str, rounded: bool, operands: Vec<Term>| match mode {
            Floats::Exact => {
                let mode = rounded.then(|| Term::atom("RNE"));
                Term::call(name, mode.into_iter().chain(operands).collect())
            }
            Floats::Real => Term::call(over_reals(name), operands),
        };
        Ok(match reading {
            Reading::Numeric { int, .. } if *ty == Type::Int => Term::call(int, terms),
            Reading::Numeric {
                float: name,
                rounded,
                ..
            }
            | Reading::Float { name, rounded } => float(name, rounded, floats()),
            Reading::Same(name) => Term::call(name, terms),
            Reading::Compare { int, float } => exactly(mode, int, float, terms, types),
            Reading::Equal { negated } => {
                let eq = if numeric(&types[0]) && numeric(&types[1]) {
                    exactly(mode, "=", "fp.eq", terms, types)
                } else if types[0] == types[1] && plain_equality(&types[0]) {
                    Term::call("=", terms)
                } else {
                    return Err(NoReading::Equality {
                        left: types[0].clone(),
                        right: types[1].clone(),
                    });
                };
                if negated {
                    Term::call("not", vec![eq])
                } else {
                    eq
                }
            }
            Reading::Concat => Term::fold("str.++", Term::atom("\"\""), terms),
            Reading::Text => {
                let term = terms.into_iter().next().unwrap_or(Term::atom("\"\""));
                match &types[0] {
                    Type::String => term,
                    Type::Bool => Term::call(
                        "ite",
                        vec![term, Term::atom("\"true\""), Term::atom("\"false\"")],
                    ),
                    Type::Int => {
                        let n = self.fresh();
                        let digits = |of: Term| Term::call("str.from_int", vec![of]);
                        let negative = Term::call("<", vec![n.clone(), Term::atom("0")]);
                        let minus = Term::call(
                            "str.++",
                            vec![
                                Term::atom("\"-\""),
                                digits(Term::call("-", vec![n.clone()])),
                            ],
                        );
                        Term::bind(
                            n.clone(),
                            term,
                            Term::call("ite", vec![negative, minus, digits(n)]),
                        )
                    }
                    other => return Err(NoReading::Text { of: other.clone() }),
                }
            }
            Reading::Length => match &types[0] {
                Type::String => Term::call("str.len", terms),
                _ => Term::call("seq.len", terms),
            },
            Reading::Round if mode == Floats::Real => {
                let x = floats().into_iter().next().unwrap_or(Term::real(0.0));
                // Half away from zero, over the reals.
                let nearest = |of: Term| {
                    let half = |t: Term| {
                        Term::call("to_int", vec![Term::call("+", vec![t, Term::real(0.5)])])
                    };
                    let negative = Term::call("<", vec![of.clone(), Term::real(0.0)]);
                    let below = Term::call("-", vec![half(Term::call("-", vec![of.clone()]))]);
                    Term::call("ite", vec![negative, below, half(of)])
                };
                match args.get(1) {
                    None => nearest(x),
                    Some(Expr::Lit { v }) => {
                        let digits = v
                            .as_i64()
                            .filter(|d| (0..=22).contains(d))
                            .ok_or(NoReading::RoundingDigits)?;
                        let scale = Term::real(10f64.powi(digits as i32));
                        let scaled = Term::call(
                            "to_real",
                            vec![nearest(Term::call("*", vec![x, scale.clone()]))],
                        );
                        Term::call("/", vec![scaled, scale])
                    }
                    Some(_) => return Err(NoReading::RoundingDigits),
                }
            }
            Reading::Round => {
                let x = floats().into_iter().next().unwrap_or(Term::float(0.0));
                let nearest =
                    |of: Term| Term::call("fp.roundToIntegral", vec![Term::atom("RNA"), of]);
                match args.get(1) {
                    None => Term::call("to_int", vec![Term::call("fp.to_real", vec![nearest(x)])]),
                    // 10^d is exact as a float for d up to 22, so the kernel's scale and this one agree.
                    Some(Expr::Lit { v }) => {
                        let digits = v
                            .as_i64()
                            .filter(|d| (0..=22).contains(d))
                            .ok_or(NoReading::RoundingDigits)?;
                        let scale = Term::float(10f64.powi(digits as i32));
                        float(
                            "fp.div",
                            true,
                            vec![
                                nearest(float("fp.mul", true, vec![x, scale.clone()])),
                                scale,
                            ],
                        )
                    }
                    Some(_) => return Err(NoReading::RoundingDigits),
                }
            }
        })
    }

    /// The term at a validator's path, or `None` where the kernel finds no field (the value there isn't a record).
    fn path(
        &mut self,
        mut term: Term,
        mut ty: Type,
        fields: &[String],
    ) -> Read<Option<(Term, Type)>> {
        for field in fields {
            match ty {
                Type::Record(declared) => {
                    let field_ty =
                        declared
                            .get(field)
                            .cloned()
                            .ok_or(NoReading::UndeclaredField {
                                field: field.clone(),
                            })?;
                    term = self.select(&declared, field, term)?;
                    ty = field_ty;
                }
                Type::Any => return Err(NoReading::Any),
                _ => return Ok(None),
            }
        }
        Ok(Some((term, ty)))
    }

    /// A validator's registered condition on `result`, or `None` for one the kernel doesn't check.
    fn condition(&mut self, condition: Condition, output: &Type) -> Read<Option<Term>> {
        let unsatisfiable = Term::atom("false");
        let result = Term::atom("result");
        Ok(Some(match condition {
            Condition::Unstated => return Ok(None),
            Condition::Bound {
                at,
                int,
                float,
                bound,
            } => match self.path(result, output.clone(), at)? {
                Some((_, Type::Any)) => return Err(NoReading::Any),
                // A whole bound is an integer, so an integer meets it by integer comparison.
                Some((term, Type::Int)) if bound.fract() == 0.0 && bound.abs() < 9.2e18 => {
                    Term::call(
                        int,
                        vec![term, Term::int(bound < 0.0, format!("{:.0}", bound.abs()))],
                    )
                }
                Some((term, ty)) if numeric(&ty) => {
                    let bound = float_literal(self.floats, bound);
                    exactly(
                        self.floats,
                        int,
                        float,
                        vec![term, bound],
                        &[ty, Type::Float],
                    )
                }
                _ => unsatisfiable,
            },
            Condition::Size { at, test, bound } => match self.path(result, output.clone(), at)? {
                Some((term, Type::String)) => {
                    Term::call(test, vec![Term::call("str.len", vec![term]), bound.into()])
                }
                Some((term, Type::List(_))) => {
                    Term::call(test, vec![Term::call("seq.len", vec![term]), bound.into()])
                }
                // The kernel counts every field a record value has, declared or not.
                Some((_, Type::Record(_))) => return Err(NoReading::RecordSize),
                Some((_, Type::Any)) => return Err(NoReading::Any),
                _ => unsatisfiable,
            },
            Condition::OneOf { at, values } => match self.path(result, output.clone(), at)? {
                Some((_, Type::Any)) => return Err(NoReading::Any),
                Some((term, ty)) if plain_equality(&ty) => {
                    let mut any = Vec::new();
                    for v in values.iter().filter(|v| inhabits(v, &ty)) {
                        any.push(Term::call("=", vec![term.clone(), self.value(v, &ty)?]));
                    }
                    Term::fold("or", unsatisfiable, any)
                }
                Some((_, ty)) => {
                    return Err(NoReading::Equality {
                        left: ty.clone(),
                        right: ty,
                    });
                }
                None => unsatisfiable,
            },
        }))
    }

    /// The Answer as a function of its spec's parameters, `(define-fun answer (params) output body)`.
    pub(super) fn answer_fun(
        &mut self,
        spec: &AskSpec,
        answer: &'w Answer,
    ) -> Result<Command, ExportError> {
        let at_spec = at_place(Place::Spec);
        let outer = std::mem::take(&mut self.scope);
        let (outer_guards, outer_defined) = (
            std::mem::take(&mut self.guards),
            std::mem::take(&mut self.defined),
        );
        let mut params = Vec::new();
        for p in &spec.params {
            params.push((
                local(&p.name).map_err(&at_spec)?,
                self.sort(&p.ty).map_err(&at_spec)?,
            ));
            self.scope.push((p.name.clone(), p.ty.clone()));
        }
        let sort = self.sort(&spec.output).map_err(&at_spec)?;
        let body = match answer {
            Answer::Value(v) => self.value(v, &spec.output).map_err(at_body("")),
            Answer::Procedure(expr) => self
                .expr(expr, "")
                .and_then(|(term, ty)| self.coerce(term, &ty, &spec.output).map_err(at_body(""))),
        };
        self.scope = outer;
        self.guards = outer_guards;
        self.answer_defined = std::mem::replace(&mut self.defined, outer_defined);
        Ok(Command::DefineFun {
            name: Term::atom("answer"),
            params,
            sort,
            body: body?,
        })
    }

    /// The sections declaring what the encoding so far uses: records, the world's facts, nested Asks, inputs with no
    /// value, and the values the responder assumes.
    pub(super) fn declarations(&mut self) -> Result<Vec<Section>, ExportError> {
        let at_spec = at_place(Place::Spec);
        let mut facts = Vec::new();
        for (name, ty) in std::mem::take(&mut self.facts) {
            let fact = &self.world.facts[&name];
            let place = at_place(Place::Fact { name: name.clone() });
            facts.push(Entry {
                comments: if fact.source.is_empty() {
                    vec![]
                } else {
                    vec![format!("from {}", fact.source)]
                },
                command: Some(Command::DefineFun {
                    name: Term::symbol("fact:", &name).map_err(&place)?,
                    params: vec![],
                    sort: self.sort(&ty).map_err(&place)?,
                    body: self.value(&fact.value, &ty).map_err(&place)?,
                }),
            });
        }

        let mut holes = Vec::new();
        for hole in std::mem::take(&mut self.holes) {
            let mut comments: Vec<String> =
                hole.spec.description.lines().map(str::to_string).collect();
            if hole.spec.effect {
                comments.push(EFFECT.into());
            }
            let mut sorts = Vec::new();
            for p in &hole.spec.params {
                sorts.push(self.sort(&p.ty).map_err(&at_spec)?);
            }
            let sort = self.sort(&hole.spec.output).map_err(&at_spec)?;
            holes.push(Entry {
                comments,
                command: Some(Command::DeclareFun {
                    name: hole.name,
                    params: sorts,
                    sort,
                }),
            });
        }

        let mut opaque = Vec::new();
        for (name, ty, note) in std::mem::take(&mut self.opaque) {
            let sort = self.sort(&ty).map_err(&at_spec)?;
            opaque.push(Entry {
                comments: note.lines().map(str::to_string).collect(),
                command: Some(Command::DeclareConst { name, sort }),
            });
        }

        let mut records = Vec::new();
        for (name, fields) in self.records.clone() {
            let mut selectors = Vec::new();
            for (field, ty) in &fields {
                selectors.push((
                    selector(&name, field).map_err(&at_spec)?,
                    self.sort(ty).map_err(&at_spec)?,
                ));
            }
            records.push(Entry {
                comments: vec![describe(&Type::Record(fields))],
                command: Some(Command::DeclareDatatype {
                    name: Term::symbol("", &name).map_err(&at_spec)?,
                    selectors,
                }),
            });
        }

        Ok(vec![
            Section {
                heading: "Records.",
                entries: records,
            },
            Section {
                heading: "The requester's declared world.",
                entries: facts,
            },
            Section {
                heading: "The Asks this Answer rests on, uninterpreted: their Answers are not part of this one.",
                entries: holes,
            },
            Section {
                heading: "Inputs with no value: the Procedure can't be evaluated past them.",
                entries: opaque,
            },
            Section {
                heading: "Values the responder assumes, which nobody checked:",
                entries: std::mem::take(&mut self.assumed)
                    .into_iter()
                    .map(|line| Entry {
                        comments: vec![line],
                        command: None,
                    })
                    .collect(),
            },
        ])
    }

    fn described(spec: &AskSpec) -> Vec<String> {
        let mut described: Vec<String> = spec.description.lines().map(str::to_string).collect();
        if spec.effect {
            described.push(EFFECT.into());
        }
        described
    }

    /// The Ask, its world and its Answer as a script.
    pub fn script(mut self, ask: &'w Ask, answer: &'w Answer) -> Result<Script, ExportError> {
        let spec = &ask.spec;
        let answer_fun = self.answer_fun(spec, answer)?;
        let Command::DefineFun {
            params,
            sort: output,
            ..
        } = &answer_fun
        else {
            unreachable!()
        };
        let (params, output) = (params.clone(), output.clone());
        for p in &spec.params {
            self.scope.push((p.name.clone(), p.ty.clone()));
        }

        let mut holds = Vec::new();
        let mut unstated = Vec::new();
        for (index, validator) in spec.validators.iter().enumerate() {
            match self
                .condition(readings::condition(validator), &spec.output)
                .map_err(at_place(Place::Validator { index }))?
            {
                Some(term) => holds.push(term),
                None => unstated.push(match validator {
                    crate::ir::Validator::Prose { text } => format!("unchecked: {text}"),
                    crate::ir::Validator::Checker { actor, note } if note.is_empty() => {
                        format!("delegated to {actor}")
                    }
                    crate::ir::Validator::Checker { actor, note } => {
                        format!("delegated to {actor}: {note}")
                    }
                    _ => String::new(),
                }),
            }
        }

        let mut args = Vec::new();
        for p in &spec.params {
            let arg = ask.args.get(&p.name).ok_or_else(|| {
                ill_formed(Problem::MissingArgument {
                    ask: spec.name.clone(),
                    param: p.name.clone(),
                })
            })?;
            args.push(self.value(arg, &p.ty).map_err(at_place(Place::Argument {
                param: p.name.clone(),
            }))?);
        }
        // Validators and arguments may declare records too, so declarations come last.
        let mut sections = self.declarations()?;

        let mut contract_params = params;
        contract_params.push((Term::atom("result"), output.clone()));
        let form = if matches!(answer, Answer::Value(_)) {
            "a raw value"
        } else {
            "a Procedure"
        };

        sections.extend([
            Section {
                heading: "The Answer, as a function of the Ask's parameters.",
                entries: vec![Entry {
                    comments: Self::described(spec),
                    command: Some(answer_fun),
                }],
            },
            Section {
                heading: "What the Ask's replayable validators say of a result.",
                entries: vec![Entry {
                    comments: unstated,
                    command: Some(Command::DefineFun {
                        name: Term::atom("contract"),
                        params: contract_params,
                        sort: Term::atom("Bool"),
                        body: Term::fold("and", Term::atom("true"), holds),
                    }),
                }],
            },
            Section {
                heading: "The closed Ask.",
                entries: vec![
                    Entry {
                        comments: vec![],
                        command: Some(Command::DefineFun {
                            name: Term::atom("value"),
                            params: vec![],
                            sort: output,
                            body: Term::app(Term::atom("answer"), args.clone()),
                        }),
                    },
                    Entry {
                        comments: vec![],
                        command: Some(Command::DefineFun {
                            name: Term::atom("holds"),
                            params: vec![],
                            sort: Term::atom("Bool"),
                            body: Term::app(
                                Term::atom("contract"),
                                args.into_iter().chain([Term::atom("value")]).collect(),
                            ),
                        }),
                    },
                ],
            },
        ]);

        Ok(Script {
            preamble: vec![
                format!("{}, answered by {form}.", spec.name),
                "Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:"
                    .into(),
                ask_key(ask, &self.world.facts),
            ],
            options: vec![],
            logic: "ALL",
            sections,
        })
    }

    /// That `term` is a value a counterexample can hold at type `ty`: a safe integer, a float neither infinite nor NaN,
    /// and the same for every field and element. `None` where every value of the sort is one.
    fn representable(&mut self, term: Term, ty: &Type) -> Read<Option<Term>> {
        Ok(match ty {
            Type::Int => Some(carried(term)),
            Type::Float if self.floats == Floats::Exact => Some(finite(term)),
            Type::Float => None,
            Type::Bool | Type::String | Type::Any => None,
            Type::Record(fields) => {
                let mut each = Vec::new();
                for (field, t) in fields {
                    let selected = self.select(fields, field, term.clone())?;
                    each.extend(self.representable(selected, t)?);
                }
                (!each.is_empty()).then(|| Term::fold("and", Term::atom("true"), each))
            }
            Type::List(t) => {
                let i = self.fresh();
                let nth = Term::call("seq.nth", vec![term.clone(), i.clone()]);
                self.representable(nth, t)?.map(|each| {
                    let within = Term::call(
                        "and",
                        vec![
                            Term::call("<=", vec![Term::atom("0"), i.clone()]),
                            Term::call("<", vec![i.clone(), Term::call("seq.len", vec![term])]),
                        ],
                    );
                    Term::List(vec![
                        Term::atom("forall"),
                        Term::List(vec![Term::List(vec![i, Term::atom("Int")])]),
                        Term::call("=>", vec![within, each]),
                    ])
                })
            }
        })
    }

    /// A nested Ask closed at values, as an application of its uninterpreted function, if the Answer declares it.
    fn application(&mut self, ask: &Ask) -> Read<Option<Term>> {
        let name = hole_name(&ask.spec)?;
        if !self.holes.iter().any(|h| h.name == name) {
            return Ok(None);
        }
        let mut args = Vec::new();
        for p in &ask.spec.params {
            let arg = ask.args.get(&p.name).unwrap_or(&Value::Null);
            args.push(self.value(arg, &p.ty)?);
        }
        Ok(Some(Term::app(name, args)))
    }

    /// A claim about an Answer, negated: the script is `sat` exactly where some values of what the witness leaves open
    /// make `holds` false. Given `reads`, it asks for their values.
    pub fn claim(
        mut self,
        claim: &'w Claim,
        answer: &'w Answer,
        witness: &Witness,
        reads: &[ValueRead],
    ) -> Result<Script, ExportError> {
        let answer_fun = self.answer_fun(&claim.about, answer)?;
        let defined_fun = match (&answer_fun, self.answer_defined.is_empty()) {
            (Command::DefineFun { params, .. }, false) => Some(Command::DefineFun {
                name: Term::atom("defined"),
                params: params.clone(),
                sort: Term::atom("Bool"),
                body: Term::fold("and", Term::atom("true"), self.answer_defined.clone()),
            }),
            _ => None,
        };
        let in_claim = |error| match error {
            ExportError::NoReading {
                place: Place::Body { at },
                reason,
            } => ExportError::NoReading {
                place: Place::Claim { at },
                reason,
            },
            error => error,
        };

        let mut consts = Vec::new();
        for p in &claim.vars {
            let place = at_place(Place::Var {
                name: p.name.clone(),
            });
            let name = local(&p.name).map_err(&place)?;
            consts.push(Entry {
                comments: vec![],
                command: Some(Command::DeclareConst {
                    name,
                    sort: self.sort(&p.ty).map_err(&place)?,
                }),
            });
            self.scope.push((p.name.clone(), p.ty.clone()));
        }
        self.about = Some(spec_key(&claim.about));
        let encoded = self.expr(&claim.holds, "");
        self.about = None;
        let (holds, _) = encoded.map_err(in_claim)?;
        let defined: Vec<Entry> = std::mem::take(&mut self.defined)
            .into_iter()
            .map(|condition| Entry {
                comments: vec![],
                command: Some(Command::Assert(condition)),
            })
            .collect();

        // Every value the counterexample is made of is one the kernel can be given: the solver's own sorts are wider.
        let mut ranges = Vec::new();
        for p in &claim.vars {
            let place = at_place(Place::Var {
                name: p.name.clone(),
            });
            if let Some(term) = self
                .representable(local(&p.name).map_err(&place)?, &p.ty)
                .map_err(&place)?
            {
                ranges.push(Entry {
                    comments: vec![],
                    command: Some(Command::Assert(term)),
                });
            }
        }

        let mut fixed = Vec::new();
        for p in &claim.vars {
            if let Some(v) = witness.vars.get(&p.name) {
                let place = at_place(Place::Var {
                    name: p.name.clone(),
                });
                let term = Term::call(
                    "=",
                    vec![
                        local(&p.name).map_err(&place)?,
                        self.value(v, &p.ty).map_err(&place)?,
                    ],
                );
                fixed.push(Entry {
                    comments: vec![],
                    command: Some(Command::Assert(term)),
                });
            }
        }
        for h in &witness.holes {
            let place = at_place(Place::Hole {
                ask: h.ask.spec.name.clone(),
            });
            if let Some(applied) = self.application(&h.ask).map_err(&place)? {
                let value = self.value(&h.value, &h.ask.spec.output).map_err(&place)?;
                let term = Term::call("=", vec![applied, value]);
                fixed.push(Entry {
                    comments: vec![],
                    command: Some(Command::Assert(term)),
                });
            }
        }
        let mut terms = Vec::new();
        for read in reads {
            match read {
                ValueRead::Var { name, .. } => {
                    terms.push(local(name).map_err(at_place(Place::Var { name: name.clone() }))?)
                }
                ValueRead::Hole { ask, .. } => {
                    let place = at_place(Place::Hole {
                        ask: ask.spec.name.clone(),
                    });
                    let applied = self.application(ask).map_err(&place)?;
                    terms.push(applied.ok_or_else(|| place(NoReading::Any))?);
                }
            }
        }

        let mut sections = self.declarations()?;
        let mut search = vec![
            Entry {
                comments: vec!["The claim is false here.".into()],
                command: Some(Command::Assert(Term::call("not", vec![holds]))),
            },
            Entry {
                comments: vec![],
                command: Some(Command::CheckSat),
            },
        ];
        if !terms.is_empty() {
            search.push(Entry {
                comments: vec![],
                command: Some(Command::GetValue(terms)),
            });
        }
        sections.extend([
            Section {
                heading: "The Answer under check, as a function of its spec's parameters.",
                entries: vec![Entry { comments: Self::described(&claim.about), command: Some(answer_fun) }],
            },
            Section {
                heading:
                    "Where the kernel evaluates it without error: no overflow, no division by zero, finite floats.",
                entries: defined_fun.into_iter().map(|c| Entry { comments: vec![], command: Some(c) }).collect(),
            },
            Section { heading: "The claim's variables.", entries: consts },
            Section {
                heading: "Only values Cronch has: integers within ±(2^53 - 1), finite floats.", entries: ranges
            },
            Section {
                heading: "Only where the claim itself, and every Answer it applies, evaluates.",
                entries: defined,
            },
            Section { heading: "What the counterexample fixes.", entries: fixed },
            Section { heading: "A counterexample: sat exactly where the claim fails.", entries: search },
        ]);
        let stance = match claim.stance {
            crate::claim::Stance::Claim => "Claim",
            crate::claim::Stance::Assumption => "Assumption",
        };
        let mut preamble = vec![format!(
            "{stance} {} about {}.",
            claim.name, claim.about.name
        )];
        preamble.extend(claim.description.lines().map(str::to_string));
        preamble.extend([
            "Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:".into(),
            claim_key(claim),
        ]);
        Ok(Script {
            preamble,
            options: if reads.is_empty() {
                vec![]
            } else {
                vec![(":produce-models", "true")]
            },
            logic: "ALL",
            sections,
        })
    }
}

const EFFECT: &str = "An effect: it changes the world, and runs only with authority.";
