//! Rules: what a capability's contracts alone rule out, whatever a harness does and whatever a model says.
//!
//! The interpreter checks an operation's precondition at the point of every call, so a call happens only where the
//! precondition holds of the grant state. A rule says that about one operation: it can't happen until a person has
//! approved it. That follows from contracts, never from a harness's step, so it holds of every harness on the
//! interface. With a new capability, only what that capability can change has anything new to show.
//!
//! The rule holds if, for every value of the call's arguments, the invariant "the precondition holds only if an
//! approval has been given" holds of the initial grant and survives every update. Each of those is an obligation
//! here, a query whose `unsat` is the claim:
//!
//! * `initial`: the precondition holds of no arguments under the initial grant.
//! * one per update (`onCall`, `onResult`, `onMsg`) that writes a field the precondition reads: an update that keeps
//!   the invariant. Only a result of an operation in `approvedBy` for which `when` holds counts as approval.
//!
//! An update that writes none of those fields is listed as `framed`: since the interpreter merges an update's fields
//! into the grant, it leaves the precondition as it found it. Every update is read branch by branch: the type a
//! checker infers for `if` drops fields only one branch gives, and those fields are written all the same. An update
//! too complex to read that way is refused rather than assumed harmless.
//!
//! Each query holds only the part of the grant it reads or writes, so a capability that touches none of that part
//! leaves the query's text as it was, and a proof of it still checks: nothing already shown is shown again.
//!
//! A solver's `unsat` is not a verdict, here as everywhere in the kernel. The queries are written for a proof to be
//! checked against them by someone else, with no part of Cronch (`proofs/` in this repository is one way). What such a
//! proof rests on: the interpreter complies with these semantics, each contract states what its capability does, the
//! rule's `when` says what approval means, the rule names every operation that reaches what it protects, and these
//! queries say what this comment says.

use crate::harness::{HarnessProblem, Interface, check_interface};
use crate::induction::each_use;
use crate::ir::{Case, Expr, Param, Type};
use crate::smtlib::{self, Floats, Refusal};
use crate::types::inhabits;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// `op` can't happen until an approval has been given.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub op: String,
    #[serde(default)]
    pub approved_by: Vec<Approval>,
}

/// A result of `op` that approves a call: `when` over `args` (the call's), `approval` (the arguments of this
/// operation) and `result`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Approval {
    pub op: String,
    pub when: Expr,
}

/// A query whose `unsat` is part of a rule.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Obligation {
    pub name: String,
    pub text: String,
}

/// What a rule needs shown, and the updates it needs nothing from.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Obligations {
    pub obligations: Vec<Obligation>,
    pub framed: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuleError {
    /// The interface's contracts don't check.
    Interface { problems: Vec<HarnessProblem> },
    /// The rule or an approval names an operation the interface doesn't have.
    NoSuchOperation { name: String },
    /// A rule reads a grant state as a record of fields.
    GrantNotARecord,
    /// The initial grant doesn't have the interface's grant type.
    Grant,
    /// This update isn't made of records, branches and bindings, so what it writes can't be read off it.
    Unreadable { update: String },
    /// A query would hold `Json`, whose reading a proof checker can't follow. A harness can run with it; a rule over
    /// it can't be checked, so the rule says so rather than writing a query nothing can check. Type the field.
    HoldsJson { place: String, name: String },
    /// A part has no faithful reading in SMT-LIB.
    Refused { place: String, refusal: Refusal },
}

const CALL: &str = "rule:call";
const APPROVED: &str = "rule:approved";
const GRANT: &str = "rule:grant";
const UPDATE_ARGS: &str = "rule:args";
const RESULT: &str = "rule:result";

fn var(name: &str) -> Expr {
    Expr::Var {
        name: name.to_string(),
    }
}

fn op(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Op {
        op: name.to_string(),
        args,
    }
}

fn binding(name: &str, be: Expr, body: Expr) -> Expr {
    Expr::Let {
        name: name.to_string(),
        be: Box::new(be),
        body: Box::new(body),
        ty: None,
    }
}

fn param(name: &str, ty: &Type) -> Param {
    Param {
        name: name.to_string(),
        ty: ty.clone(),
    }
}

