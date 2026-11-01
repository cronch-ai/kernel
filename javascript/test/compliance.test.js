import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import { test } from "node:test";
import { decode, encode } from "../cbor.js";

// The compiled kernel must answer every compliance vector exactly as the Rust kernel does, through its raw ABI.
const bytes = await readFile(new URL("../kernel.wasm", import.meta.url));
const { instance } = await WebAssembly.instantiate(bytes, {});
const exports = instance.exports;

const call = (request) => {
  const encoded = encode(request);
  const pointer = exports.cronch_alloc(encoded.length);
  new Uint8Array(exports.memory.buffer, pointer, encoded.length).set(encoded);
  const packed = exports.cronch_call(pointer, encoded.length);
  const outPointer = Number(packed >> 32n);
  const outLength = Number(packed & 0xffffffffn);
  const out = new Uint8Array(exports.memory.buffer, outPointer, outLength).slice();
  exports.cronch_free(outPointer, outLength);
  const decoded = decode(out);
  assert.ok(decoded.ok, JSON.stringify(decoded));
  return decoded.value;
};

const dir = new URL("../../compliance/cases/", import.meta.url);
for (const file of (await readdir(dir)).filter((f) => f.endsWith(".json")).sort()) {
  const vector = JSON.parse(await readFile(new URL(file, dir), "utf8"));
  test(`compliance: ${file}`, () => {
    vector.exchanges.forEach(({ request, response }, i) => {
      assert.deepEqual(call(request), response, `exchange ${i}`);
    });
  });
}
