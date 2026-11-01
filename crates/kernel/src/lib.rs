//! The Cronch kernel: a formal semantics for agent harnesses, and the trusted checker of what is claimed about them.
//!
//! A harness is a state machine whose step function is a pure expression (`harness`). It runs against an interface
//! of capabilities, whose contracts the interpreter enforces at the point of each call (`run`). Its runs are traces of
//! events, and properties are past-time temporal formulas over them (`property`). The kernel reaches verdicts on
//! properties itself, by replaying counterexamples or searching runs, and records them as evidence that names what it
//! is about by content key (`evidence`). Values carry integrity labels throughout (`labeled`), so a property can say
//! which data may reach which effect. Language models, people and other agents are never trusted: they are
//! capabilities whose results a proof quantifies over.
//!
//! The kernel does no I/O and keeps no state: every request carries what it needs, in deterministic CBOR (`api`).

pub mod api;
pub mod cbor;
pub mod check;
pub mod evidence;
pub mod harness;
pub mod induction;
pub mod ir;
pub mod key;
pub mod labeled;
pub mod ops;
pub mod property;
pub mod run;
pub mod smtlib;
pub mod types;
