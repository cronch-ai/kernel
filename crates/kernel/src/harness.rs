//! Harnesses and the interfaces they run against.
//!
//! A harness is a state machine `⟨A, S, s₀, δ⟩`: what a run starts with, the type of its state, the initial state,
//! and the step function. `δ` is an expression over `state` and `input` that gives the next state and a list of
//! actions. It is data: the term that is hashed is the term that is checked, proved and run, and it is never compiled.
//! All nondeterminism is in the inputs, which the environment chooses.
//!
//! An interface is what a harness can do and be reached by: operations it may call and channels the world may send on.
//! None are built in: every one comes from an installed capability.

use crate::check::{Problem, check, infer};
use crate::ir::{Expr, Type};
use crate::labeled::Label;
use crate::types::{conforms, inhabits};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Harness {
    /// `A`: the type of the value a run is started with.
    pub arg: Type,
    /// `R`: the type of the value a run halts with. A halt with a value outside it is a step failure. Absent: `Any`.
    #[serde(default = "any", skip_serializing_if = "is_any")]
    pub result: Type,
    /// The children it may spawn, by name. A spawn that names one gets a child whose term has that signature, and
    /// its result arrives as `ret` on the operation `spawn.<name>`, at the signature's result type.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub spawns: BTreeMap<String, Signature>,
    /// `S`: the type of the state.
    pub state: Type,
    /// `s₀`.
    pub init: Value,
    /// `δ : S × Inp → S × List(Act)`, over the variables `state` and `input`, giving `{state, actions}`.
    pub step: Expr,
}

/// What a child declared in `spawns` takes and gives.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Signature {
    pub arg: Type,
    pub result: Type,
}

impl Signature {
    /// Whether a child harness has this signature: it accepts every argument the signature does, and gives only
    /// results the signature does.
    pub fn admits(&self, child: &Harness) -> bool {
        conforms(&self.arg, &child.arg) && conforms(&child.result, &self.result)
    }
}

/// The operation a declared child's results arrive on.
pub fn spawn_op(name: &str) -> String {
    format!("spawn.{name}")
}

fn any() -> Type {
    Type::Any
}

fn is_any(ty: &Type) -> bool {
    *ty == Type::Any
}

/// Whether a call delivers one result, or any number of items and then an end.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    Single,
    Stream,
}

impl Mode {
    fn is_single(&self) -> bool {
        *self == Mode::Single
    }
}

/// An operation a harness may call, with its contract.
///
/// The contract is what a harness may rely on about the operation, and what the interpreter enforces:
///
/// * `pre`, over `args` and the grant state `grant`: the call is performed only if it holds, checked by the
///   interpreter at the point of the call, whatever `δ` does. Absent: `true`.
/// * `post`, over `args` and `result`: every result the environment delivers satisfies it, or it is withheld.
///   Absent: `true`, the adversary stance, under which a proof covers every result the implementation could give.
/// * `ret`, over `args`: the label of what the call delivers, as a list of sources. Absent: the join of the
///   arguments' labels, since a result may depend on any of them. `raise` adds sources to it.
/// * `effect`: whether performing the call changes the world outside the harness, and `footprint`, over `args`,
///   what it may touch. The kernel never inspects the world: the footprint is the publisher's promise, recorded on
///   the trace for properties to state limits on.
/// * `onCall`, over `args` and `grant`, and `onResult`, over `args`, `result` and `grant`: the next grant state.
///   `onResult` applies only when the result is trusted (its label is empty), so authority only ever comes from a
///   trusted source, such as a person's answer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub input: Type,
    pub output: Type,
    #[serde(default, skip_serializing_if = "Mode::is_single")]
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre: Option<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post: Option<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ret: Option<Expr>,
    #[serde(default, skip_serializing_if = "Label::is_empty")]
    pub raise: Label,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub effect: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_call: Option<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_result: Option<Expr>,
}

