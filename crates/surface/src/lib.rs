//! The surface language: harnesses written as graphs of named nodes, elaborated into the kernel's terms.
//!
//! A node is a call of an operation of a capability the harness `uses`, or a pure value. The references between
//! nodes are their dependencies, so the order of nodes in a file means nothing. Elaboration turns the graph into a
//! harness whose state has one slot per call node, and whose step starts every call whose dependencies have settled.
//! The design is in `docs/2026-11-01-surface-language.md`.
//!
//! Nothing here is trusted. The kernel checks the term elaboration produces, and everything it proves is about that
//! term: a bug here can make a harness do something other than what its source says, but never something its
//! contracts, grant or proved properties forbid.

pub mod api;
mod claims;
pub mod elaborate;
pub mod lex;
pub mod parse;
pub mod syntax;

use kernel::harness::HarnessProblem;
use kernel::ir::Type;
use lex::Span;
use serde::Serialize;

/// Something wrong with a source, and where.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Problem {
    pub span: Span,
    #[serde(flatten)]
    pub what: What,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum What {
    Syntax {
        expected: String,
        found: String,
    },
    /// Two declarations bind the same name, or a harness has two `return`s.
    Duplicate {
        name: String,
    },
    UnknownName {
        name: String,
    },
    UnknownType {
        name: String,
    },
    RecursiveType {
        name: String,
    },
    /// The harness has no `return`.
    NoReturn,
    /// A capability or a type, used as a value.
    NotAValue {
        name: String,
    },
    UnknownFunction {
        name: String,
    },
    /// A `uses` binding names a capability whose contract wasn't given.
    UnknownCapability {
        name: String,
        capability: String,
    },
    UnknownOperation {
        op: String,
    },
    UnknownChannel {
        channel: String,
    },
    /// A `uses ... = harness` binding names a harness whose term wasn't given.
    UnknownHarness {
        name: String,
        harness: String,
    },
    /// `spawn` of something that isn't a harness the header binds.
    NotAHarness {
        name: String,
    },
    /// A spawned harness that doesn't check against the interface it would run against.
    Child {
        name: String,
        problems: Vec<HarnessProblem>,
    },
    /// `until` names something that isn't another call or a receive, or is on a receive.
    Until {
        name: String,
    },
    /// A receive's filter reads a node.
    FilterReadsNode {
        node: String,
    },
    /// A call of an operation inside an expression, rather than as a node of its own.
    NestedCall {
        op: String,
    },
    /// An operation's arguments are named; a function's aren't.
    Argument {
        expected: String,
    },
    /// `when` on a node that isn't a call.
    GuardOnValue {
        node: String,
    },
    /// Nodes that depend on one another in a loop.
    Cycle {
        nodes: Vec<String>,
    },
    /// `return` reads, as a value, a node that may be skipped.
    MayBeSkipped {
        node: String,
        via: String,
    },
    /// A `match` on a node's outcome that doesn't cover one it can have.
    Uncovered {
        node: String,
        outcome: String,
    },
    NoSuchOutcome {
        outcome: String,
    },
    /// A `match` mixing case and literal patterns.
    MixedPatterns,
    /// A literal `match` with no `_` arm.
    NoDefault,
    /// `since` or a quantifier outside a property.
    PropertyOnly {
        construct: String,
    },
    /// A property or invariant reads a node without a slot: a pure value, which the state doesn't hold.
    NotASlot {
        node: String,
    },
    /// A temporal operator or event helper with the wrong arguments.
    Helper {
        name: String,
        expected: String,
    },
    /// `always` anywhere but around a whole property, which is always `□` already.
    NestedAlways,
    /// `otherwise` reads a node.
    OtherwiseReadsNode {
        node: String,
    },
    /// The kernel's type checker found a problem in an expression.
    Type {
        problem: Box<kernel::check::Problem>,
    },
    Mismatch {
        expected: Box<Type>,
        got: Box<Type>,
    },
    /// The kernel rejected the term elaboration produced: a bug in the elaborator, not in the source.
    Internal {
        problems: Vec<HarnessProblem>,
    },
}
