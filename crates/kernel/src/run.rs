//! The interpreter: what running a harness means.
//!
//! A configuration holds the state, the pending calls, the next call identifier and a queue of inputs the
//! interpreter produced itself. The environment delivers one input at a time; the interpreter checks that the input
//! is one the environment could admissibly produce, steps `δ` on it, and processes the actions in order:
//!
//! * `call(op, args)` gets the next identifier. It is performed and becomes pending if the interface has the operation
//!   and the arguments have its input type; otherwise it is refused, and `refused(k, op)` is queued.
//! * `cancel(k)` is passed to the world. It is a request: `k` stays pending and its results may still arrive.
//! * `halt(v)` ends the run; later actions of the step are discarded.
//!
//! Queued inputs are delivered before the world gets another turn. An input the environment couldn't admissibly
//! produce (a result for a call that isn't pending, a value of the wrong type) is recorded as withheld and changes
//! nothing: the run stays a run under an admissible environment, one that hasn't delivered that input yet.
//!
//! Every step is recorded as events: one for the input, then one per processed action, each carrying the state the
//! step produced. The trace is what properties are stated over.

use crate::harness::{Harness, HarnessProblem, Interface, Mode, check_harness};
use crate::labeled::{self, EvalError, LValue, Label};
use crate::types::inhabits;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::VecDeque;

/// A call performed and not yet ended.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pending {
    pub call: u64,
    pub op: String,
    pub args: LValue,
}

/// An input with its label, as `δ` receives it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inbound {
    pub input: Value,
    #[serde(default, skip_serializing_if = "Label::is_empty")]
    pub label: Label,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outcome {
    Halted {
        value: Value,
    },
    /// `δ` failed on an input, or gave something that isn't a step. The run can't go on.
    Failed {
        error: StepError,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Run {
    pub harness: Harness,
    pub interface: Interface,
    pub state: LValue,
    /// In the order they were performed.
    #[serde(default)]
    pub pending: Vec<Pending>,
    #[serde(default)]
    pub next: u64,
    #[serde(default)]
    pub queue: VecDeque<Inbound>,
    #[serde(default)]
    pub started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
}

/// A configuration of the whole system.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct System {
    pub root: Run,
}

/// What the environment delivers, and to which run. `to` is a path of spawn identifiers from the root; `[]` is the
/// root itself.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Delivery {
    #[serde(default)]
    pub to: Vec<u64>,
    pub input: WorldInput,
}

/// An input the world may produce. Refusals are the interpreter's own, so no delivery can carry one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WorldInput {
    Start {
        value: Value,
        /// Who started the run, as the sources it is no more trusted than. Empty: a trusted principal.
        #[serde(default)]
        label: Label,
    },
    Ret {
        call: u64,
        value: Value,
    },
    End {
        call: u64,
    },
    Failed {
        call: u64,
        error: Value,
    },
    Msg {
        channel: String,
        value: Value,
    },
}

/// Why a delivery was withheld.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Withheld {
    NoSuchRun,
    Ended,
    NotStarted,
    AlreadyStarted,
    NotPending {
        call: u64,
    },
    NotAStream {
        call: u64,
    },
    NoSuchChannel {
        channel: String,
    },
    /// The value doesn't have the type the interface or the harness declares.
    Mismatch {
        expected: crate::ir::Type,
        got: Value,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StepError {
    /// The evaluation error, as data so it can be kept with the configuration.
    Eval { error: Value },
    /// `δ` gave something other than `{state, actions}` with a state of type `S` and a list of actions.
    NotAStep { got: Value },
    /// An action that isn't `call`, `cancel` or `halt` with the fields they need.
    BadAction { action: Value },
}

fn eval_error(e: EvalError) -> StepError {
    StepError::Eval {
        error: serde_json::to_value(e).unwrap_or(Value::Null),
    }
}

/// What happened, in order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    In {
        origin: Vec<u64>,
        input: Value,
        #[serde(default, skip_serializing_if = "Label::is_empty")]
        label: Label,
        state: Value,
    },
    Out {
        origin: Vec<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call: Option<u64>,
        action: Value,
        performed: bool,
        state: Value,
    },
    Withheld {
        origin: Vec<u64>,
        input: WorldInput,
        reason: Withheld,
    },
    Failed {
        origin: Vec<u64>,
        input: Value,
        error: StepError,
    },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InitError {
    Harness { problems: Vec<HarnessProblem> },
}

