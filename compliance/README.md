# Compliance vectors

Exchanges with the kernel's interface that every implementation must reproduce exactly. They are part of the specification: an implementation conforms when it gives every request here the response recorded with it.

Each file in `cases/` is one case:

```json
{
  "description": "What the case shows.",
  "exchanges": [{ "request": { "op": "…" }, "response": { "ok": true, "result": {} } }]
}
```

Requests and responses are shown as JSON. On the wire they are deterministic CBOR (RFC 8949 §4.2.1), the encoding of exactly this data, so an implementation encodes each request, calls the kernel, decodes the response, and compares it with the recorded one as data. Within a case, later requests carry configurations that earlier responses returned, so each exchange can be replayed on its own.

The Rust kernel replays them in `crates/kernel/tests/compliance.rs`, and the WebAssembly build in `javascript/test/compliance.test.js`.

## Changing them

The cases are defined in `crates/kernel/tests/compliance.rs`. After changing a definition or the kernel's behaviour, write the vectors again and format them:

```sh
CRONCH_BLESS=1 cargo test -p kernel --test compliance
npm run format
```

Review the diff as a change to the specification.
