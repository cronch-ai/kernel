# Boundaries

Written on 2026-11-05, and only partly agreed. What a value's type promises as it passes between a harness, its capabilities, its children and the world. It replaces [Typed boundaries](2026-11-02-typed-boundaries.md), and is meant to become the first section of the VHF spec.

## Status

Agreed by the owner:

- [The rule](#the-rule).
- No `Json` and no `is`.
- A model's answer is a plain variant, read with `match`.
- No new per-interface types: see [what this leaves out](#what-this-leaves-out-on-purpose).
- [Approvals](#approvals).

Proposed, and not yet seen by the owner:

1. A model operation per set of tools, from [what it costs](#a-models-chosen-call).
2. Adding a tool changes the harness and its hash, from the same list.
3. [Per-capability approvals](#grants).
4. [No recursive types](#data-with-no-fixed-shape).
5. What a property reads becomes event variants, the last row of [the table](#the-table).

`main` may already work this way. That doesn't make these five agreed. The rest of this document carries no mark either way.

## The rule

**A value that crosses a boundary has a declared type made of pure Cronch datatypes, and the interpreter checks it on arrival. What doesn't fit doesn't cross.**

There is no `Json` type and no `is`. A value is an `Int`, `Float`, `String`, `Bool`, list, record or variant, and `match` is how a variant is read. JSON stays a wire format where the kernel uses one. It isn't a type.

## The table

| Boundary                         | Its type comes from                                             | Checked                      | What doesn't fit  |
| -------------------------------- | --------------------------------------------------------------- | ---------------------------- | ----------------- |
| Start value → harness            | the harness's `arg`                                             | on delivery                  | withheld          |
| Capability result → harness      | the operation's `output`, then `post`                           | on delivery                  | withheld          |
| Message → harness                | the channel's `type`, then `inv`                                | on delivery                  | withheld          |
| Harness → call's arguments       | the operation's `input`, then `pre`                             | statically, then at the call | refused           |
| Model's chosen call → harness    | the model operation's `output`: a variant of its tools          | on delivery                  | withheld          |
| Parent → child's argument        | the signature's `arg`, then the child's own                     | at the spawn                 | refused           |
| Child's result → parent          | the signature's `result`                                        | statically, then at its halt | the child fails   |
| Harness's result                 | the harness's `result`, which it must declare                   | statically, then at the halt | the run fails     |
| Harness's next state             | the harness's `state`                                           | after each step              | the run fails     |
| Initial grant                    | the interface's `grant`                                         | at `init`                    | refused           |
| Update → grant                   | the interface's `grant`                                         | statically                   | refused at `init` |
| World's failure detail → harness | it is `String`                                                  | always text                  | nothing           |
| A failed child's error → parent  | it is `String`                                                  | always text                  | nothing           |
| Event → property                 | the event's variant, and `String` for what one type can't cover | never: it only reads         | nothing to refuse |

[boundaries.rs](../crates/kernel/tests/boundaries.rs) and [chosen_call.rs](../crates/kernel/tests/chosen_call.rs) pin the rows of a single run and the model's chosen call, and [spawn.rs](../crates/kernel/tests/spawn.rs) the refused spawn. Three rows have no test of their own yet: a child's result, the next state, and what a property reads.

## A model's chosen call

A model's answer is a variant of the tools it is offered, such as `answer {text} | net.fetch {url} | notes.append {text}`. A harness reads it with `match`, and each arm makes its call. In the surface language, `x = perform choice` writes those arms: a case naming an operation issues it with the case's fields as its arguments, any other case skips, and `x` holds the result as text. This needs no new type: no `Call`, no `Returned`, and nothing resolved per interface.

- **A choice of the wrong shape never reaches the harness.** A bad argument, or a tool that isn't offered, doesn't inhabit the variant, so the result is withheld.
- **A contract still has the last word.** A well-shaped call the contract forbids is refused at the call, and the harness hears so.
- **A child is offered only what it may do.** It has its own model operation, whose variant lists fewer tools. A child whose step calls what its narrowing dropped can't start.
- **A performed call's result reaches the model as text,** through `str`. Each arm knows its own result's type.

What it costs:

- **A model operation per set of tools.** A capability offering a narrower model declares that operation too.
- **Adding a tool changes the harness.** It gains a case and an arm, so its hash changes. The surface language can write those arms. [Rules](2026-11-05-rules.md) aren't affected: they read contracts, not steps.
- **The model's capability must answer in the variant.** Its implementation turns an answer it can't fit into a failure. A withheld result leaves the call pending.

`Action` as a model's answer was the alternative. Its cases would offer a model `halt` and `cancel` as well. That type is gone: a step's actions are checked against the interface's action type, which is plain data.

## Approvals

A grant is plain data, keyed on what the precondition reads. `net`'s precondition reads a URL, so its grant holds the approved URLs. An entry is there or it isn't, and a call that needs one is refused without it.

The format has no once, always, session, expiry or revocation. Duration state belongs to the host: how long an entry stays, its scope, and removing it. "Approve this one call" is a host feature: grant, then remove. It isn't part of the kernel, and the kernel has no way to consume an approval of its own.

Two claims follow, and they rest on different things:

- **No call without a grant entry.** The interpreter checks the precondition at every call, so this holds whatever a host does about durations.
- **A one-time approval is used once.** Its trusted base is the host removing the entry. A host that fails to do so leaves an approval standing for longer than a person meant. That weakens this claim, which is only ergonomic, and not the one above.

A harness run elsewhere gets whatever duration semantics its host implements. The format promises none.

If a future property needs exactly one call per approval, that is a new owner decision.

A fact about today, not a decision: a harness author can write consumption with ordinary terms: a contract's `onCall` that filters a list with `fold`. Some test fixtures do, in a grant field they name `once`. That field is a name in those contracts, and the kernel gives it no meaning.

## Grants

**Proposed.** Each capability keeps its own approvals: `net` keeps the URLs a person approved. [Approvals](#approvals) says what such a grant holds. There is no generic `approve` holding calls of any operation, so nothing needs a type for "any call". A proof reads a grant, so this also removes what stopped Ethos checking a real approval.

## Data with no fixed shape

**Proposed.** There are no recursive types, so a deeply nested value such as an arbitrary tool schema can't be described. It crosses as `String`, and a capability parses it under a contract. A fetched page is text for the same reason.

A value that may be absent is a variant, `none | some {value: T}`.

## What this leaves out on purpose

The closed stack's `Call`, `Returned`, `@none`, approvals as text, and `op` in contracts. They existed to type a generic call, and nothing here needs one.

## Order

1. The harness of T1, on this.
2. The platform adopts it when it hosts that harness, including its `model`, `notes` and `approve`.

## Open

- **How a host adds an entry.** The owner is deciding this. A person's answer through an `onResult` that the rule's `approvedBy` names is in a query, and proves. A trusted channel's `onMsg` that appends gets a counterexample from its query. Editing the `System` a host keeps adds an entry and the call is performed, but no query reads it, and [index.d.ts](../javascript/index.d.ts) says a host hands that value back unchanged. A non-empty start grant appears in `initial`, which is then `sat`.
- **How a host removes one entry.** The owner is deciding this. The kernel has no operation for it, and each way today has a cost. A trusted channel's `onMsg` can filter the list with a `fold`, but `ruleObligations` refuses that update. An `onMsg` that empties the list proves if the empty list is typed with a `let`, since a bare `[]` is refused as `untyped`, but it removes every entry. Editing the `System` a host keeps works, but [index.d.ts](../javascript/index.d.ts) says a host hands it back unchanged and [run.rs](../crates/kernel/src/run.rs) says only contracts change a grant. An `onMsg` also reaches only the run it's delivered to and that run's ancestors, not a child's copy.
- **Update → grant is only checked statically.** The interpreter merges an update's fields without checking them again. That is sound while the checker is, and a second interpreter has nothing to compare against.
- **`Float` in a query.** Ethos reads `Float64` no more than it did under `Json`, so a rule over a `Float` probably still can't be checked. Nothing needs one yet.
