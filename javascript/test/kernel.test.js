import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { instantiate, KernelError } from "../index.js";

const bytes = await readFile(new URL("../kernel.wasm", import.meta.url));
const kernel = await instantiate(bytes);

const multiply = {
  name: "Multiply",
  params: [
    { name: "a", type: "Int" },
    { name: "b", type: "Int" },
  ],
  output: "Int",
};
const square = {
  spec: { name: "Square", params: [{ name: "n", type: "Int" }], output: "Int" },
  args: { n: 3 },
};

test("instantiates from a compiled module too", async () => {
  const fromModule = await instantiate(new WebAssembly.Module(bytes));
  assert.equal(fromModule.key(square), kernel.key(square));
});

test("assesses a Procedure up to its holes, then to its value", () => {
  const answer = {
    procedure: {
      e: "ask",
      spec: multiply,
      args: { a: { e: "var", name: "n" }, b: { e: "var", name: "n" } },
    },
  };
  const partial = kernel.assess({ ask: square, answer });
  assert.equal(partial.state, "constructed");
  const hole = partial.open[0];
  assert.equal(hole?.kind, "ask");
  assert.deepEqual(hole.ask.args, { a: 3, b: 3 });
  const done = kernel.assess({ ask: square, answer, resolved: { [hole.key]: 9 } });
  assert.equal(done.state, "satisfied");
  assert.equal(done.value, 9);
});

test("keys content, not requests", () => {
  assert.equal(kernel.key(square), kernel.key({ ...square }));
  assert.notEqual(kernel.key(square), kernel.key({ ...square, args: { n: 4 } }));
  assert.match(kernel.specKey(square.spec), /^sha256:[0-9a-f]{64}$/);
});

test("reports problems with a closed Ask", () => {
  assert.deepEqual(kernel.checkAsk({ ...square, args: { n: "three" } }), [
    { kind: "argumentMismatch", ask: "Square", param: "n", expected: "Int", got: "three" },
  ]);
});

test("reads an Answer without throwing, or says why it can't", () => {
  assert.deepEqual(kernel.readAnswer({ value: 5 }), { kind: "readable", answer: { value: 5 } });
  const bad = kernel.readAnswer({ procedure: { e: "ask", spec: 42 } });
  assert.equal(bad.kind, "unreadable");
  assert.equal(typeof bad.detail, "string");
});

test("throws on requests it doesn't understand", () => {
  assert.throws(() => kernel.assess({ ask: square, answer: { nonsense: true } }), KernelError);
});
