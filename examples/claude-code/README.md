# A Claude Code-style harness

The capabilities of the T1 harness, as contracts. Each file is a capability in kernel JSON, passed to `install` and `elaborate`. The surface language has no syntax for contracts or grants.

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

In this harness's vocabulary, internet access means `net.fetch` and `shell.online`.

## Assumptions

- **`shell.run` has no network.** A host sandbox enforces it. Nothing in the contract can.
- **`shell.online` reaches only its `url`.** A host sandbox enforces that too. A call names one URL.
- **`net.ask` is a person's answer.** Its result is trusted, so an `allow` adds the URL to the grant.
- **`footprint` is the publisher's promise.** `net.fetch` and `shell.online` declare the URL they reach. The kernel records it and doesn't check it.

## Approvals

`netAllow` is plain data: a list of URLs. Every run starts with it empty. An entry is there or it isn't, and a network call needs one. Only `net.ask`'s trusted `allow` adds one, and nothing removes one, so an entry stands for the rest of the run. Matching is exact, so another path on the same host is another URL.

Once, always and session belong to the platform, not the kernel. The platform implements them by how it answers `net.ask`. It never fills or edits the grant. The kernel guarantees only that a network call comes after an `allow` for its URL in the same run. So a one-time approval holds for a harness that asks before every network call, and a harness that doesn't ask can repeat a call its grant already allows.

## What this doesn't restrict

- **`fs` is unrestricted.** Any path can be read or written, with no approval. T1 is about the network.
- **`shell.run` is unrestricted** apart from its network. A command can touch any local file.

`fs` and `shell.run` declare no footprint, since no contract narrows what they touch.

## The model

`brain.choose` is an opaque interface. Its answer is a variant of the tools it is offered, so `net.ask` isn't one of its choices.
