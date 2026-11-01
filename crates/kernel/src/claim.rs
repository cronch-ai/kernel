//! Claims: `∀ vars. holds`, stated about the Answers to one spec, and the kernel's re-check of a proposed counterexample.
//!
//! A claim is a property someone wants of every Answer to a spec; an assumption is one the claimant is committed to.
//! The kernel tells them apart by stance, not mechanism: both are checked by looking for values of `vars` that make
//! `holds` false. Inside `holds`, an `ask` of the spec the claim is `about` means the Answer under check, applied to
//! those arguments. No other Ask may appear, and nothing epistemic: a claim is a derivation over its variables.
//!
//! Finding counterexamples is not the kernel's: a solver proposes one (`smtlib::claim`), or anyone posts one. The
//! kernel only re-checks it, by evaluating the Answer and the claim under it. A nested Ask inside the Answer stays
//! uninterpreted, as in the SMT-LIB export: a counterexample may give it any value of its output type, so a claim that
//! survives holds whatever those Asks are answered with. A solver's `unsat` comes with no certificate, so it is never a
//! verdict here.

use crate::check::{Assumption, Path, Problem, check, check_ask};
use crate::eval::{EvalError, Evaluation, Open, evaluate};
use crate::ir::{Answer, Ask, AskSpec, Expr, Form, Param, Type, World};
use crate::key::{ask_key, spec_key};
use crate::types::{inhabits, of_value};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Whether a claim is wanted of the Answers, or something their author is committed to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Stance {
    #[default]
    Claim,
    Assumption,
}

impl Stance {
    fn is_claim(&self) -> bool {
        *self == Stance::Claim
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Claim {
    pub name: String,
    #[serde(default, skip_serializing_if = "Stance::is_claim")]
    pub stance: Stance,
    /// The spec whose Answers the claim is about.
    pub about: AskSpec,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vars: Vec<Param>,
    /// A Bool over `vars`, true for every value of them if the claim holds.
    pub holds: Expr,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A value for one of the Answer's nested Asks, closed at the arguments its evaluation reached it with.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HoleValue {
    pub ask: Ask,
    pub value: Value,
}

/// Values for a claim's variables, and for the nested Asks of the Answer its evaluation reaches.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Witness {
    #[serde(default)]
    pub vars: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<HoleValue>,
}

impl Witness {
    /// The hole values by content key, as evaluation looks them up.
    pub fn resolved(&self) -> BTreeMap<String, Value> {
        self.holes
            .iter()
            .map(|h| (ask_key(&h.ask, &BTreeMap::new()), h.value.clone()))
            .collect()
    }
}

/// What makes a claim ill-formed, beyond the problems of its body as a Procedure.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ClaimProblem {
    /// `holds`, read as a Procedure over `vars` with a Bool result, doesn't check.
    Body { problem: Problem },
    /// `holds` asks something other than the spec the claim is about.
    ForeignAsk { at: Path, ask: String },
}

