# A Claude Code-style harness

T1 from the north star: a Claude Code-style harness in Pata, proved unable to reach the network without permission, then extended with a capability, measuring what is proved again.

- [agent.pata](agent.pata) is the harness, in the surface language. The model chooses a tool. A network tool first asks a person about its URL through `net.ask`, and is performed only after an `allow`. The result goes back to the model, round after round, until it answers, the user says `stop`, or 20 turns pass.
- The other `.json` files are its capabilities, as contracts in kernel JSON, passed to `install` and `elaborate`. The surface language has no syntax for contracts or grants.
- [mcp.json](mcp.json) is the capability T1 adds.

## The claim

**`net.fetch` and `shell.online` can't happen until a person has allowed, through `net.ask`, the URL they reach.** It is two rules, one per operation, each approved by `net.ask` when its result is `allow` and its URL is the call's.

The claim is about the contracts, so it holds for every harness on this interface and whatever the model says, not only `agent.pata`. The kernel writes each rule's queries (`ruleObligations`), cvc5 proves them, and Ethos checks the proofs. [proofs/t1.test.mjs](../../proofs/t1.test.mjs) does this, and also breaks the contracts to show the proofs depend on them: with `net.fetch` ungated, or with `net.ask` adding a URL whatever the person says, there is a counterexample.

[crates/surface/tests/t1.rs](../../crates/surface/tests/t1.rs) runs the harness itself: a denied URL isn't fetched, an allowed one is, a local tool needs no ask, and each URL is asked for separately.

## Adding a capability

`mcp.json` adds `mcp.call`, a client for a remote tool server. It reaches the network too, so it is gated on `netAllow` like the others, and gets the same rule.

What adding it costs, measured in the proof test:

- **The four proofs already made are kept.** Their queries are unchanged to the byte, so each is checked again against its query and not proved again.
- **Two new proofs, for `mcp.call`'s own rule.** That is all that is proved.

Two more cases show where the line falls:

- **A capability that writes another part of the grant** (a counter) is listed as `framed` by each rule. It costs a check, not a proof.
- **A capability that writes `netAllow`** owes a new obligation for each rule, for its update. A rogue one that adds any URL gets a counterexample.

## What the claim rests on

- **The rules name every operation that reaches the network.** Nothing checks that. It's the vocabulary below, and adding `mcp.call` meant adding its rule.
- **The interpreter complies with the semantics**, and the host keeps the [host protocol](../../docs/2026-11-06-host-protocol.md).
- **`net.ask`'s answer is a person's**, given now or earlier under a policy they set.
- **The encoder writes queries that say what the rules say**, and **the checker that accepts a proof is sound**: Ethos with `check.mjs`'s restrictions, or Lean's kernel with Comparator. `check.mjs` refuses a `define` that would overload a name, which closed [CRO-61](https://linear.app/cronch-ai/issue/CRO-61): an overload could let a proof of a false query pass.

See [Rules](../../docs/2026-11-05-rules.md) for the general case.

## What each capability does

A contract can't state all of this. What it can't state is an assumption about the host (F7), marked below.

| Operation      | Needs a `netAllow` entry for | What it does                                       |
| -------------- | ---------------------------- | -------------------------------------------------- |
| `net.fetch`    | `url`                        | Fetches `url`.                                     |
| `net.ask`      | nothing                      | Asks a person whether to allow `url`.              |
| `shell.run`    | nothing                      | Runs `command` with no network.                    |
| `shell.online` | `url`                        | Runs `command`, with network access to `url` only. |
| `fs.read`      | nothing                      | Reads `path`.                                      |
| `fs.write`     | nothing                      | Writes `text` to `path`.                           |
| `brain.choose` | nothing                      | Answers, or chooses one of the tools above.        |
| `mcp.call`     | `server`                     | Calls `tool` on the server at `server`. Added.     |

In this harness's vocabulary, internet access means `net.fetch` and `shell.online`, and `mcp.call` once it is added.

## Assumptions

- **`shell.run` has no network.** A host sandbox enforces it. Nothing in the contract can.
- **`shell.online` reaches only its `url`.** A host sandbox enforces that too. A call names one URL.
- **`mcp.call` reaches only its `server`.** The host's client enforces it.
- **`net.ask` is a person's answer.** Its result is trusted, so an `allow` adds the URL to the grant.
- **`footprint` is the publisher's promise.** `net.fetch`, `shell.online` and `mcp.call` declare the URL they reach. The kernel records it and doesn't check it.

## Approvals

`netAllow` is plain data: a list of URLs. Every run starts with it empty. An entry is there or it isn't, and a network call needs one. Only `net.ask`'s trusted `allow` adds one, and nothing removes one, so an entry stands for the rest of the run. Matching is exact, so another path on the same host is another URL.

Once, always and session belong to the platform, not the kernel. The platform implements them by how it answers `net.ask`. It never fills or edits the grant. The kernel guarantees only that a network call comes after an `allow` for its URL in the same run. So a one-time approval holds for a harness that asks before every network call, and a harness that doesn't ask can repeat a call its grant already allows.

## What this doesn't restrict

- **`fs` is unrestricted.** Any path can be read or written, with no approval. T1 is about the network.
- **`shell.run` is unrestricted** apart from its network. A command can touch any local file.

`fs` and `shell.run` declare no footprint, since no contract narrows what they touch.

## The model

`brain.choose` is an opaque interface. Its answer is a variant of the tools it is offered, so `net.ask` isn't one of its choices.
