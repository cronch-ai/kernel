# Agentic loops

Plan for open question 1 of [the surface language](2026-11-01-surface-language.md), "iteration across steps", agreed on 2026-11-02. The [design session](2026-11-01-harness-design-session.md) set the goal that harnesses be "expressive enough to describe systems like Claude Code". A graph runs each node at most once, so until now the language could only describe one turn of such a system.

## What an agentic loop is

ReAct, Claude Code, and most agents built on tool calling share one shape:

1. Ask the model what to do next, given the conversation so far.
2. If it chose an action, perform it. If it gave an answer, stop.
3. Add what happened to the conversation, and go back to 1.

Two things in that shape were missing:

- **Iteration.** A run takes as many steps as the model wants, each one deciding from what the last one did.
- **Acting on a choice.** The action is a value the model produced, not a call written in the harness. Since A3d, a step can test a value against `Action`, but no node could perform one.

Everything else is already in the kernel:
- the conversation is state;
- approval is a capability whose trusted answer widens the grant;
- an interrupt is a message on a channel, with `until`;
- a subagent is a spawn against a narrowed interface;
- what the model may do is the precondition of each operation, checked by the interpreter at every call.

## The model: rounds

A looping harness is the graph run again and again, with a record carried from each run to the next. One run of the graph is a **round**.

- **Carried values** `c : C`, with a start value `c₀(arg)`. Throughout a round they are constants that any expression may read, like the parameter.
- **The round**: the file's graph, as before, over `arg` and `c`. Every slot starts the round `waiting`.
- **At the end of a round**, once every call (and everything the end reads) has settled:
  - if the stop condition holds, the run halts with `return`'s value;
  - otherwise each carried value takes its `next` value, every slot goes back to `waiting`, and the next round starts in the same step.

As a state machine over the kernel's `δ`, nothing changes. The generated state is `{arg, next, carry, nodes}`, and `carry` is the only addition. A round boundary is one more case of the generated step: it updates `carry`, resets `nodes` to their initial slots, and runs the scheduling pass again. Rounds use no kernel feature that one-round graphs don't.

A run is still a sequence of steps, each total and finite, driven by inputs the environment chooses. A loop that never stops is a run that never halts, which the kernel already allows: properties are safety properties, and none of them depends on a run ending.

## Decisions

1. **The loop is the harness, not a block inside it.** The whole graph is the round.
   - There is one level of slots, the state keeps a fixed shape, and properties and invariants name nodes as they did.
   - Work before the loop is guarded on the carried state (`when first`). A loop inside a loop is a spawned child with a loop of its own, as Claude Code's turns contain its tool steps (see below).
   - **Rejected:** a `loop` block holding a subgraph. It would need two kinds of slot, scoping rules between the block and the graph around it, and a state whose shape depends on nesting.
   - **Rejected:** a harness that spawns itself each round. That is in the kernel already, but every round adds a child to the run tree, with its own copy of the grant, and a property must then follow the origin path down the tree.
   - **Rejected:** leaving the loop to the host. No property could then span rounds, and the ones that matter most do. "An interrupt stops effects until the next prompt" and "no effect without an approval since the model's last choice" are both properties of a whole conversation.
2. **`carry name: T = e`** declares a carried value. `e` may read only the parameter, since it is evaluated when the run starts. Every expression may read the carried values, including filters and `otherwise`, as it reads the parameter.
3. **`next name = e`** gives a carried value for the next round. `e` may read nodes and carried values. A carried value with no `next` keeps its value.
4. **`return e when c`** ends a round: when `c` holds, the run halts with `e`, and otherwise the next round starts. A `return` without `when` halts after the first round, so every existing harness elaborates to the same term. `next` without a looping `return` is an error, since it could never be read.
   - `c`, `e` and every `next` read nodes as `return` does, and under the same rules: a node that can be skipped is read through `match`, and a failure read as a value ends the run through `otherwise`.
5. **A round that starts and settles in the same step stalls.** The next round starts in the step that ended the last one, so its first calls go out at once. If it settles in that step anyway, with nothing to wait for, no input could ever start the round after it. The run then halts through `otherwise`, or with `{"failed": "round"}` without one, so a looping harness whose result type doesn't admit that must declare `otherwise`.
   - The alternative was to run rounds until one waits. That makes `δ` partial: a round that never waits would loop inside a single step.
