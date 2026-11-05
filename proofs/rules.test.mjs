// Rules, end to end: the kernel writes the queries, cvc5 proves them, and Ethos checks the proofs against them.
// Run with `npm run test:proofs` after `scripts/install-proof-tools.sh` and `npm run build`.

import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { instantiate } from "../javascript/index.js";
import { check, forbidden } from "./check.mjs";
import { prove } from "./prove.mjs";

const kernel = await instantiate(
  readFileSync(new URL("../javascript/kernel.wasm", import.meta.url)),
);
const dir = mkdtempSync(join(tmpdir(), "rules-"));

const lit = (v) => ({ e: "lit", v });
const v = (name) => ({ e: "var", name });
const get = (of, field) => ({ e: "get", of, field });
const op = (name, ...args) => ({ e: "op", op: name, args });
const eq = (a, b) => op("eq", a, b);
const answers = lit(["once", "always", "deny"]);

/** `act` needs its argument on the grant's `field`, and a person's answer to `ask` adds it. */
const gated = (field, key, update) => {
  const args = { Record: { [key]: "String" } };
  const allow = get(v("grant"), field);
  const added = { e: "record", fields: { [field]: op("append", allow, get(v("args"), key)) } };
  return {
    operations: {
      act: {
        input: args,
        output: "String",
        effect: true,
        pre: op("contains", allow, get(v("args"), key)),
      },
      ask: {
        input: args,
        output: "String",
        ret: { e: "list", items: [] },
        post: op("contains", answers, v("result")),
        onResult: update?.(added) ?? {
          e: "if",
          cond: eq(v("result"), lit("always")),
          // biome-ignore lint/suspicious/noThenProperty: a field of a kernel expression, never awaited.
          then: added,
          else: v("grant"),
        },
      },
    },
    grant: { Record: { [field]: { List: "String" } } },
  };
};

const install = (...capabilities) => {
  const out = kernel.install(capabilities);
  assert.equal(out.kind, "installed");
  return out.interface;
};

const net = (update) => ({ prefix: "net", capability: gated("netAllow", "url", update) });
const fs = { prefix: "fs", capability: gated("fsAllow", "path") };

const approvedBy = (prefix, key) => [
  {
    op: `${prefix}.ask`,
    when: op(
      "and",
      eq(v("result"), lit("always")),
      eq(get(v("approval"), key), get(v("args"), key)),
    ),
  },
];
const netRule = { op: "net.act", approvedBy: approvedBy("net", "url") };

const obligations = (iface, rule, grant) => {
  const out = kernel.ruleObligations({ interface: iface, rule, grant });
  assert.equal(out.kind, "obligations", JSON.stringify(out));
  return out;
};

let written = 0;
const write = (text) => {
  const path = join(dir, `query-${written++}.smt2`);
  writeFileSync(path, text);
  return path;
};

/** Every obligation proved, and each proof checked. */
function assertProved(out) {
  const proved = {};
  for (const { name, text } of out.obligations) {
    const query = write(text);
    const { answer, proof } = prove(query);
    assert.equal(answer, "unsat", name);
    assert.deepEqual(check(query, proof), { ok: true }, name);
    proved[name] = { query, proof, text };
  }
  return proved;
}

/** Some obligation has a counterexample, and none of the others is in doubt. */
function assertCounterexample(out, expected) {
  const results = Object.fromEntries(
    out.obligations.map(({ name, text }) => [name, prove(write(text)).answer]),
  );
  assert.equal(results[expected], "sat", JSON.stringify(results));
}

const grant = { netAllow: [] };
const proved = assertProved(obligations(install(net()), netRule, grant));

test("a rule's obligations are proved, and the proofs check against them", () => {
  assert.deepEqual(Object.keys(proved), ["initial", "net.ask.onResult"]);
});

test("a capability that leaves the field alone changes no query and needs no new proof", () => {
  const both = obligations(install(net(), fs), netRule, { netAllow: [], fsAllow: [] });
  assert.deepEqual(both.framed, ["fs.ask.onResult"]);
  // The same queries to the byte, so the proofs already made check against them unchanged.
  for (const { name, text } of both.obligations) {
    assert.equal(text, proved[name].text, name);
    assert.deepEqual(check(write(text), proved[name].proof), { ok: true }, name);
  }
  // The capability's own rule is proved over its own part.
  const own = obligations(
    install(fs),
    { op: "fs.act", approvedBy: approvedBy("fs", "path") },
    { fsAllow: [] },
  );
  assert.deepEqual(Object.keys(assertProved(own)), ["initial", "fs.ask.onResult"]);
});

