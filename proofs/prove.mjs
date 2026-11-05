#!/usr/bin/env node
// Asks cvc5 about a query. Its answer is only a search: `check.mjs` is what a proof has to pass.
// `node proofs/prove.mjs query.smt2 > proof.cpc`

import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { tools } from "./check.mjs";

/** `{ answer: "unsat", proof }`, or `{ answer }` with `sat`, `unknown` or an error. */
export function prove(queryPath) {
  const run = spawnSync(
    join(tools, "cvc5"),
    [
      "--dump-proofs",
      "--proof-format-mode=cpc",
      "--proof-print-reference",
      "--proof-granularity=dsl-rewrite",
      "--dag-thresh=0",
      queryPath,
    ],
    { encoding: "utf8", maxBuffer: 1 << 28 },
  );
  if (run.error) return { answer: `cvc5 didn't run: ${run.error.message}` };
  const lines = run.stdout.trim().split("\n");
  if (lines[0] !== "unsat") return { answer: lines[0] || run.stderr.trim() };
  // cvc5 wraps the commands in one pair of parentheses.
  return { answer: "unsat", proof: `${lines.slice(2, -1).join("\n")}\n` };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const out = prove(process.argv[2]);
  if (out.answer !== "unsat") {
    console.error(out.answer);
    process.exit(1);
  }
  process.stdout.write(out.proof);
}
