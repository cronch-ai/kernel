import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { instantiate, KernelError } from "../index.js";

const bytes = await readFile(new URL("../kernel.wasm", import.meta.url));
const kernel = await instantiate(bytes);

const lit = (v) => ({ e: "lit", v });
const v = (name) => ({ e: "var", name });
const get = (of, field) => ({ e: "get", of, field });
const eq = (a, b) => ({ e: "op", op: "eq", args: [a, b] });
const step = (actions) => ({
  e: "record",
  fields: { state: v("state"), actions: { e: "list", items: actions } },
});

/** Writes to the path its start value names, then halts with the result. */
const writer = {
  arg: "String",
  state: { Record: {} },
  init: {},
  step: {
    e: "if",
    cond: eq(get(v("input"), "kind"), lit("start")),
    // biome-ignore lint/suspicious/noThenProperty: an `if` expression's branch is named `then` on the wire.
    then: step([
      {
        e: "record",
        fields: {
          kind: lit("call"),
          op: lit("fs.write"),
          args: { e: "record", fields: { path: get(v("input"), "value") } },
        },
      },
    ]),
    else: step([{ e: "record", fields: { kind: lit("halt"), value: get(v("input"), "value") } }]),
  },
};

const files = {
  operations: {
    "fs.write": {
      input: { Record: { path: "String" } },
      output: "Int",
      effect: true,
      pre: { e: "op", op: "prefix", args: [lit("/workspace/"), get(v("args"), "path")] },
    },
  },
};

test("instantiates from a compiled module too", async () => {
  const fromModule = await instantiate(new WebAssembly.Module(bytes));
  assert.equal(fromModule.key("harness", writer), kernel.key("harness", writer));
});

test("runs a harness, checking preconditions at the point of the call", () => {
  const ready = kernel.init({ harness: writer, interface: files });
  assert.equal(ready.kind, "ready");
  const inside = kernel.deliver(ready.system, { input: { kind: "start", value: "/workspace/a" } });
  assert.deepEqual(
    inside.events.map((e) => e.kind),
    ["in", "out"],
  );
  assert.equal(inside.events[1].performed, true);
  const done = kernel.deliver(inside.system, { input: { kind: "ret", call: 0, value: 1 } });
  assert.deepEqual(done.system.root.outcome, { kind: "halted", value: 1 });

  const outside = kernel.deliver(ready.system, { input: { kind: "start", value: "/etc/passwd" } });
  assert.equal(outside.events[1].performed, false);
  assert.equal(outside.events[2].input.kind, "refused");
});

test("proves and re-checks evidence", () => {
  const performedOutside = {
    e: "op",
    op: "and",
    args: [
      eq(get(v("event"), "kind"), lit("out")),
      {
        e: "op",
        op: "and",
        args: [
          get(v("event"), "performed"),
          {
            e: "op",
            op: "not",
            args: [
              {
                e: "op",
                op: "prefix",
                args: [lit("/workspace/"), get(get(get(v("event"), "action"), "args"), "path")],
              },
            ],
          },
        ],
      },
    ],
  };
  const property = {
    name: "staysInWorkspace",
    always: { f: "not", of: { f: "atom", holds: performedOutside } },
  };
  const subject = { harness: writer, interface: files, property };
  const alphabet = [
    { input: { kind: "start", value: "/workspace/a" } },
    { input: { kind: "start", value: "/etc/passwd" } },
    { input: { kind: "ret", value: 1 } },
  ];
  const proved = kernel.prove(subject, { kind: "exhaustive", alphabet });
  assert.equal(proved.kind, "evidence");
  assert.equal(proved.evidence.verdict, "holds");
  assert.deepEqual(kernel.verify(subject, proved.evidence), { valid: true });
});

test("monitors a run step by step as it would the whole trace", () => {
  const property = {
    name: "staysInWorkspace",
    always: {
      f: "not",
      of: {
        f: "atom",
        holds: {
          e: "op",
          op: "and",
          args: [
            eq(get(v("event"), "kind"), lit("out")),
            {
              e: "op",
              op: "not",
              args: [
                {
                  e: "op",
                  op: "prefix",
                  args: [lit("/workspace/"), get(get(get(v("event"), "action"), "args"), "path")],
                },
              ],
            },
          ],
        },
      },
    },
  };
  const { system } = kernel.init({ harness: writer, interface: files });
  const started = kernel.monitorStart(property);
  assert.equal(started.kind, "started");
  const outside = kernel.deliver(system, { input: { kind: "start", value: "/etc/passwd" } });
  const stepped = kernel.monitorStep(property, started.state, outside.events);
  assert.equal(stepped.kind, "stepped");
  assert.deepEqual(
    { verdict: stepped.verdict, at: stepped.at },
    kernel.monitor(property, outside.events),
  );
  const wrong = kernel.monitorStep(property, { last: [true] }, outside.events);
  assert.equal(wrong.kind, "refused");
  assert.equal(wrong.error.kind, "state");
});

test("types, evaluates and exports expressions", () => {
  const sum = { e: "op", op: "add", args: [v("x"), lit(1)] };
  assert.deepEqual(kernel.infer(sum, { x: "Int" }), { type: "Int", problems: [] });
  assert.deepEqual(kernel.evaluate(sum, { x: 41 }), { kind: "value", value: 42 });
  const out = kernel.smtlib([{ name: "next", params: [{ name: "x", type: "Int" }], body: sum }]);
  assert.equal(out.kind, "exported");
  assert.match(out.text, /\(define-fun \|def:next\| \(\(\|v:x\| Int\)\) Int \(\+ \|v:x\| 1\)\)/);
});

test("keys content, not requests", () => {
  assert.equal(kernel.key("harness", writer), kernel.key("harness", { ...writer }));
  assert.notEqual(kernel.key("harness", writer), kernel.key("property", writer));
  assert.match(kernel.key("harness", writer), /^sha256:[0-9a-f]{64}$/);
  const entry = { seq: 0, prev: `sha256:${"0".repeat(64)}`, delivery: {}, events: [] };
  assert.notEqual(kernel.key("logEntry", entry), kernel.key("harness", entry));
});

test("throws on requests it doesn't understand", () => {
  assert.throws(
    () => kernel.deliver({ nonsense: true }, { input: { kind: "start", value: 1 } }),
    KernelError,
  );
});