impl System {
    /// A system that hasn't started: the harness at `s₀`, against the interface.
    pub fn new(harness: Harness, interface: Interface) -> Result<System, InitError> {
        let problems = check_harness(&harness);
        if !problems.is_empty() {
            return Err(InitError::Harness { problems });
        }
        Ok(System {
            root: Run::new(harness, interface),
        })
    }

    fn run_mut(&mut self, path: &[u64]) -> Option<&mut Run> {
        if path.is_empty() {
            Some(&mut self.root)
        } else {
            None
        }
    }

    /// Deliver one input from the environment, then every input the interpreter queued in response.
    pub fn deliver(&mut self, delivery: &Delivery) -> Vec<Event> {
        let mut events = Vec::new();
        let withheld = |reason| Event::Withheld {
            origin: delivery.to.clone(),
            input: delivery.input.clone(),
            reason,
        };
        let Some(run) = self.run_mut(&delivery.to) else {
            return vec![withheld(Withheld::NoSuchRun)];
        };
        match run.admit(&delivery.input) {
            Err(reason) => events.push(withheld(reason)),
            Ok(inbound) => {
                run.step(&delivery.to, inbound, &mut events);
                run.end(&delivery.input);
            }
        }
        self.drain(&mut events);
        events
    }

    fn drain(&mut self, events: &mut Vec<Event>) {
        while let Some(inbound) = (self.root.outcome.is_none())
            .then(|| self.root.queue.pop_front())
            .flatten()
        {
            self.root.step(&[], inbound, events);
        }
    }
}

fn input_value(input: &WorldInput) -> Value {
    match input {
        WorldInput::Start { value, .. } => json!({ "kind": "start", "value": value }),
        WorldInput::Ret { call, value } => json!({ "kind": "ret", "call": call, "value": value }),
        WorldInput::End { call } => json!({ "kind": "end", "call": call }),
        WorldInput::Failed { call, error } => {
            json!({ "kind": "failed", "call": call, "error": error })
        }
        WorldInput::Msg { channel, value } => {
            json!({ "kind": "msg", "channel": channel, "value": value })
        }
    }
}

impl Run {
    fn new(harness: Harness, interface: Interface) -> Run {
        Run {
            state: LValue::trusted(&harness.init),
            harness,
            interface,
            pending: Vec::new(),
            next: 0,
            queue: VecDeque::new(),
            started: false,
            outcome: None,
        }
    }

    fn typed(value: &Value, expected: &crate::ir::Type) -> Result<(), Withheld> {
        if inhabits(value, expected) {
            Ok(())
        } else {
            Err(Withheld::Mismatch {
                expected: expected.clone(),
                got: value.clone(),
            })
        }
    }

    /// The input as `δ` will receive it, if the environment could admissibly produce it now.
    fn admit(&self, input: &WorldInput) -> Result<Inbound, Withheld> {
        if self.outcome.is_some() {
            return Err(Withheld::Ended);
        }
        if let WorldInput::Start { value, label } = input {
            if self.started {
                return Err(Withheld::AlreadyStarted);
            }
            Self::typed(value, &self.harness.arg)?;
            return Ok(Inbound {
                input: input_value(input),
                label: label.clone(),
            });
        }
        if !self.started {
            return Err(Withheld::NotStarted);
        }
        let label = match input {
            WorldInput::Start { .. } => unreachable!("handled above"),
            WorldInput::Ret { call, value } => {
                let pending = self.pending(*call)?;
                Self::typed(value, &self.interface.operations[&pending.op].output)?;
                pending.args.label()
            }
            WorldInput::End { call } => {
                let pending = self.pending(*call)?;
                if self.interface.operations[&pending.op].mode != Mode::Stream {
                    return Err(Withheld::NotAStream { call: *call });
                }
                pending.args.label()
            }
            WorldInput::Failed { call, .. } => self.pending(*call)?.args.label(),
            WorldInput::Msg { channel, value } => {
                let declared = self.interface.channels.get(channel).ok_or_else(|| {
                    Withheld::NoSuchChannel {
                        channel: channel.clone(),
                    }
                })?;
                Self::typed(value, &declared.ty)?;
                Label::new()
            }
        };
        Ok(Inbound {
            input: input_value(input),
            label,
        })
    }

