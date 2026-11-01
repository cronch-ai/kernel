//! From Cronch definitions to a `Script`: the walk over types, values and expressions, reading each through the
//! registries.

use super::readings::{self, Reading, SORTS};
use super::script::{Command, Entry, Section, Term};
use super::{Definition, ExportError, NoReading, Place};
use crate::cbor;
use crate::check::{Path, Problem};
use crate::ir::{Expr, Type};
use crate::ops::{self, Op};
use crate::types::{conforms, of_value};
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

pub struct Encoder {
    /// Record sorts, each after the records its fields use.
    records: Vec<(String, BTreeMap<String, Type>)>,
    /// Variables in scope, innermost last.
    scope: Vec<(String, Type)>,
    fresh: usize,
    /// The branch conditions on the way to the node being encoded.
    guards: Vec<Term>,
    /// Where the kernel's evaluation of what is encoded so far is defined: no overflow, no division by zero, no
    /// infinite or NaN result, each under the branch it is on. SMT-LIB's functions are total, so these are stated.
    defined: Vec<Term>,
    floats: Floats,
}

impl Encoder {
    pub fn new() -> Self {
        Encoder {
            records: Vec::new(),
            scope: Vec::new(),
            fresh: 0,
            guards: Vec::new(),
            defined: Vec::new(),
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

    pub fn expr(&mut self, expr: &Expr, at: &str) -> Result<(Term, Type), ExportError> {
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
            Expr::Label { .. } => Err(here(NoReading::Label)),
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
            Reading::Prefix | Reading::Contains => {
                let (string, sequence) = if reading == Reading::Prefix {
                    ("str.prefixof", "seq.prefixof")
                } else {
                    ("str.contains", "seq.contains")
                };
                match (&types[0], &types[1], reading) {
                    (Type::String, Type::String, _) => Term::call(string, terms),
                    (Type::List(a), Type::List(b), Reading::Prefix)
                        if a == b && plain_equality(a) =>
                    {
                        Term::call(sequence, terms)
                    }
                    (Type::List(a), b, Reading::Contains) if **a == *b && plain_equality(a) => {
                        let mut terms = terms;
                        let item = terms.pop().expect("two operands");
                        terms.push(Term::call("seq.unit", vec![item]));
                        Term::call(sequence, terms)
                    }
                    (left, right, _) => {
                        return Err(NoReading::Equality {
                            left: left.clone(),
                            right: right.clone(),
                        });
                    }
                }
            }
            Reading::Append => {
                let Type::List(element) = ty else {
                    return Err(NoReading::Any);
                };
                let mut terms = terms.into_iter();
                let (list, item) = (
                    terms.next().expect("two operands"),
                    terms.next().expect("two operands"),
                );
                let list = self.coerce(list, &types[0], ty)?;
                let item = self.coerce(item, &types[1], element)?;
                Term::call("seq.++", vec![list, Term::call("seq.unit", vec![item])])
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

    /// The section declaring the record sorts the encoding so far uses, each after the records its fields use.
    pub(super) fn records(&mut self) -> Result<Section, ExportError> {
        let at = at_place(Place::Body { at: String::new() });
        let mut records = Vec::new();
        for (name, fields) in self.records.clone() {
            let mut selectors = Vec::new();
            for (field, ty) in &fields {
                selectors.push((
                    selector(&name, field).map_err(&at)?,
                    self.sort(ty).map_err(&at)?,
                ));
            }
            records.push(Entry {
                comments: vec![describe(&Type::Record(fields))],
                command: Some(Command::DeclareDatatype {
                    name: Term::symbol("", &name).map_err(&at)?,
                    selectors,
                }),
            });
        }
        Ok(Section {
            heading: "Records.",
            entries: records,
        })
    }

    /// A definition as `define-fun`s: the function, and where the kernel's evaluation of it is defined, if that isn't
    /// everywhere.
    pub(super) fn definition(&mut self, def: &Definition) -> Result<Vec<Entry>, ExportError> {
        self.scope.clear();
        self.guards.clear();
        self.defined.clear();
        let mut params = Vec::new();
        for p in &def.params {
            let at = at_place(Place::Param {
                name: p.name.clone(),
            });
            params.push((local(&p.name).map_err(&at)?, self.sort(&p.ty).map_err(&at)?));
            self.scope.push((p.name.clone(), p.ty.clone()));
        }
        let (body, ty) = self.expr(&def.body, "")?;
        let sort = self.sort(&ty).map_err(at_body(""))?;
        let name = Term::symbol("def:", &def.name).map_err(at_body(""))?;
        let mut entries = vec![Entry {
            comments: def.description.lines().map(str::to_string).collect(),
            command: Some(Command::DefineFun {
                name,
                params: params.clone(),
                sort,
                body,
            }),
        }];
        if !self.defined.is_empty() {
            entries.push(Entry {
                comments: vec![format!(
                    "Where the kernel evaluates {} without an error; elsewhere the function above is unconstrained.",
                    def.name
                )],
                command: Some(Command::DefineFun {
                    name: Term::symbol("defined:", &def.name).map_err(at_body(""))?,
                    params,
                    sort: Term::atom("Bool"),
                    body: Term::fold("and", Term::atom("true"), std::mem::take(&mut self.defined)),
                }),
            });
        }
        Ok(entries)
    }
}
