//! From Cronch definitions to a `Script`: the walk over types, values and expressions, reading each through the
//! registries.

use super::readings::{self, Reading, SORTS};
use super::script::{Command, Entry, Section, Term};
use super::{Definition, ExportError, NoReading, Place};
use crate::cbor;
use crate::check::{Path, Problem};
use crate::ir::{Expr, Type};
use crate::ops::{self, Op};
use crate::types::{conforms, join, of_value, record_view};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

type Read<T> = Result<T, NoReading>;

pub(super) fn at_place(place: Place) -> impl Fn(NoReading) -> ExportError {
    move |reason| ExportError::NoReading {
        place: place.clone(),
        reason: Box::new(reason),
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
        Type::Json => "Json".into(),
        Type::List(t) => format!("List({})", describe(t)),
        Type::Record(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(f, t)| format!("{f}: {}", describe(t)))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        Type::Variant { tag, cases } => {
            let cases: Vec<String> = cases
                .iter()
                .map(|(c, t)| format!("{c}: {}", describe(t)))
                .collect();
            format!("<{tag}: {}>", cases.join(" | "))
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
        Type::Float | Type::Any | Type::Json | Type::Record(_) | Type::Variant { .. } => false,
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

/// The variant sort's name; its constructors are `name#case` and their selectors `name#case.field`.
pub(super) fn variant_names(tag: &str, cases: &BTreeMap<String, Type>) -> String {
    let ty = Type::Variant {
        tag: tag.to_string(),
        cases: cases.clone(),
    };
    format!("variant:{}", short_hash(&json!(ty)))
}

fn case_constructor(variant: &str, case: &str) -> Read<Term> {
    Term::symbol(&format!("{variant}#"), case)
}

fn case_selector(variant: &str, case: &str, field: &str) -> Read<Term> {
    Term::symbol(&format!("{variant}#{case}."), field)
}

/// What a case's constructor carries: a record case's fields, or one unnamed field holding a nested variant.
pub(super) fn case_fields(case: &Type) -> Read<Vec<(String, Type)>> {
    match case {
        Type::Record(fields) => Ok(fields.iter().map(|(f, t)| (f.clone(), t.clone())).collect()),
        Type::Variant { .. } => Ok(vec![(String::new(), case.clone())]),
        _ => Err(NoReading::Any),
    }
}

/// `((_ is C) term)`: whether the term was built by the case's constructor.
fn tester(variant: &str, case: &str, term: Term) -> Read<Term> {
    let is = Term::List(vec![
        Term::atom("_"),
        Term::atom("is"),
        case_constructor(variant, case)?,
    ]);
    Ok(Term::List(vec![is, term]))
}

#[derive(Clone)]
enum Datatype {
    Record(BTreeMap<String, Type>),
    Variant(BTreeMap<String, Type>),
}

pub struct Encoder {
    /// Record and variant sorts, each after the sorts it uses.
    datatypes: Vec<(String, Datatype)>,
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
            datatypes: Vec::new(),
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
        if !self.datatypes.iter().any(|(n, _)| *n == name) {
            for (field, ty) in fields {
                selector(&name, field)?;
                self.sort(ty)?;
            }
            self.datatypes
                .push((name.clone(), Datatype::Record(fields.clone())));
        }
        Ok(name)
    }

    /// The variant sort's name, unquoted, declaring it (after the sorts its cases use) on first sight.
    fn variant(&mut self, tag: &str, cases: &BTreeMap<String, Type>) -> Read<String> {
        if cases.is_empty() {
            return Err(NoReading::EmptyVariant);
        }
        let name = variant_names(tag, cases);
        if !self.datatypes.iter().any(|(n, _)| *n == name) {
            for (case, ty) in cases {
                case_constructor(&name, case)?;
                for (field, t) in case_fields(ty)? {
                    case_selector(&name, case, &field)?;
                    self.sort(&t)?;
                }
            }
            self.datatypes
                .push((name.clone(), Datatype::Variant(cases.clone())));
        }
        Ok(name)
    }

    /// A field of a variant-typed term: its tag, or a field every case has, read from whichever case it is.
    fn variant_field(
        &mut self,
        term: Term,
        tag: &str,
        cases: &BTreeMap<String, Type>,
        field: &str,
    ) -> Read<(Term, Type)> {
        let name = self.variant(tag, cases)?;
        let field_ty = if field == tag {
            Type::String
        } else {
            match record_view(&Type::Variant {
                tag: tag.to_string(),
                cases: cases.clone(),
            }) {
                Type::Record(view) => view.get(field).cloned().ok_or(NoReading::Any)?,
                _ => return Err(NoReading::Any),
            }
        };
        let mut branches = Vec::new();
        for (case, case_ty) in cases {
            let value = if field == tag {
                Term::string(case)?
            } else {
                match case_ty {
                    Type::Record(fields) => {
                        let read =
                            Term::app(case_selector(&name, case, field)?, vec![term.clone()]);
                        self.coerce(read, &fields[field], &field_ty)?
                    }
                    Type::Variant {
                        tag: inner_tag,
                        cases: inner,
                    } => {
                        let nested = Term::app(case_selector(&name, case, "")?, vec![term.clone()]);
                        let (read, ty) = self.variant_field(nested, inner_tag, inner, field)?;
                        self.coerce(read, &ty, &field_ty)?
                    }
                    _ => return Err(NoReading::Any),
                }
            };
            branches.push((tester(&name, case, term.clone())?, value));
        }
        let (_, last) = branches.pop().ok_or(NoReading::Any)?;
        let chained = branches
            .into_iter()
            .rev()
            .fold(last, |rest, (test, value)| {
                Term::call("ite", vec![test, value, rest])
            });
        Ok((chained, field_ty))
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
            Type::Variant { tag, cases } => Term::symbol("", &self.variant(tag, cases)?),
            Type::Json => Err(NoReading::Json),
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
            (Type::Json, _) => Err(NoReading::Json),
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
            (Type::Variant { tag, cases }, Value::Object(map)) => {
                let case = map.get(tag).and_then(Value::as_str).ok_or_else(mismatch)?;
                let case_ty = cases.get(case).ok_or_else(mismatch)?;
                let name = self.variant(tag, cases)?;
                let mut args = Vec::new();
                for (field, t) in case_fields(case_ty)? {
                    let part = if field.is_empty() {
                        v
                    } else {
                        map.get(&field).ok_or_else(mismatch)?
                    };
                    args.push(self.value(part, &t)?);
                }
                Ok(Term::app(case_constructor(&name, case)?, args))
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
        self.encode(expr, at, None)
    }

    /// `expr` encoded at `expected`, as the checker checks it: a record literal whose tag names a case is built with
    /// that case's constructor, a list literal has the expected element sort (so `[]` has one), and `let`, `if` and
    /// `match` pass `expected` on. Anything else is encoded as it is and coerced.
    pub fn expr_as(
        &mut self,
        expr: &Expr,
        expected: &Type,
        at: &str,
    ) -> Result<(Term, Type), ExportError> {
        self.encode(expr, at, Some(expected))
    }

    fn encode(
        &mut self,
        expr: &Expr,
        at: &str,
        expected: Option<&Type>,
    ) -> Result<(Term, Type), ExportError> {
        let Some(want) = expected.filter(|t| **t != Type::Any) else {
            return self.encode_inner(expr, at, None);
        };
        let here = at_body(at);
        match (expr, want) {
            (Expr::Record { fields }, Type::Variant { tag, cases }) => {
                let case = match fields.get(tag) {
                    Some(Expr::Lit {
                        v: Value::String(case),
                    }) => cases.get(case).map(|t| (case.clone(), t)),
                    _ => None,
                };
                if let Some((case, case_ty)) = case {
                    let name = self.variant(tag, cases).map_err(&here)?;
                    let constructor = case_constructor(&name, &case).map_err(&here)?;
                    let mut rest = fields.clone();
                    rest.remove(tag);
                    let args = match case_ty {
                        Type::Record(wanted) => {
                            let mut args = Vec::new();
                            for (field, field_ty) in wanted {
                                let e = rest.get(field).ok_or_else(|| {
                                    here(NoReading::UndeclaredField {
                                        field: field.clone(),
                                    })
                                })?;
                                let field_at = child(at, &format!("fields/{field}"));
                                args.push(self.encode(e, &field_at, Some(field_ty))?.0);
                            }
                            args
                        }
                        nested => {
                            let rest = Expr::Record { fields: rest };
                            vec![self.encode(&rest, at, Some(nested))?.0]
                        }
                    };
                    return Ok((Term::app(constructor, args), want.clone()));
                }
            }
            (Expr::Record { fields }, Type::Record(wanted)) => {
                let mut terms = Vec::new();
                let mut types = BTreeMap::new();
                for (field, e) in fields {
                    let field_at = child(at, &format!("fields/{field}"));
                    let (t, ty) = self.encode(e, &field_at, wanted.get(field))?;
                    terms.push(t);
                    types.insert(field.clone(), ty);
                }
                let constructor = self.constructor(&types).map_err(&here)?;
                let got = Type::Record(types);
                let term = self
                    .coerce(Term::app(constructor, terms), &got, want)
                    .map_err(here)?;
                return Ok((term, want.clone()));
            }
            (Expr::Op { op, args }, Type::List(item)) if op == "append" && args.len() == 2 => {
                let (list, _) = self.encode(&args[0], &child(at, "args/0"), Some(want))?;
                let (added, _) = self.encode(&args[1], &child(at, "args/1"), Some(item))?;
                let unit = Term::call("seq.unit", vec![added]);
                return Ok((Term::call("seq.++", vec![list, unit]), want.clone()));
            }
            (Expr::List { items }, Type::List(item)) => {
                let sort = self.sort(want).map_err(&here)?;
                let mut units = Vec::new();
                for (i, e) in items.iter().enumerate() {
                    let item_at = child(at, &format!("items/{i}"));
                    let (t, _) = self.encode(e, &item_at, Some(item))?;
                    units.push(Term::call("seq.unit", vec![t]));
                }
                let empty = Term::List(vec![Term::atom("as"), Term::atom("seq.empty"), sort]);
                return Ok((Term::fold("seq.++", empty, units), want.clone()));
            }
            _ => {}
        }
        let (t, ty) = self.encode_inner(expr, at, Some(want))?;
        Ok((self.coerce(t, &ty, want).map_err(here)?, want.clone()))
    }

    /// `expr` encoded by its form; `let`, `if` and `match` encode their bodies at `expected`.
    fn encode_inner(
        &mut self,
        expr: &Expr,
        at: &str,
        expected: Option<&Type>,
    ) -> Result<(Term, Type), ExportError> {
        let here = at_body(at);
        match expr {
            Expr::Is { .. } => Err(here(NoReading::Json)),
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
            Expr::Let { name, be, body, ty } => {
                let (bound, bound_ty) = self.encode(be, &child(at, "be"), ty.as_ref())?;
                let var = local(name).map_err(here)?;
                self.scope.push((name.clone(), bound_ty));
                let from = self.defined.len();
                let result = self.encode(body, &child(at, "in"), expected);
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
                let then = self.encode(then, &child(at, "then"), expected);
                self.guards.pop();
                self.guards.push(Term::call("not", vec![c.clone()]));
                let otherwise = self.encode(otherwise, &child(at, "else"), expected);
                self.guards.pop();
                let ((t, then_ty), (e, else_ty)) = (then?, otherwise?);
                let ty = if let Some(want) = expected {
                    want.clone()
                } else if conforms(&else_ty, &then_ty) {
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
                    Type::Variant { tag, cases } => {
                        self.variant_field(term, &tag, &cases, field).map_err(here)
                    }
                    _ => Err(here(NoReading::Any)),
                }
            }
            Expr::Match {
                of,
                on,
                cases,
                otherwise,
            } => {
                let (matched, ty) = self.expr(of, &child(at, "of"))?;
                let Type::Variant {
                    tag,
                    cases: declared,
                } = &ty
                else {
                    return Err(here(NoReading::Any));
                };
                if tag != on {
                    return Err(here(NoReading::Any));
                }
                let name = self.variant(tag, declared).map_err(&here)?;
                // The matched value is evaluated once, and every case reads it.
                let x = self.fresh();
                let from = self.defined.len();
                let mut branches = Vec::new();
                for (case, case_ty) in declared {
                    let test = tester(&name, case, x.clone()).map_err(&here)?;
                    self.guards.push(test.clone());
                    let encoded = match cases.get(case) {
                        Some(c) => {
                            let body_at = child(at, &format!("cases/{case}/body"));
                            let bound = match case_ty {
                                Type::Record(fields) => {
                                    let mut parts = Vec::new();
                                    for f in fields.keys() {
                                        let sel = case_selector(&name, case, f).map_err(&here)?;
                                        parts.push(Term::app(sel, vec![x.clone()]));
                                    }
                                    Term::app(self.constructor(fields).map_err(&here)?, parts)
                                }
                                _ => Term::app(
                                    case_selector(&name, case, "").map_err(&here)?,
                                    vec![x.clone()],
                                ),
                            };
                            let var = local(&c.bind).map_err(&here)?;
                            self.scope.push((c.bind.clone(), case_ty.clone()));
                            let inner = self.defined.len();
                            let body = self.encode(&c.body, &body_at, expected);
                            self.scope.pop();
                            body.map(|(t, bt)| {
                                for condition in &mut self.defined[inner..] {
                                    *condition =
                                        Term::bind(var.clone(), bound.clone(), condition.clone());
                                }
                                (Term::bind(var, bound, t), bt, body_at)
                            })
                        }
                        None => match otherwise {
                            Some(o) => {
                                let body_at = child(at, "otherwise");
                                self.encode(o, &body_at, expected)
                                    .map(|(t, bt)| (t, bt, body_at))
                            }
                            None => Err(here(NoReading::Any)),
                        },
                    };
                    self.guards.pop();
                    let (t, bt, body_at) = encoded?;
                    branches.push((test, t, bt, body_at));
                }
                let mut joined = branches
                    .first()
                    .map(|b| b.2.clone())
                    .ok_or_else(|| here(NoReading::Any))?;
                match expected {
                    Some(want) => joined = want.clone(),
                    None => {
                        for b in &branches[1..] {
                            joined = join(&joined, &b.2).ok_or_else(|| here(NoReading::Any))?;
                        }
                    }
                }
                let mut chained: Option<Term> = None;
                for (test, t, bt, body_at) in branches.into_iter().rev() {
                    let t = self.coerce(t, &bt, &joined).map_err(at_body(&body_at))?;
                    chained = Some(match chained {
                        None => t,
                        Some(rest) => Term::call("ite", vec![test, t, rest]),
                    });
                }
                for condition in &mut self.defined[from..] {
                    *condition = Term::bind(x.clone(), matched.clone(), condition.clone());
                }
                let body = chained.ok_or_else(|| here(NoReading::Any))?;
                Ok((Term::bind(x, matched, body), joined))
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
        for (name, datatype) in self.datatypes.clone() {
            let entry = match datatype {
                Datatype::Record(fields) => {
                    let mut selectors = Vec::new();
                    for (field, ty) in &fields {
                        selectors.push((
                            selector(&name, field).map_err(&at)?,
                            self.sort(ty).map_err(&at)?,
                        ));
                    }
                    Entry {
                        comments: vec![describe(&Type::Record(fields))],
                        command: Some(Command::DeclareDatatype {
                            name: Term::symbol("", &name).map_err(&at)?,
                            selectors,
                        }),
                    }
                }
                Datatype::Variant(cases) => {
                    let mut constructors = Vec::new();
                    for (case, case_ty) in &cases {
                        let mut selectors = Vec::new();
                        for (field, ty) in case_fields(case_ty).map_err(&at)? {
                            selectors.push((
                                case_selector(&name, case, &field).map_err(&at)?,
                                self.sort(&ty).map_err(&at)?,
                            ));
                        }
                        constructors.push((case_constructor(&name, case).map_err(&at)?, selectors));
                    }
                    let cases: Vec<&String> = cases.keys().collect();
                    Entry {
                        comments: vec![format!(
                            "A variant with the cases {}.",
                            cases
                                .iter()
                                .map(|c| c.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )],
                        command: Some(Command::DeclareVariant {
                            name: Term::symbol("", &name).map_err(&at)?,
                            constructors,
                        }),
                    }
                }
            };
            records.push(entry);
        }
        Ok(Section {
            heading: "Records and variants.",
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
        let (body, ty) = match &def.result {
            Some(result) => self.expr_as(&def.body, result, "")?,
            None => self.expr(&def.body, "")?,
        };
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
