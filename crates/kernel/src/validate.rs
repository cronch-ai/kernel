//! The Ask's contract, checked against the value its Answer evaluated to.
//!
//! Each check says how strongly it held: `pass` and `fail` are the kernel's own, replayable verdicts; `delegated` waits
//! on a Checker Actor's Evidence; `unchecked` is a prose criterion Cronch never claims to have checked. Validators are
//! scoped checks: a value that passes them is well-typed and in range, not thereby true.

use crate::ir::{AskSpec, Type, Validator};
use crate::ops::order;
use crate::types::inhabits;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::cmp::Ordering;

/// A Checker Actor's verdict on one validator, as Evidence the runtime received and authenticated.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Verdict {
    /// The index of the validator in the Ask's spec.
    pub validator: usize,
    /// The Checker that gave it. It counts only if it is the Checker the validator names.
    pub checker: String,
    pub verdict: bool,
}

/// What was checked: the output type, or one of the spec's validators.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "on", rename_all = "camelCase")]
pub enum Subject {
    Type { expected: Type },
    Validator { index: usize, validator: Validator },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Failure {
    NotInhabited,
    NoSuchField { at: Vec<String> },
    NotANumber { got: Value },
    NotSized { got: Value },
    BelowMin { min: f64, got: f64 },
    AboveMax { max: f64, got: f64 },
    Empty,
    NotOneOf { got: Value },
    TooShort { min: u64, got: u64 },
    TooLong { max: u64, got: u64 },
    Rejected { checker: String },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "result", rename_all = "camelCase")]
pub enum Outcome {
    Pass,
    /// Passed on a Checker's word: not replayable by the kernel.
    Attested {
        checker: String,
    },
    Fail {
        failure: Failure,
    },
    Delegated {
        checker: String,
    },
    Unchecked,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Check {
    pub subject: Subject,
    pub outcome: Outcome,
}

fn at<'v>(value: &'v Value, path: &[String]) -> Result<&'v Value, Failure> {
    path.iter()
        .try_fold(value, |v, field| v.get(field))
        .ok_or_else(|| Failure::NoSuchField { at: path.to_vec() })
}

/// The number at `v` against a bound, by exact values: an integer is never rounded to a float to be compared.
fn against(v: &Value, bound: f64) -> Result<(f64, Ordering), Failure> {
    let not_a_number = || Failure::NotANumber { got: v.clone() };
    let order = Number::from_f64(bound)
        .and_then(|b| order(v, &Value::Number(b)))
        .ok_or_else(not_a_number)?;
    Ok((v.as_f64().ok_or_else(not_a_number)?, order))
}

fn size(v: &Value) -> Result<u64, Failure> {
    match v {
        Value::String(s) => Ok(s.chars().count() as u64),
        Value::Array(items) => Ok(items.len() as u64),
        Value::Object(map) => Ok(map.len() as u64),
        _ => Err(Failure::NotSized { got: v.clone() }),
    }
}

fn holds(test: bool, failure: impl FnOnce() -> Failure) -> Result<(), Failure> {
    if test { Ok(()) } else { Err(failure()) }
}

fn kernel_check(validator: &Validator, value: &Value) -> Result<(), Failure> {
    match validator {
        Validator::Min {
            value: min,
            at: path,
        } => {
            let (got, order) = against(at(value, path)?, *min)?;
            holds(order.is_ge(), || Failure::BelowMin { min: *min, got })
        }
        Validator::Max {
            value: max,
            at: path,
        } => {
            let (got, order) = against(at(value, path)?, *max)?;
            holds(order.is_le(), || Failure::AboveMax { max: *max, got })
        }
        Validator::NonEmpty { at: path } => holds(size(at(value, path)?)? > 0, || Failure::Empty),
        Validator::OneOf { values, at: path } => {
            let got = at(value, path)?;
            holds(values.contains(got), || Failure::NotOneOf {
                got: got.clone(),
            })
        }
        Validator::MinLength {
            value: min,
            at: path,
        } => {
            let got = size(at(value, path)?)?;
            holds(got >= *min, || Failure::TooShort { min: *min, got })
        }
        Validator::MaxLength {
            value: max,
            at: path,
        } => {
            let got = size(at(value, path)?)?;
            holds(got <= *max, || Failure::TooLong { max: *max, got })
        }
        Validator::Checker { .. } | Validator::Prose { .. } => Ok(()),
    }
}

