//! Evidence: the kernel's verdict on a property of a harness, with what it rests on.
//!
//! A statement is about a subject: a harness, the interface it runs against, the grant state its root starts with,
//! and a property. Evidence names each by key, says how the verdict was reached, and lists the assumptions it rests
//! on. The kernel reaches every verdict itself, deterministically, so anyone can re-check evidence offline by asking
//! the kernel again; a solver's bare answer is never a verdict.
//!
//! The methods:
//!
//! * `counterexample`: a sequence of deliveries. The kernel replays it, refusing it unless every delivery was one the
//!   environment could admissibly make, and finds where the property fails. A refutation needs no trust in whoever
//!   found it.
//! * `bounded`: every run of up to `depth` deliveries, the environment choosing from an alphabet of inputs. No
//!   violation: the property holds up to that depth, for that alphabet.
//! * `exhaustive`: every configuration reachable under the alphabet, explored until no new one appears (or `limit`
//!   is reached, which is inconclusive). No violation: the property holds at every depth, for that alphabet. This
//!   needs the incremental monitor, so the property must be quantifier-free.
//!
//! The alphabet is an assumption about the environment, stated in the evidence: a proof over it says nothing about
//! inputs outside it. Proofs over all inputs, by inductive invariants discharged with certificates, are the next tier.

use crate::harness::{Harness, Interface};
use crate::key::{Kind, digest, key};
use crate::labeled::Label;
use crate::property::{
    MonitorState, Property, PropertyProblem, advance, check_property, first_violation,
    quantifier_free,
};
use crate::run::{Delivery, Event, InitError, Run, System, Withheld, WorldInput};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

/// What a statement is about.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Subject {
    pub harness: Harness,
    #[serde(default)]
    pub interface: Interface,
    #[serde(default)]
    pub grant: Value,
    pub property: Property,
}

/// An input the environment may choose, wherever it would be admissible.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Template {
    Start {
        value: Value,
        #[serde(default)]
        label: Label,
    },
    /// A result for any pending call (of `op`, if given).
    Ret {
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op: Option<String>,
    },
    End {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op: Option<String>,
    },
    Failed {
        error: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op: Option<String>,
    },
    Msg {
        channel: String,
        value: Value,
    },
}

/// One letter of the alphabet: a template, for one run (`to`) or for every run.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Choice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Vec<u64>>,
    pub input: Template,
}

pub type Alphabet = Vec<Choice>;

