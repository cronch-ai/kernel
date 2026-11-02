# Typed boundaries

Agreed on 2026-11-02. This deletes `Json` and `is` from the kernel and the surface language, and replaces them with one rule:

**Nothing untyped crosses into a harness.** Every value a harness, a contract or a property reads has a precise type, given by a contract or by the harness, and the interpreter checks it on delivery. What no contract has typed is text.

## Why

A3 was right to remove `Any`, and wrong to add a top type in its place. `Json` took over every job `Any` had: results whose operation is known only at run time, a model's chosen call, approval arguments, what the world says when it fails, children nobody declared, and event fields that differ across events. Each of those cost something:

- **SMT-LIB:** a recursive datatype; axioms for unique keys and finite numbers; `1` and `1.0` read as one number; recursive maps for every list that crosses; text a solver chooses. The agent loop's obligation is 260 KB, and finds nothing within the platform's default budget ([After loops](2026-11-02-after-loops.md)).
- **The language:** `is` alongside `match`, and no way to write `null`. Two arms (`Json` and `{}`) have no type in common.

Most of those values aren't untyped. Their type is decided by the interface the harness runs against, or by a contract that could have said it. A value that really has no schema, such as a fetched page, is text until a capability parses it under a contract.

## Decisions

1. **`Json` and `is` are deleted.** There is no top type. Narrowing a value is `match` on a variant. `null` has no type, so a value that may be absent is a variant (`none | some {value: T}`). An empty list is typed by the list type expected where it appears, and is a problem where none is.
   - A field of a variant whose cases give it types with no join can't be read without `match` first, where it used to read as `Json`.
   - The checker's recovery after a problem stays as A3d made it: a binding with no type.
2. **What no contract has typed is text: `String`, written as the kernel's `str` writes it.**

   | Value                                                                     | Was    | Is       |
   | ------------------------------------------------------------------------- | ------ | -------- |
   | A world failure's `detail`                                                | `Json` | `String` |
   | A failed child's step error                                               | `Json` | `String` |
   | In a property's view: a withheld delivery's value, and what it expected   | `Json` | `String` |
   | In a property's view: a step error's details                              | `Json` | `String` |
   | In a property's view: a run's start and halt values, and a spawn's fields | `Json` | `String` |

   One event type covers the root and its children, so a start or halt value is text for every run: a property reads the root's start value as the state's `arg`. The trace is recorded as before, so hashes and audits don't change. Only what a step and a property read changes.
3. **Every spawn names a declared child.** Undeclared spawns, and their `spawn` result case, go. A child chosen at run time is one of the declared children. A spawn with no name isn't an action, and one naming a child the harness doesn't declare is refused.
4. **A harness declares its result type,** with no default. An interface whose capabilities declare no part of the grant has the grant `{}`.
5. **`Call`, a type resolved per interface like `Action`.** It is a variant over `op`, with one case per operation of the interface the step runs against, holding that operation's `args`. It is also `Action`'s `call` case, so a `Call` is a call action wherever one is wanted.
   - `Call` may appear in a harness's types and in an operation's output. It may not appear in an operation's input or a grant: an operation's input is part of `Call`, so `Call` would contain itself.
   - An operation's output is resolved against the interface the call is made in. That happens where the kernel reads an output type: the step's input type, the check of a delivered result, and the contracts that read `result`.
   - Under a narrowed interface, a child's model can only return calls the child may make. A result naming any other operation is withheld as a mismatch. A capability whose output holds a `Call` therefore offers the model only the interface's operations, and turns an invalid choice into a failure. That is its publisher's promise under A1.
6. **`perform e` takes a `Call`,** or a variant over `kind` whose `call` case is a `Call`. It is lowered to a `match`: the `call` case is performed, and any other case skips the slot. A `perform` of any other type is a problem with the source.
   - The platform's `model` operation answers `answer {text: String} | call Call` (part of L3). With L2, it also answers `calls: List(Call)`.