fn outcome(index: usize, validator: &Validator, value: &Value, evidence: &[Verdict]) -> Outcome {
    match validator {
        Validator::Prose { .. } => Outcome::Unchecked,
        Validator::Checker { actor, .. } => {
            match evidence
                .iter()
                .find(|v| v.validator == index && &v.checker == actor)
            {
                None => Outcome::Delegated {
                    checker: actor.clone(),
                },
                Some(v) if v.verdict => Outcome::Attested {
                    checker: actor.clone(),
                },
                Some(_) => Outcome::Fail {
                    failure: Failure::Rejected {
                        checker: actor.clone(),
                    },
                },
            }
        }
        _ => match kernel_check(validator, value) {
            Ok(()) => Outcome::Pass,
            Err(failure) => Outcome::Fail { failure },
        },
    }
}

/// Check the value against the spec's output type and every validator, using whatever Evidence applies.
pub fn validate(spec: &AskSpec, value: &Value, evidence: &[Verdict]) -> Vec<Check> {
    let typed = inhabits(value, &spec.output);
    let mut checks = vec![Check {
        subject: Subject::Type {
            expected: spec.output.clone(),
        },
        outcome: if typed {
            Outcome::Pass
        } else {
            Outcome::Fail {
                failure: Failure::NotInhabited,
            }
        },
    }];
    checks.extend(
        spec.validators
            .iter()
            .enumerate()
            .map(|(index, validator)| Check {
                subject: Subject::Validator {
                    index,
                    validator: validator.clone(),
                },
                outcome: outcome(index, validator, value, evidence),
            }),
    );
    checks
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(validators: Value) -> AskSpec {
        serde_json::from_value(json!({
            "name": "GDP",
            "output": { "Record": { "value": "Float", "year": "Int" } },
            "validators": validators
        }))
        .unwrap()
    }

    #[test]
    fn kernel_validators_pass_and_fail() {
        let s = spec(
            json!([{ "kind": "min", "value": 0, "at": ["value"] }, { "kind": "min", "value": 2020, "at": ["year"] }]),
        );
        let checks = validate(&s, &json!({ "value": 2100.5, "year": 2019 }), &[]);
        assert_eq!(checks[0].outcome, Outcome::Pass);
        assert_eq!(checks[1].outcome, Outcome::Pass);
        assert_eq!(
            checks[2].outcome,
            Outcome::Fail {
                failure: Failure::BelowMin {
                    min: 2020.0,
                    got: 2019.0
                }
            }
        );
    }

    #[test]
    fn prose_is_never_passed() {
        let s = spec(json!([{ "kind": "prose", "text": "compatible GDP/population basis" }]));
        assert_eq!(
            validate(&s, &json!({ "value": 1.0, "year": 2023 }), &[])[1].outcome,
            Outcome::Unchecked
        );
    }

    #[test]
    fn evidence_counts_only_from_the_named_checker() {
        let s = spec(json!([{ "kind": "checker", "actor": "auditor" }]));
        let v = json!({ "value": 1.0, "year": 2023 });
        let impostor = [Verdict {
            validator: 0,
            checker: "someone".into(),
            verdict: true,
        }];
        assert_eq!(
            validate(&s, &v, &impostor)[1].outcome,
            Outcome::Delegated {
                checker: "auditor".into()
            }
        );
        let real = [Verdict {
            validator: 0,
            checker: "auditor".into(),
            verdict: true,
        }];
        assert_eq!(
            validate(&s, &v, &real)[1].outcome,
            Outcome::Attested {
                checker: "auditor".into()
            }
        );
    }
}