    fn pending(&self, call: u64) -> Result<&Pending, Withheld> {
        self.pending
            .iter()
            .find(|p| p.call == call)
            .ok_or(Withheld::NotPending { call })
    }

    /// Remove the call an admitted input ended, if it ended one.
    fn end(&mut self, input: &WorldInput) {
        let ended = match input {
            WorldInput::Ret { call, .. } => self
                .pending(*call)
                .is_ok_and(|p| self.interface.operations[&p.op].mode == Mode::Single)
                .then_some(*call),
            WorldInput::End { call } | WorldInput::Failed { call, .. } => Some(*call),
            _ => None,
        };
        if let Some(call) = ended {
            self.pending.retain(|p| p.call != call);
        }
    }

    fn fail(&mut self, origin: &[u64], input: Value, error: StepError, events: &mut Vec<Event>) {
        events.push(Event::Failed {
            origin: origin.to_vec(),
            input,
            error: error.clone(),
        });
        self.outcome = Some(Outcome::Failed { error });
    }

    /// One step of `δ` on an admitted input, then its actions in order.
    fn step(&mut self, origin: &[u64], inbound: Inbound, events: &mut Vec<Event>) {
        if inbound.input["kind"] == "start" {
            self.started = true;
        }
        let input = LValue::new(&inbound.input, &inbound.label);
        let scope = vec![
            ("state".to_string(), self.state.clone()),
            ("input".to_string(), input),
        ];
        let result = match labeled::eval(&self.harness.step, scope) {
            Ok(r) => r,
            Err(e) => return self.fail(origin, inbound.input, eval_error(e), events),
        };
        let (Some(state), Some(LValue::List { label, items })) =
            (result.field("state"), result.field("actions"))
        else {
            let got = result.value();
            return self.fail(origin, inbound.input, StepError::NotAStep { got }, events);
        };
        if !inhabits(&state.value(), &self.harness.state) {
            let got = result.value();
            return self.fail(origin, inbound.input, StepError::NotAStep { got }, events);
        }
        self.state = state;
        let state = self.state.value();
        events.push(Event::In {
            origin: origin.to_vec(),
            input: inbound.input.clone(),
            label: inbound.label,
            state: state.clone(),
        });
        for action in items {
            let action = action.raised(&label);
            let out = |call, performed| Event::Out {
                origin: origin.to_vec(),
                call,
                action: action.value(),
                performed,
                state: state.clone(),
            };
            let value = action.value();
            match value["kind"].as_str() {
                Some("call") if value["op"].is_string() => {
                    let op = value["op"].as_str().unwrap_or_default().to_string();
                    let args = action
                        .field("args")
                        .unwrap_or_else(|| LValue::new(&Value::Null, action.top()));
                    let k = self.next;
                    self.next += 1;
                    let performed = self
                        .interface
                        .operations
                        .get(&op)
                        .is_some_and(|o| inhabits(&args.value(), &o.input));
                    events.push(out(Some(k), performed));
                    if performed {
                        self.pending.push(Pending { call: k, op, args });
                    } else {
                        self.queue.push_back(Inbound {
                            input: json!({ "kind": "refused", "call": k, "op": op }),
                            label: Label::new(),
                        });
                    }
                }
                Some("cancel") if value["call"].is_u64() => {
                    let k = value["call"].as_u64().unwrap_or_default();
                    events.push(out(Some(k), self.pending(k).is_ok()));
                }
                Some("halt") => {
                    events.push(out(None, true));
                    self.outcome = Some(Outcome::Halted {
                        value: value.get("value").cloned().unwrap_or(Value::Null),
                    });
                    break;
                }
                _ => {
                    return self.fail(
                        origin,
                        inbound.input,
                        StepError::BadAction { action: value },
                        events,
                    );
                }
            }
        }
    }
}