7. **A performed call's value is the variant over `op` of `{value: Out}`,** for each operation in the interface the header installs. It is recorded in the `ret` case of the operation that answered, from a literal tag, at its precise type.
8. **Contracts read their own operation's name, `op`,** the name it was installed under. Approvals record calls as text:
   - `approve.ask` takes `{op: String, call: String}`. A harness passes `op: c.op` and `call: str(c)`.
   - `onResult` adds `op` to `allow` for `always`, and adds `call` to `once` for `once`. The grant's `once` is `List(String)`.
   - A precondition checks `contains(grant.allow, op) or contains(grant.once, str({kind: "call", op: op, args: args}))`. With `op` given, a capability installed under any prefix names its calls as the harness does. Today the approval names `notes.append` literally, and is right only when the capability is installed under `notes`.
   - A call delivered with fields its type doesn't declare writes different text, so its one-time approval isn't found. The call is then refused, never wrongly allowed.
9. **Properties read the exact event variant.** The view's type is the variant over `kind` the kernel already builds (`view::exact`), instead of a record of every field some event has. An atom reads a case's fields after `match`, and the rule that an atom reading a field its event lacks is false goes, along with gap 5 of the design session.
   - The surface language's event helpers (`called`, `returned` and the rest) generate the `match`. Their conditions read a value at its operation's precise type.
   - A quantifier's `of` gives a list at each event, empty where the event has nothing to offer, and its variable ranges over the items so far, at the list's item type. A value that exists only at some events no longer needs evaluation to fail at the others.
   - A child's events have no `state`, which the type can't say. Reading it at one fails to evaluate, and that atom is false, so read it where `origin` is empty, as the helpers do.
10. **SMT-LIB loses `Json`:** its datatype and axioms, and every conversion into and out of it, including the list maps of After loops (2). What stays:
    - the map that reads a `List(Int)` where a `List(Float)` is wanted;
    - text as a function a solver chooses, one per type (After loops, 2a), for `str` of a structured value.

## What it deletes

- `Type::Json`, `Expr::Is`, and their rules in checking, evaluation, labels and SMT-LIB.
- `smtlib/json.rs`, and the `Json` paths in `encode.rs` and `values.rs`.
- Undeclared spawns, default result and grant types, and partial reads in properties.
- In the surface language: the `Json` type, `is … as`, and `Action` as a written type.

## Migration

This is a breaking version of the kernel, and the platform follows in lockstep:
- **Contracts:** `approve`, `notes` and `model` are rewritten with `op` and text approvals.
- **Fixtures:** properties are rewritten against the exact event variant, and harnesses that declare `-> Json` declare their result.
- **Vectors:** every compliance vector is blessed again, and elaborated harnesses get new keys. Nothing registered on the platform performs or loops yet.

## Pieces

Each is a kernel PR with compliance vectors, released together as one breaking version.

- **B1:** `Call`, output types resolved against the call's interface, and `op` in contracts. This is additive, so it can merge first.
- **B2:** text for what no contract typed, declared spawns only, required results, and the grant `{}`.
- **B3:** properties over the exact event variant, and quantified variables typed by what they range over.
- **B4:** `Json` and `is` deleted from the checker, the interpreter, labels and SMT-LIB.
- **B5:** the surface language, with typed `perform` and the agent loop of `loops.rs` against the `model` contract of decision 6. Then After loops' spike runs again: the target is the counterexample to `turns <= 2` within the default `rlimit` of 5M.
- **B6** (platform): the contracts and fixtures, as the first part of L3.

## Open questions

1. **Matching on tags other than `kind` in source.** A performed result and a `Call` are tagged by `op`, and the surface language's `match` reads `kind`. A harness can read `r.op` and write `str(r)`, but can't `match` by operation.
2. **Typed grants.** Recording approvals as text works under any prefix, and fails closed on extra fields. A grant typed per interface could record the `Call` itself, but a grant would then depend on the interface it is used with.
3. **MCP tools.** An MCP tool's JSON Schema becomes Cronch types where it fits. Where it doesn't (`oneOf`, and open records), the tool takes and gives `String`, and the capability parses it. Where the line falls is the platform's to settle when it adds MCP.
4. **Binary data.** A `String` is valid Unicode, so a capability that returns an image or a PDF has no type for it. Text stays right for what the kernel writes: it comes from JSON, and models, `prefix` and `contains` read it. Base64 inside a `String` would type-check, but invites a harness to read it as text. An opaque `Bytes` type, base64 on the wire and only ever held and passed on, would say what it is. It is additive, so it waits for the first capability that needs it.
