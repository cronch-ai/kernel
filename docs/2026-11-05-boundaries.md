# Boundaries

Agreed on 2026-11-05. What a value's type promises as it passes between a harness, its capabilities, its children and the world. It replaces [Typed boundaries](2026-11-02-typed-boundaries.md), and is meant to become the first section of the VHF spec.

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

- **A performed call's result reaches the model as text.** In the surface language, several operations may answer one `perform`, so `x` holds `str` of what arrived. A string stays as it is. Anything else is JSON text with its keys sorted. Equal values can still read differently: `0.5 + 0.5 == 1` holds, but `str` gives `1.0` and `1`. The model keeps every field and loses the type: `1` and `"1"` read the same, and so do a record and the string that spells it. A failure read through `failed(e)` gives its kind, `world`, without the world's detail, and `refused` carries no text. The text keeps the result's label, and the model's next answer is checked on arrival like any other. A rule reads contracts, not steps, so nothing it shows rests on this text.
- **A model operation per set of tools.** A capability offering a narrower model declares that operation too.
- **Adding a tool changes the harness.** It gains a case and an arm, so its hash changes. The surface language can write those arms. [Rules](2026-11-05-rules.md) aren't affected: they read contracts, not steps.
- **The model's capability must answer in the variant.** Its implementation turns an answer it can't fit into a failure. A withheld result leaves the call pending.

`Action` as a model's answer was the alternative. Its cases would offer a model `halt` and `cancel` as well. That type is gone: a step's actions are checked against the interface's action type, which is plain data.

## Grants

A grant holds pure datatypes too, and each capability keeps its own approvals: `net` keeps the URLs a person approved. There is no generic `approve` holding calls of any operation, so nothing needs a type for "any call". A proof reads a grant, so this also removes what stopped Ethos checking a real approval.

## Data with no fixed shape

There are no recursive types, so a deeply nested value such as an arbitrary tool schema can't be described. It crosses as `String`, and a capability parses it under a contract. A fetched page is text for the same reason.

A value that may be absent is a variant, `none | some {value: T}`.

## What this leaves out on purpose

The closed stack's `Call`, `Returned`, `@none`, approvals as text, and `op` in contracts. They existed to type a generic call, and nothing here needs one.

## Order

1. The harness of T1, on this.
2. The platform adopts it when it hosts that harness, including its `model`, `notes` and `approve`.

## Open

- **Update → grant is only checked statically.** The interpreter merges an update's fields without checking them again. That is sound while the checker is, and a second interpreter has nothing to compare against.
- **`Float` in a query.** Ethos reads `Float64` no more than it did under `Json`, so a rule over a `Float` probably still can't be checked. Nothing needs one yet.
