/**
 * The kernel, compiled to WebAssembly (`crates/kernel-abi`). Requests and responses cross in deterministic CBOR.
 */

import { decode, encode } from "./cbor.js";

export { decode, EncodeError, encode } from "./cbor.js";

/** The kernel refused a request: it was not one it understands. A bug in the caller, never a verdict. */
export class KernelError extends Error {
  name = "KernelError";

  /** @param {unknown} detail */
  constructor(detail) {
    super(`The kernel refused the request: ${JSON.stringify(detail)}`);
    this.detail = detail;
  }
}

/** @param {BufferSource | WebAssembly.Module} source */
export async function instantiate(source) {
  // A Worker can't compile bytes at runtime; it imports a compiled module instead.
  const instance =
    source instanceof WebAssembly.Module
      ? await WebAssembly.instantiate(source, {})
      : (await WebAssembly.instantiate(source, {})).instance;
  const exports = instance.exports;

  const call = (request) => {
    const bytes = encode(request);
    const pointer = exports.cronch_alloc(bytes.length);
    new Uint8Array(exports.memory.buffer, pointer, bytes.length).set(bytes);
    const packed = exports.cronch_call(pointer, bytes.length);
    const outPointer = Number(packed >> 32n);
    const outLength = Number(packed & 0xffffffffn);
    // Copy out before freeing: the call may have grown, and so replaced, the memory buffer.
    const out = new Uint8Array(exports.memory.buffer, outPointer, outLength).slice();
    exports.cronch_free(outPointer, outLength);
    const decoded = decode(out);
    if (!decoded.ok) throw new KernelError({ kind: "undecodableResponse", error: decoded.error });
    const response = decoded.value;
    if (!response.ok) throw new KernelError(response.error);
    return response.result;
  };

  return {
    key: (ask, world) => call({ op: "key", ask, world: world ?? {} }).key,
    specKey: (spec) => call({ op: "specKey", spec }).key,
    checkAsk: (ask) => call({ op: "checkAsk", ask }).problems,
    readAnswer: (value) => {
      const reading = call({ op: "readAnswer", answer: value });
      return reading.readable
        ? { kind: "readable", answer: value }
        : { kind: "unreadable", detail: reading.detail };
    },
    assess: ({ ask, world, answer, resolved, evidence }) =>
      call({
        op: "assess",
        ask,
        world: world ?? {},
        answer,
        resolved: resolved ?? {},
        evidence: evidence ?? [],
      }),
    smtlib: ({ ask, world, answer }) => call({ op: "smtlib", ask, world: world ?? {}, answer }),
    claimKey: (claim) => call({ op: "claimKey", claim }).key,
    checkClaim: (claim) => call({ op: "checkClaim", claim }).problems,
    assessClaim: ({ claim, answer, witness }) =>
      call({ op: "assessClaim", claim, answer, witness: witness ?? {} }),
    claimQuery: ({ claim, answer, witness, floats }) =>
      call({
        op: "claimQuery",
        claim,
        answer,
        witness: witness ?? {},
        floats: floats ?? "exact",
      }),
    readValues: (reply, reads) => call({ op: "readValues", reply, reads }),
  };
}