6. **`x = perform e`** is a call node whose action is the value of `e`, of any type.
   - When the round reaches it, `e` is tested against `Action`, the action type of the interface the harness runs against. If it is a `call`, it is issued like any other call and checked against its operation's precondition. If not, the slot is `skipped`.
   - Only calls are performed. A model can't choose to halt, cancel or spawn: those stay the harness's.
   - Under a narrowed interface (a subagent), `Action` is the narrowed action type, so a choice of a dropped operation isn't an `Action` and is skipped. The term is the same under every narrowing.
   - The slot's value is `Json`, since which operation answers is known only at run time. The harness reads it as it reads any `Json`, with `is`.
   - A performed call settles at its first result. A performed stream's later items and its end find no slot running their call, and change nothing.
   - **Skipped rather than refused**, because refusal is the interpreter's verdict on a precondition. A value that isn't an action never reaches the interpreter. A harness tells the model which happened through `match`:

     ```
     seen = match result {
       ok(v)     => str(v)
       failed(e) => concat("failed: ", e)
       refused   => "not allowed"
       skipped   => "not an action you can take"
     }
     ```

7. **Messages between rounds.** A receive starts listening again each round. A message that arrives while nothing listens changes nothing, as before. A message sent to a running harness is therefore heard only if some receive is listening for it. That is how an interrupt works. Queueing prompts typed mid-turn is left to the session loop below.
8. **A receive read through `match` is polled.** It is read as it stands and never waited for, and one that hasn't heard anything yet reads as `skipped`, so such a `match` must cover `skipped`. Reading a receive as a value still waits for it.
   - Without this, the end of a round could only learn of an interrupt by waiting for one. An interrupt that came after the round's last call had settled would be lost, and the next round would act again.
   - No source matched on a receive before, so nothing that elaborated before changes meaning.
9. **No round counter in the state.** A harness that needs one carries it (`carry turns: Int = 0`, `next turns = turns + 1`), so it is named in the source like everything else a property can read.

## The language

```text
decl = ...
     | "carry" name ":" type "=" expr NL
     | "next" name "=" expr NL
     | "return" expr ("when" expr)? NL
     | name "=" "perform" expr ("when" expr)? ("until" name)? NL
```

`carry`, `next` and `perform` become keywords. A field may still be called any of them.

## ReAct

The model is a capability whose `choose` gives `answer {text: String} | call {op: String, args: Json}`. The `call` case is shaped like an action, so `perform` can test it directly.

```
harness react(task: String) -> String

uses brain, tools

type Message = {role: String, content: String}

carry history: List[Message] = [{role: "user", content: task}]
carry turns: Int = 0

reply  = brain.choose(messages: history)
choice = reply
result = perform choice

answer = match choice { answer(a) => a.text, call(c) => "" }
seen   = match result {
  ok(v)     => str(v)
  failed(e) => concat("failed: ", e)
  refused   => "not allowed"
  skipped   => ""
}

next history = append(append(history, {role: "assistant", content: str(choice)}), {role: "tool", content: seen})
next turns   = turns + 1

return (if answer != "" then answer else "gave up") when answer != "" or turns >= 20

otherwise return "failed"
```

`choice` is a pure node. `match reply { ... }` would read the call's outcome (`ok`, `failed`, ...), while `match choice` reads the variant's case.

## Claude Code

Claude Code has two loops: one over the user's prompts, and inside each turn, one over the model's tool calls. They are two harnesses.

**The agent loop** is ReAct with approval and interrupts:

```
harness agent(task: String) -> String

uses brain, approve, fs, shell, session

type Message = {role: String, content: String}

carry history: List[Message] = [{role: "user", content: task}]
carry turns: Int = 0

stop   = receive said from session.user when said == "stop"
reply  = brain.choose(messages: history) until stop
choice = reply
wants  = match choice { call(c) => c.op, answer(a) => "" }
asked  = approve.ask(action: wants, args: match choice { call(c) => c.args, answer(a) => {} }) when wants != "" until stop
result = perform choice when (match asked { ok(a) => a != "deny", _ => false }) until stop

answer = match choice { answer(a) => a.text, call(c) => "" }
seen   = match result {
  ok(v)     => str(v)
  failed(e) => concat("failed: ", e)
  refused   => "not allowed"
  skipped   => "denied"
}
heard  = match stop { ok(s) => true, skipped => false }

next history = append(append(history, {role: "assistant", content: str(choice)}), {role: "tool", content: seen})
next turns   = turns + 1

return (if heard then "interrupted" else answer) when heard or answer != "" or turns >= 50

otherwise return "interrupted"

property approved_since_chosen:
  effect() implies (not returned("brain.choose")) since returned("approve.ask", value in ["once", "always"])
```

