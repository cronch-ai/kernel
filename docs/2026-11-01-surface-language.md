# The surface language

Plan for the Python-like surface language and its elaborator, listed as later work in the [harness design session](2026-11-01-harness-design-session.md) and in [Kernel architecture](https://linear.app/cronch-ai/document/kernel-architecture-harness-semantics-capabilities-properties-and-be2d9d7b9bcd). Tracked in Linear under the "Cronch v1" project.

Today a harness is a JSON `Expr` term and a property a JSON `Formula`. The platform's `approved-note` fixture is about 150 lines of JSON for what reads as ten lines of Python, and a property is harder still. Nobody but the kernel's own tests should have to write either.

## The idea: wishful thinking

A harness is written as though everything it needs already exists. `draft = model.generate(...)` is a wish: the author states what they need, a value of the operation's output type that meets its contract, and the next line uses it.

**Wishes are capability calls.** That makes the code read imperatively, but the meaning stays declarative:

- **A wish states a need, not how it is met.** What `model.generate` may do and return is its contract. Who fills it (a model, a person through `approve`, a sandboxed module, a child harness) is the installation and the grant, never the harness code.
- **What runs is still the pure `δ`.** The elaborator turns sequential code into `δ`'s state machine:
  - Each wish becomes a `call` action and a point to resume at.
  - The state becomes "where the run is waiting" plus the variables it still needs.
  - The `ret`, `failed` and `refused` handling is generated.

  This is how `async`/`await` compiles: every wish is a suspension point. The author never writes `match input` or call identifiers.

Everything the kernel proves is about the elaborated `Expr`, which is the term that is hashed, proved and run. The elaborator is untrusted, as [Harness lifecycle](https://linear.app/cronch-ai/document/harness-lifecycle-write-register-prove-run-audit-f8f7aa476e2c) already says: a bug in it can make a harness do something other than what its source suggests, but never something its contracts, grant or proved properties forbid.

## Decisions

1. **The first version covers harnesses and properties.** A file declares the harness (its argument, its state, its body) and the properties and invariants claimed about it. Contracts stay JSON and JS: the platform writes them for its own capabilities, and sandboxed modules return them from `contract()`.
2. **The elaborator lives in this repo, in Rust.** A new crate, `crates/surface`, depends on `crates/kernel` for `Expr`, `Type` and `infer`. It ships in the same `@cronch/kernel` wasm as an `elaborate` request, so the gateway, a CLI and an editor all use one definition. That's the reasoning that put the `module` key kind in the kernel.
   - `crates/kernel` stays the trusted core; nothing in it depends on `surface`.
   - The `cronch-ai/lang` repo, created empty on 2026-11-01, isn't used.
3. **The parser is hand-written, with no new dependencies.** The language is a small subset of Python's syntax, and anything outside it is rejected with a source span, never approximated. A hand-written lexer (with indentation) and recursive-descent parser keep the wasm small (972 KB today) and the errors precise. Python parser crates were considered: the maintained one (`ruff_python_parser`) isn't published on crates.io, and any of them accepts far more than the subset only to reject it afterwards.
4. **Calls are counted in the state.** `δ` sees a call's identifier only on its result. Identifiers are handed out in order, one per `call` or `spawn` action, refused or not, so the generated state keeps its own counter and records which resume point each pending call belongs to. This needs no kernel change.
   - The alternative was to give `δ` the next identifier as a third variable. That changes the step rule, the SMT-LIB export and every proof for one convenience.
   - If the counter ever disagreed with the interpreter, the harness would resume at the wrong point. That breaks the author's intent, not a contract: preconditions are still checked at every call.
5. **No source object in the kernel yet.** The harness's key is the key of the elaborated term. `elaborate` returns a source map with it, and the platform stores the source next to the harness it registers. Elaboration is deterministic for a given elaborator version, so anyone with the source and the same `@cronch/kernel` can recompute the term.

## The language

A sketch, to fix the shape. Each piece below names the PR that builds it.

```python
harness note(prompt: str) -> str:
    reply = model.generate(messages=[{"role": "user", "content": prompt}])
    match approve.ask(action="notes.append", args={"text": reply.text}):
        case "once" | "always":
            notes.append(text=reply.text)
            return reply.text
        case _:
            return ""

property no_unapproved_writes:
    always(called("notes.append") implies once(returned("approve.ask")))
```

### Harnesses (S1)

- **`harness name(arg: T) -> R:`** declares the harness. `arg`'s type is `A`. `return v` is `halt(v)`.
- **Types** are written as Python annotations: `str`, `int`, `float`, `bool`, `list[T]`, `dict` literals as records (`{"text": str}`), `Any`, and named record and variant types declared with `type`. A `|` of tagged records is a variant.
- **Expressions** are the pure fragment, mapped one-to-one onto `Expr`:
  - literals, names, field access (`r.text`), records, lists;
  - the operators in `ops.rs`, in Python spelling (`a + b`, `a // b`, `len(xs)`, `x in xs`, `s.startswith(p)` for `prefix`, `str(x)`);
  - `and` and `or`, which short-circuit as in Python, so they become `If` rather than the kernel's strict `and` and `or`;
  - conditional expressions;
  - `match` with class-like patterns onto `Match`, and with literal patterns onto a chain of `If`.
- **Statements:** assignment (rebinding a name shadows it), `if`/`elif`/`else`, `match`, `return`, `pass`. Rebinding inside a branch joins at the end of the branch.
- **Calls are wishes.** `x = op(arg=...)` is a call of the operation `op` with a record of its keyword arguments. Its value is what `ret` delivers.
- **Failure:** a call can fail or be refused. `match op(...)` with `case failed(e)` and `case refused()` handles that explicitly. A call used any other way, if it fails or is refused, ends the run with `halt({"failed": op})`.
  - The kernel has no exceptions, so `try`/`except` isn't offered. It would suggest exception semantics the harness doesn't have.
- **Loops:**
  - `for x in xs:` whose body makes no call becomes a `fold`, so it is bounded by `xs`.
  - A `for` or `while` whose body makes a call on every path is a loop across steps: each turn suspends, so each `δ` step still ends and only the run goes on. That is what makes a ReAct loop (`while not done:`) natural.
  - A loop that can go round without making a call is rejected, since `δ` must terminate.
- **Rejected outright:** `def` (no functions besides the harness, so no recursion), `while` without a call, `try`, `with`, `global`, `import`, `lambda`, classes, comprehensions with calls in them, and any name the harness doesn't bind or the interface doesn't have.

### Concurrency, streams and messages (S2)

- **`gather(a(...), b(...))`** issues the calls in one step and resumes when every one has ended. Its value is a tuple of their results, each a variant over `ok`, `failed` and `refused`, since one failure shouldn't hide the others.
- **Streams:** `for chunk in op(...):` over a streaming operation runs the body for each item `ret` delivers and leaves the loop on `end`. The body can't wish, since another item may arrive first.
- **Messages:** `on msg channel as m:` handlers sit next to the body.
  - A handler runs when a message arrives on its channel, whatever the body is waiting for. A message on a channel with no handler changes nothing.
  - Handlers and the body share only the fields declared in a `state:` block. The body's locals are its own.
  - A handler may assign state fields, make calls, and `cancel` the body's current wish. A cancelled wish fails, which the body handles like any other failure.
  - That is enough for the Claude Code exercise's P4, "an interrupt stops effects until the next prompt".
- **`spawn(harness, narrow, arg)`** is a wish filled by a child run, with the kernel's spawn semantics. Its result is `Any`, as in the kernel.

### Properties and invariants (S3)

- **`property name:`** gives `□φ` with `φ` written as an expression:
  - `implies`, `and`, `or`, `not`;
  - `prev`, `since`, `once`, `historically`;
  - `exists x in e:` and `forall x in e:`, bounded by the trace as the kernel requires.
- **Event helpers** spell the common atoms: `called(op, **args)`, `returned(op)`, `failed(op)`, `effect()`, `msg(channel)`, `labelled(...)`.
- **Guards are explicit.** The elaborator puts every field access behind a guard atom, so a property can't be vacuously violated by reading a field an event doesn't have. That is gap 5 of the design session.
- **Properties read only declared `state:` fields,** never the generated parts of the state, so a property means the same thing whatever the elaborator generates.
- **`invariant name: expr`** over declared state fields becomes an invariant certificate's expression over the generated state. Invariants at a particular resume point (over the body's locals there) are left for later; they need the elaborator to say which resume points exist, in source terms.

### Source maps (S3)

- `elaborate` returns, with the terms, a map from each resume point and each `Expr` path to a source span.
- Counterexamples, refusals and monitor verdicts can then be reported as source lines: which wish was waiting, which line made the call. The kernel API gains no new verdicts; the map is enough for a caller to translate.

## Elaboration

- **Input:** the source, and the interface the harness will run against. The interface types each wish's result, so the generated state is typed, and proofs must exclude `Any`. A wish of an operation the interface lacks is an elaboration error, not a refusal at run time.
- **Output:** `{harness, properties, invariants, sourceMap}`, or a list of problems, each with a span.
- **The generated state** is a record:
  - the declared `state:` fields;
  - the call counter;
  - the pending calls, each with its resume point;
  - `at`, a variant over resume points, each case holding the locals live there.
- **The generated `δ`** matches on the input, finds the resume point the result belongs to, binds the result and that point's locals, and runs the source from there to the next wish or `return`. A result for a call the state doesn't know (only possible after a cancel) changes nothing.
- **Checking:** the kernel checks the generated harness (`checkHarness`) and each property, as for hand-written terms. A problem there is an elaborator bug, and is reported as one.

## Pieces

All in this repo except S4. Each is a PR with compliance vectors, and S1 to S3 together are `@cronch/kernel` v0.4.0.

- **S1, harnesses:**
  - `crates/surface`: the lexer, the parser and elaboration of the pure fragment, wishes, failure handling and loops.
  - The `elaborate` request.
  - Vectors: each platform fixture harness written in source elaborates to a term whose runs match the hand-written one's on the same inputs.
- **S2, concurrency:** `gather`, streams, `on msg` handlers, shared `state:`, `cancel`, `spawn`.
  - Acceptance: the Claude Code exercise's ReAct loop, with an interrupt channel, elaborates. It runs against the kernel's test interface, and P4 holds on its monitored runs.
- **S3, properties:** properties, invariants and source maps, with explicit guards.
  - Vectors: each platform fixture property in source elaborates to a formula with the same verdicts.
- **S4, the platform:** `POST /objects/harness` accepts `{ source, capabilities }` as well as a term.
  - The gateway builds the interface, elaborates through `KERNEL`, registers the harness and its properties, and stores the source.
  - Problems come back with spans. Audit and evidence responses gain source lines through the source map.
  - The integration test's fixtures move to source.

## Open questions

1. **Typing without an interface.** Elaborating against an interface ties a harness's term to the capabilities it was written for. The alternative is for the harness to declare what it wishes for (`uses model.generate`), with types from a registered contract, so it can be registered before any run's interface exists. Start with the interface, and revisit when the platform needs to register harnesses on their own.
2. **Failure by default.** A wish that fails, used outside a `match`, ends the run. It's the least surprising default for a total `δ`, but it's a policy built into the language.
3. **How far "Python-like" stretches.** Comprehensions, f-strings and slicing are easy to elaborate and feel like Python. Each adds surface for models to get wrong. Add them when a real harness needs them, not before.
