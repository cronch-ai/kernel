# Boundaries

Proposed on 2026-11-05. What a value's type promises as it passes between a harness, its capabilities, its children and the world. It replaces [Typed boundaries](2026-11-02-typed-boundaries.md), and is meant to become the first section of the VHF spec.

[boundaries.rs](../crates/kernel/tests/boundaries.rs) pins the rows of a single run, and [spawn.rs](../crates/kernel/tests/spawn.rs) the refused spawn. Three rows have no test of their own: a child's result, the next state, and what a property reads.

## The rule

**A value that crosses a boundary has a declared type, and the interpreter checks it on arrival. What doesn't fit doesn't cross.**

`Json` is the type that says nothing, so it only gets past the rule where a row below says why.

## The table

| Boundary                         | Its type comes from                         | Checked                              | What doesn't fit  |
| -------------------------------- | ------------------------------------------- | ------------------------------------ | ----------------- |
| Start value → harness            | the harness's `arg`                         | on delivery                          | withheld          |
| Capability result → harness      | the operation's `output`, then `post`       | on delivery                          | withheld          |
| Message → harness                | the channel's `type`, then `inv`            | on delivery                          | withheld          |
| Harness → call's arguments       | the operation's `input`, then `pre`         | at the call                          | refused           |
| Model's chosen call → call       | `Action`, of this interface                 | by the step's `is`, then at the call | no call happens   |
| Parent → child's argument        | the signature's `arg`, then the child's own | at the spawn                         | refused           |
| Child's result → parent          | the signature's `result`                    | statically, then at its halt         | the child fails   |
| Harness's result                 | the harness's `result`                      | statically, then at the halt         | the run fails     |
| Harness's next state             | the harness's `state`                       | after each step                      | the run fails     |
| Initial grant                    | the interface's `grant`                     | at `init`                            | refused           |
| Update → grant                   | the interface's `grant`                     | statically                           | refused at `init` |
| World's failure detail → harness | nothing                                     | never                                | always crosses    |
| Event → property                 | a view of the event                         | never: it only reads                 | nothing to refuse |

## Where `Json` stays, and why

- **A model's chosen call.** A harness holds it as `Json` until it narrows it to `Action`, whose cases are exactly this interface's operations and their input types. That holds under narrowing, since a child's `Action` is its own interface's. So `Json` costs a harness some precision here, and never lets through a call of the wrong shape.
- **A world's failure detail.** The world picks the shape. A step reads it with `is`.
- **An undeclared child's result.** Its harness is chosen at run time. Declaring the child types it.
- **What a property reads.** One event type covers every run in a tree, and nothing a property reads reaches an effect.

## Grants

A rule's proof reads a grant, so grants are where `Json` costs something: its SMT-LIB reading uses `Float64` and recursive functions, which Ethos can't follow.

**A harness can run with `Json` in its grant, and a rule can't be proved over it.** `ruleObligations` refuses a query that would hold `Json` (`holdsJson`), naming where. A query holds only what it reads or writes, so `Json` in a field the rule has nothing to do with costs nothing.

Forbidding `Json` in grants outright would cost more than it gives. A generic approval keeps calls of any operation, and giving those a type is what grew `Call`, `Returned` and approvals as text in the closed stack. T1 doesn't need one: a typed capability can keep exactly what it approves, as `net` keeps URLs.

## What this leaves out on purpose

- **`Call` and `Returned`**, resolved per interface. `Action` and `is` already enforce the boundary.
- **Deleting `Json` and `is`.** No row needs it.
- **Requiring every child to be declared, or every harness to declare a result.** Useful tidying, but neither moves T1.

A row changes when a harness or a proof needs it to, and not before.

## Open

- **Update → grant is only checked statically.** The interpreter merges an update's fields without checking them again. That is sound while the checker is, and a compliant second interpreter has nothing to compare against.
- **`Float64` for `Json` numbers** is what stops Ethos first. A reading of `Json` Ethos could follow would lift the `holdsJson` refusal, but nothing needs it yet.
