# Host protocol

Written on 2026-11-06, and proposed. What a host must do to run a harness under the interpreter's semantics. It is meant to become a section of the Pata spec. Tracked by [CRO-59](https://linear.app/cronch-ai/issue/CRO-59).

## Status

Agreed by the owner, and only restated here:

- [H2](#h2-start-every-run-with-an-empty-grant): every run starts with an empty grant. See [Approvals](2026-11-05-boundaries.md#approvals).

Everything else is proposed, and not yet seen by the owner. It states what hosts already have to do. It adds no kernel behaviour.

## What a host is

The interpreter is pure: `init` makes a configuration and `deliver` steps it. It does no I/O and keeps no state. A host does the rest:

- keeps the configuration between deliveries;
- performs the calls it asks for, through capability implementations;
- delivers what the world produces;
- keeps the log.

The platform is one host. A local host, for running a harness without Cronch (G2), would be another. A proof about deployed runs rests on assumption A3, "the runtime conforms to the interpreter" (`Assumption::Runtime` in [evidence.rs](../crates/kernel/src/evidence.rs)). This document is what conforming means. Interpreters are judged against the semantics (F7), and hosts are judged against this.

## Obligations

Each obligation has an identifier, which design decisions can cite. Identifiers are never reused or renumbered.

### H1. Hand the configuration back unchanged

The `System` that `init` or `deliver` returns is the one the next `deliver` gets. The same goes for a `MonitorState` and `monitorStep`. [index.d.ts](../javascript/index.d.ts) says so for both.

A host that edits the configuration steps outside the semantics. For example, editing `root.grant` adds an approval that no contract's update made. The call it unlocks is then performed, and no rule's query reads that entry ([#45](https://github.com/cronch-ai/kernel/pull/45) ran this).

### H2. Start every run with an empty grant

The kernel enforces this: `init` takes no grant, and starts every run at the empty value of the interface's grant type. Every entry then comes through a contract's update, such as a person's answer to `net.ask`. A saved "always" approval reaches a run by the host answering an ask with yes. What is left to a host is not to work around it, for example by editing the configuration (H1).

### H3. One delivery at a time

Each `deliver` takes the configuration the previous one returned, so a run's inputs are totally ordered. Two deliveries stepped from the same configuration would fork it, and one of them would be lost.

### H4. Deliver only what the world produced

A delivery is one of `start`, `ret`, `end`, `failed` and `msg`. Refusals are the interpreter's own, so no delivery carries one. Who may deliver each:

- `start` and `msg`: a principal. A `start`'s `label` names the sources the principal is no more trusted than. Empty means a trusted principal, so a host must not give an untrusted principal's start an empty label.
- `ret`, `end` and `failed` for a call: only the implementation that performs that call.

The interpreter withholds what the world couldn't have produced: a result for a call that isn't pending, a value of the wrong type, a result that breaks its postcondition, a message that breaks its channel's invariant. It can't tell a well-typed forged result from a real one. A host that lets anyone but the implementation deliver results makes assumption A1, "every implementation keeps its contracts", meaningless.

### H5. Perform exactly the calls the interpreter performed

A call the interpreter performed becomes pending, under its identifier. The host performs each one once, through an implementation of that operation's capability, with those arguments. It performs nothing else on the harness's behalf.

This is how F4 ("effects only through declared capabilities") reaches the world. The interpreter checks the precondition at the point of the call. A call it refused is not performed.

### H6. Pass cancellation on

`cancel(k)` is a request to the world. The call stays pending, and its results may still arrive. A host passes the request to the implementation and delivers whatever comes back.

Cancelling a spawn is different: the interpreter revokes the child at once, with nothing for the host to do.

### H7. Keep the log

Every delivery and the events it produced are recorded in order, as `logEntry` objects whose `prev` is the previous entry's key ([key.rs](../crates/kernel/src/key.rs)). Monitoring, audit and counterexamples about deployed runs read the log. A host that drops or reorders entries leaves nothing to check those against.

### H8. End every call, eventually

This one isn't safety. Under F8, a run that waits forever breaks no safety property, so H8 is a liveness obligation: no proof rests on it, and it only keeps runs moving.

- A host that can't finish a call delivers `failed`.
- A result the interpreter withheld leaves its call pending. A compliant implementation then delivers one that fits, or `failed`. The `failed` half is pinned in `crates/kernel/tests/chosen_call.rs` and `compliance/cases/withheld-then-failed.json`.

## What the kernel checks

| Obligation                     | Checked by the kernel                         | If a host breaks it                                        |
| ------------------------------ | --------------------------------------------- | ---------------------------------------------------------- |
| H1 configuration unchanged     | no                                            | the run is no run of the semantics, and no proof covers it |
| H2 empty start grant           | yes: `init` takes none                        | it can't                                                   |
| H3 one at a time               | no                                            | inputs are lost, and the log forks                         |
| H4 only the world's inputs     | partly: anything inadmissible is withheld     | forged well-typed results are admitted                     |
| H5 exactly the performed calls | no                                            | effects happen that no event records                       |
| H6 cancellation passed on      | no                                            | a cancelled call runs to completion; still safe            |
| H7 log kept                    | no: it keys entries, and the host chains them | nothing to audit against                                   |
| H8 calls end                   | no                                            | the run waits; still safe                                  |

Most obligations rest on the host. They are what assumption A3 stands for.

## Host policy

These are a host's own choices. The protocol says nothing about them:

- **How a host answers an ask.** Once, always, session and expiry are policies for answering `net.ask`. They aren't grant semantics.
- **What a host does on a violation.** The platform freezes a run: it admits nothing more and dispatches nothing.
- **Who may start a run, and with what label.** H4 asks only that the label be honest.
- **Timeouts, retries and recovery.** H8 asks only that calls end.

## The platform today

What each obligation looks like in the platform, from reading its `main`:

- **H1:** the run object stores the configuration as returned, and hands it back.
- **H2:** `create` takes a user's grant by key and passes it to `init`. The kernel ignores it, since `init` takes no grant, and the platform should stop taking one.
- **H3:** a delivery is stepped and stored with no `await` in between, in one Durable Object ([run.gleam](https://github.com/cronch-ai/platform/blob/main/src/run.gleam)).
- **H4:** `admits` lets users deliver only `start` and `msg`, and only the dispatcher `ret`, `end` and `failed`. A user's `start` reaches the kernel with whatever label the user gave. That fits H4 only while every user who starts a run is a trusted principal.
- **H5:** the dispatcher routes each performed call to its capability.
- **H7:** `log.gleam` hash-chains entries as `logEntry`.
- **H8:** on restart, every pending call not waiting for an approval gets `failed` with `lost`.
- **Approvals:** `answer` takes `once`, `always` or `deny`, through the platform's generic `approve`. Both are under review in [Boundaries](2026-11-05-boundaries.md#grants).

[CRO-57](https://linear.app/cronch-ai/issue/CRO-57) fixes H2 and the approvals when the platform hosts T1.

## Open

- **Should H1 be checkable?** The kernel could key the configuration it returns, and a host could log that key, so an audit replaying the log would show an edit. Replaying already shows one if the log is complete.
- **Vectors.** H4's withholding is pinned by the compliance vectors. H8 could get vectors of its own. The others are about what a host does outside the kernel, so no vector reaches them.
- **A local host.** `@pata/kernel/run` ([run.js](../javascript/run.js)) is one, and marks where it keeps each obligation. Its test replays `withheld-then-failed.json` through it, so H1, H3 and H8's `failed` half are checked against a vector. A second host is now there to test this document against.
