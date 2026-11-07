# An MCP proxy

T2 from the north star: an agent's MCP tool calls, performed through its owner's servers by a Pata harness. The agent plays the model's part, and is untrusted (F3).

- [proxy.pata](proxy.pata) is the harness. Each round, `agent.next` hands the agent the result of its last call and waits for its next one. The harness performs that call through the server it names, and the result goes back on the next `agent.next`. The agent ends the session with `close`.
- [agent.json](agent.json) is the agent's side. Its `next` answers with a variant whose cases are the servers' operations, each taking `{ input }`, the tool's arguments as JSON text. A reply belongs to the call it answers by the kernel's call id, so the agent's calls need no id of their own.
- [docs.json](docs.json) and [mail.json](mail.json) are two stub servers, as the contract the platform builds for a registered server, `mcp:<key>` (`contract` in the platform's `src/mcp.gleam`). Each tool is an effect gated on its server's URL in `netAllow`, unless its owner marked it `noEffect`. Here `docs.search` is marked and `mail.send` isn't.
- [net.json](net.json) is T1's, for `net.ask`.

[crates/surface/tests/t2.rs](../../crates/surface/tests/t2.rs) runs the harness, with its interface built from the list of server keys, each installed under its own prefix. A marked tool goes through. An unmarked one asks for its server's URL first, and goes through only after an `allow`.

## What depends on the owner

The `uses` line names the owner's servers, and the `url` match names the URL of each server with an unmarked tool. `agent.json`'s variant lists their operations. A platform writes all three from the owner's servers. The rest is the same for every owner.

Real server keys are hashes of the server's record. These stubs use readable ones.
