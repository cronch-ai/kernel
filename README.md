# kernel

The reference kernel for Pata (Proven Agents, Trusted Actions), the open format for agent harnesses whose safety can be proved. A harness's rules are proved over its capabilities' contracts, so they hold whatever the model says. Cronch is one platform that runs Pata.

## Quickstart

With Node 24 and nothing else:

```console
$ npx @cronch/kernel run
task: Fetch https://example.com/ and tell me its title.
model: chooses net.fetch
ask: allow https://example.com/? [y/N] y
ask: allowed
net.fetch: https://example.com/
model: answers
{"kind":"halted","label":["model"],"value":"Its title is \"Example Domain\"."}
```

That ran T1, a Claude Code-style harness ([examples/claude-code](examples/claude-code)), on your machine. The model chose to fetch a URL, and the harness asked you first. Answer `n` and nothing is fetched. `--input "<task>"` gives it another task, and `--model <module.js>` plugs in your own model.

The ask isn't the harness being polite. It's a rule, proved over the contracts of `net` and `shell`: **`net.fetch` and `shell.online` can't happen until a person has allowed, through `net.ask`, the URL they reach.** Any harness on those contracts keeps it, whatever its model says. Run `npx @cronch/kernel run <dir>` on a directory of your own harness and contracts.

### Checking the proof yourself

