//! The interpreter: what running a harness means.
//!
//! A configuration holds the state, the pending calls, the next call identifier and a queue of inputs the
//! interpreter produced itself. The environment delivers one input at a time; the interpreter checks that the input
//! is one the environment could admissibly produce, steps `δ` on it, and processes the actions in order:
//!
//! * `call(op, args)` gets the next identifier. It is performed and becomes pending if the interface has the operation,
//!   the arguments have its input type and its precondition holds; otherwise it is refused, and `refused(k, op)` is
//!   queued. The interpreter checks the precondition itself, at the point of the call, so it holds whatever `δ` does.
//! * `cancel(k)` is passed to the world. It is a request: `k` stays pending and its results may still arrive.
//! * `spawn(harness, narrow, arg)` gets the next identifier and starts a child run of `harness` with `arg`, against the
//!   parent's interface narrowed by `narrow`. The child's calls go through the same implementations, its
//!   preconditions are the parent's strengthened, and it shares the grant state, so a child can never do what its
//!   parent couldn't, and an approval given to a child widens what its ancestors may do too. When the child halts,
//!   its value comes back to the parent as `ret(k, v)`, labelled as the child's own evaluation labelled it; if it
//!   fails, as `failed(k, e)`. A spawn that can't start (an ill-formed harness, a narrowing of something the
//!   interface lacks, an argument of the wrong type) is refused.
//! * `halt(v)` ends the run; later actions of the step are discarded.
//!
//! Cancelling a spawn revokes the child at once: the interpreter runs it, so unlike a call, there is no world to ask.
//! A run that ends takes its children with it.
//!
//! Queued inputs are delivered before the world gets another turn. An input the environment couldn't admissibly
//! produce (a result for a call that isn't pending, a value of the wrong type, a result that breaks its operation's
//! postcondition) is recorded as withheld and changes nothing: the run stays a run under an admissible environment,
//! one that hasn't delivered that input yet. A withheld result is evidence against the capability's publisher.
//!
//! The grant state is the interpreter's, not `δ`'s: preconditions read it, and only contracts change it. A result's
//! `onResult` and a message's `onMsg` apply only when what arrived is trusted, so a harness can't grant itself
//! anything, and neither can anything untrusted it talks to.
//!
//! Every step is recorded as events: one for the input, then one per processed action, each carrying the state the
//! step produced. The trace is what properties are stated over.

use crate::harness::{
    Harness, HarnessProblem, Interface, Mode, Narrowing, Operation, check_harness, check_interface,
    narrow,
};
use crate::ir::{Expr, Type};
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
        #[serde(default, skip_serializing_if = "Label::is_empty")]
        label: Label,
    },
    /// `δ` failed on an input, or gave something that isn't a step. The run can't go on.
    Failed { error: StepError },
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
    /// The join of the labels of every input delivered: what the run's behaviour may depend on.
    #[serde(default, skip_serializing_if = "Label::is_empty")]
    pub seen: Label,
    /// Children spawned and still running, in the order they were spawned.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Child>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Child {
    /// The identifier of the spawn that started it, which is also its name in delivery paths.
    pub call: u64,
    pub run: Run,
}

/// How many inputs the interpreter may deliver to itself in response to one delivery. A tree of runs that keeps
/// spawning and answering itself without the world ever moving would otherwise never give the world another turn.
pub const DRAIN_LIMIT: usize = 4096;