fn default_limit() -> usize {
    10_000
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Method {
    Counterexample {
        deliveries: Vec<Delivery>,
    },
    Bounded {
        depth: usize,
        alphabet: Alphabet,
    },
    Exhaustive {
        alphabet: Alphabet,
        #[serde(default = "default_limit")]
        limit: usize,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "verdict", rename_all = "camelCase")]
pub enum Verdict {
    /// The property fails at event `at` of the run these deliveries produce.
    Refuted {
        counterexample: Vec<Delivery>,
        at: usize,
    },
    HoldsUpTo {
        depth: usize,
    },
    Holds,
}

/// What a verdict rests on that the kernel didn't check.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Assumption {
    /// A1: every installed capability's implementation keeps its contracts. Attributed to their publishers.
    Contracts { interface: String },
    /// A2: the checker is sound.
    Checker { kernel: String },
    /// The environment chooses only inputs from this alphabet.
    Environment { alphabet: String },
    /// A3: for statements about deployed runs, the runtime conforms to the interpreter.
    Runtime,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Evidence {
    pub harness: String,
    pub interface: String,
    pub grant: String,
    pub property: String,
    pub method: Method,
    #[serde(flatten)]
    pub verdict: Verdict,
    pub assumptions: Vec<Assumption>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProveError {
    Init {
        error: InitError,
    },
    Property {
        problems: Vec<PropertyProblem>,
    },
    /// A counterexample's delivery the environment couldn't have made.
    Inadmissible {
        index: usize,
        reason: Withheld,
    },
    /// The counterexample's run satisfies the property.
    NotViolated,
    /// `exhaustive` needs a quantifier-free property.
    Quantified,
    /// The search reached its limit before finishing.
    Inconclusive {
        explored: usize,
    },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VerifyError {
    /// The evidence names a different harness, interface, grant or property.
    WrongSubject,
    /// The kernel doesn't reach this verdict.
    Disagrees {
        kernel: Box<Result<Verdict, ProveError>>,
    },
}

/// How many deliveries a bounded search may try in all.
pub const BUDGET: usize = 200_000;

fn kernel_version() -> String {
    format!("cronch-kernel/{}", env!("CARGO_PKG_VERSION"))
}

fn events_as_values(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect()
}

fn start(subject: &Subject) -> Result<System, ProveError> {
    System::new(
        subject.harness.clone(),
        subject.interface.clone(),
        subject.grant.clone(),
    )
    .map_err(|error| ProveError::Init { error })
}

/// Every live run with its path, in spawn order.
fn live<'a>(run: &'a Run, path: &mut Vec<u64>, out: &mut Vec<(Vec<u64>, &'a Run)>) {
    if run.outcome.is_some() {
        return;
    }
    out.push((path.clone(), run));
    for c in &run.children {
        path.push(c.call);
        live(&c.run, path, out);
        path.pop();
    }
}

/// The deliveries the alphabet allows now, each with the system and events it leads to. Inadmissible ones are left
/// out: the environment can't make them.
fn successors(system: &System, alphabet: &Alphabet) -> Vec<(Delivery, System, Vec<Event>)> {
    let mut runs = Vec::new();
    live(&system.root, &mut Vec::new(), &mut runs);
    let mut candidates = Vec::new();
    for choice in alphabet {
        for (path, run) in &runs {
            if choice.to.as_ref().is_some_and(|to| to != path) {
                continue;
            }
            let calls = |op: &Option<String>| {
                run.pending
                    .iter()
                    .filter(|p| op.as_ref().is_none_or(|o| *o == p.op))
                    .map(|p| p.call)
                    .collect::<Vec<_>>()
            };
            let inputs: Vec<WorldInput> = match &choice.input {
                Template::Start { value, label } => vec![WorldInput::Start {
                    value: value.clone(),
                    label: label.clone(),
                }],
                Template::Ret { value, op } => calls(op)
                    .into_iter()
                    .map(|call| WorldInput::Ret {
                        call,
                        value: value.clone(),
                    })
                    .collect(),
                Template::End { op } => calls(op)
                    .into_iter()
                    .map(|call| WorldInput::End { call })
                    .collect(),
                Template::Failed { error, op } => calls(op)
                    .into_iter()
                    .map(|call| WorldInput::Failed {
                        call,
                        error: error.clone(),
                    })
                    .collect(),
                Template::Msg { channel, value } => vec![WorldInput::Msg {
                    channel: channel.clone(),
                    value: value.clone(),
                }],
            };
            candidates.extend(inputs.into_iter().map(|input| Delivery {
                to: path.clone(),
                input,
            }));
        }
    }
    candidates
        .into_iter()
        .filter_map(|d| {
            let mut next = system.clone();
            let events = next.deliver(&d);
            let admitted = !matches!(events.first(), Some(Event::Withheld { .. }));
            admitted.then_some((d, next, events))
        })
        .collect()
}

fn replay(subject: &Subject, deliveries: &[Delivery]) -> Result<Verdict, ProveError> {
    let mut system = start(subject)?;
    let mut trace = Vec::new();
    for (index, d) in deliveries.iter().enumerate() {
        let events = system.deliver(d);
        if let Some(Event::Withheld { reason, .. }) = events.first() {
            return Err(ProveError::Inadmissible {
                index,
                reason: reason.clone(),
            });
        }
        trace.extend(events_as_values(&events));
    }
    match first_violation(&subject.property, &trace, 0) {
        Some(at) => Ok(Verdict::Refuted {
            counterexample: deliveries.to_vec(),
            at,
        }),
        None => Err(ProveError::NotViolated),
    }
}

fn bounded(subject: &Subject, depth: usize, alphabet: &Alphabet) -> Result<Verdict, ProveError> {
    struct Search<'a> {
        property: &'a Property,
        alphabet: &'a Alphabet,
        tried: usize,
    }
    impl Search<'_> {
        fn go(
            &mut self,
            system: &System,
            trace: &[Value],
            path: &mut Vec<Delivery>,
            depth: usize,
        ) -> Result<Option<(Vec<Delivery>, usize)>, ProveError> {
            if depth == 0 {
                return Ok(None);
            }
            for (d, next, events) in successors(system, self.alphabet) {
                self.tried += 1;
                if self.tried > BUDGET {
                    return Err(ProveError::Inconclusive {
                        explored: self.tried,
                    });
                }
                let mut longer = trace.to_vec();
                longer.extend(events_as_values(&events));
                path.push(d);
                if let Some(at) = first_violation(self.property, &longer, trace.len()) {
                    return Ok(Some((path.clone(), at)));
                }
                if let Some(found) = self.go(&next, &longer, path, depth - 1)? {
                    return Ok(Some(found));
                }
                path.pop();
            }
            Ok(None)
        }
    }
    let system = start(subject)?;
    let mut search = Search {
        property: &subject.property,
        alphabet,
        tried: 0,
    };
    Ok(match search.go(&system, &[], &mut Vec::new(), depth)? {
        Some((counterexample, at)) => Verdict::Refuted { counterexample, at },
        None => Verdict::HoldsUpTo { depth },
    })
}

fn exhaustive(subject: &Subject, alphabet: &Alphabet, limit: usize) -> Result<Verdict, ProveError> {
    let formula = &subject.property.always;
    if !quantifier_free(formula) {
        return Err(ProveError::Quantified);
    }
    let state_key = |system: &System, monitor: &MonitorState| {
        digest(&json!({ "system": system, "monitor": monitor }))
    };
    let system = start(subject)?;
    let monitor = MonitorState::default();
    let first = state_key(&system, &monitor);
    // Each configuration seen, with the configuration and delivery it was first reached from.
    let mut seen: BTreeMap<String, Option<(String, Delivery)>> =
        BTreeMap::from([(first.clone(), None)]);
    let mut queue = VecDeque::from([(system, monitor, first)]);
    let path_to = |seen: &BTreeMap<String, Option<(String, Delivery)>>, mut at: String| {
        let mut deliveries = Vec::new();
        while let Some(Some((parent, d))) = seen.get(&at) {
            deliveries.push(d.clone());
            at = parent.clone();
        }
        deliveries.reverse();
        deliveries
    };
    while let Some((system, monitor, here)) = queue.pop_front() {
        for (d, next, events) in successors(&system, alphabet) {
            let mut m = monitor.clone();
            let mut violated = false;
            for e in events_as_values(&events) {
                let (after, holds) =
                    advance(formula, &m, &e).map_err(|_| ProveError::Quantified)?;
                m = after;
                if !holds {
                    violated = true;
                    break;
                }
            }
            if violated {
                let mut counterexample = path_to(&seen, here.clone());
                counterexample.push(d);
                return replay(subject, &counterexample);
            }
            let k = state_key(&next, &m);
            if !seen.contains_key(&k) {
                if seen.len() >= limit {
                    return Err(ProveError::Inconclusive {
                        explored: seen.len(),
                    });
                }
                seen.insert(k.clone(), Some((here.clone(), d)));
                queue.push_back((next, m, k));
            }
        }
    }
    Ok(Verdict::Holds)
}

fn verdict(subject: &Subject, method: &Method) -> Result<Verdict, ProveError> {
    let problems = check_property(&subject.property);
    if !problems.is_empty() {
        return Err(ProveError::Property { problems });
    }
    match method {
        Method::Counterexample { deliveries } => replay(subject, deliveries),
        Method::Bounded { depth, alphabet } => bounded(subject, *depth, alphabet),
        Method::Exhaustive { alphabet, limit } => exhaustive(subject, alphabet, *limit),
    }
}

/// The kernel's verdict on the subject by this method, as evidence.
pub fn prove(subject: &Subject, method: &Method) -> Result<Evidence, ProveError> {
    let verdict = verdict(subject, method)?;
    let mut assumptions = vec![
        Assumption::Contracts {
            interface: key(Kind::Interface, &subject.interface),
        },
        Assumption::Checker {
            kernel: kernel_version(),
        },
    ];
    match (method, &verdict) {
        (_, Verdict::Refuted { .. }) => {}
        (Method::Bounded { alphabet, .. } | Method::Exhaustive { alphabet, .. }, _) => {
            assumptions.push(Assumption::Environment {
                alphabet: key(Kind::Alphabet, alphabet),
            });
            assumptions.push(Assumption::Runtime);
        }
        (Method::Counterexample { .. }, _) => {}
    }
    Ok(Evidence {
        harness: key(Kind::Harness, &subject.harness),
        interface: key(Kind::Interface, &subject.interface),
        grant: key(Kind::Grant, &subject.grant),
        property: key(Kind::Property, &subject.property),
        method: method.clone(),
        verdict,
        assumptions,
    })
}

/// Re-check evidence: it must be about this subject, and the kernel must reach its verdict. A refutation is checked
/// by replaying its counterexample alone.
pub fn verify(subject: &Subject, evidence: &Evidence) -> Result<(), VerifyError> {
    let about = key(Kind::Harness, &subject.harness) == evidence.harness
        && key(Kind::Interface, &subject.interface) == evidence.interface
        && key(Kind::Grant, &subject.grant) == evidence.grant
        && key(Kind::Property, &subject.property) == evidence.property;
    if !about {
        return Err(VerifyError::WrongSubject);
    }
    let kernel = match &evidence.verdict {
        Verdict::Refuted { counterexample, .. } => verdict(
            subject,
            &Method::Counterexample {
                deliveries: counterexample.clone(),
            },
        ),
        _ => verdict(subject, &evidence.method),
    };
    match &kernel {
        Ok(v) if *v == evidence.verdict => Ok(()),
        _ => Err(VerifyError::Disagrees {
            kernel: Box::new(kernel),
        }),
    }
}
