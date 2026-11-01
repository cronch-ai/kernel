# kernel

Rust implementation of the Cronch kernel, compiled to WebAssembly and published as `@cronch/kernel`.

The kernel gives agent harnesses a formal semantics. A harness is a state machine whose step function is data, not code: the term that is hashed is the term that is checked, run and proved. The kernel interprets harnesses against capabilities whose contracts it enforces, monitors their runs against safety properties, and reaches verdicts on those properties as evidence anyone can re-check. Language models, people and other agents are never trusted: they are capabilities whose results a proof quantifies over. The kernel does no I/O and keeps no state.

## Model

- **Harness**: `⟨A, S, s₀, δ⟩`. `δ` is a pure expression over `state` and `input` that gives the next state and a list of actions: `call`, `spawn`, `cancel` or `halt`. Calls run concurrently and their results arrive in any order.
- **Capability**: operations and channels with contracts. A precondition (`pre`) is checked by the interpreter at the point of the call, whatever `δ` does. A postcondition (`post`) and a channel invariant (`inv`) filter what arrives. `ret` gives the label of results. `effect` and `footprint` record what a call may change. There are no built-in operations.
- **Grant state**: held by the interpreter, not the harness, one per run. Preconditions read it. Only trusted results and messages widen it, through `onResult` and `onMsg`, and they widen the run that received them and its ancestors, never its siblings. An approval allowlist is one example.
- **Types**: structural, with records, lists and variants. A harness's input is a variant typed by its interface, which `match` reads case by case.
- **Spawning**: a child runs against its parent's interface narrowed by combinators that can only drop, restrict or distrust. A child can never do what its parent couldn't. Cancelling the spawn revokes the child.
- **Labels**: every value carries the untrusted sources it may depend on, including through control flow. Preconditions and properties can read them.
- **Properties**: `□φ`, with `φ` in first-order past-time temporal logic over the events of a run. Every property is a safety property, monitored online or offline.
- **Evidence**: a verdict on a property, reached by the kernel itself:
  - by replaying a counterexample;
  - by searching every run to a depth;
  - by exploring every reachable configuration;
  - by checking an invariant certificate.

  Evidence names its harness, interface, grant and property by content key, and lists the assumptions it rests on.
- **Induction**: for an invariant of a harness's state, the kernel decides initiation and writes consecution as an SMT-LIB query. A solver searches for counterexamples, and the kernel checks any it finds. A solver's `unsat` is never a verdict.
- **Keys**: SHA-256 of the deterministic CBOR (RFC 8949 §4.2.1) of an object, prefixed by its kind and version.

The definitions are in [harness.rs](crates/kernel/src/harness.rs), [run.rs](crates/kernel/src/run.rs), [property.rs](crates/kernel/src/property.rs), [evidence.rs](crates/kernel/src/evidence.rs) and [induction.rs](crates/kernel/src/induction.rs), and in [index.d.ts](javascript/index.d.ts) for TypeScript. Each module explains its semantics in its doc comment. The design and its open questions are in [docs/](docs/). Worked examples are in [crates/kernel/tests/](crates/kernel/tests/).

## Layout

- `crates/kernel`: the library. Requests enter through `api::call` (CBOR in, CBOR out).
- `crates/kernel-abi`: the WASM exports `cronch_alloc`, `cronch_call` and `cronch_free`.
- `javascript`: the JS loader, the CBOR codec and the TypeScript types.
- `compliance`: cross-implementation test vectors, replayed by both the Rust kernel and the WASM build.

## Development

You need the toolchain in `rust-toolchain.toml`, Node 24, and `wasm-opt` (installed by `scripts/install-binaryen.sh`). The dev container provides all of these.

```sh
npm ci
cargo test --workspace
npm run build                  # javascript/kernel.wasm + kernel.sha256
npm test --prefix javascript
npm run lint
```

Pushes to `main` publish a GitHub release once `javascript/package.json` is at version 1.0 or higher.

## License

Apache 2.0