/// A channel the world may reach a harness on, unprompted, with its contract: every message has type `type`,
/// satisfies `inv` (over `value`), and is no more trusted than `label`. `onMsg`, over `value` and `grant`, gives the
/// next grant state, and applies only on a trusted channel.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    #[serde(rename = "type")]
    pub ty: Type,
    #[serde(default, skip_serializing_if = "Label::is_empty")]
    pub label: Label,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inv: Option<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_msg: Option<Expr>,
}

/// Operations and channels. A capability is one, published and signed by whoever vouches for its contracts; an
/// interface is the union of the capabilities installed for a run.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Interface {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operations: BTreeMap<String, Operation>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, Channel>,
}

/// A capability to install, under a prefix that keeps its names apart from others' (`fs` + `read` is `fs.read`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Installation {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefix: String,
    pub capability: Interface,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InstallError {
    /// Two capabilities give the same name.
    Clash { name: String },
}

/// The interface of these capabilities together, defined when their names are disjoint.
pub fn install(installations: &[Installation]) -> Result<Interface, InstallError> {
    let mut out = Interface::default();
    let named = |prefix: &str, name: &str| {
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        }
    };
    for i in installations {
        for (name, op) in &i.capability.operations {
            let name = named(&i.prefix, name);
            if out.operations.insert(name.clone(), op.clone()).is_some() {
                return Err(InstallError::Clash { name });
            }
        }
        for (name, channel) in &i.capability.channels {
            let name = named(&i.prefix, name);
            if out.channels.insert(name.clone(), channel.clone()).is_some() {
                return Err(InstallError::Clash { name });
            }
        }
    }
    Ok(out)
}

/// One way to narrow an interface for a child. There is no way to widen one: every combinator removes an operation
/// or channel, strengthens a precondition, or makes results less trusted.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Narrowing {
    Drop {
        op: String,
    },
    /// Keep only these operations.
    Only {
        ops: Vec<String>,
    },
    DropChannel {
        channel: String,
    },
    /// The precondition becomes `pre ∧ extra`. `extra` is over `args` and `grant`, like `pre`.
    Restrict {
        op: String,
        pre: Expr,
    },
    /// Results of the operation carry these sources as well.
    Raise {
        op: String,
        label: Label,
    },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NarrowError {
    NoSuchOperation { op: String },
    NoSuchChannel { channel: String },
}

/// The interface after these narrowings, in order.
///
/// Never-amplifies holds by construction: a narrowed interface's operations and channels are a subset of the
/// original's, each precondition is the original's conjoined with something (so it holds only where the original
/// does), and each return label contains the original's. Nothing here can make a call possible that wasn't, or a
/// result more trusted than it was.
pub fn narrow(interface: &Interface, narrowings: &[Narrowing]) -> Result<Interface, NarrowError> {
    let mut out = interface.clone();
    let no_op = |op: &str| NarrowError::NoSuchOperation { op: op.to_string() };
    for n in narrowings {
        match n {
            Narrowing::Drop { op } => {
                out.operations.remove(op).ok_or_else(|| no_op(op))?;
            }
            Narrowing::Only { ops } => {
                if let Some(op) = ops.iter().find(|o| !out.operations.contains_key(*o)) {
                    return Err(no_op(op));
                }
                out.operations.retain(|name, _| ops.contains(name));
            }
            Narrowing::DropChannel { channel } => {
                out.channels
                    .remove(channel)
                    .ok_or_else(|| NarrowError::NoSuchChannel {
                        channel: channel.clone(),
                    })?;
            }
            Narrowing::Restrict { op, pre } => {
                let o = out.operations.get_mut(op).ok_or_else(|| no_op(op))?;
                let base = o.pre.take().unwrap_or(Expr::Lit {
                    v: Value::Bool(true),
                });
                o.pre = Some(Expr::Op {
                    op: "and".to_string(),
                    args: vec![base, pre.clone()],
                });
            }
            Narrowing::Raise { op, label } => {
                let o = out.operations.get_mut(op).ok_or_else(|| no_op(op))?;
                o.raise.extend(label.iter().cloned());
            }
        }
    }
    Ok(out)
}

