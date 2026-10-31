import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { instantiate } from "../index.js";

test("ping pongs", async () => {
  const kernel = await instantiate(await readFile(new URL("../kernel.wasm", import.meta.url)));
  assert.equal(kernel.ping(1), 2);
  assert.equal(kernel.ping(0xffffffff), 0);
});