- Approval is asked for every chosen call. A harness that asks only for some operations names them (`when wants in ["fs.write", "shell.run"]`). The preconditions still refuse what the grant doesn't allow, whatever the harness asks.
- `approved_since_chosen` holds for every run of `agent`, and is refuted for the same harness without the guard on `result`, under a grant that already allows the operation. That is the same structure as P1 in the design session, now stated across rounds.
- P4 from the design session ("an interrupt stops effects until the next prompt") holds because `stop` cancels every call still waiting or running in the round, and `heard` ends the turn once the round settles. An interrupt that comes while the model is thinking fails `reply`, and ends the turn through `otherwise`.
- `approve.ask`'s argument is a `match` written in place, where it is checked against `Json`. As a node of its own, its two arms (`Json` and `{}`) would have no type in common. The language has no `null`.

**The session loop** carries the conversation across turns and spawns the agent for each one:

```
harness session(first: String) -> Json

uses agent = harness "harness:sha256-…", session

carry prompt: String = first

turn  = spawn agent(prompt)
heard = receive said from session.user when said != "stop"

next prompt = heard

return turn when heard == "/exit"
```

- `heard` listens throughout the round, so a prompt typed while the agent is working becomes the next turn's prompt. The round ends when both the turn and the prompt have settled.
- A subagent (Claude Code's Task tool) is `spawn agent(...) without shell.run, fs.write` inside the agent loop. The kernel guarantees it can never do what its parent couldn't, and an approval it receives widens its parent's grant, never its siblings'.

## What can be proved

- **Contracts** hold in every round by construction: preconditions are checked at every call, including performed ones.
- **Properties** span rounds, since past-time LTL reads the whole trace. They are monitored online, and searched up to a depth.
- **Exhaustive search** doesn't finish for a loop. Every call takes a fresh identifier, and carried values grow (`history`).
- **Induction** is the route for every depth. With one slot per node and a fixed set of carried values, the state of a loop has a fixed shape, so an invariant names it in a few words (`invariant bounded: turns <= 50`). An obligation reads the carried values as it reads slots.
  - A step that performs chosen actions tests them against `Action`, which the kernel resolves against the interface before encoding, so the obligation is about that interface's actions.
  - A performed call's result is read as `Json`, whichever operation gave it. SMT-LIB has no reading of a list of anything but `Json` as `Json` (A3b), so an obligation is refused for a harness that performs against an interface where some operation returns such a list, as `notes.read` does. Until that conversion has a reading, such a harness is proved by search up to a depth and monitored.
- **Termination** is a liveness property, which no proof here claims. A bound is a carried counter, a stop condition that reads it, and an invariant that the counter never passes it.

## Pieces

- **L1** (this change): `carry`, `next`, `return … when`, `perform` and polled receives, in the parser, the elaborator and claims, with the agent loop above as a test (with `notes` as its tool): it runs, `approved_since_chosen` holds up to a depth, and the heedless version is refuted. A compliance vector for a loop that performs. Minor version: no existing source elaborates differently.
- **L2:** parallel tool calls, `results = for a in choice.calls: perform a`, a fan-out of performs.
- **L3:** the platform runs looping harnesses, and gives a model writing a harness the agent loop as a starting point.

## Open questions

1. **Guards on receives.** A receive's `when` is its filter, so a receive can't be skipped. A harness that listens only in some rounds (waiting for a prompt only after an answer) has to listen in all of them, or split into two harnesses as the session loop does. Whether to add a guard is left until a harness needs one in a single loop.
2. **Context compaction.** Summarizing `history` when it grows is a model call guarded on its length, writing `next history`. It needs nothing new, but a property about what the summary may drop would need labels to say it.
3. **Streaming performs.** A performed stream settles at its first item. A harness that wants the rest calls the streaming operation by name.
