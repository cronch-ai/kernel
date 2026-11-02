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

- **Errors:** `failed`'s `error` gets a fixed type, a variant over `kind`: `cancelled`, made by the interpreter when a child is cancelled; `child`, a failed child's step error; and `world`, whose `detail` is what the world said. The world still delivers any value, which the interpreter wraps. Until A3, `detail` and a child's `error` are `Any`, so an obligation leaves out those two cases, but no longer every failure; in A3 they become `Json`.
- **Results:** a harness declares `result: R`, and `halt` with a value outside `R` is a step failure. A harness declares the children it may spawn by name, with their argument and result types. A `spawn` action names one, the interpreter checks the child's term against that signature when it starts, and its result arrives as `ret` on the operation `spawn.<name>`, at that type. Until A3, `result` defaults to `Any`, and an unnamed spawn still arrives on `spawn` as `Any`.
- **Actions:** `actions` is `List(Action)`, with `Action` a variant built from the interface: one `call` case per operation, keyed by `op`, with its input type; `cancel`; `halt` at `R`; and `spawn`, open until A3. With A1, a step's action literals check against it, so an ill-formed action is caught by `checkHarness`, not refused at run time.
  - A step that can call an operation its interface lacks is now a problem with the harness, found when it is checked or started. A failed precondition is still a refusal at run time: it depends on the grant and the arguments' values, which only the run has.
  - An action chosen at run time, such as a tool call a model names by a string, has no literal `op` to check. Until A3, it is bound at `Any` (`let a: Any = … in a`) and refused when it runs if it isn't an action. In A3, a harness reads such a choice from `Json` and builds the call for one of the interface's operations, which a proof can then enumerate.
- **Grants:** each capability declares the type of the part of the grant state its contracts read and write, as a record type. `install` merges them, a field two capabilities declare at different types is an install error (`grantClash`), and a run's grant must inhabit the merge. Contracts are checked against it, so a misspelt field, or an `onCall`, `onResult` or `onMsg` that gives a next grant state of the wrong type, is caught. A capability that declares nothing reads the grant as `Any` until A3.
- **Events:** a property is checked against a harness and an interface, and reads each event as a view the kernel builds from the recorded one, so traces, their hashes and their audits don't change.
  - The view has every field its event can have. An input's `label` is `[]` when it carries none. A performed action is `out`, with its `effect`. A call, spawn or cancel that wasn't performed is `refused`, so `performed` goes. A call or spawn carries its identifier as `action.call`, as a cancel already did, and a call to an operation with a footprint carries it as `action.footprint`. A child run's events have no `state`: `state` is always the root's.
  - `event` is typed as the record of every field some event has, each at the type every event that has it gives it (`eventType`). An atom may read a field the event at hand lacks, which makes it false, as before, but not a field no event has, nor at a type no event gives it, so a misspelt field no longer makes a property hold of every run. A child's start value and result are its own, so they are `Any`. Equality takes any two values, so comparing a field with a value of another type is still not caught.
  - Hosts keep passing recorded events to `monitor` and `monitorStep`; `view` gives what a property sees of them.
- **Harnesses checked alone** go: `checkHarness` takes an interface.

A2 changes the shape of `Harness`, properties and responses, so it is a breaking version, with every compliance vector blessed again.

### A3: `Json` replaces `Any`

- `Json` is any JSON value: null, a boolean, a number, a string, a list of them or a record of them. It holds what a contract really means by "any value", such as `approve.ask`'s `args`. Every type conforms to it, and it conforms to nothing else.
- **Decided: a `Json` value is read by testing it against a type,** not by matching on its shape. `is` (`if e is T as x then a else b` in source) takes `a` with `x` holding the value at `T` when it inhabits `T`, and `b` otherwise. A model's tool call is tested against the call it may make, and a world error's detail against what the harness knows how to handle, without nested matches. Which branch runs depends on all of the value, so both carry its sources.
- **Decided: an undeclared child takes and gives `Json`,** so a harness may still spawn something chosen at run time, and inspects what it gives like any other `Json`.
- SMT reads `Json` as a recursive datatype, and `is` as a recursive predicate saying whether a `Json` term inhabits a type, so a state that holds one has a sort.
- Then `Type::Any` and the "conforms both ways" rule are deleted, and a partial field read is an error.

The platform's contracts change with A3: `approve.ask`'s `args` becomes `Json`, and any contract that reads `grant` declares its part (A2).

## What the surface language gets

Slots stay records, not variants: see the surface language plan.

Once A1 lands, the workarounds go, each in its own change:
- fan-out lists keep their type, and actions are typed;
- with A2, a spawn's value has its child's result type, so a harness that spawns gets an induction obligation too;
- with A3, `error` stops being text and keeps the world's detail as `Json`.

## Pieces

Each is a kernel PR with compliance vectors, and the platform follows A2 and A3 in lockstep.

- **A1:** expected types in `check.rs`, variant introduction, typed `[]`, and `type` annotations on `let` and `fold`. The SMT encoder encodes against an expected type the same way, so a variant built from a literal gets its constructor: an SMT-LIB definition gains an optional `result`, and an induction obligation's step is encoded at the state type. Minor version.
- **A2:** errors, harness results and declared children, typed actions, declared grant parts, typed events, and `checkHarness` against an interface. With them, the label rule decided in the surface plan: an input's `kind`, `call` and `op` are the interpreter's, so trusted, and only `value` and `error` carry the sources of what arrived. Breaking version.
- **A3**, in four pieces:
  - **A3a:** `Json` and `is`, in the checker, the interpreter, induction and the surface language. SMT refuses both until A3b, as it refuses `Any`. Minor version.
  - **A3b:** `Json`'s SMT datatype and `is`'s predicate.
  - **A3c:** what the interpreter makes up becomes `Json`: a world error's `detail`, a child's error, an undeclared child's argument and result, `result`, `arg` and a capability's grant when none is declared. An action chosen at run time is tested with `is` against the interface's actions.
  - **A3d:** `Any` deleted, a partial field read an error, and `null` refused but at `Json`. Breaking version, with the platform's contracts updated.

## Open questions

1. **Recursive user types.** `Json` is recursive and built in; user types declared with `type` still aren't recursive.
2. **Record keys in SMT.** Encoding a `Json` record as a list of pairs loses the fact that keys are unique. A proof that relies on that would need it as an axiom.
3. **How strict width subtyping stays.** A record with more fields conforms to one with fewer. Against a variant case, an extra field is harmless, but against an action it may be a mistake worth reporting.
4. **Spawning without a declared child.** Decided: such a child takes and gives `Json` (A3c).
