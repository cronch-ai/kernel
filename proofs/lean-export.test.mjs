// The kernel's Lean statements of T1's obligations elaborate against `lean/Cronch/Smt.lean`. Proving them is CRO-89's.
// Run with `npm run test:proofs` after `scripts/install-lean.sh` and `npm run build`.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { instantiate } from "../javascript/index.js";
import { tools } from "./check.mjs";

const kernel = await instantiate(
  readFileSync(new URL("../javascript/kernel.wasm", import.meta.url)),
);
const project = fileURLToPath(new URL("../lean", import.meta.url));
const lake = (...args) =>
  spawnSync(join(tools, "lean", "bin", "lake"), args, { cwd: project, encoding: "utf8" });
const dir = mkdtempSync(join(tmpdir(), "lean-"));

const example = (name) =>
  JSON.parse(readFileSync(new URL(`../examples/claude-code/${name}.json`, import.meta.url)));
const installed = kernel.install(
  ["brain", "fs", "shell", "net", "session"].map((prefix) => ({
    prefix,
    capability: example(prefix),
  })),
);
assert.equal(installed.kind, "installed", JSON.stringify(installed));

const lit = (v) => ({ e: "lit", v });
const v = (name) => ({ e: "var", name });
const get = (of, field) => ({ e: "get", of, field });
const op = (name, ...args) => ({ e: "op", op: name, args });
const allowedFirst = (operation) => ({
  op: operation,
  approvedBy: [
    {
      op: "net.ask",
      when: op(
        "and",
        op("eq", v("result"), lit("allow")),
        op("eq", get(v("approval"), "url"), get(v("args"), "url")),
      ),
    },
  ],
});

before(() => {
  const built = lake("build", "Cronch");
  assert.equal(built.status, 0, built.stdout + built.stderr);
});

for (const rule of ["net.fetch", "shell.online"]) {
  test(`${rule}'s obligations are Lean statements`, () => {
    const out = kernel.ruleObligations({
      interface: installed.interface,
      rule: allowedFirst(rule),
      format: "lean",
    });
    assert.equal(out.kind, "obligations", JSON.stringify(out));
    assert.deepEqual(
      out.obligations.map((o) => o.name),
      ["initial", "net.ask.onResult"],
    );
    for (const { name, text } of out.obligations) {
      assert.match(text, /^abbrev statement : Prop :=$/m, name);
      const file = join(dir, `${rule}.${name}.lean`);
      writeFileSync(file, `${text}\n#check (statement : Prop)\n`);
      const checked = lake("env", "lean", file);
      assert.equal(checked.status, 0, `${name}: ${checked.stdout}${checked.stderr}`);
    }
  });
}
