/**
 * Deterministic CBOR (RFC 8949 §4.2.1): the one representation of every object Cronch stores, exchanges or hashes.
 * The same rules as the kernel's (`crates/kernel/src/cbor.rs`), so both produce the same bytes for the same value.
 *
 * Integers take their shortest form, map keys are sorted by their encoded bytes, and a number with no fractional part
 * is encoded as an integer. Other numbers are float64. The data model is JSON's; map entries whose value is
 * `undefined` are omitted, as absent. Decoding accepts only the deterministic encoding of what it decodes to.
 */

/** Thrown when encoding something outside the data model: a non-finite number, a function, a symbol. */
export class EncodeError extends Error {
  name = "EncodeError";

  /** @param {string} found */
  constructor(found) {
    super(`Not encodable as CBOR: ${found}`);
    this.found = found;
  }
}

const MAX_DEPTH = 256;
const utf8 = new TextEncoder();
// `ignoreBOM` keeps a leading BOM as text, rather than silently dropping it from the decoded value.
const fromUtf8 = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

/** @param {number[]} out @param {number} major @param {bigint} n */
const head = (out, major, n) => {
  const m = major << 5;
  if (n < 24n) out.push(m | Number(n));
  else if (n <= 0xffn) out.push(m | 24, Number(n));
  else if (n <= 0xffffn) out.push(m | 25, Number(n >> 8n), Number(n & 0xffn));
  else {
    const width = n <= 0xffffffffn ? 4 : 8;
    out.push(m | (width === 4 ? 26 : 27));
    for (let i = width - 1; i >= 0; i--) out.push(Number((n >> BigInt(i * 8)) & 0xffn));
  }
};

/** @param {Uint8Array} a @param {Uint8Array} b */
const compareBytes = (a, b) => {
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
};

/** @param {number[]} out @param {unknown} value */
const write = (out, value) => {
  if (value === null) out.push(0xf6);
  else if (value === false) out.push(0xf4);
  else if (value === true) out.push(0xf5);
  else if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new EncodeError(String(value));
    if (Number.isInteger(value) && Math.abs(value) < 9.2e18) {
      const n = BigInt(value);
      if (n >= 0n) head(out, 0, n);
      else head(out, 1, -1n - n);
    } else {
      const view = new DataView(new ArrayBuffer(8));
      view.setFloat64(0, value);
      out.push(0xfb, ...new Uint8Array(view.buffer));
    }
  } else if (typeof value === "string") {
    const bytes = utf8.encode(value);
    head(out, 3, BigInt(bytes.length));
    for (const b of bytes) out.push(b);
  } else if (Array.isArray(value)) {
    head(out, 4, BigInt(value.length));
    for (const item of value) write(out, item);
  } else if (typeof value === "object") {
    const entries = Object.entries(value)
      .filter(([, v]) => v !== undefined)
      .map(([k, v]) => [encode(k), v])
      .sort((a, b) => compareBytes(a[0], b[0]));
    head(out, 5, BigInt(entries.length));
    for (const [k, v] of entries) {
      for (const b of k) out.push(b);
      write(out, v);
    }
  } else throw new EncodeError(typeof value);
};

/** Encode any JSON-shaped value. @param {unknown} value @returns {Uint8Array} */
export const encode = (value) => {
  const out = [];
  write(out, value);
  return Uint8Array.from(out);
};

class Failure {
  constructor(error) {
    this.error = error;
  }
}

/** @param {Uint8Array} bytes */
export const decode = (bytes) => {
  let offset = 0;
  const fail = (error) => {
    throw new Failure(error);
  };
  const take = (n) => {
    if (offset + n > bytes.length) fail({ kind: "truncated", offset });
    const slice = bytes.subarray(offset, offset + n);
    offset += n;
    return slice;
  };
  const argument = (info, at) => {
    if (info < 24) return BigInt(info);
    const width = { 24: 1, 25: 2, 26: 4, 27: 8 }[info];
    if (width === undefined) return fail({ kind: "unsupported", offset: at, initial: bytes[at] });
    return take(width).reduce((n, b) => (n << 8n) | BigInt(b), 0n);
  };
  const safe = (n, at) =>
    n > BigInt(Number.MAX_SAFE_INTEGER) ? fail({ kind: "outOfRange", offset: at }) : Number(n);

  const value = (depth) => {
    const at = offset;
    if (depth > MAX_DEPTH) fail({ kind: "tooDeep", offset: at });
    const initial = take(1)[0];
    const major = initial >> 5;
    const info = initial & 0x1f;
    switch (major) {
      case 0:
        return safe(argument(info, at), at);
      case 1:
        return -1 - safe(argument(info, at), at);
      case 3: {
        const raw = take(safe(argument(info, at), at));
        try {
          return fromUtf8.decode(raw);
        } catch {
          return fail({ kind: "invalidUtf8", offset: at });
        }
      }
      case 4: {
        const length = safe(argument(info, at), at);
        return Array.from({ length }, () => value(depth + 1));
      }
      case 5: {
        const length = safe(argument(info, at), at);
        const map = {};
        for (let i = 0; i < length; i++) {
          const keyAt = offset;
          const key = value(depth + 1);
          if (typeof key !== "string") fail({ kind: "nonTextKey", offset: keyAt });
          if (Object.hasOwn(map, key)) fail({ kind: "duplicateKey", offset: keyAt });
          map[key] = value(depth + 1);
        }
        return map;
      }
      case 7:
        if (info === 20) return false;
        if (info === 21) return true;
        if (info === 22) return null;
        if (info === 27) {
          const f = new DataView(take(8).slice().buffer).getFloat64(0);
          return Number.isFinite(f) ? f : fail({ kind: "notFinite", offset: at });
        }
        return fail({ kind: "unsupported", offset: at, initial });
      default:
        return fail({ kind: "unsupported", offset: at, initial });
    }
  };

  try {
    const decoded = value(0);
    if (offset !== bytes.length) return { ok: false, error: { kind: "trailingBytes", offset } };
    if (compareBytes(encode(decoded), bytes) !== 0) {
      return { ok: false, error: { kind: "nonCanonical" } };
    }
    return { ok: true, value: decoded };
  } catch (error) {
    if (error instanceof Failure) return { ok: false, error: error.error };
    throw error;
  }
};
