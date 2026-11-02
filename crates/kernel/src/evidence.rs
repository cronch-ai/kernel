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
//! * `invariant`: a certificate, an inductive invariant with the configurations it holds of, which the kernel checks
//!   without searching: initiation, consecution and safety, step by step. Finding the configurations is anyone's job
//!   (`certificate` finds them by search); checking them is the kernel's.
//!
//! The alphabet is an assumption about the environment, stated in the evidence: a proof over it says nothing about
//! inputs outside it. For invariants of `δ` over every state and every well-typed input, see `obligation`: a solver
//! looks for a counterexample to induction, and the kernel checks what it finds.

use crate::harness::{Harness, Interface};
use crate::ir::Expr;
use crate::key::{Kind, digest, key};
use crate::labeled::Label;
use crate::labeled::{self, LValue};
use crate::property::{
    MonitorState, Property, PropertyProblem, advance, check_property, first_violation,
    quantifier_free,
};
use crate::run::{Delivery, Event, InitError, Run, System, Withheld, WorldInput};
use crate::view::view;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

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
    /// An inductive invariant over configurations (the variable `config`), with the configurations it holds of.
    Invariant {
        invariant: Expr,
        configurations: Vec<Configuration>,
        alphabet: Alphabet,
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
    /// A certificate was asked for, and the property fails.
    Refuted {
        counterexample: Vec<Delivery>,
    },
    /// The certificate's configurations don't include the initial one.
    Initiation,
    /// The invariant doesn't hold of a configuration the certificate lists.
    NotInvariant {
        index: usize,
    },
    /// A step from a listed configuration leads outside the certificate.
    NotClosed {
        index: usize,
        delivery: Delivery,
    },
    /// The property fails at an event of a step from a listed configuration.
    Unsafe {
        index: usize,
        delivery: Delivery,
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
    events.iter().map(view).collect()
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

/// A configuration as a certificate lists it: the system, and the property's monitor after the events so far.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Configuration {
    pub system: System,
    #[serde(default)]
    pub monitor: MonitorState,
}

fn configuration_key(c: &Configuration) -> String {
    digest(&json!(c))
}

/// The monitor after a step's events, or `None` if the property fails at one of them.
fn monitored(
    formula: &crate::property::Formula,
    monitor: &MonitorState,
    events: &[Event],
) -> Result<Option<MonitorState>, ProveError> {
    let mut m = monitor.clone();
    for e in events_as_values(events) {
        let (after, holds) = advance(formula, &m, &e).map_err(|_| ProveError::Quantified)?;
        if !holds {
            return Ok(None);
        }
        m = after;
    }
    Ok(Some(m))
}

/// Every configuration reachable under the alphabet, or a counterexample.
enum Explored {
    Closed(Vec<Configuration>),
    Refuted(Vec<Delivery>),
}

fn explore(subject: &Subject, alphabet: &Alphabet, limit: usize) -> Result<Explored, ProveError> {
    let formula = &subject.property.always;
    if !quantifier_free(formula) {
        return Err(ProveError::Quantified);
    }
    let initial = Configuration {
        system: start(subject)?,
        monitor: MonitorState::default(),
    };
    let first = configuration_key(&initial);
    // Each configuration seen, with the configuration and delivery it was first reached from.
    let mut seen: BTreeMap<String, Option<(String, Delivery)>> =
        BTreeMap::from([(first.clone(), None)]);
    let mut found = vec![initial.clone()];
    let mut queue = VecDeque::from([(initial, first)]);
    let path_to = |seen: &BTreeMap<String, Option<(String, Delivery)>>, mut at: String| {
        let mut deliveries = Vec::new();
        while let Some(Some((parent, d))) = seen.get(&at) {
            deliveries.push(d.clone());
            at = parent.clone();
        }
        deliveries.reverse();
        deliveries
    };
    while let Some((here, here_key)) = queue.pop_front() {
        for (d, system, events) in successors(&here.system, alphabet) {
            let Some(monitor) = monitored(formula, &here.monitor, &events)? else {
                let mut counterexample = path_to(&seen, here_key.clone());
                counterexample.push(d);
                return Ok(Explored::Refuted(counterexample));
            };
            let next = Configuration { system, monitor };
            let k = configuration_key(&next);
            if !seen.contains_key(&k) {
                if seen.len() >= limit {
                    return Err(ProveError::Inconclusive {
                        explored: seen.len(),
                    });
                }
                seen.insert(k.clone(), Some((here_key.clone(), d)));
                found.push(next.clone());
                queue.push_back((next, k));
            }
        }
    }
    Ok(Explored::Closed(found))
}

fn exhaustive(subject: &Subject, alphabet: &Alphabet, limit: usize) -> Result<Verdict, ProveError> {
    match explore(subject, alphabet, limit)? {
        Explored::Closed(_) => Ok(Verdict::Holds),
        Explored::Refuted(counterexample) => replay(subject, &counterexample),
    }
}

/// The configurations an `invariant` certificate needs: every one reachable under the alphabet, found by search.
/// Anyone may find them another way; the kernel only checks what it is given.
pub fn certificate(
    subject: &Subject,
    alphabet: &Alphabet,
    limit: usize,
) -> Result<Vec<Configuration>, ProveError> {
    let problems = check_property(&subject.property, &subject.harness, &subject.interface);
    if !problems.is_empty() {
        return Err(ProveError::Property { problems });
    }
    match explore(subject, alphabet, limit)? {
        Explored::Closed(found) => Ok(found),
        Explored::Refuted(counterexample) => Err(ProveError::Refuted { counterexample }),
    }
}

/// Check an inductive invariant with the configurations it holds of: the initial configuration is one of them,
/// the invariant holds of each, every step the alphabet allows from one leads to another, and the property holds at
/// every event of those steps. Then, by induction on the length of a run, every configuration reachable under the
/// alphabet is one of them, and the property holds at every event of every run, at every depth.
fn invariant(
    subject: &Subject,
    expr: &Expr,
    configurations: &[Configuration],
    alphabet: &Alphabet,
) -> Result<Verdict, ProveError> {
    let formula = &subject.property.always;
    if !quantifier_free(formula) {
        return Err(ProveError::Quantified);
    }
    let holds_of = |c: &Configuration| {
        let config = LValue::trusted(&json!(c));
        labeled::eval(expr, vec![("config".to_string(), config)])
            .is_ok_and(|v| v.value() == Value::Bool(true))
    };
    let keys: BTreeSet<String> = configurations.iter().map(configuration_key).collect();
    let initial = Configuration {
        system: start(subject)?,
        monitor: MonitorState::default(),
    };
    if !keys.contains(&configuration_key(&initial)) {
        return Err(ProveError::Initiation);
    }
    for (index, c) in configurations.iter().enumerate() {
        if !holds_of(c) {
            return Err(ProveError::NotInvariant { index });
        }
        for (delivery, system, events) in successors(&c.system, alphabet) {
            let Some(monitor) = monitored(formula, &c.monitor, &events)? else {
                return Err(ProveError::Unsafe { index, delivery });
            };
            if !keys.contains(&configuration_key(&Configuration { system, monitor })) {
                return Err(ProveError::NotClosed { index, delivery });
            }
        }
    }
    Ok(Verdict::Holds)
}

fn verdict(subject: &Subject, method: &Method) -> Result<Verdict, ProveError> {
    let problems = check_property(&subject.property, &subject.harness, &subject.interface);
    if !problems.is_empty() {
        return Err(ProveError::Property { problems });
    }
    match method {
        Method::Counterexample { deliveries } => replay(subject, deliveries),
        Method::Bounded { depth, alphabet } => bounded(subject, *depth, alphabet),
        Method::Exhaustive { alphabet, limit } => exhaustive(subject, alphabet, *limit),
        Method::Invariant {
            invariant: expr,
            configurations,
            alphabet,
        } => invariant(subject, expr, configurations, alphabet),
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
        (
            Method::Bounded { alphabet, .. }
            | Method::Exhaustive { alphabet, .. }
            | Method::Invariant { alphabet, .. },
            _,
        ) => {
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