- [app.cronch.ai/t1](https://app.cronch.ai/t1) shows the rule, the statements its proof had to prove, and the verdicts, each with what you need to check it again.
- Or check it here, using no part of the kernel but the statements it writes. cvc5 proves each of the rule's queries `unsat` and Ethos checks the proofs, and Lake's `comparator` checks the same rule's proofs in Lean. With the toolchain in [Development](#development), and `bwrap` for `comparator`:

  ```sh
  ./scripts/install-proof-tools.sh
  ./scripts/install-lean.sh
  npm run build
  node --test proofs/t1.test.mjs
  ```

  The test also breaks the contracts (`net.fetch` ungated, or `net.ask` allowing whatever the person says) and finds a counterexample each time.

## What the kernel does

The kernel gives agent harnesses a formal semantics. A harness is a state machine whose step function is data, not code: the term that is hashed is the term that is checked, run and proved. The kernel interprets harnesses against capabilities whose contracts it enforces, monitors their runs against safety properties, and reaches verdicts on those properties as evidence anyone can re-check. Language models, people and other agents are never trusted: they are capabilities whose results a proof quantifies over. The kernel does no I/O and keeps no state. It is written in Rust, compiled to WebAssembly and published as `@cronch/kernel`.

### Model

- **Harness**: `(A, S, s0, step)`. `step` is a pure expression over `state` and `input` that gives the next state and a list of actions: `call`, `spawn`, `cancel` or `halt`. Calls run concurrently and their results arrive in any order.
- **Capability**: operations and channels with contracts. A precondition (`pre`) is checked by the interpreter at the point of the call, whatever `step` does. A postcondition (`post`) and a channel invariant (`inv`) filter what arrives. `ret` gives the label of results. `effect` and `footprint` record what a call may change. There are no built-in operations.
- **Grant state**: held by the interpreter, not the harness, one per run. Every run starts with the empty value of its type, and nothing pre-fills it. Preconditions read it. Only trusted results and messages widen it, through `onResult` and `onMsg`, and they widen the run that received them and its ancestors, never its siblings. Each capability declares the part it reads and writes (`grant`), and an update gives only the fields it changes, so one capability's update leaves another's part as it was. An approval allowlist is one example.
- **Types**: structural, with records, lists and variants. A harness's input is a variant typed by its interface, which `match` reads case by case.
- **Spawning**: a child runs against its parent's interface narrowed by combinators that can only drop, restrict or distrust. A child can never do what its parent couldn't. Cancelling the spawn revokes the child. A spawn names a declared child, and carries its harness and narrowing as text, since a term has no Cronch type.
- **Labels**: every value carries the untrusted sources it may depend on, including through control flow. Of an input, only what arrived (its `value`, or a failure's `error`) carries its sources: which call it answers is the interpreter's. Preconditions and properties can read them.
- **Properties**: `always(phi)`, with `phi` in first-order past-time temporal logic over the events of a run, each read as a view (`view`), and checked against the harness and interface as a variant over the event's `kind` (`eventType`), so an atom reads a case's fields after `match`. A quantifier ranges over the items of lists the events give. Every property is a safety property, monitored online or offline. Quantifier-free properties also have an incremental monitor (`monitorStart`, `monitorStep`) whose small state a host keeps between deliveries.
- **Evidence**: a verdict on a property, reached by the kernel itself:
  - by replaying a counterexample;
  - by searching every run to a depth;
  - by exploring every reachable configuration;
  - by checking an invariant certificate.

  Evidence names its harness, interface and property by content key, and lists the assumptions it rests on. Every run starts with the empty grant, so evidence names none.
- **Induction**: for an invariant of a harness's state, the kernel decides initiation and writes consecution as an SMT-LIB query. A solver searches for counterexamples, and the kernel checks any it finds. A solver's `unsat` is never a verdict.
- **Rules**: what a capability's contracts alone rule out, whatever a harness does and whatever a model says: an operation can't happen until a person has approved it. The kernel writes the queries whose `unsat` shows it (`ruleObligations`), or, with `format: "lean"`, the Lean statements that they are `unsat`. Those need a proof checked outside the kernel: [proofs/](proofs/) is one way. The design is in [Rules](docs/2026-11-05-rules.md).
- **Keys**: SHA-256 of the deterministic CBOR (RFC 8949 section 4.2.1) of an object, prefixed by its kind and version. Entries of a run's log are keyed too (`logEntry`), so a host can hash-chain them in the same encoding, and so is capability code a host sandboxes (`module`), which labels and evidence can then name.

Terms are defined briefly in the [glossary](docs/glossary.md). The definitions are in [harness.rs](crates/kernel/src/harness.rs), [run.rs](crates/kernel/src/run.rs), [property.rs](crates/kernel/src/property.rs), [evidence.rs](crates/kernel/src/evidence.rs) and [induction.rs](crates/kernel/src/induction.rs), and in [index.d.ts](javascript/index.d.ts) for TypeScript. Each module explains its semantics in its doc comment. The design and its open questions are in [docs/](docs/). Worked examples are in [crates/kernel/tests/](crates/kernel/tests/).

## Layout

- `crates/kernel`: the library. Requests enter through `api::call` (CBOR in, CBOR out).
- `crates/surface`: the surface language and its elaborator, which turns a harness written as a graph of named calls into a harness term. It answers the `elaborate` request and passes every other request to the kernel. It is untrusted: the kernel checks what it produces. The design is in [the surface language](docs/2026-11-01-surface-language.md).
- `crates/kernel-abi`: the WASM exports `cronch_alloc`, `cronch_call` and `cronch_free`, through `surface`.
- `javascript`: the JS loader, the CBOR codec and the TypeScript types. `@cronch/kernel/run` is a local host for Node: `run(harness, capabilities, handlers)` runs a harness with each operation answered by a JavaScript function, keeping the [host protocol](docs/2026-11-06-host-protocol.md).
- `compliance`: cross-implementation test vectors, replayed by both the Rust crates and the WASM build.
- `proofs`: proves a rule's queries with cvc5 and checks the proofs with Ethos, using no part of the kernel but its queries. `check-lean.mjs` checks a Lean proof of a rule's Lean statement with Lake's `comparator`, which needs `bwrap`.

## Development

You need the toolchain in `rust-toolchain.toml`, Node 24, and `wasm-opt` (installed by `scripts/install-binaryen.sh`). The dev container provides all of these.

```sh
npm ci
cargo test --workspace
npm run build                  # javascript/kernel.wasm + kernel.sha256
npm test --prefix javascript
npm run lint

./scripts/install-proof-tools.sh  # cvc5 and Ethos, in .proof-tools
./scripts/install-lean.sh         # Lean, in .proof-tools/lean, for lean/
npm run test:proofs              # after npm run build
```

Pushes to `main` publish a GitHub release of the version in `javascript/package.json`, and publish its tarball to npm as `@cronch/kernel`. A push that doesn't change the version skips the release, since that release exists already: bump the version to publish. npm is checked separately, so a version missing there is published from its release's tarball on the next push.

npm publishes by [trusted publishing](https://docs.npmjs.com/trusted-publishers): npm trusts `release.yml` in this repo, so no npm token is stored anywhere. npm can't trust a workflow for a package that doesn't exist yet, so the package's first version was published once by hand, and the trusted publisher was set in the package's settings.

## License

Apache 2.0