/// Which expression of a harness or interface a problem is in.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "in", rename_all = "camelCase")]
pub enum Place {
    Step,
    Operation { name: String, part: String },
    Channel { name: String, part: String },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HarnessProblem {
    /// A problem inside one of the expressions.
    Expr { place: Place, problem: Problem },
    /// `s₀` doesn't inhabit `S`.
    Init { expected: Type, got: Value },
    /// `δ` doesn't give `{state: S, actions: List}`.
    Step { expected: Type, got: Type },
    /// A declared child whose results would arrive on an operation the interface already has.
    SpawnClash { name: String },
    /// A contract's expression has the wrong type: a condition that isn't `Bool`, a label that isn't a list of sources.
    Part {
        place: Place,
        expected: Type,
        got: Type,
    },
}

fn scope(vars: &[(&str, &Type)]) -> Vec<(String, Type)> {
    vars.iter()
        .map(|(n, t)| (n.to_string(), (*t).clone()))
        .collect()
}

fn check_part(
    place: Place,
    expr: &Option<Expr>,
    vars: &[(&str, &Type)],
    expected: Option<Type>,
    problems: &mut Vec<HarnessProblem>,
) {
    let Some(expr) = expr else { return };
    let Some(expected) = expected else {
        let (_, found) = infer(expr, scope(vars));
        problems.extend(found.into_iter().map(|problem| HarnessProblem::Expr {
            place: place.clone(),
            problem,
        }));
        return;
    };
    let (_, found) = check(expr, &expected, scope(vars));
    for problem in found {
        problems.push(match problem {
            // The part as a whole has the wrong type.
            Problem::Mismatch { at, expected, got } if at.is_empty() => HarnessProblem::Part {
                place: place.clone(),
                expected,
                got,
            },
            problem => HarnessProblem::Expr {
                place: place.clone(),
                problem,
            },
        });
    }
}

/// Problems with the expressions of an interface's contracts.
pub fn check_interface(interface: &Interface) -> Vec<HarnessProblem> {
    let mut problems = Vec::new();
    let grant = Type::Any;
    let sources = Type::List(Box::new(Type::String));
    for (name, o) in &interface.operations {
        let at = |part: &str| Place::Operation {
            name: name.clone(),
            part: part.to_string(),
        };
        let (args, result) = (&o.input, &o.output);
        let p = &mut problems;
        check_part(
            at("pre"),
            &o.pre,
            &[("args", args), ("grant", &grant)],
            Some(Type::Bool),
            p,
        );
        check_part(
            at("post"),
            &o.post,
            &[("args", args), ("result", result)],
            Some(Type::Bool),
            p,
        );
        check_part(
            at("ret"),
            &o.ret,
            &[("args", args)],
            Some(sources.clone()),
            p,
        );
        check_part(at("footprint"), &o.footprint, &[("args", args)], None, p);
        check_part(
            at("onCall"),
            &o.on_call,
            &[("args", args), ("grant", &grant)],
            None,
            p,
        );
        let vars = [("args", args), ("result", result), ("grant", &grant)];
        check_part(at("onResult"), &o.on_result, &vars, None, p);
    }
    for (name, c) in &interface.channels {
        let at = |part: &str| Place::Channel {
            name: name.clone(),
            part: part.to_string(),
        };
        let p = &mut problems;
        check_part(at("inv"), &c.inv, &[("value", &c.ty)], Some(Type::Bool), p);
        check_part(
            at("onMsg"),
            &c.on_msg,
            &[("value", &c.ty), ("grant", &grant)],
            None,
            p,
        );
    }
    problems
}

/// What `δ` must give: the next state and the actions.
pub fn step_type(state: &Type) -> Type {
    Type::Record(BTreeMap::from([
        ("state".to_string(), state.clone()),
        ("actions".to_string(), Type::List(Box::new(Type::Any))),
    ]))
}

/// The type of what `δ` receives against this interface: a variant over input kinds. A result's case is itself a
/// variant over the operation it answers (`op`), so matching on it gives the operation's output type. A declared
/// child's result arrives on `spawn.<name>` at its signature's result type; an undeclared one's on `spawn`, as `Any`,
/// since its harness is chosen at run time. A message's case is a variant over
/// channels, each with its type.
pub fn input_type(harness: &Harness, interface: &Interface) -> Type {
    let record = |fields: Vec<(&str, Type)>| {
        Type::Record(
            fields
                .into_iter()
                .map(|(n, t)| (n.to_string(), t))
                .collect(),
        )
    };
    let variant = |tag: &str, cases: BTreeMap<String, Type>| Type::Variant {
        tag: tag.to_string(),
        cases,
    };
    let mut results: BTreeMap<String, Type> = interface
        .operations
        .iter()
        .map(|(name, o)| {
            let case = record(vec![("call", Type::Int), ("value", o.output.clone())]);
            (name.clone(), case)
        })
        .collect();
    results.insert(
        "spawn".to_string(),
        record(vec![("call", Type::Int), ("value", Type::Any)]),
    );
    for (name, signature) in &harness.spawns {
        let case = record(vec![
            ("call", Type::Int),
            ("value", signature.result.clone()),
        ]);
        results.insert(spawn_op(name), case);
    }
    let ends: BTreeMap<String, Type> = interface
        .operations
        .iter()
        .filter(|(_, o)| o.mode == Mode::Stream)
        .map(|(name, _)| (name.clone(), record(vec![("call", Type::Int)])))
        .collect();
    let messages: BTreeMap<String, Type> = interface
        .channels
        .iter()
        .map(|(name, c)| (name.clone(), record(vec![("value", c.ty.clone())])))
        .collect();
    let called = || record(vec![("call", Type::Int), ("op", Type::String)]);
    let mut failed = called();
    if let Type::Record(fields) = &mut failed {
        fields.insert("error".to_string(), Type::Any);
    }
    variant(
        "kind",
        BTreeMap::from([
            (
                "start".to_string(),
                record(vec![("value", harness.arg.clone())]),
            ),
            ("ret".to_string(), variant("op", results)),
            ("end".to_string(), variant("op", ends)),
            ("failed".to_string(), failed),
            ("refused".to_string(), called()),
            ("msg".to_string(), variant("channel", messages)),
        ]),
    )
}

/// Problems with a harness. Against an interface, its input has the type `input_type` gives; without one, it is
/// typed `Any` and checked only when it arrives.
pub fn check_harness(harness: &Harness, interface: Option<&Interface>) -> Vec<HarnessProblem> {
    let mut problems = Vec::new();
    if !inhabits(&harness.init, &harness.state) {
        problems.push(HarnessProblem::Init {
            expected: harness.state.clone(),
            got: harness.init.clone(),
        });
    }
    if let Some(i) = interface {
        for name in harness.spawns.keys() {
            if i.operations.contains_key(&spawn_op(name)) {
                problems.push(HarnessProblem::SpawnClash { name: name.clone() });
            }
        }
    }
    let input = interface.map_or(Type::Any, |i| input_type(harness, i));
    let scope = vec![
        ("state".to_string(), harness.state.clone()),
        ("input".to_string(), input),
    ];
    // The step is checked against its type, so the state it builds can hold variants.
    let (_, found) = check(&harness.step, &step_type(&harness.state), scope);
    for problem in found {
        problems.push(match problem {
            // The step as a whole has the wrong type.
            Problem::Mismatch { at, expected, got } if at.is_empty() => {
                HarnessProblem::Step { expected, got }
            }
            problem => HarnessProblem::Expr {
                place: Place::Step,
                problem,
            },
        });
    }
    problems
}