test("broken contracts give counterexamples", () => {
  const unguarded = gated("netAllow", "url");
  delete unguarded.operations.act.pre;
  assertCounterexample(
    obligations(install({ prefix: "net", capability: unguarded }), netRule, grant),
    "initial",
  );
  // A grant that starts open.
  assertCounterexample(
    obligations(install(net()), netRule, { netAllow: ["https://example.com"] }),
    "initial",
  );
  // A person's "deny" grants as well.
  assertCounterexample(
    obligations(install(net((added) => added)), netRule, grant),
    "net.ask.onResult",
  );
  // No approval is named, so nothing can count as one.
  assertCounterexample(obligations(install(net()), { op: "net.act" }, grant), "net.ask.onResult");
  // A new capability writes the field with no person behind it.
  const rogue = {
    prefix: "rogue",
    capability: {
      operations: {
        unlock: {
          input: { Record: { url: "String" } },
          output: "String",
          ret: { e: "list", items: [] },
          onResult: {
            e: "record",
            fields: { netAllow: op("append", get(v("grant"), "netAllow"), get(v("args"), "url")) },
          },
        },
      },
      grant: { Record: { netAllow: { List: "String" } } },
    },
  };
  assertCounterexample(obligations(install(net(), rogue), netRule, grant), "rogue.unlock.onResult");
});

const { query, proof } = proved["net.ask.onResult"];
const lines = proof.split("\n");
const steps = lines.flatMap((l, i) => (l.startsWith("(step ") ? [i] : []));
const middle = steps[Math.floor(steps.length / 2)];
const denyGrants = write(
  obligations(install(net((added) => added)), netRule, grant).obligations.find(
    (o) => o.name === "net.ask.onResult",
  ).text,
);

const attacks = {
  "a sound proof against a broken contract's query": [denyGrants, proof],
  "a sound proof against another true query": [write(proved.initial.text), proof],
  "a proof rewritten to be about something else": [query, proof.replaceAll('"always"', '"deny"')],
  "an assumption of false": [query, "(assume @p1 false)\n"],
  "an assumption of false, then a step to false": [
    query,
    "(assume @p1 false)\n(step @p2 false :rule refl :premises (@p1))\n",
  ],
  "a deleted step": [query, lines.filter((_, i) => i !== middle).join("\n")],
  "a step with another rule": [
    query,
    lines.map((l, i) => (i === middle ? l.replace(/:rule \S+/, ":rule refl") : l)).join("\n"),
  ],
  "a proof that stops before false": [query, lines.slice(0, steps.at(-1)).join("\n")],
  "a query's constant redefined": [query, `(define |v:rule:result| () "deny")\n${proof}`],
  "a query's selector redefined": [query, `(define |rule:selector| () 0)\n${proof}`],
  "a constant declared again": [query, `(declare-const |v:rule:result| String)\n${proof}`],
  "another reference": [query, `(reference "${denyGrants}")\n${proof}`],
  "an include": [query, `(include "${denyGrants}")\n${proof}`],
  "text outside a command": [query, `x\n${proof}`],
  "an unclosed command": [query, `${proof}(step`],
};

test("the untouched proof passes, so the attacks below fail for their own reasons", () => {
  assert.deepEqual(check(query, proof), { ok: true });
});

for (const [name, [against, text]] of Object.entries(attacks)) {
  test(`rejected: ${name}`, () => {
    const result = check(against, text);
    assert.equal(result.ok, false, `accepted ${name}`);
    assert.ok(result.reason.length > 0);
  });
}

test("what a real selector's redefinition is rejected as", () => {
  const selector = readFileSync(query, "utf8").match(/\|record:[0-9a-f]+\.netAllow\|/)[0];
  const text = `(define ${selector} () 0)\n${proof}`;
  assert.match(forbidden(text), /a definition of/);
  assert.equal(check(query, text).ok, false);
});

test("cvc5's proofs hold nothing but what the verifier allows", () => {
  assert.equal(forbidden(proof), null);
});
