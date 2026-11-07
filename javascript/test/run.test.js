import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { GENESIS, kernel, run } from "../run.js";

// T1's harness, run by the local host with canned capabilities, as `crates/surface/tests/t1.rs` runs it in Rust.
const example = (name) =>
  readFile(new URL(`../../examples/claude-code/${name}`, import.meta.url), "utf8");
const source = await example("agent.pata");
const capabilities = Object.fromEntries(
  await Promise.all(
    ["brain", "fs", "shell", "net", "session"].map(async (c) => [
      c,
      JSON.parse(await example(`${c}.json`)),
    ]),
  ),
);
const A = "https://a.example/";

/** A model that says each of `turns` in order. */
const scripted = (turns) => {
  const heard = [];
  let i = 0;
  return {
    heard,
    choose: async ({ messages }) => {
      heard.push(messages);
      return turns[i++];
    },
  };
};

const performed = (events) =>
  events
    .filter((e) => e.kind === "out" && e.performed && e.action.kind === "call")
    .map((e) => e.action.op);

test("runs T1: an allowed URL is fetched, and the model hears the page", async () => {
  const brain = scripted([
    { kind: "net.fetch", url: A },
    { kind: "answer", text: "done" },
  ]);
  const fetched = [];
  const { outcome, system, events } = await run(
    source,
    capabilities,
    {
      "brain.choose": brain.choose,
      "net.ask": async () => "allow",
      "net.fetch": async ({ url }) => {
        fetched.push(url);
        return "<html>";
      },
    },
    { input: "fix the build" },
  );
  assert.deepEqual(outcome, { kind: "halted", label: ["model"], value: "done" });
  assert.deepEqual(performed(events), ["brain.choose", "net.ask", "net.fetch", "brain.choose"]);
  assert.deepEqual(fetched, [A]);
  assert.deepEqual(brain.heard[1].at(-1), { role: "tool", content: "<html>" });
  assert.deepEqual(system.root.grant.netAllow, [A]);
});

test("runs T1: a denied URL is not fetched", async () => {
  const brain = scripted([
    { kind: "net.fetch", url: A },
    { kind: "answer", text: "couldn't" },
  ]);
  const { outcome, events } = await run(
    source,
    capabilities,
    {
      "brain.choose": brain.choose,
      "net.ask": async () => "deny",
      "net.fetch": async () => assert.fail("fetched"),
    },
    { input: "fix the build" },
  );
  assert.deepEqual(outcome, { kind: "halted", label: ["model"], value: "couldn't" });
  assert.deepEqual(performed(events), ["brain.choose", "net.ask", "brain.choose"]);
  assert.deepEqual(brain.heard[1].at(-1), { role: "tool", content: "denied" });
});

test("a handler that throws fails its call, and the model hears it", async () => {
  const brain = scripted([
    { kind: "fs.read", path: "nope" },
    { kind: "answer", text: "gone" },
  ]);
  const { outcome } = await run(
    source,
    capabilities,
    {
      "brain.choose": brain.choose,
      "fs.read": async () => {
        throw new Error("ENOENT");
      },
    },
    { input: "read it" },
  );
  assert.deepEqual(outcome, { kind: "halted", label: ["model"], value: "gone" });
  assert.match(brain.heard[1].at(-1).content, /^failed: /);
});

test("a message from a handler reaches the run: the user stops it", async () => {
  const { outcome } = await run(
    source,
    capabilities,
    {
      "brain.choose": async (_, { send, signal }) => {
        send("session.user", "stop");
        await new Promise((resolve) => signal.addEventListener("abort", resolve));
        throw new Error("cancelled");
      },
    },
    { input: "fix the build" },
  );
  assert.deepEqual(outcome, { kind: "halted", value: "interrupted" });
});

test("keeps the log, each entry chained to the last", async () => {
  const k = await kernel();
  const brain = scripted([{ kind: "answer", text: "hi" }]);
  const { log } = await run(
    source,
    capabilities,
    { "brain.choose": brain.choose },
    { input: "hello" },
  );
  assert.equal(log.length, 2);
  assert.equal(log[0].prev, GENESIS);
  assert.equal(log[1].prev, log[0].key);
  for (const { key, ...entry } of log) assert.equal(key, k.key("logEntry", entry));
});

// The host protocol, checked against a compliance vector: a host whose implementation answers with a result that
// doesn't fit must step exactly as `withheld-then-failed.json` records, delivering `failed` for the call (H8), and
// stepping each configuration as the last delivery returned it (H1, H3).
test("compliance: a host delivers failed for a result the kernel withheld", async () => {
  const vector = JSON.parse(
    await readFile(
      new URL("../../compliance/cases/withheld-then-failed.json", import.meta.url),
      "utf8",
    ),
  );
  const [init] = vector.exchanges;
  const k = await kernel();
  const exchanges = [];
  const recorded = (request, result) => {
    exchanges.push({ request, response: { ok: true, result } });
    return result;
  };
  const recording = {
    ...k,
    init: ({ harness, interface: iface }) =>
      recorded({ op: "init", harness, interface: iface }, k.init({ harness, interface: iface })),
    deliver: (system, delivery) =>
      recorded({ op: "deliver", system, delivery }, k.deliver(system, delivery)),
  };
  await run(
    init.request.harness,
    {},
    { "net.fetch": async () => 3 },
    {
      kernel: recording,
      interface: init.request.interface,
      input: {},
    },
  );
  assert.deepEqual(exchanges, vector.exchanges.slice(0, 4));
});