/// A configuration of the whole system: the root run, and the grant state the root principal gave it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct System {
    pub root: Run,
    #[serde(default)]
    pub grant: Value,
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
        expected: Type,
        got: Value,
    },
    /// The result doesn't satisfy the operation's postcondition.
    Postcondition {
        call: u64,
    },
    /// The message doesn't satisfy the channel's invariant.
    Invariant {
        channel: String,
    },
    /// The operation's return label doesn't evaluate to a list of sources.
    ReturnLabel {
        call: u64,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StepError {
    /// The evaluation error, as data so it can be kept with the configuration.
    Eval { error: Value },
    /// `δ` gave something other than `{state, actions}` with a state of type `S` and a list of actions.
    NotAStep { got: Value },
    /// An action that isn't `call`, `spawn`, `cancel` or `halt` with the fields they need.
    BadAction { action: Value },
    /// The runs kept answering each other for `DRAIN_LIMIT` inputs without the world getting a turn.
    Runaway,
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
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        effect: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        footprint: Option<Value>,
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
    Interface { problems: Vec<HarnessProblem> },
}

/// The value of a contract's expression, or `None` if it fails to evaluate.
fn contract(expr: &Expr, vars: Vec<(&str, LValue)>) -> Option<LValue> {
    let scope = vars.into_iter().map(|(n, v)| (n.to_string(), v)).collect();
    labeled::eval(expr, scope).ok()
}

/// Whether a contract's condition holds. Absent: it does. One that fails to evaluate doesn't: checks fail closed.
fn holds(expr: &Option<Expr>, vars: Vec<(&str, LValue)>) -> bool {
    expr.as_ref()
        .is_none_or(|e| contract(e, vars).is_some_and(|v| v.value() == Value::Bool(true)))
}

/// The label of what a call of this operation delivers.
fn result_label(op: &Operation, args: &LValue) -> Option<Label> {
    let mut label = match &op.ret {
        None => args.label(),
        Some(e) => contract(e, vec![("args", args.clone())])?
            .value()
            .as_array()?
            .iter()
            .map(|s| s.as_str().map(str::to_string))
            .collect::<Option<Label>>()?,
    };
    label.extend(op.raise.iter().cloned());
    Some(label)
}

/// An input `δ` will receive, and the grant state its arrival leaves.
struct Admitted {
    inbound: Inbound,
    grant: Option<Value>,
}

impl System {
    /// A system that hasn't started: the harness at `s₀`, against the interface, with the root grant state.
    pub fn new(harness: Harness, interface: Interface, grant: Value) -> Result<System, InitError> {
        let problems = check_harness(&harness);
        if !problems.is_empty() {
            return Err(InitError::Harness { problems });
        }
        let problems = check_interface(&interface);
        if !problems.is_empty() {
            return Err(InitError::Interface { problems });
        }
        Ok(System {
            root: Run::new(harness, interface),
            grant,
        })
    }

    /// Deliver one input from the environment, then every input the interpreter queued in response.
    pub fn deliver(&mut self, delivery: &Delivery) -> Vec<Event> {
        let mut events = Vec::new();
        let withheld = |reason| Event::Withheld {
            origin: delivery.to.clone(),
            input: delivery.input.clone(),
            reason,
        };
        let System { root, grant } = self;
        let Some(run) = root.find_mut(&delivery.to) else {
            return vec![withheld(Withheld::NoSuchRun)];
        };
        match run.admit(&delivery.input, grant) {
            Err(reason) => events.push(withheld(reason)),
            Ok(admitted) => {
                if let Some(g) = admitted.grant {
                    *grant = g;
                }
                run.step(&delivery.to, admitted.inbound, grant, &mut events);
                run.end(&delivery.input);
            }
        }
        self.drain(&mut events);
        events
    }

    fn drain(&mut self, events: &mut Vec<Event>) {
        self.root.settle();
        for _ in 0..DRAIN_LIMIT {
            let mut path = Vec::new();
            if !self.root.queued(&mut path) {
                return;
            }
            let run = self.root.find_mut(&path).expect("a queued run exists");
            let inbound = run.queue.pop_front().expect("its queue isn't empty");
            run.step(&path, inbound, &mut self.grant, events);
            self.root.settle();
        }
        let input = json!({ "kind": "drain", "limit": DRAIN_LIMIT });
        self.root.fail(&[], input, StepError::Runaway, events);
        self.root.settle();
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
            seen: Label::new(),
            children: Vec::new(),
        }
    }

    fn find_mut(&mut self, path: &[u64]) -> Option<&mut Run> {
        match path.split_first() {
            None => Some(self),
            Some((first, rest)) => self
                .children
                .iter_mut()
                .find(|c| c.call == *first)
                .and_then(|c| c.run.find_mut(rest)),
        }
    }

    /// The path of the first run, in spawn order, with an input queued.
    fn queued(&self, path: &mut Vec<u64>) -> bool {
        if self.outcome.is_none() && !self.queue.is_empty() {
            return true;
        }
        for c in &self.children {
            path.push(c.call);
            if c.run.queued(path) {
                return true;
            }
            path.pop();
        }
        false
    }

    /// Hand finished children's outcomes to their parents, and end the children of runs that ended.
    fn settle(&mut self) {
        for c in &mut self.children {
            c.run.settle();
        }
        if self.outcome.is_some() {
            self.children.clear();
            return;
        }
        let (finished, running): (Vec<Child>, Vec<Child>) = std::mem::take(&mut self.children)
            .into_iter()
            .partition(|c| c.run.outcome.is_some());
        self.children = running;
        for c in finished {
            let inbound = match c.run.outcome {
                Some(Outcome::Halted { value, label }) => Inbound {
                    input: json!({ "kind": "ret", "call": c.call, "value": value }),
                    label,
                },
                Some(Outcome::Failed { error }) => Inbound {
                    input: json!({ "kind": "failed", "call": c.call, "error": error }),
                    label: c.run.seen,
                },
                None => unreachable!("partitioned on outcome"),
            };
            self.queue.push_back(inbound);
        }
    }

    fn typed(value: &Value, expected: &Type) -> Result<(), Withheld> {
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
    fn admit(&self, input: &WorldInput, grant: &Value) -> Result<Admitted, Withheld> {
        if self.outcome.is_some() {
            return Err(Withheld::Ended);
        }
        let admitted = |label, grant| Admitted {
            inbound: Inbound {
                input: input_value(input),
                label,
            },
            grant,
        };
        if let WorldInput::Start { value, label } = input {
            if self.started {
                return Err(Withheld::AlreadyStarted);
            }
            Self::typed(value, &self.harness.arg)?;
            return Ok(admitted(label.clone(), None));
        }
        if !self.started {
            return Err(Withheld::NotStarted);
        }
        let trusted_grant = || LValue::trusted(grant);
        match input {
            WorldInput::Start { .. } => unreachable!("handled above"),
            WorldInput::Ret { call, value } => {
                let (pending, op) = self.call(*call)?;
                Self::typed(value, &op.output)?;
                let result = LValue::trusted(value);
                if !holds(
                    &op.post,
                    vec![("args", pending.args.clone()), ("result", result.clone())],
                ) {
                    return Err(Withheld::Postcondition { call: *call });
                }
                let label =
                    result_label(op, &pending.args).ok_or(Withheld::ReturnLabel { call: *call })?;
                let update = op
                    .on_result
                    .as_ref()
                    .filter(|_| label.is_empty())
                    .and_then(|e| {
                        let vars = vec![
                            ("args", pending.args.clone()),
                            ("result", result),
                            ("grant", trusted_grant()),
                        ];
                        contract(e, vars).map(|g| g.value())
                    });
                Ok(admitted(label, update))
            }
            WorldInput::End { call } => {
                let (pending, op) = self.call(*call)?;
                if op.mode != Mode::Stream {
                    return Err(Withheld::NotAStream { call: *call });
                }
                let label =
                    result_label(op, &pending.args).ok_or(Withheld::ReturnLabel { call: *call })?;
                Ok(admitted(label, None))
            }
            WorldInput::Failed { call, .. } => {
                let (pending, op) = self.call(*call)?;
                let label =
                    result_label(op, &pending.args).ok_or(Withheld::ReturnLabel { call: *call })?;
                Ok(admitted(label, None))
            }
            WorldInput::Msg { channel, value } => {
                let declared = self.interface.channels.get(channel).ok_or_else(|| {
                    Withheld::NoSuchChannel {
                        channel: channel.clone(),
                    }
                })?;
                Self::typed(value, &declared.ty)?;
                let message = LValue::new(value, &declared.label);
                if !holds(&declared.inv, vec![("value", message.clone())]) {
                    return Err(Withheld::Invariant {
                        channel: channel.clone(),
                    });
                }
                let update = declared
                    .on_msg
                    .as_ref()
                    .filter(|_| declared.label.is_empty())
                    .and_then(|e| {
                        contract(e, vec![("value", message), ("grant", trusted_grant())])
                            .map(|g| g.value())
                    });
                Ok(admitted(declared.label.clone(), update))
            }
        }
    }

    /// A pending call and its operation.
    fn call(&self, call: u64) -> Result<(&Pending, &Operation), Withheld> {
        let pending = self.pending(call)?;
        Ok((pending, &self.interface.operations[&pending.op]))
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

    /// The child a spawn action starts, if it can start.
    fn child(&self, action: &Value, arg: &LValue) -> Option<Run> {
        let harness: Harness = serde_json::from_value(action.get("harness")?.clone()).ok()?;
        let narrowings: Vec<Narrowing> = match action.get("narrow") {
            None => Vec::new(),
            Some(n) => serde_json::from_value(n.clone()).ok()?,
        };
        let interface = narrow(&self.interface, &narrowings).ok()?;
        let well_formed = check_harness(&harness).is_empty()
            && check_interface(&interface).is_empty()
            && inhabits(&arg.value(), &harness.arg);
        well_formed.then(|| Run::new(harness, interface))
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
    fn step(
        &mut self,
        origin: &[u64],
        inbound: Inbound,
        grant: &mut Value,
        events: &mut Vec<Event>,
    ) {
        if inbound.input["kind"] == "start" {
            self.started = true;
        }
        self.seen.extend(inbound.label.iter().cloned());
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
            let out = |call, performed, effect, footprint| Event::Out {
                origin: origin.to_vec(),
                call,
                action: action.value(),
                performed,
                effect,
                footprint,
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
                    let declared = self.interface.operations.get(&op).filter(|o| {
                        inhabits(&args.value(), &o.input)
                            && holds(
                                &o.pre,
                                vec![("args", args.clone()), ("grant", LValue::trusted(grant))],
                            )
                    });
                    if let Some(o) = declared {
                        let footprint = o
                            .footprint
                            .as_ref()
                            .and_then(|e| contract(e, vec![("args", args.clone())]))
                            .map(|f| f.value());
                        events.push(out(Some(k), true, o.effect, footprint));
                        if let Some(next) = o.on_call.as_ref().and_then(|e| {
                            contract(
                                e,
                                vec![("args", args.clone()), ("grant", LValue::trusted(grant))],
                            )
                        }) {
                            *grant = next.value();
                        }
                        self.pending.push(Pending { call: k, op, args });
                    } else {
                        events.push(out(Some(k), false, false, None));
                        self.queue.push_back(Inbound {
                            input: json!({ "kind": "refused", "call": k, "op": op }),
                            label: Label::new(),
                        });
                    }
                }
                Some("spawn") => {
                    let k = self.next;
                    self.next += 1;
                    let arg = action
                        .field("arg")
                        .unwrap_or_else(|| LValue::new(&Value::Null, action.top()));
                    match self.child(&value, &arg) {
                        Some(mut run) => {
                            events.push(out(Some(k), true, false, None));
                            run.queue.push_back(Inbound {
                                input: json!({ "kind": "start", "value": arg.value() }),
                                label: arg.label(),
                            });
                            self.children.push(Child { call: k, run });
                        }
                        None => {
                            events.push(out(Some(k), false, false, None));
                            self.queue.push_back(Inbound {
                                input: json!({ "kind": "refused", "call": k, "op": "spawn" }),
                                label: Label::new(),
                            });
                        }
                    }
                }
                Some("cancel") if value["call"].is_u64() => {
                    let k = value["call"].as_u64().unwrap_or_default();
                    if let Some(i) = self.children.iter().position(|c| c.call == k) {
                        self.children.remove(i);
                        events.push(out(Some(k), true, false, None));
                        self.queue.push_back(Inbound {
                            input: json!({ "kind": "failed", "call": k, "error": { "cancelled": true } }),
                            label: Label::new(),
                        });
                    } else {
                        events.push(out(Some(k), self.pending(k).is_ok(), false, None));
                    }
                }
                Some("halt") => {
                    events.push(out(None, true, false, None));
                    let value = action
                        .field("value")
                        .unwrap_or_else(|| LValue::new(&Value::Null, action.top()));
                    self.outcome = Some(Outcome::Halted {
                        value: value.value(),
                        label: value.label(),
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
