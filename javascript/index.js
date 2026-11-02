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
    infer: (expr, vars, expected) =>
      call({ op: "infer", expr, vars: vars ?? {}, ...(expected ? { expected } : {}) }),
    evaluate: (expr, vars) => call({ op: "evaluate", expr, vars: vars ?? {} }),
    checkHarness: (harness, iface) =>
      call({ op: "checkHarness", harness, interface: iface ?? null }).problems,
    inputType: (harness, iface) => call({ op: "inputType", harness, interface: iface }).type,
    checkInterface: (iface) => call({ op: "checkInterface", interface: iface }).problems,
    install: (capabilities) => call({ op: "install", capabilities }),
    narrow: (iface, narrowings) => call({ op: "narrow", interface: iface, narrowings }),
    init: ({ harness, interface: iface, grant }) =>
      call({ op: "init", harness, interface: iface ?? {}, grant: grant ?? null }),
    deliver: (system, delivery) => call({ op: "deliver", system, delivery }),
    checkProperty: (property) => call({ op: "checkProperty", property }).problems,
    monitor: (property, trace) => call({ op: "monitor", property, trace }),
    monitorStart: (property) => call({ op: "monitorStart", property }),
    monitorStep: (property, state, events) =>
      call({ op: "monitorStep", property, state: state ?? {}, events }),
    prove: (subject, method) => call({ op: "prove", ...subject, method }),
    verify: (subject, evidence) => call({ op: "verify", ...subject, evidence }),
    certificate: (subject, alphabet, limit) =>
      call({ op: "certificate", ...subject, alphabet, limit: limit ?? 10000 }),
    obligation: ({ harness, interface: iface, invariant, floats }) =>
      call({
        op: "obligation",
        harness,
        interface: iface ?? {},
        invariant,
        floats: floats ?? "exact",
      }),
    checkInduction: ({ harness, interface: iface, invariant, state, input }) =>
      call({ op: "checkInduction", harness, interface: iface ?? {}, invariant, state, input }),
    elaborate: (source, capabilities, harnesses) =>
      call({ op: "elaborate", source, capabilities, harnesses: harnesses ?? {} }),
    key: (kind, object) => call({ op: "objectKey", kind, object }).key,
    smtlib: (definitions, floats) => call({ op: "smtlib", definitions, floats: floats ?? "exact" }),
    readValues: (reply, types) => call({ op: "readValues", reply, types }),
  };
}