/// Why a counterexample can't be read against the Answer at all.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Invalid {
    IllFormedClaim {
        problems: Vec<ClaimProblem>,
    },
    /// The Answer doesn't check against the spec the claim is about.
    IllFormedAnswer {
        problems: Vec<Problem>,
    },
    UnexpectedVar {
        name: String,
    },
    VarMismatch {
        name: String,
        expected: Type,
        got: Value,
    },
    /// A hole value isn't for a closed Ask, or doesn't inhabit the Ask's output type.
    HoleMismatch {
        ask: String,
        problems: Vec<Problem>,
    },
    /// The Answer has an input with no value, so it can't be evaluated anywhere.
    Opaque {
        at: Path,
    },
    /// Evaluation went deeper than the kernel follows.
    TooDeep,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Verdict {
    /// The claim is false at the witness: it doesn't hold of this Answer.
    Refutes,
    /// The assumption is false at the witness: the Answer contradicts it.
    Inconsistent,
    /// `holds` is true at the witness: a proposed counterexample that isn't one.
    NotRefuted,
    /// The Answer, or the claim, has no value at the witness (division by zero, overflow): it neither holds nor fails.
    Undefined {
        error: EvalError,
    },
    Invalid {
        reason: Invalid,
    },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Assessment {
    Assessed {
        verdict: Verdict,
    },
    /// The witness lacks values the evaluation needs: these variables, and these nested Asks of the Answer.
    Incomplete {
        vars: Vec<Param>,
        holes: Vec<Open>,
    },
}

/// How many times the kernel goes back to the Answer for an Ask of the claim before giving up.
const MAX_ROUNDS: usize = 256;

pub fn claim_key(claim: &Claim) -> String {
    crate::key::digest(&json!({ "kind": "cronch.claim/1", "claim": claim }))
}

/// `holds` as a Procedure: the Ask it answers has the claim's variables as parameters and a Bool output.
pub fn as_ask(claim: &Claim, vars: &BTreeMap<String, Value>) -> Ask {
    Ask {
        spec: AskSpec {
            name: claim.name.clone(),
            params: claim.vars.clone(),
            output: Type::Bool,
            description: String::new(),
            validators: Vec::new(),
            form: Form::Derivation,
            effect: false,
        },
        args: vars.clone(),
    }
}

fn foreign_asks(expr: &Expr, about: &str, at: &str, out: &mut Vec<ClaimProblem>) {
    let child = |segment: &str| format!("{at}/{segment}");
    match expr {
        Expr::Lit { .. }
        | Expr::Var { .. }
        | Expr::Fact { .. }
        | Expr::Assume { .. }
        | Expr::Opaque { .. } => {}
        Expr::Op { args, .. } => args
            .iter()
            .enumerate()
            .for_each(|(i, a)| foreign_asks(a, about, &child(&format!("args/{i}")), out)),
        Expr::Ask { spec, args } => {
            if spec_key(spec) != about {
                out.push(ClaimProblem::ForeignAsk {
                    at: at.to_string(),
                    ask: spec.name.clone(),
                });
            }
            args.iter()
                .for_each(|(n, a)| foreign_asks(a, about, &child(&format!("args/{n}")), out))
        }
        Expr::Let { be, body, .. } => {
            foreign_asks(be, about, &child("be"), out);
            foreign_asks(body, about, &child("in"), out);
        }
        Expr::If {
            cond,
            then,
            otherwise,
        } => {
            foreign_asks(cond, about, &child("cond"), out);
            foreign_asks(then, about, &child("then"), out);
            foreign_asks(otherwise, about, &child("else"), out);
        }
        Expr::Record { fields } => fields
            .iter()
            .for_each(|(n, e)| foreign_asks(e, about, &child(&format!("fields/{n}")), out)),
        Expr::List { items } => items
            .iter()
            .enumerate()
            .for_each(|(i, e)| foreign_asks(e, about, &child(&format!("items/{i}")), out)),
        Expr::Get { of, .. } => foreign_asks(of, about, &child("of"), out),
    }
}

/// Problems with a claim. `holds` is checked as a derivation in an empty world, so it has no facts or assumptions.
pub fn check_claim(claim: &Claim) -> Vec<ClaimProblem> {
    let ask = as_ask(claim, &BTreeMap::new());
    let mut problems: Vec<ClaimProblem> = crate::check::check_spec(&ask.spec)
        .into_iter()
        .chain(
            check(
                &ask,
                &World::default(),
                &Answer::Procedure(claim.holds.clone()),
            )
            .problems,
        )
        .map(|problem| ClaimProblem::Body { problem })
        .collect();
    problems.extend(
        crate::check::check_spec(&claim.about)
            .into_iter()
            .map(|problem| ClaimProblem::Body { problem }),
    );
    foreign_asks(&claim.holds, &spec_key(&claim.about), "", &mut problems);
    problems
}

/// Problems with an Answer as the subject of a claim: it must check against the spec the claim is about.
pub fn check_subject(claim: &Claim, answer: &Answer) -> Result<(), Invalid> {
    let about = Ask {
        spec: claim.about.clone(),
        args: BTreeMap::new(),
    };
    let construction = check(&about, &World::default(), answer);
    if !construction.problems.is_empty() {
        return Err(Invalid::IllFormedAnswer {
            problems: construction.problems,
        });
    }
    match construction.assumptions.iter().find_map(|a| match a {
        Assumption::Opaque { at, .. } => Some(at.clone()),
        _ => None,
    }) {
        Some(at) => Err(Invalid::Opaque { at }),
        None => Ok(()),
    }
}

fn verdict(claim: &Claim, value: &Value) -> Verdict {
    match (value, claim.stance) {
        (Value::Bool(true), _) => Verdict::NotRefuted,
        (_, Stance::Claim) => Verdict::Refutes,
        (_, Stance::Assumption) => Verdict::Inconsistent,
    }
}

/// Re-check a counterexample: evaluate the claim at the witness, going to the Answer for each Ask of the claim's spec.
pub fn assess_claim(claim: &Claim, answer: &Answer, witness: &Witness) -> Assessment {
    let invalid = |reason| Assessment::Assessed {
        verdict: Verdict::Invalid { reason },
    };
    let problems = check_claim(claim);
    if !problems.is_empty() {
        return invalid(Invalid::IllFormedClaim { problems });
    }
    if let Err(reason) = check_subject(claim, answer) {
        return invalid(reason);
    }
    if let Some(name) = witness
        .vars
        .keys()
        .find(|n| !claim.vars.iter().any(|p| &p.name == *n))
    {
        return invalid(Invalid::UnexpectedVar { name: name.clone() });
    }
    let mut missing = Vec::new();
    for p in &claim.vars {
        match witness.vars.get(&p.name) {
            None => missing.push(p.clone()),
            Some(v) if !inhabits(v, &p.ty) => {
                return invalid(Invalid::VarMismatch {
                    name: p.name.clone(),
                    expected: p.ty.clone(),
                    got: v.clone(),
                });
            }
            Some(_) => {}
        }
    }
    if !missing.is_empty() {
        return Assessment::Incomplete {
            vars: missing,
            holes: Vec::new(),
        };
    }
    for hole in &witness.holes {
        let mut problems = check_ask(&hole.ask);
        if !inhabits(&hole.value, &hole.ask.spec.output) {
            problems.push(Problem::Mismatch {
                at: String::new(),
                expected: hole.ask.spec.output.clone(),
                got: of_value(&hole.value),
            });
        }
        if !problems.is_empty() {
            return invalid(Invalid::HoleMismatch {
                ask: hole.ask.spec.name.clone(),
                problems,
            });
        }
    }
    let resolved = witness.resolved();

    let ask = as_ask(claim, &witness.vars);
    let body = Answer::Procedure(claim.holds.clone());
    let world = World::default();
    let mut applied: BTreeMap<String, Value> = BTreeMap::new();
    for _ in 0..MAX_ROUNDS {
        let open = match evaluate(&ask, &world, &body, &applied) {
            Evaluation::Done { value } => {
                return Assessment::Assessed {
                    verdict: verdict(claim, &value),
                };
            }
            Evaluation::Failed { error } => {
                return Assessment::Assessed {
                    verdict: Verdict::Undefined { error },
                };
            }
            Evaluation::Blocked { open } => open,
        };
        let mut needs = Vec::new();
        for o in open {
            // A claim is a derivation, so it has no opaque inputs of its own: every hole is an Ask of its spec.
            let Open::Ask {
                key,
                ask: applied_to,
                ..
            } = o
            else {
                return invalid(Invalid::IllFormedClaim {
                    problems: Vec::new(),
                });
            };
            match evaluate(&applied_to, &world, answer, &resolved) {
                Evaluation::Done { value } => {
                    applied.insert(key, value);
                }
                Evaluation::Failed { error } => {
                    return Assessment::Assessed {
                        verdict: Verdict::Undefined { error },
                    };
                }
                Evaluation::Blocked { open } => {
                    for hole in open {
                        match hole {
                            Open::Opaque { at, .. } => return invalid(Invalid::Opaque { at }),
                            Open::Ask { key, .. }
                                if needs.iter().any(
                                    |n| matches!(n, Open::Ask { key: k, .. } if *k == key),
                                ) => {}
                            hole => needs.push(hole),
                        }
                    }
                }
            }
        }
        if !needs.is_empty() {
            return Assessment::Incomplete {
                vars: Vec::new(),
                holes: needs,
            };
        }
    }
    invalid(Invalid::TooDeep)
}
