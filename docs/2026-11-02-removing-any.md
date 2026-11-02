# Removing `Any`

Plan for taking `Any` out of the kernel's types, agreed on 2026-11-02 while building the [surface language](2026-11-01-surface-language.md).

`Any` is the kernel's gradual type: it conforms both ways, so whatever has it passes the checker and is checked, if at all, when a value arrives. That was the right start: it let the interpreter, contracts and properties be written before every type was known. It now costs more than it saves:

- **Proofs can't see through it.** SMT has no sort for `Any`. An induction obligation refuses a state that holds one outside a variant, and drops the cases of a step's input that hold one, so a harness that keeps a failure's error or a child's result in its state can't be proved by induction at all.
- **It hides mistakes.** A field read through `Any` is checked only when it runs, so a misspelt field in a contract or a step is found by a run that fails, not by `checkHarness`.
- **It forces workarounds in generated code.** The surface language's slots are records, not variants. Its fan-out lists are `List(Any)` once a result arrives, and its slot errors are stored as text. Each of these exists only because of where `Any` enters.

## Where `Any` comes from

| Source                        | Where                            | What it is                                                                                                                |
| ----------------------------- | -------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| No way to introduce a variant | `check.rs`, `types.rs`           | A record literal has a record type, which never conforms to a variant, so no expression builds a value at a variant type. |
| Empty and null literals       | `types.rs` (`of_value`)          | `[]` is `List(Any)`, and `null` is `Any`.                                                                                 |
| Partial field reads           | `check.rs` (`Get`)               | Reading a field only some cases of a variant have gives `Any`, checked when it runs.                                      |
| A failure's error             | `harness.rs` (`input_type`)      | `failed`'s `error` is `Any`: whatever the world said.                                                                     |
| A child's result              | `harness.rs` (`input_type`)      | `ret` from `spawn` is `Any`, since the child is chosen at run time and a harness declares no result type.                 |
| Actions                       | `harness.rs` (`step_type`)       | `δ` gives `actions: List(Any)`, checked one by one when processed.                                                        |
| The grant state               | `harness.rs` (`check_interface`) | Contracts read `grant` as `Any`; no capability declares what it keeps there.                                              |
| Events                        | `property.rs`                    | A property's atoms read `event` as `Any`.                                                                                 |
| Harnesses checked alone       | `harness.rs` (`check_harness`)   | Without an interface, a step's input is `Any`.                                                                            |
| Contracts                     | the capabilities themselves      | `approve.ask` takes `args: Any`, the arguments of whichever action it is asked about.                                     |
| The checker's recovery        | `check.rs`                       | After reporting a problem, the checker carries on with `Any`. This one stays: it never reaches a term that checks.        |

They are three kinds of thing:
- a gap in the checker (the first three);
- types the interpreter makes up (the next six);
- dynamic data a contract really means (the last but one).

Each kind has its own replacement.

## The plan

### A1: check against an expected type

Bidirectional checking: an expression is checked against the type it must have wherever one is known, and inferred only where none is. Expected types come from:
- a harness's state type;
- an operation's input type, for a call's arguments;
- a step's type, `{state, actions}`;
- a `let`'s bound value, or a fold's accumulator, when it is annotated.

Checked against a type:
- **A record literal against a variant** is that variant's case when its tag field is a string literal naming one: `{kind: "ok", value: v}` against `waiting | ok {value: T} | …` is the `ok` case, and `v` is checked against `T`. This is variant introduction, the missing rule.
- **`[]` against `List(T)`** is a `List(T)`, and so is any list literal whose items each check against `T`, and an `append` whose list and item check against it.
- **Annotations:** `let` and `fold` gain an optional `type`, for the bound value and the accumulator. A sequential step builds its state in `let`s, where nothing else would say what type is expected, so without them variant introduction would reach only the last expression. An absent annotation isn't serialized, so every existing term keeps its key.

Partial field reads stay `Any` in A1, and `null` stays `Any`. Both become errors in A3, when there is no `Any` to give them, except `null` at `Json`.

