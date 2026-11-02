# Types the interface decides

A proposal, 2026-11-02, to keep `Json` for data nobody can type, and stop using it for values whose type the interface decides. It follows [After loops](2026-11-02-after-loops.md), and changes what L3 gives the platform's `model` operation.

## The problem

`Json` replaced `Any` (A3) as the type of values the kernel can't know the shape of. In SMT-LIB it is a recursive datatype. Every place a typed value meets it needs a conversion in each direction, item by item for lists. Some of its facts have to be asserted (unique keys), and some are lost (`1` and `1.0`, JSON text). [After loops](2026-11-02-after-loops.md) closed the last refusals, but the agent loop's obligation is 260 KB, and finds nothing within the platform's default budget.

Most of the `Json` in the agent loop isn't untyped data. It is there because the harness doesn't know, when it is written, which interface it will run against:

| Where                      | Type today                      | What it really is                                                            |
| -------------------------- | ------------------------------- | ---------------------------------------------------------------------------- |
| The model's chosen call    | `call {op: String, args: Json}` | A call of one of the interface's operations, with that operation's arguments |
| `perform`'s test           | `is Action`, through `Json`     | Which case the choice is                                                     |
| A performed call's result  | `Json`                          | The output of whichever of the header's operations was called                |
| `approve.ask`'s `args`     | `Json`                          | The arguments of any call (see decision 6)                                   |
| A world failure's `detail` | `Json`                          | Untyped: whatever the world said                                             |

The kernel already has the pattern for the first two. `Action` is written once in a term and resolved against whatever interface the step is checked, run or encoded under (A3d). The proposal extends that pattern, and keeps `Json` for the last row.

## Decisions

1. **`Call`, a type resolved like `Action`.** It is the `call` case of `Action`: a variant over `op`, with one case per operation of the interface, holding that operation's `args`. `Action`'s `call` case becomes `Call` by definition, so a value of type `Call` is a call action wherever one is wanted.
   - `Call` may appear in a harness's types and in an operation's output. It may not appear in an operation's input or a grant. An operation's input is part of `Call`, so `Call` would contain itself.
2. **An operation's output type is resolved against the interface the call is made in.** The kernel reads an output type in three places: the input type of a step (`input_type`), the check of a delivered result (`run.rs`), and the contracts that read `result` (`check_interface`). Each resolves `Call` (and `Action`) against the run's interface.
   - Under a narrowed interface, a child's model can only return calls the child may make. A result naming a dropped operation is withheld as a mismatch, like any result outside its operation's output type.
   - So a capability whose output holds a `Call` must offer the model only the interface's operations, and must turn an invalid choice into a failure rather than return it. That is its publisher's promise under A1, as every contract is. The platform's `model` operation needs the run's interface in its context to do so (decision 5).
3. **`is` on a typed value is decided without `Json`.** Today SMT-LIB reads `e is T` by converting `e` to `Json`, testing that, and reading it back at `T`. When `e`'s type `V` isn't `Json`:
   - if `V` conforms to `T`, the test is `true`;
   - if `V` and `T` are variants over the same tag, the test is the disjunction of `V`'s cases whose type conforms to `T`'s case of the same name. A case `T` lacks can't be a `T`. The bound value is the same constructor in `T`'s sort.
   - Otherwise, including a case whose type only partly fits, it goes through `Json` as today.

   The kernel's evaluation (`inhabits`) doesn't change. This only reads the same test exactly, in fewer terms.
4. **A performed call's value is typed.** The elaborator gives a `perform` slot's `value` the variant over `op` of `{value: Out}`, for each operation in the interface the header installs. A performed call is only ever one of those, under any narrowing. The result is recorded in the `ret` case of the operation that answered, so the variant's case is introduced from a literal tag, at its precise type.
   - A harness writes it into a conversation with `str`, which is one function per type (decision 2a of After loops), with no `Json` in between.
5. **The platform's `model` operation answers `answer {text: String} | call Call`.** This is part of L3. The `call` case has exactly `Action`'s `call` case type, so the agent's `perform choice`, still a test against `Action`, is a single case test (decision 3).
   - Its implementation offers the run's interface's operations as tools, and validates the call the model returns against them (decision 2).
   - When L2 comes, the answer gains `calls: List(Call)` for parallel tool calls, with no new type.
6. **`approve.ask` stays `{action: String, args: Json}`.** Its input names any call, and `Call` can't be in an input (decision 1). The harness builds `args` from a `Call` and never reads it back, and it never enters the harness's state. So it is a one-way conversion in the step, not a datatype the obligation quantifies over. The grant's `once` list keeps recording `Json`.
7. **`Json` stays for what is untyped:** a world failure's `detail`, an undeclared child's argument and result, and capabilities whose data has no schema (a fetched page, an MCP tool without one).

## What the agent loop gets

After 1 to 5, the agent loop's state holds no `Json`:
- `reply` is `answer | call Call`;
- `result` is a variant over the header's operations;
- the history is strings.

The obligation no longer declares the `Json` datatype for its state or input, nor maps over lists of it. `perform`'s test is a tester. The measure of success is the spike of After loops run again: the counterexample to `turns <= 2` found within the platform's default `rlimit` of 5M, where it now needs 100M.

## Compatibility

- **`Call`** is a new type and its serialization (`"Call"`). Interfaces without it are unchanged.
- **Decision 3** changes the text of an exported obligation that tests a typed value, not what it means. No compliance vector tests a typed value: the `json` vector tests a `Json` argument.
- **Decision 4** changes the term elaborated from any source that performs. So such a harness's key changes, and `compliance/surface/loops.json` is blessed again. No registered harness performs yet, since L3 hasn't shipped.
- The kernel stays on a minor version; the platform adopts this with L3.

## Pieces

- **T1 (kernel):** `Call`, `Action`'s `call` case as `Call`, and output types resolved at the three sites. Tests: a narrowed child's model result naming a dropped operation is withheld, and `checkHarness` types a `perform` of a `Call` without a test.
- **T2 (kernel):** `is` on typed values in SMT-LIB, with a compliance vector. Tests: a test on a variant narrowing exports with no `json:` symbol.
- **T3 (surface):** typed `perform` values, and the agent loop of `loops.rs` with the `model` contract of decision 5. Then the spike, with its numbers recorded here.
- **T4 (platform):** the `model` operation of decision 5, as part of L3.

T1 and T2 are independent. T3 needs both.

## Open questions

1. **Matching on tags other than `kind` in source.** A performed result is tagged by `op`, and the surface language's `match` reads only `kind`. A harness can write it with `str`, read `r.op`, or test it with `is`, but it can't `match` it by case. Whether `match` should take a tag is left until a harness needs to.
2. **`Call` in grants.** A one-time approval could record the `Call` itself instead of `{action, args}`, if a grant's type could be resolved per interface. Approvals name unprefixed actions today, so that change would come with deciding how a grant names a call.
3. **Contracts over `Call` arguments.** A precondition reads its own operation's `args`, typed. A contract that reads another operation's arguments, such as `approve.ask`'s, still reads `Json`.
