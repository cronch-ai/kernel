import assert from "node:assert/strict";
import { test } from "node:test";
import { decode, encode } from "../cbor.js";

const hex = (bytes) => Buffer.from(bytes).toString("hex");

test("matches the RFC's vectors and the kernel's rules", () => {
  assert.equal(hex(encode(0)), "00");
  assert.equal(hex(encode(24)), "1818");
  assert.equal(hex(encode(1000)), "1903e8");
  assert.equal(hex(encode(-1000)), "3903e7");
  assert.equal(hex(encode(1.5)), "fb3ff8000000000000");
  assert.equal(hex(encode(3.0)), hex(encode(3)));
  assert.equal(hex(encode({ aa: 1, b: 2 })), "a261620262616101");
});

test("omits undefined map entries", () => {
  assert.equal(hex(encode({ a: 1, b: undefined })), hex(encode({ a: 1 })));
});

test("round-trips", () => {
  const value = {
    spec: { name: "Square", output: "Int" },
    args: { n: -3 },
    x: [1.5, true, null, "é"],
  };
  assert.deepEqual(decode(encode(value)), { ok: true, value });
});

test("rejects every other encoding", () => {
  assert.deepEqual(decode(Uint8Array.from([0x18, 0x01])), {
    ok: false,
    error: { kind: "nonCanonical" },
  });
  assert.deepEqual(decode(Uint8Array.from([0xa2, 0x62, 0x61, 0x61, 0x01, 0x61, 0x62, 0x02])), {
    ok: false,
    error: { kind: "nonCanonical" },
  });
  assert.deepEqual(decode(Uint8Array.from([0x01, 0x01])), {
    ok: false,
    error: { kind: "trailingBytes", offset: 1 },
  });
  assert.deepEqual(decode(Uint8Array.from([0x40])), {
    ok: false,
    error: { kind: "unsupported", offset: 0, initial: 0x40 },
  });
});
