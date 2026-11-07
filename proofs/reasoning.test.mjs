// Constrained reasoning, end to end: `examples/refund/`'s rule that a refund needs a checked derivation. The kernel
// writes its queries, cvc5 proves them and Ethos checks the proofs, and Lean proves the same obligations with `grind`.
// Then the contracts are broken, to show the proofs depend on them.
// Run with `npm run test:proofs` after `scripts/install-proof-tools.sh`, `scripts/install-lean.sh` and `npm run build`.

import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { instantiate } from "../javascript/index.js";
import { check } from "./check.mjs";
import { checkLean } from "./check-lean.mjs";
import { prove } from "./prove.mjs";

const kernel = await instantiate(
  readFileSync(new URL("../javascript/kernel.wasm", import.meta.url)),
);
const dir = mkdtempSync(join(tmpdir(), "reasoning-"));

const v = (name) => ({ e: "var", name });
const get = (of, field) => ({ e: "get", of, field });
const op = (name, ...args) => ({ e: "op", op: name, args });

const example = (name) =>
  JSON.parse(readFileSync(new URL(`../examples/refund/${name}.json`, import.meta.url)));
const PREFIXES = ["orders", "carrier", "policy", "refund"];

const install = (changed = {}, extra = []) => {
  const out = kernel.install([
    ...PREFIXES.map((prefix) => ({ prefix, capability: changed[prefix] ?? example(prefix) })),
    ...extra,
  ]);
  assert.equal(out.kind, "installed", JSON.stringify(out));
  return out.interface;
};

const rule = {
  op: "refund.issue",
  approvedBy: [
    {
      op: "policy.prove_refundable",
      when: op(
        "and",
        get(v("result"), "ok"),
        op("eq", get(v("approval"), "order"), get(v("args"), "order")),
      ),
    },
  ],
};

const obligations = (iface, format = "smtlib") => {
  const out = kernel.ruleObligations({ interface: iface, rule, format });
  assert.equal(out.kind, "obligations", JSON.stringify(out));
  return out;
};

let written = 0;
const write = (text) => {
  const path = join(dir, `query-${written++}.smt2`);
  writeFileSync(path, text);
  return path;
};

const answers = (out) =>
  Object.fromEntries(out.obligations.map(({ name, text }) => [name, prove(write(text)).answer]));

test("a refund's derivation is proved, and the proofs check against the queries", () => {
  const out = obligations(install());
  assert.deepEqual(
    out.obligations.map((o) => o.name),
    ["initial", "policy.prove_refundable.onResult", "refund.issue.onCall"],
  );
  for (const { name, text } of out.obligations) {
    const query = write(text);
    const { answer, proof } = prove(query);
    assert.equal(answer, "unsat", name);
    assert.deepEqual(check(query, proof), { ok: true }, name);
  }
});

test("the same obligations in Lean are proved by grind", () => {
  for (const { name, text } of obligations(install(), "lean").obligations) {
    const solution = `${text}\ntheorem obligation : statement := by\n  grind\n`;
    assert.deepEqual(checkLean(text, solution), { ok: true }, name);
  }
});

test("a checker that derives whatever it says gives a counterexample", () => {
  const policy = example("policy");
  const derive = policy.operations.prove_refundable.onResult.then;
  policy.operations.prove_refundable.onResult = derive;
  assert.equal(
    answers(obligations(install({ policy })))["policy.prove_refundable.onResult"],
    "sat",
  );
});

test("a capability that writes a derivation gives a counterexample", () => {
  const rogue = {
    prefix: "rogue",
    capability: {
      operations: {
        mark: {
          input: { Record: { order: "String" } },
          output: "String",
          ret: { e: "list", items: [] },
          onResult: {
            e: "record",
            fields: {
              refundable: op("append", get(v("grant"), "refundable"), get(v("args"), "order")),
            },
          },
        },
      },
      grant: { Record: { refundable: { List: "String" } } },
    },
  };
  assert.equal(answers(obligations(install({}, [rogue])))["rogue.mark.onResult"], "sat");
});

test("a refund that needs no derivation gives a counterexample", () => {
  const refund = example("refund");
  refund.operations.issue.pre = op(
    "not",
    op("contains", get(v("grant"), "refunded"), get(v("args"), "order")),
  );
  assert.equal(answers(obligations(install({ refund }))).initial, "sat");
});