/// Whether `e` is `grant.field`.
fn is_grant_field(e: &Expr, field: &str) -> bool {
    matches!(e, Expr::Get { of, field: f }
        if f == field && matches!(&**of, Expr::Var { name } if name == "grant"))
}

/// The grant's fields `pre` reads.
fn reads(pre: &Expr, declared: &BTreeMap<String, Type>) -> BTreeSet<String> {
    let mut fields = BTreeSet::new();
    let mut whole = false;
    each_use(pre, "grant", &mut |e| match e {
        Expr::Get { of, field } if matches!(&**of, Expr::Var { name } if name == "grant") => {
            fields.insert(field.clone());
            false
        }
        Expr::Var { name } if name == "grant" => {
            whole = true;
            true
        }
        _ => true,
    });
    if whole {
        declared.keys().cloned().collect()
    } else {
        fields
    }
}

/// The whole grant state after `update`, branch by branch, with the fields it writes added to `written`. A field set
/// to itself isn't written. `None` if it can't be read this way.
fn merged(
    update: &Expr,
    declared: &BTreeMap<String, Type>,
    written: &mut BTreeSet<String>,
) -> Option<Expr> {
    match update {
        Expr::Var { name } if name == "grant" => Some(var("grant")),
        Expr::Record { fields } => {
            let mut whole = BTreeMap::new();
            for name in declared.keys() {
                let value = match fields.get(name) {
                    Some(e) => {
                        if !is_grant_field(e, name) {
                            written.insert(name.clone());
                        }
                        e.clone()
                    }
                    None => Expr::Get {
                        of: Box::new(var("grant")),
                        field: name.clone(),
                    },
                };
                whole.insert(name.clone(), value);
            }
            // The interpreter merges every field an update gives: one the grant doesn't declare isn't this shape.
            fields
                .keys()
                .all(|f| declared.contains_key(f))
                .then_some(Expr::Record { fields: whole })
        }
        Expr::If {
            cond,
            then,
            otherwise,
        } => Some(Expr::If {
            cond: cond.clone(),
            then: Box::new(merged(then, declared, written)?),
            otherwise: Box::new(merged(otherwise, declared, written)?),
        }),
        Expr::Let { name, be, body, ty } if name != "grant" => Some(Expr::Let {
            name: name.clone(),
            be: be.clone(),
            body: Box::new(merged(body, declared, written)?),
            ty: ty.clone(),
        }),
        Expr::Match {
            of,
            on,
            cases,
            otherwise,
        } => {
            let mut out = BTreeMap::new();
            for (name, case) in cases {
                if case.bind == "grant" {
                    return None;
                }
                out.insert(
                    name.clone(),
                    Case {
                        bind: case.bind.clone(),
                        body: merged(&case.body, declared, written)?,
                    },
                );
            }
            let otherwise = match otherwise {
                Some(o) => Some(Box::new(merged(o, declared, written)?)),
                None => None,
            };
            Some(Expr::Match {
                of: of.clone(),
                on: on.clone(),
                cases: out,
                otherwise,
            })
        }
        _ => None,
    }
}

/// The grant's fields `update` reads, reading it as `merged` does: returning the grant whole, in a branch that changes
/// nothing, reads no field of it.
fn update_reads(update: &Expr, declared: &BTreeMap<String, Type>, read: &mut BTreeSet<String>) {
    match update {
        Expr::Var { name } if name == "grant" => {}
        Expr::Record { fields } => {
            for e in fields.values() {
                read.extend(reads(e, declared));
            }
        }
        Expr::If {
            cond,
            then,
            otherwise,
        } => {
            read.extend(reads(cond, declared));
            update_reads(then, declared, read);
            update_reads(otherwise, declared, read);
        }
        Expr::Let { be, body, .. } => {
            read.extend(reads(be, declared));
            update_reads(body, declared, read);
        }
        Expr::Match {
            of,
            cases,
            otherwise,
            ..
        } => {
            read.extend(reads(of, declared));
            for case in cases.values() {
                update_reads(&case.body, declared, read);
            }
            if let Some(o) = otherwise {
                update_reads(o, declared, read);
            }
        }
        // `merged` refuses every other shape before this is asked.
        _ => read.extend(declared.keys().cloned()),
    }
}