A1 changes no term's meaning, and a term that checks today still checks, so it is a minor version. The surface language can then make slots variants, keep fan-out lists typed, and type its actions.

### A2: type what the interpreter makes up

- **Errors:** `failed`'s `error` gets a fixed type, a variant of the kernel's own (`cancelled`, `revoked`) and the world's, whose detail is `Json` (A3). A failure the world delivers with the wrong shape is withheld, as a result of the wrong type already is.
- **Results:** a harness declares `result: R`, and `halt` with a value outside `R` is a step failure. A harness declares the children it may spawn by name, with their argument and result types. A `spawn` action names one, the interpreter checks the child's term against that signature when it starts, and its result arrives as `ret` on the operation `spawn.<name>`, at that type.
- **Actions:** `actions` is `List(Action)`, with `Action` a variant built from the interface: one `call` case per operation, keyed by `op`, with its input type; `spawn` per declared child; `cancel`; and `halt` at `R`. With A1, a step's action literals check against it, so an ill-formed action is caught by `checkHarness`, not refused at run time.
- **Grants:** each capability declares the type of the part of the grant state its contracts read and write. `install` merges them, a clash between two capabilities' declarations is an install error, and a run's grant must inhabit the merge.
- **Events:** a property is checked against an interface (and, for slot reads, a harness), and `event` is a variant over `in`, `out`, `withheld` and `failed`, with each input and action typed by it. A property that names operations is tied to an interface already; this makes the tie explicit.
- **Harnesses checked alone** go: `checkHarness` takes an interface.

A2 changes the shape of `Harness`, properties and responses, so it is a breaking version, with every compliance vector blessed again.

### A3: `Json` replaces `Any`

- `Json` is a closed recursive type: null, bool, number, string, a list of `Json`, or a record of `Json`. It holds what a contract really means by "any value", such as `approve.ask`'s `args`.
- A `Json` value is inspected before it is used, with a `match` on its shape whose cases bind the value at the matching type. Nothing reads through it unchecked.
- SMT encodes it as a recursive datatype, records as lists of key and value pairs, so a state that holds one has a sort.
- Then `Type::Any` and the "conforms both ways" rule are deleted, and a partial field read is an error.

The platform's contracts change with A3: `approve.ask`'s `args` becomes `Json`, and any contract that reads `grant` declares its part (A2).

## What the surface language gets

Once A1 lands, the workarounds go, each in its own change:
- slots become a variant, `waiting | running {call} | ok {value} | failed {error} | refused | skipped`, matched rather than read by `kind`;
- fan-out lists keep their type, and actions are typed;
- with A2, a spawn's value has its child's result type, so a harness that spawns gets an induction obligation too;
- with A3, `error` stops being text and keeps the world's detail as `Json`.

## Pieces

Each is a kernel PR with compliance vectors, and the platform follows A2 and A3 in lockstep.

- **A1:** expected types in `check.rs`, variant introduction, typed `[]`, and `type` annotations on `let` and `fold`. The SMT encoder encodes against an expected type the same way, so a variant built from a literal gets its constructor: an SMT-LIB definition gains an optional `result`, and an induction obligation's step is encoded at the state type. Minor version.
- **A2:** errors, harness results and declared children, typed actions, declared grant parts, typed events, and `checkHarness` against an interface. Breaking version.
- **A3:** `Json`, its `match`, its SMT encoding, the removal of `Any`, and `null` refused but at `Json`. Breaking version, with the platform's contracts updated.

## Open questions

1. **Recursive user types.** `Json` is recursive; user types declared with `type` aren't. Allowing recursion in named types would make `Json` one of them, but complicates SMT encoding and termination of the checker.
2. **Record keys in SMT.** Encoding a `Json` record as a list of pairs loses the fact that keys are unique. A proof that relies on that would need it as an axiom.
3. **How strict width subtyping stays.** A record with more fields conforms to one with fewer. Against a variant case, an extra field is harmless, but against an action it may be a mistake worth reporting.
4. **Spawning without a declared child.** A2 asks a harness to name its children up front. A harness that spawns whatever a model chose would need a child typed at `Json` to `Json`, which is A3's escape hatch, not `Any`'s.
