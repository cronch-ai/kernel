# The surface language

Plan for the surface language and its elaborator, listed as later work in the [harness design session](2026-11-01-harness-design-session.md) and in [Kernel architecture](https://linear.app/cronch-ai/document/kernel-architecture-harness-semantics-capabilities-properties-and-be2d9d7b9bcd). Tracked in Linear under the "Cronch v1" project.

Today a harness is a JSON `Expr` term and a property a JSON `Formula`. The platform's `approved-note` fixture is about 150 lines of JSON for what reads as ten lines of source, and a property is harder still. Nobody but the kernel's own tests should have to write either.

Revised on 2026-11-02: the first draft of this plan proposed a Python-like language whose sequential code compiles to `δ` the way `async`/`await` does. It is replaced by the declarative graph style of `cronch-ai/cronch-gleam`.

## The idea: a harness is a graph

`cronch-ai/cronch-gleam` wrote programs as graphs:

- Named nodes, each invoking a kernel with named arguments.
- References to the program's inputs and to other nodes.
- One output expression.
- The effects the program needs, declared up front.

Order meant nothing, since the references are the dependencies, and a verifier checked that the graph was acyclic before anything ran.

The surface language takes that shape and gives it the kernel's semantics:

- **A node is a wish or a value.** `draft = model.generate(...)` names a call of an operation. Its value is what `ret` delivers: a value of the operation's output type that meets its contract. Who fills it (a model, a person through `approve`, a sandboxed module, a child harness) is the installation and the grant, never the harness. A node can also be a pure expression over other nodes.
- **What runs is still the pure `δ`.** The elaborator turns the graph into `δ`'s state machine:
  - The state holds one slot per call node: waiting, running, returned, failed, refused or skipped.
  - Each step records the result it was given. It then starts, in dependency order, every call whose dependencies have settled.
  - The run halts once every call has settled, so a call nothing reads still runs.

The graph suits the kernel better than sequential code would:

- **Concurrency is free.** Calls with no path between them are issued in the same step, and their results arrive in any order, as the kernel already allows. There is no `gather`.
- **Elaboration is a schedule, not a continuation transform.** There are no resume points, live locals or call stacks to generate. The generated `δ` is linear in the size of the graph.
- **The state has a fixed shape**, one slot per node, named in the source. Exhaustive exploration and invariant certificates work over a small, legible state, and properties can name nodes.
- **There is little to reject.** The language has no loops, exceptions or functions to forbid, so a program that looks valid almost always is.

Everything the kernel proves is about the elaborated `Expr`, which is the term that is hashed, proved and run. The elaborator is untrusted, as [Harness lifecycle](https://linear.app/cronch-ai/document/harness-lifecycle-write-register-prove-run-audit-f8f7aa476e2c) already says: a bug in it can make a harness do something other than what its source suggests, but never something its contracts, grant or proved properties forbid.

## Decisions

1. **The first version covers harnesses and properties.** A file declares the harness (its argument, the capabilities it uses, its nodes and its return) and the properties and invariants claimed about it. Contracts stay JSON and JS: the platform writes them for its own capabilities, and sandboxed modules return them from `contract()`.
2. **The elaborator lives in this repo, in Rust.** A new crate, `crates/surface`, depends on `crates/kernel` for `Expr`, `Type` and `infer`. It ships in the same `@cronch/kernel` wasm as an `elaborate` request, so the gateway, a CLI and an editor all use one definition. That's the reasoning that put the `module` key kind in the kernel.
   - `crates/kernel` stays the trusted core; nothing in it depends on `surface`.
   - The `cronch-ai/lang` repo, created empty on 2026-11-01, isn't used.
3. **The parser is hand-written, with no new dependencies.** The grammar is small: declarations, calls with named arguments, and the pure expressions. Anything outside it is rejected with a source span. Declarations end at a newline, and braces and brackets group, so the lexer needs no indentation tracking. A hand-written recursive-descent parser keeps the wasm small (972 KB today) and the errors precise.
4. **Calls are counted in the state.** `δ` sees a call's identifier only on its result. Identifiers are handed out in order, one per `call` or `spawn` action, refused or not, so the generated state keeps its own counter, and each running slot records its call's identifier. This needs no kernel change.
   - The alternative was to give `δ` the next identifier as a third variable. That changes the step rule, the SMT-LIB export and every proof for one convenience.
   - If the counter ever disagreed with the interpreter, a result would land in the wrong slot. That breaks the author's intent, not a contract: preconditions are still checked at every call.
5. **No source object in the kernel yet.** The harness's key is the key of the elaborated term. `elaborate` returns a source map with it, and the platform stores the source next to the harness it registers. Elaboration is deterministic for a given elaborator version, so anyone with the source and the same `@cronch/kernel` can recompute the term.
6. **Node slots are part of the language, not the elaborator.** The `nodes` field of the generated state, and each slot's fields, are defined here. Properties and invariants may read them and mean the same thing whatever else the elaborator generates. The call counter and anything else the elaborator adds stay out of reach.
7. **The harness names its capabilities.** Capabilities have identifiers: a platform capability's is a short string (`notes`), and a user's sandboxed module's is a content key that nobody should have to type. `uses` binds a readable name to each one, and that name is the prefix the capability is installed under. A run no longer picks prefixes; the harness does.
   - The body only ever says `search.query(...)`, never a key. A model writes readable names, which it is good at. The identifiers sit in the `uses` header, which the platform can write from its catalog of the owner's capabilities.
   - `elaborate` takes the contract of each identifier the header names, installs them under their names with the kernel's `install`, and types the harness against the result. It returns the header's bindings with the term, and the platform installs exactly those for every run of the harness.
   - The kernel treats identifiers as opaque strings. What they mean, and how a module's key is checked, is the platform's business.

## The language

`approved-note`, which asks the model for a note, asks the owner to approve appending it, and appends it only if approved:

```
harness approved_note(prompt: String) -> Any

uses model, approve, notes

draft    = model.generate(messages: [{role: "user", content: prompt}])
decision = approve.ask(action: "notes.append", args: {text: draft})
saved    = notes.append(text: draft) when decision != "deny"

return match saved {
  ok(note) => note
  skipped  => "denied"
}

otherwise return "stopped"

property no_effect_without_approval:
  always(effect() implies once(returned("approve.ask", value in ["once", "always"])))
```

`decision` reads `draft`, so it waits for it. `saved` reads both, so it waits for both. Nothing reads `saved` but `return`, so the run halts once `saved` settles.

### Harnesses (S1)

- **`harness name(arg: T) -> R`** opens the file. `arg` is the run's start value, of type `A`.
- **`uses`** binds names to capabilities (decision 7). `uses notes` binds `notes` to the capability whose identifier is `notes`. `uses search = "module:sha256-…"` binds `search` to that identifier. A call `name.op(...)` is of the operation `op` of the capability bound to `name`. A call of a name the header doesn't bind is an error, and so is an operation that capability doesn't have. A reader, or the platform, sees what the harness can do without reading its nodes, as with cronch-gleam's `requested_effects`.
- **Nodes:** `name = expr` binds a pure value, and `name = op(field: e, ...)` a call. A call's argument is the record of its named arguments. Each name is bound once, and the order of nodes in the file has no meaning.
- **Graph checks**, before anything is typed, as in cronch-gleam's verifier:
  - names are unique;
  - every reference resolves to the argument, a node or a `type`;
  - the graph is acyclic.
- **Guards:** `name = op(...) when cond` starts the call only if `cond` holds once its dependencies have settled. Otherwise the slot is `skipped`.
- **Settling:** a call node is settled once it has returned, failed, been refused or been skipped. A pure node is settled once everything it reads has settled.
- **Reading a node:**
  - Its name, used as a value, is what it returned.
  - A node that reads a skipped node is skipped too.
  - `match node { ok(v) => ..., failed(e) => ..., refused => ..., skipped => ... }` reads its outcome instead. It must cover `ok`, and `skipped` if the node can be skipped, or have a `_` arm.
  - A call node that fails or is refused ends the run through `otherwise`, unless every read of it is a `match` with arms for both, or a `_`. A node nothing reads ends the run too, since nothing handles its failure.
- **`return expr`**, once per harness, is `halt(expr)`, emitted once every call, and everything `expr` reads, has settled. `return` may not read, as a value, a node that can be skipped (one with a guard, or that reads such a node). It must `match` it.
- **`otherwise return expr`** is what a failure or refusal read as a value halts with. Without it, the run halts with `{"failed": name}`. `expr` can't read nodes.
- **Types:** `String`, `Int`, `Float`, `Bool`, `Any`, `List[T]`, records as `{field: T}`, and named record and variant types declared with `type`. A `|` of tagged records is a variant. The names are the kernel's.
- **Expressions** are the pure fragment, mapped one-to-one onto `Expr`:
  - literals, names, field access (`r.text`), records (`{text: s}`), lists;
  - the operators in `ops.rs`, with infix spelling where there is one (`a + b`, `a == b`, `x in xs`), and calls otherwise (`len(xs)`, `prefix(p, s)`, `str(x)`);
  - `and` and `or`, which short-circuit, so they become `If` rather than the kernel's strict `and` and `or`;
  - `if c then a else b`;
  - `match` on a variant onto `Match`, and on literals onto a chain of `If`;
  - `fold xs from init with (acc, x) => body` onto `Fold`.

### Streams, messages and cancellation (S2a)

- **Streams:** `chunks = op(...)` on a streaming operation collects its items in order and settles at `end`, with the list as its value. Reading a stream before it ends is left until a harness needs it.
- **Messages:** `stop = receive said from session.user when said == "stop"` starts listening once the run starts, and settles at the first message on the channel that passes its filter, with that message as its value.
  - The filter reads the message and the parameter, never a node, so it can be decided when the message arrives.
  - A message nobody is listening for changes nothing.
  - A receive nothing reads doesn't keep the run going: the run halts once every call, and everything `return` reads, has settled.
- **Cancellation:** `reply = model.generate(...) until stop` cancels the call once `stop` returns. The slot fails with `{"cancelled": true}`, whether the call was running or still waiting, and is handled like any other failure.
  - The kernel's `cancel` of a call is a request to the world, and the call stays pending. The elaborator settles the slot itself, and any result that still arrives finds no slot running its call.
  - That is enough for the Claude Code exercise's P4, "an interrupt stops effects until the next prompt".

### Fan-out and spawning (S2b)

- **Fan-out:** `notes = for doc in docs: model.generate(text: doc)` issues one call per item of a list, in one step. Its value is the list of their results, in the list's order.
  - It settles when every call has, and fails if any call failed or was refused; `match` reads that outcome like any other. An empty list settles at once.
  - The calls get consecutive identifiers, so the slot keeps the first, and `items`, each call's kind. The number of calls is bounded by the list, not the graph.
  - Only operations with one result fan out, and a fan-out can't be cancelled with `until`.
  - The kernel has no typed empty list, so the slot's lists are rebuilt as lists of `Any` when a result arrives. The source's own expressions still see the node at `List[T]`.
- **Spawning:** `summary = spawn summarizer(draft) only model.generate without fs.write` is a call node filled by a child run, with the kernel's spawn semantics. Its value is `Any`, as in the kernel.
  - `only` and `without` are the kernel's `only` and `drop` narrowings, so the child can never do what the parent couldn't.
  - The target is bound in the header: `uses summarizer = harness "harness:sha256-…"` names a registered harness, so the body never carries a key. A `spawn` action carries the child's whole term, so `elaborate` takes the terms of the harnesses the header names, as it takes the contracts of capabilities, and returns the bindings as `harnesses`.
  - The child is checked against the narrowed interface when the parent is elaborated, so a child that would be refused at run time is reported with its problems instead.
- **Iteration across steps** (a ReAct loop) is the one thing a fixed graph can't say. See the open questions.

### Properties and invariants (S3)

- **`property name:`** gives `□φ` with `φ` written as an expression:
  - `implies`, `and`, `or`, `not`;
  - `prev`, `since`, `once`, `historically`;
  - `exists x in e:` and `forall x in e:`, bounded by the trace as the kernel requires.
- **Event helpers** spell the common atoms: `called(op, ...)`, `returned(op, ...)`, `failed(op)`, `effect()`, `msg(channel)`, `labelled(...)`. A trailing expression filters on the event's fields, as `value in [...]` does above.
- **Node helpers** read the slots in the state each event carries: `ok(saved)`, `skipped(saved)`, `saved.value`. This is what decision 6 allows.
- **Guards are explicit.** The elaborator puts every field access behind a guard atom, so a property can't be vacuously violated by reading a field an event doesn't have. That is gap 5 of the design session.
- **`invariant name: expr`** over node slots becomes an invariant certificate's expression over the generated state. With one slot per node, the invariants worth stating, such as "`saved` never runs unless `decision` returned something other than `deny`", are short.

### Source maps (S3)

- `elaborate` returns, with the terms, a map from each node and each `Expr` path to a source span.
- Counterexamples, refusals and monitor verdicts can then be reported as source lines: which node was waiting, which line made the call. The kernel API gains no new verdicts; the map is enough for a caller to translate.

## Elaboration

- **Input:** the source, the contract of each capability its header names, and the term of each harness it names, by identifier. Installed under the header's names, they type each call node, so the generated state is typed, and proofs must exclude `Any`.
- **Output:** `{harness, uses, harnesses, properties, invariants, sourceMap}`, or a list of problems, each with a span. `uses` is the header's capability bindings, `[{name, capability}]`: what to install, and under which prefix. `harnesses` is its harness bindings, `[{name, harness}]`.
- **The generated state** is a record:
  - `arg`, the start value;
  - `next`, the call counter;
  - `nodes`, a record with one slot per call or receive node. A slot is a record `{kind, call, value, error}`. `kind` is `waiting`, `running`, `ok`, `failed`, `refused` or `skipped`, and says which of the other fields mean anything. `value` is the operation's output, a stream's items as a list, or the message.
  - A slot is a record rather than a variant because the kernel has no expression that builds a value at a variant type: a record literal never conforms to one. `value` is carried from slot to slot, never rebuilt from a literal, so it keeps its type in every branch.
- **The generated `δ`** is one pass:
  1. **Record:** `start` stores `arg`. `ret`, `failed` or `refused` for call `k` settles the slot running `k`; on a stream, `ret` appends an item and `end` settles it. A message settles every receive listening on its channel whose filter it passes. An input for a call no slot is running (only possible after a cancel) changes nothing.
  2. **Cancel:** every call whose `until` node has returned fails as cancelled, and `cancel` is emitted for it if it was running.
  3. **Fail:** if a node whose failure isn't handled has failed or been refused, halt through `otherwise`.
  4. **Schedule:** for each waiting call or receive node, in a topological order, whose dependencies have all settled:
     - if one read as a value was skipped, or the guard is false, mark the slot `skipped`;
     - otherwise emit a `call` with identifier `next`, mark the slot `running`, and add one to `next`. A receive is marked `running` and emits nothing.

     Each node is a `Let` over the record the previous one produced, so nodes settled in this pass are seen by the nodes after them. Pure nodes are `Let`s over the slots they read, so the pass needs no `fold`.
  5. **Return:** once every call, and everything `return` reads, has settled, emit `halt` with `return`'s value.
- **Checking:** the kernel checks the generated harness (`checkHarness`) and each property, as for hand-written terms. A problem there is an elaborator bug, and is reported as one.

## Pieces

All in this repo except S4. Each is a PR with compliance vectors, and S1 to S3 together are `@cronch/kernel` v0.4.0.

- **S1, harnesses:**
  - `crates/surface`: the lexer, the parser, the graph checks, and elaboration of nodes, guards, `match` on outcomes, `return` and `otherwise`.
  - The `elaborate` request.
  - Vectors: each platform fixture harness written in source elaborates to a term whose runs match the hand-written one's on the same inputs.
- **S2a, streams and messages:** streams, `receive`, `until`.
  - Acceptance: the Claude Code exercise's single turn, with an interrupt channel, elaborates and runs. P4 holds on its monitored runs and up to a depth, and is refuted for the same turn without `until`. The loop across turns waits for open question 1.
- **S2b, fan-out and spawning:** `for` fan-out and `spawn`.
- **S3, properties:** properties, invariants and source maps, with explicit guards and node helpers.
  - Vectors: each platform fixture property in source elaborates to a formula with the same verdicts.
- **S4, the platform:** `POST /objects/harness` accepts `{ source, capabilities }` as well as a term.
  - The gateway looks up the contract of each capability the header names, elaborates through `KERNEL`, registers the harness, its bindings and its properties, and stores the source. A run installs the harness's bindings rather than choosing its own.
  - The platform gives a model writing a harness its owner's catalog: each capability's identifier, a suggested name, and its contract.
  - Problems come back with spans. Audit and evidence responses gain source lines through the source map.
  - The integration test's fixtures move to source.

## Open questions

1. **Iteration across steps.** A graph runs each node at most once, so a ReAct loop, which calls until the model says it's done, can't be a graph. Three candidates:
   - a `loop` block: a subgraph run once per round, carrying a declared record from one round to the next, until a condition holds. Its slot holds the round number and the carried record;
   - a harness that spawns itself with the carried record. This is already in the kernel, but each round adds a child to the run tree;
   - leaving the loop to the host, which starts a new run per turn.

   `loop` is the likeliest. It is postponed past v0.4.0; until then, a host that needs a loop starts a run per turn.
2. **Failure by default.** A node that fails, read as a value, ends the run through `otherwise`. That is the least surprising default for a total `δ`, and declaring `otherwise` keeps the policy in the harness rather than the language. Whether a harness should also be able to say "retry" is left until a real one needs it.
3. **Versions.** A binding names one identifier, and a module's key pins its exact code. A platform capability's identifier pins nothing, so a contract can change under a registered harness. Whether identifiers should carry versions, as cronch-gleam's `{id, version}` references did, is the platform's call, and the kernel needs no change either way.
4. **Labels on control state.** The interpreter labels a step's whole input, `kind`, `call` and `op` included, with its result's sources. So a step taken on the model's result runs under the label `model`, and so does every slot it writes. A later step decides what to do from those slots, so its actions and its halt carry `model` too. A hand-written harness that keeps no control state doesn't have this problem. In S1's `approved-note`, the denied run halts with `"denied"` labelled `model`, where the hand-written one's is trusted.
   - Labels only ever over-approximate, so this loses precision, not safety. But a property about information flow, such as "no untrusted data reaches a send", could be refuted for an elaborated harness and hold for the hand-written one.
   - The fix is in the kernel: treat the interpreter's own fields of an input (`kind`, `call`, `op`) as trusted, and label only `value` and `error`. Whether _that_ a result arrived, and when, is information about the untrusted source is the question to decide first.
5. **Surface for models.** Each construct is something a model can get wrong. The graph form has fewer of them than Python did, but it's less familiar. Measure how often models write valid harnesses before adding sugar.
