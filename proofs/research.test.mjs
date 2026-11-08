// The research template in `examples/research/`: a fetch whose URL depends on a page can't happen until a person
// allows the URL, proved by cvc5 and checked by Ethos, and proved in Lean and checked by Comparator. A URL the
// planner chose from the task needs no ask, and the claim says nothing about it.
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
const dir = mkdtempSync(join(tmpdir(), "research-"));

const example = (name) =>
  JSON.parse(readFileSync(new URL(`../examples/research/${name}.json`, import.meta.url)));
const install = (net = example("net")) => {
  const out = kernel.install([
    { prefix: "brain", capability: example("brain") },
    { prefix: "reader", capability: example("reader") },
    { prefix: "net", capability: net },
    { prefix: "report", capability: example("report") },
  ]);
  assert.equal(out.kind, "installed", JSON.stringify(out));
  return out.interface;
};

const lit = (v) => ({ e: "lit", v });
const v = (name) => ({ e: "var", name });
const get = (of, field) => ({ e: "get", of, field });
const op = (name, ...args) => ({ e: "op", op: name, args });
const url = get(v("args"), "url");

/** No exfiltration: `net.fetch` of a URL that depends on something untrusted needs a person's allow of that URL. */
const rule = {
  op: "net.fetch",
  when: op("ne", op("len", { e: "label", of: url }), lit(0)),
  approvedBy: [
    {
      op: "net.ask",
      when: op(
        "and",
        op("eq", v("result"), lit("allow")),
        op("eq", get(v("approval"), "url"), url),
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

test("a URL that depends on a page can't be fetched until a person allows it", () => {
  const iface = install();
  const lean = Object.fromEntries(
    obligations(iface, "lean").obligations.map((o) => [o.name, o.text]),
  );
  const names = [];
  for (const { name, text } of obligations(iface).obligations) {
    const query = write(text);
    const { answer, proof } = prove(query);
    assert.equal(answer, "unsat", name);
    assert.deepEqual(check(query, proof), { ok: true }, name);
    const solution = `${lean[name]}\ntheorem obligation : statement := by\n  grind\n`;
    assert.deepEqual(checkLean(lean[name], solution), { ok: true }, `${name} in Lean`);
    names.push(name);
  }
  assert.deepEqual(names, ["initial", "net.ask.onResult"]);
});

test("an ungated fetch is a counterexample; an unlabelled result isn't, which is why the claim rests on `ret`", () => {
  // `net.fetch` gated on nothing: a page's URL is fetched with no ask.
  const open = example("net");
  delete open.operations.fetch.pre;
  const {
    obligations: [initial],
  } = obligations(install(open));
  assert.equal(prove(write(initial.text)).answer, "sat");
  // `net.fetch` with no `ret`: its result would be trusted, so the claim would say nothing of a page's URLs. The
  // rule still holds of the contracts, which is why it rests on each `ret` being honest.
  const unlabelled = example("net");
  delete unlabelled.operations.fetch.ret;
  for (const { name, text } of obligations(install(unlabelled)).obligations) {
    assert.equal(prove(write(text)).answer, "unsat", name);
  }
});
