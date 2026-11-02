# After loops

What comes after [agentic loops](2026-11-02-agentic-loops.md) (cronch-ai/kernel#30, merged), agreed on 2026-11-02. It spans the kernel and the platform repos.

## Where things stand

- A harness can describe ReAct and Claude Code: rounds with carried values, `perform` for actions a model chose, interrupts as polled receives.
- The merge isn't published. A push to `main` releases only when `javascript/package.json`'s version changes, so the platform can't adopt loops yet.
- What can be shown of a loop today:
  - **Contracts** are enforced at every call, so they hold in every round by construction.
  - **Properties** are monitored on runs, and searched up to a depth (`holdsUpTo`).
  - **Induction obligations** are exported for loops that don't `perform`. A harness that performs is refused whenever some operation of its interface returns a list of anything but `Json`, since SMT-LIB has no reading of that list as `Json` (A3b). `notes.read` is such an operation.

## Decisions

1. **Release v0.7.0 first.** A minor version: no source that elaborated before elaborates differently. One commit bumping `javascript/package.json`, as for v0.6.0.
2. **Read lists as `Json` in SMT-LIB.** One recursive function per item type maps a sequence item by item, declared with `define-fun-rec` the first time a query needs it. It covers:
   - `List(T)` as `Json` (`json_of`), and so a performed call's result;
   - `Json` tested against `List(T)` (`json_is`): a list whose every item inhabits `T`;
   - `Json` read at `List(T)` (`json_as`);
   - `List(T)` where `List(U)` is required (`coerce`), item by item.

   It starts as a spike with a time limit. z3 must still find counterexamples to induction for the plan's agent loop within the platform's `rlimit` budget, with the functions declared. If it can't, the fallback is to unroll the map to a fixed length and assert that every list in the query is no longer than that, which the obligation then lists as an assumption.
3. **This doesn't prove loops at every depth.** An exported obligation lets a solver search for counterexamples to induction, which the kernel checks. A solver's `unsat` is still never a verdict (design session, gap 3). So decision 2 lets the agent loop's obligations be searched, but they still prove nothing.
   - What does prove a loop at every depth is a proof the kernel can check: an `unsat` certificate (Alethe from cvc5, or LFSC), or a Lean proof. That is a larger piece of kernel work, with its own plan, and it comes after L3.
   - Until then, the evidence a loop can carry is `holdsUpTo`, its monitored runs, and the counterexamples to induction a search did or didn't find. The platform shows these as what they are.
4. **L3 next: the platform runs looping harnesses.** In the platform repo, after it adopts v0.7.0:
   - A `model` operation whose answer is a variant: `answer {text}`, or `call {op, args}` shaped like an action. It is built on Workers AI tool calling, with the interface's operations offered as tools, and labelled `["model"]` like `generate`.
   - The plan's agent loop as a fixture, proved up to a depth against `approved_since_chosen` and run end to end in the integration test with a stub model.
   - Long runs in the `Run` object. A loop takes a fresh call identifier every round, and its carried history grows with every turn. The run's storage must hold both, and the platform must refuse a run that would outgrow its limits rather than lose its trace.
   - The catalog a model writing a harness sees gains the agent loop as a starting point.
5. **L2, parallel tool calls, after L3.** It is worth building only once the platform's model returns several calls in one answer.
6. **Deferred until a real harness needs them:**
   - a `null` literal;
   - guards on receives, so one loop can wait for a prompt only after an answer;
   - streaming performs.

## Order

1. Release v0.7.0 (kernel).
2. The spike, then lists as `Json` in SMT-LIB (kernel). A minor version: an obligation that was refused is now exported, and nothing that was exported changes.
3. L3 (platform), on the release that includes 2.
4. Then the plan for checking `unsat` certificates, and L2.
