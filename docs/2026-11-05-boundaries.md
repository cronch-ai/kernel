# Boundaries

Written on 2026-11-05, and agreed but for one item. What a value's type promises as it passes between a harness, its capabilities, its children and the world. It replaces [Typed boundaries](2026-11-02-typed-boundaries.md), and is meant to become the first section of the Pata spec.

## Status

Agreed by the owner:

- [The rule](#the-rule).
- No `Json` and no `is`.
- A model's answer is a plain variant, read with `match`.
- No new per-interface types: see [what this leaves out](#what-this-leaves-out-on-purpose).
- [Approvals](#approvals), on 2026-11-05 and 2026-11-06.
- On 2026-11-06:
  1. A model operation per set of tools, from [what it costs](#a-models-chosen-call).
  2. Adding a tool changes the harness and its hash, from the same list.
  3. [Per-capability approvals](#grants).
  4. [No recursive types](#data-with-no-fixed-shape), for T1.
  5. What a property reads becomes event variants, the last row of [the table](#the-table).

Proposed, and not yet agreed:

- The model's capability must end every call, from [what it costs](#a-models-chosen-call). The [host protocol](2026-11-06-host-protocol.md) has it as H8, a liveness obligation.

## The rule

**A value that crosses a boundary has a declared type made of pure Cronch datatypes, and the interpreter checks it on arrival. What doesn't fit doesn't cross.**

What arrives from the world fits exactly: a record has the fields its type declares and no others, at any depth. See [undeclared fields](#undeclared-fields).

There is no `Json` type and no `is`. A value is an `Int`, `Float`, `String`, `Bool`, list, record or variant, and `match` is how a variant is read. JSON stays a wire format where the kernel uses one. It isn't a type.

## The table

| Boundary                          | Its type comes from                                              | Checked                       | What doesn't fit  |
| --------------------------------- | ---------------------------------------------------------------- | ----------------------------- | ----------------- |
| Start value -> harness            | the harness's `arg`                                              | on delivery                   | withheld          |
| Capability result -> harness      | the operation's `output`, then `post`                            | on delivery                   | withheld          |
| Message -> harness                | the channel's `type`, then `inv`                                 | on delivery                   | withheld          |
| Harness -> call's arguments       | the operation's `input`, then `pre`                              | statically, then at the call  | refused           |
| Model's chosen call -> harness    | the model operation's `output`: a variant of its tools           | on delivery                   | withheld          |
| Parent -> child's argument        | the signature's `arg`, then the child's own                      | statically, then at the spawn | refused           |
| Parent -> child's harness         | `String`: the term as text, then the signature and the narrowing | at the spawn                  | refused           |
| Child's result -> parent          | the signature's `result`                                         | statically, then at its halt  | the child fails   |
| Harness's result                  | the harness's `result`, which it must declare                    | statically, then at the halt  | the run fails     |
| Harness's next state              | the harness's `state`                                            | after each step               | the run fails     |
| Initial grant                     | the empty value of the interface's `grant`: nothing passes one   | when the interface is checked | refused           |
| Update -> grant                   | the interface's `grant`                                          | statically                    | refused at `init` |
| World's failure detail -> harness | it is `String`                                                   | always text                   | nothing           |
| A failed child's error -> parent  | it is `String`                                                   | always text                   | nothing           |
| Event -> property                 | the event's variant, and `String` for what one type can't cover  | never: it only reads          | nothing to refuse |

[boundaries.rs](../crates/kernel/tests/boundaries.rs) and [chosen_call.rs](../crates/kernel/tests/chosen_call.rs) pin the rows of a single run and the model's chosen call, and [spawn.rs](../crates/kernel/tests/spawn.rs) the refused spawn. [boundaries.rs](../crates/kernel/tests/boundaries.rs) also pins a child's result, the next state, and what a property reads.

## Undeclared fields

Agreed on 2026-11-06: a value from the world with a field its type doesn't declare is rejected, not stripped. A start value, a capability's result and a message are checked exactly on arrival, at any depth, and one with such a field is withheld like any other value that doesn't fit. A variant's tags sit beside its case's fields, so they are declared.

Agreed on 2026-11-06 too: a call's arguments are checked exactly at the call, so a capability never receives a field its operation's `input` doesn't declare. The static checker keeps width subtyping, so a step may make such arguments, and the call is refused at run time, as one whose precondition fails is. A `pre` of `args != {url: "evil"}` and one of `args.url != "evil"` then agree. A spawn's argument is checked exactly too, against the child's signature and its harness, and so is a halt's value, against the harness's result: an undeclared field refuses the spawn, or fails the run. A step's state never leaves its run, so it is still checked by width subtyping.

A spawn names its child's harness and narrowing as text. A term has no Cronch type, since there are no [recursive types](#data-with-no-fixed-shape), so it crosses as `String`, and the interpreter reads it at the spawn. A spawn is a variant over the declared children, each with its signature's `arg`, so a step that spawns an undeclared child, or with the wrong argument, doesn't check. A world that chooses a child, such as a model writing one, sends its harness as text, and it is checked at the spawn like any other.

## A model's chosen call

A model's answer is a variant of the tools it is offered, such as `answer {text} | net.fetch {url} | notes.append {text}`. A harness reads it with `match`, and each arm makes its call. In the surface language, `x = perform choice` writes those arms: a case naming an operation issues it with the case's fields as its arguments, any other case skips, and `x` holds the result as text. This needs no new type: no `Call`, no `Returned`, and nothing resolved per interface.

- **A choice of the wrong shape never reaches the harness.** A bad argument, or a tool that isn't offered, doesn't inhabit the variant, so the result is withheld.
- **A contract still has the last word.** A well-shaped call the contract forbids is refused at the call, and the harness hears so.
- **A child is offered only what it may do.** It has its own model operation, whose variant lists fewer tools. A child whose step calls what its narrowing dropped can't start.
- **A performed call's result reaches the model as text,** through `str`. Each arm knows its own result's type.

What it costs:

- **A performed call's result reaches the model as text.** In the surface language, several operations may answer one `perform`, so `x` holds `str` of what arrived. A string stays as it is. Anything else is JSON text with its keys sorted. Equal values can still read differently: `0.5 + 0.5 == 1` holds, but `str` gives `1.0` and `1`. The model keeps every field and loses the type: `1` and `"1"` read the same, and so do a record and the string that spells it. A failure read through `failed(e)` gives its kind, `world`, without the world's detail, and `refused` carries no text. The text keeps the result's label, and the model's next answer is checked on arrival like any other. A rule reads contracts, not steps, so nothing it shows rests on this text.
- **A model operation per set of tools.** A capability offering a narrower model declares that operation too.
- **Adding a tool changes the harness.** It gains a case and an arm, so its hash changes. The surface language can write those arms. [Rules](2026-11-05-rules.md) aren't affected: they read contracts, not steps.
- **The model's capability must end every call.** Proposed, not yet agreed. A result that doesn't fit is withheld: the call stays pending, the configuration doesn't change, and the host sees the `withheld` event. A compliant capability then delivers a result that fits, or `failed`. A kernel step with no `failed` arm hears the failure and waits, with no call pending. A surface harness doesn't wait: a failure it doesn't handle halts the run through `otherwise`, or with `{"failed": "<node>"}` without one, and a harness whose result type can't hold that doesn't elaborate (`needsOtherwise`). The kernel enforces none of this. It is a capability spec (F7), and a run that waits breaks no safety property (F8). [chosen_call.rs](../crates/kernel/tests/chosen_call.rs) pins what the kernel does.

`Action` as a model's answer was the alternative. Its cases would offer a model `halt` and `cancel` as well. That type is gone: a step's actions are checked against the interface's action type, which is plain data.

## Approvals

Agreed on 2026-11-05 and 2026-11-06.

A grant is plain data, keyed on what the precondition reads. `net`'s precondition reads a URL, so its grant holds the approved URLs. An entry is there or it isn't, and a call that needs one is refused without it.

Every run starts with an empty grant, by construction: `init`, `ruleObligations` and evidence take none. The kernel starts every run at the empty value of the interface's grant type: lists empty, records empty field by field, numbers at zero, strings empty, `Bool` false, and a variant at a `none` case that holds nothing. A grant type with no such value, a variant with no empty `none` case, is refused when the interface is checked. A mode starts empty, too, and is set through a contract, such as a trusted channel's `onMsg`.

Every entry comes through a contract's update. A saved "always" or "session" approval reaches a run by the host answering yes when the harness asks through the approving operation, such as `net.ask`. The host never pre-fills or edits the grant. So an entry still means a person's answer, and the proof of that stays intact.

An entry stands for the rest of the run: the kernel never removes one on its own. The format has no once, always, session, expiry or revocation. Those are how the host decides to answer an ask, and whatever its own capability implementation enforces. The kernel proves none of them, and a harness run elsewhere gets whatever its host implements.

The proved claim, in the owner's words: "there is no path to the sequence of events where a certain action occurs before approval is given." It rests on the interpreter checking the precondition at every call, and on the approving operation's answer being a person's, given now or earlier under a policy they set.

If a future property needs exactly one call per approval, that is a new owner decision.

A fact about today, not a decision: a harness author can write consumption with ordinary terms: a contract's `onCall` that filters a list with `fold`. Some test fixtures do, in a grant field they name `once`. That field is a name in those contracts, and the kernel gives it no meaning.

## Grants

**Agreed on 2026-11-06.** Each capability keeps its own approvals: `net` keeps the URLs a person approved. [Approvals](#approvals) says what such a grant holds. There is no generic `approve` holding calls of any operation, so nothing needs a type for "any call". A proof reads a grant, so this also removes what stopped Ethos checking a real approval.

## Data with no fixed shape

**Agreed on 2026-11-06, for T1.** There are no recursive types, so a deeply nested value such as an arbitrary tool schema can't be described. It crosses as `String`, and a capability parses it under a contract. A fetched page is text for the same reason.

What such a string holds is out of reach of properties and proofs: neither can read a field inside it. Recursive types may come back after T1, if a property needs to.

A value that may be absent is a variant, `none | some {value: T}`.

## What this leaves out on purpose

The closed stack's `Call`, `Returned`, `@none`, approvals as text, and `op` in contracts. They existed to type a generic call, and nothing here needs one.

## Order

1. The harness of T1, on this.
2. The platform adopts it when it hosts that harness, including its `model` and `notes`. Its generic `approve` gives way to each capability's own, as [Grants](#grants) says.

## Open

- **Update -> grant is only checked statically.** The interpreter merges an update's fields without checking them again. That is sound while the checker is, and a second interpreter has nothing to compare against.
- **A `ret` label that can't evaluate withholds `failed` too.** `admit` needs the label for a failure as it does for a result. `init` accepts an operation whose `ret` is `[str(div(1, 0))]`. A `failed` on its call is then withheld as `returnLabel` and the call stays pending, so a compliant host can't end that call.
- **A step's state can still hold fields its type doesn't declare.** Every value a step sends across a boundary is checked exactly (see [undeclared fields](#undeclared-fields)), but its state is checked by width subtyping, so such a field is one the harness wrote and never leaves the run. The encoder still has no reading of record equality because of these fields ([smtlib/mod.rs](../crates/kernel/src/smtlib/mod.rs)).
- **`Float` in a query.** Ethos reads `Float64` no more than it did under `Json`, so a rule over a `Float` can't be checked as the kernel computes it: Ethos says `Could not find symbol Float64`. [Rules](2026-11-05-rules.md) has the detail. Nothing needs one yet.
