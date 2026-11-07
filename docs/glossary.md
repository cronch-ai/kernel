# Glossary

The kernel's terms, one or two sentences each, with where each is defined. Pata was called VHF until 2026-11-07, and older docs use that name.

## The kernel

- **Action:** what a step returns besides the next state: a `call`, `spawn`, `cancel` or `halt`. Calls run concurrently, and their results arrive in any order. See [harness.rs](../crates/kernel/src/harness.rs).
- **Assumption:** something a verdict rests on that isn't proved, such as "the runtime conforms to the interpreter". Evidence lists the assumptions it rests on. See [evidence.rs](../crates/kernel/src/evidence.rs).
- **Capability:** a set of operations and channels, each with a contract, installed in an interface under a prefix, as in `net.fetch`. A harness can do only what its capabilities permit, and there are no built-in operations. See [harness.rs](../crates/kernel/src/harness.rs).
- **Channel:** a stream of messages a capability delivers without being called. Its invariant (`inv`) filters what arrives.
- **Claim:** an address and a statement about what's there. Its key is the hash of both, so the same claim has the same key wherever it's made. See [Claims](2026-11-06-claims.md).
- **Compliance vectors:** test cases in [compliance/](../compliance/), replayed by the Rust crates and by the WebAssembly build. A second interpreter can check itself against them.
- **Configuration:** the interpreter's whole state for a run. `init` makes one and `deliver` steps it. The host keeps it between deliveries. See [run.rs](../crates/kernel/src/run.rs).
- **Contract:** what an operation requires and promises. The precondition (`pre`) is checked at the call; the postcondition (`post`) filters results; `ret` gives their label; `effect` and `footprint` record what the call may change; `grant` names the part of the grant state it reads and writes.
- **Effect and footprint:** whether a call changes the world outside the harness, and what it may touch. The kernel never inspects the world: the footprint is the publisher's promise, recorded on the trace.
- **Elaborate:** turn a harness in the surface language into a harness term. The `surface` crate does it, and is untrusted: the kernel checks what it produces.
- **Evidence:** the kernel's own verdict on a property. It replays a counterexample, searches every run to a depth, explores every reachable configuration, or checks an invariant certificate. It names its harness, interface and property by key. See [evidence.rs](../crates/kernel/src/evidence.rs).
- **Framed:** in `ruleObligations`, a capability update that writes none of the fields a rule's precondition reads. Its query is unchanged to the byte, so the proof already made still holds: it costs a check, not a proof. See [Claims](2026-11-06-claims.md).
- **Grant state:** state the interpreter holds for each run, not the harness. Every run starts with it empty. Only trusted results and messages widen it, and preconditions read it. An approval allowlist is one example. See [Boundaries](2026-11-05-boundaries.md#approvals).
- **Harness:** `(A, S, s0, step)`: an interface, a state type, an initial state and a step. A harness is a state machine whose step is data.
- **Host:** everything the pure interpreter doesn't do. It keeps the configuration, performs the calls, delivers what the world produces, and keeps the log. Cronch is one host.
- **Host protocol:** what a host must do for its runs to keep the interpreter's semantics. See [Host protocol](2026-11-06-host-protocol.md).
- **Induction:** proving an invariant of a harness's state. The kernel decides initiation and writes consecution as an SMT-LIB query; a solver searches for counterexamples, and the kernel checks any it finds. See [induction.rs](../crates/kernel/src/induction.rs).
- **Interface:** the capabilities a harness runs against, each under its own prefix.
- **Key:** the SHA-256 of an object's deterministic CBOR, prefixed by its kind and version. See [key.rs](../crates/kernel/src/key.rs).
- **Label:** the untrusted sources a value may depend on, including through control flow. Preconditions and properties can read labels.
- **Narrowing:** the combinators that make a child's interface from its parent's. They can only drop, restrict or distrust, so a child can never do what its parent couldn't.
- **Obligation:** a query whose `unsat` shows part of a rule holds, or, in Lean, the statement that it's `unsat`. `ruleObligations` writes them, and a proof checked outside the kernel discharges them. See [Rules](2026-11-05-rules.md).
- **Property:** `always(phi)`, with `phi` in first-order past-time temporal logic over a run's events. Every property is a safety property, monitored online or offline. See [property.rs](../crates/kernel/src/property.rs).
- **Rule:** what a capability's contracts alone rule out, whatever a harness does and whatever a model says. For example: an operation can't happen until a person has approved it. See [Rules](2026-11-05-rules.md).
- **Spawn:** the action that starts a child harness, against its parent's interface narrowed. Cancelling the spawn revokes the child.
- **Statement:** what a claim says, in the one statement language for properties, rules and invariants. For a rule's Lean obligation, it's the theorem a proof must prove.
- **Step:** a pure expression over `state` and `input` that gives the next state and a list of actions. It's data, not code: the term that is hashed is the term that is checked, run and proved.
- **Surface language:** a syntax for writing a harness as a graph of named calls, which elaborates to a harness term. See [the surface language](2026-11-01-surface-language.md).
- **Verdict:** proved (a proof, the checker that accepts it, and the claims it rests on), refuted (a counterexample the kernel replays) or vouched (a signature by whoever stands behind it). A claim with no verdict is open. A solver's `unsat` is never a verdict. See [Claims](2026-11-06-claims.md).

## On Cronch

These belong to the platform, not the kernel, but appear alongside its terms.

- **Mark:** an owner's statement about an MCP tool, such as `"noEffect"`. Until marked, every tool counts as an effect. A mark is part of the capability's contract.
- **Pin:** a Lean statement as Lean's kernel sees it, exported when a rule is registered. Proofs are checked against the pin, not against Lean source.
- **`restsOn`:** the list on a verdict of what it's trusted on, such as the checker's build by hash and the image that made the pin.