/// The grant's type and value cut down to `fields`.
fn part(declared: &BTreeMap<String, Type>, fields: &BTreeSet<String>) -> BTreeMap<String, Type> {
    declared
        .iter()
        .filter(|(name, _)| fields.contains(*name))
        .map(|(name, ty)| (name.clone(), ty.clone()))
        .collect()
}

/// Whether `ty` holds `Json` at any depth.
fn holds_json(ty: &Type) -> bool {
    match ty {
        Type::Json => true,
        Type::List(t) => holds_json(t),
        Type::Record(fields) => fields.values().any(holds_json),
        Type::Variant { cases, .. } => cases.values().any(holds_json),
        _ => false,
    }
}

/// `Err` naming the first of these parameters that holds `Json`.
fn checkable(place: &str, params: &[Param]) -> Result<(), RuleError> {
    match params.iter().find(|p| holds_json(&p.ty)) {
        Some(p) => Err(RuleError::HoldsJson {
            place: place.to_string(),
            name: p.name.clone(),
        }),
        None => Ok(()),
    }
}

/// One update of the grant state.
struct Update<'a> {
    name: String,
    expr: &'a Expr,
    /// What its contract reads besides the grant, each bound to a parameter of the query.
    vars: Vec<(&'static str, &'static str, &'a Type)>,
    /// What the interpreter has established by the time it applies: over the same variables and `grant`.
    given: Option<&'a Expr>,
    /// What counts as an approval when this update happens.
    approves: Vec<&'a Expr>,
}

fn updates<'a>(interface: &'a Interface, rule: &'a Rule) -> Vec<Update<'a>> {
    let mut all = Vec::new();
    for (name, o) in &interface.operations {
        if let Some(expr) = &o.on_call {
            all.push(Update {
                name: format!("{name}.onCall"),
                expr,
                vars: vec![("args", UPDATE_ARGS, &o.input)],
                given: o.pre.as_ref(),
                approves: vec![],
            });
        }
        if let Some(expr) = &o.on_result {
            all.push(Update {
                name: format!("{name}.onResult"),
                expr,
                vars: vec![
                    ("args", UPDATE_ARGS, &o.input),
                    ("result", RESULT, &o.output),
                ],
                given: o.post.as_ref(),
                approves: rule
                    .approved_by
                    .iter()
                    .filter(|a| a.op == *name)
                    .map(|a| &a.when)
                    .collect(),
            });
        }
    }
    for (name, c) in &interface.channels {
        if let Some(expr) = &c.on_msg {
            all.push(Update {
                name: format!("{name}.onMsg"),
                expr,
                vars: vec![("value", UPDATE_ARGS, &c.ty)],
                given: c.inv.as_ref(),
                approves: vec![],
            });
        }
    }
    all
}

/// Binds an update's contract variables to the query's parameters, and `grant` to the grant before it.
fn in_update(update: &Update, body: Expr) -> Expr {
    let with_grant = binding("grant", var(GRANT), body);
    update
        .vars
        .iter()
        .fold(with_grant, |e, (name, parameter, _)| {
            binding(name, var(parameter), e)
        })
}

/// The invariant, of a grant given by `grant`: the precondition holds of the call only if `approved`.
fn invariant(pre: &Expr, grant: Expr, approved: Expr) -> Expr {
    op(
        "or",
        vec![
            op(
                "not",
                vec![binding(
                    "args",
                    var(CALL),
                    binding("grant", grant, pre.clone()),
                )],
            ),
            approved,
        ],
    )
}

