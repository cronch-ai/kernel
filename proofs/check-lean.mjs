#!/usr/bin/env node
// Checks a Lean proof against one of the kernel's Lean statements, with no part of Cronch:
// `node proofs/check-lean.mjs statement.lean solution.lean`.
//
// The statement is what `ruleObligations` writes with `format: "lean"`: a module defining `statement`. The solution is
// a Lean module that proves `theorem obligation : statement`, restating whatever it needs, as the statement does.
//
// Lake's `comparator` (Lean's own, since 4.35.0) is the judge:
//
// - The challenge is the statement and `theorem obligation : statement := sorry`, written here, so the solution never
//   sees it. Everything it imports is `lean/Cronch/Smt.lean`, copied from this repository.
// - Comparator builds and exports the solution in a `bwrap` sandbox, and never loads its `.olean` files.
// - It checks that `obligation` in the solution has the challenge's statement, with every declaration it uses the same
//   as in the challenge, so a solution can't change what a name, a notation or an instance means.
// - It checks that the proof uses no axiom but `propext`, `Quot.sound` and `Classical.choice`, which refuses `sorry`,
//   `admit`, a new `axiom` and `native_decide`, and that Lean's kernel accepts it.

import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { tools } from "./check.mjs";

const lean = fileURLToPath(new URL("../lean/", import.meta.url));

export const PERMITTED_AXIOMS = ["propext", "Quot.sound", "Classical.choice"];

/** `{ ok: true }` when `solution` proves `statement`, or `{ ok: false, error }` with Comparator's output. */
export function checkLean(statement, solution) {
  const dir = mkdtempSync(join(tmpdir(), "check-lean-"));
  try {
    mkdirSync(join(dir, "Cronch"));
    copyFileSync(join(lean, "lean-toolchain"), join(dir, "lean-toolchain"));
    copyFileSync(join(lean, "Cronch.lean"), join(dir, "Cronch.lean"));
    copyFileSync(join(lean, "Cronch", "Smt.lean"), join(dir, "Cronch", "Smt.lean"));
    writeFileSync(
      join(dir, "lakefile.toml"),
      'name = "check"\n\n[[lean_lib]]\nname = "Cronch"\n\n[[lean_lib]]\nname = "Challenge"\n\n[[lean_lib]]\nname = "Solution"\n',
    );
    // Comparator resolves dependencies in its sandbox, where it can't write one.
    writeFileSync(
      join(dir, "lake-manifest.json"),
      JSON.stringify({
        version: "1.3.0",
        packagesDir: ".lake/packages",
        packages: [],
        name: "check",
        lakeDir: ".lake",
      }),
    );
    writeFileSync(
      join(dir, "Challenge.lean"),
      `${statement}\ntheorem obligation : statement := sorry\n`,
    );
    writeFileSync(join(dir, "Solution.lean"), solution);
    writeFileSync(
      join(dir, "comparator.json"),
      JSON.stringify({
        challenge_module: "Challenge",
        solution_module: "Solution",
        theorem_names: ["obligation"],
        permitted_axioms: PERMITTED_AXIOMS,
      }),
    );
    const run = spawnSync(join(tools, "lean", "bin", "lake"), ["comparator"], {
      cwd: dir,
      encoding: "utf8",
      maxBuffer: 1 << 26,
    });
    if (run.error) return { ok: false, error: `lake didn't run: ${run.error.message}` };
    const output = `${run.stdout}${run.stderr}`;
    // Comparator says so when it accepts; anything else is a refusal.
    if (run.status === 0 && output.includes("Your solution is okay!")) return { ok: true };
    return { ok: false, error: output.trim() };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [statement, solution] = process.argv.slice(2).map((p) => readFileSync(resolve(p), "utf8"));
  const out = checkLean(statement, solution);
  if (!out.ok) {
    console.error(out.error);
    process.exit(1);
  }
  console.log("correct");
}