/// The obligations of `rule` over `interface`, for runs that start from `grant`.
pub fn obligations(
    interface: &Interface,
    rule: &Rule,
    grant: &Value,
    floats: Floats,
) -> Result<Obligations, RuleError> {
    let problems = check_interface(interface);
    if !problems.is_empty() {
        return Err(RuleError::Interface { problems });
    }
    let missing = |name: &str| RuleError::NoSuchOperation {
        name: name.to_string(),
    };
    let guarded = interface
        .operations
        .get(&rule.op)
        .ok_or_else(|| missing(&rule.op))?;
    if let Some(a) = rule
        .approved_by
        .iter()
        .find(|a| !interface.operations.contains_key(&a.op))
    {
        return Err(missing(&a.op));
    }
    let Type::Record(declared) = &interface.grant else {
        return Err(RuleError::GrantNotARecord);
    };
    if !inhabits(grant, &interface.grant) {
        return Err(RuleError::Grant);
    }
    // Absent, a precondition is `true`, and `initial` gives the counterexample.
    let pre = guarded.pre.clone().unwrap_or(Expr::Lit {
        v: Value::Bool(true),
    });
    let header = |name: &str| {
        format!(
            "Rule: {} needs an approval. Obligation: {name}.\nunsat is what the rule needs; sat gives a counterexample. Unsat is not the kernel's verdict: check a proof against this query.",
            rule.op
        )
    };
    let refused = |place: &str| {
        let place = place.to_string();
        move |refusal| RuleError::Refused { place, refusal }
    };

    let read = reads(&pre, declared);
    let initial_part = part(declared, &read);
    let initial_grant = Value::Object(
        grant
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(name, _)| read.contains(*name))
            .map(|(name, v)| (name.clone(), v.clone()))
            .collect(),
    );
    let initial_params = [
        param("args", &guarded.input),
        param("grant", &Type::Record(initial_part)),
    ];
    checkable("initial", &initial_params)?;
    let initial = smtlib::refutation(
        &header("initial"),
        &initial_params,
        &[("grant", &initial_grant)],
        &pre,
        floats,
    )
    .map_err(refused("initial"))?;
    let mut out = Obligations {
        obligations: vec![Obligation {
            name: "initial".to_string(),
            text: initial,
        }],
        framed: Vec::new(),
    };

    for update in updates(interface, rule) {
        let unreadable = || RuleError::Unreadable {
            update: update.name.clone(),
        };
        let mut written = BTreeSet::new();
        merged(update.expr, declared, &mut written).ok_or_else(unreadable)?;
        if written.is_disjoint(&read) {
            out.framed.push(update.name);
            continue;
        }
        // What this query needs of the grant: all the precondition, the update and what is given read, and all the
        // update writes. A field outside that is written by nothing here and read by nothing here.
        let mut needed = read.clone();
        needed.extend(written);
        update_reads(update.expr, declared, &mut needed);
        if let Some(given) = update.given {
            needed.extend(reads(given, declared));
        }
        let needed = part(declared, &needed);
        let after = merged(update.expr, &needed, &mut BTreeSet::new()).ok_or_else(unreadable)?;
        let mut params = vec![
            param(CALL, &guarded.input),
            param(APPROVED, &Type::Bool),
            param(GRANT, &Type::Record(needed)),
        ];
        params.extend(update.vars.iter().map(|(_, p, ty)| param(p, ty)));
        checkable(&update.name, &params)?;
        // The approval reads the call as `args`, and this update's arguments as `approval`.
        let approved = update.approves.iter().fold(var(APPROVED), |acc, when| {
            let reading = update.vars.iter().find(|(n, ..)| *n == "result");
            let mut e = binding("args", var(CALL), (*when).clone());
            e = binding("approval", var(UPDATE_ARGS), e);
            if let Some((name, parameter, _)) = reading {
                e = binding(name, var(parameter), e);
            }
            op("or", vec![acc, e])
        });
        let mut holds = vec![invariant(&pre, var(GRANT), var(APPROVED))];
        if let Some(given) = update.given {
            holds.push(in_update(&update, given.clone()));
        }
        holds.push(op(
            "not",
            vec![invariant(&pre, in_update(&update, after), approved)],
        ));
        let formula = holds
            .into_iter()
            .reduce(|a, b| op("and", vec![a, b]))
            .expect("at least the invariant");
        let text = smtlib::refutation(&header(&update.name), &params, &[], &formula, floats)
            .map_err(refused(&update.name))?;
        out.obligations.push(Obligation {
            name: update.name,
            text,
        });
    }
    Ok(out)
}
