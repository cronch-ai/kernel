#!/usr/bin/env node
// Checks a proof against a query, with no part of Cronch: `node proofs/check.mjs query.smt2 proof.cpc`.
//
// A checker's "correct" means nothing unless the proof is tied to the query, so this is strict about how:
//
// - The reference is a command inside the file Ethos reads. Ethos's `--reference` flag doesn't check a proof's
//   assumptions against the query: it accepts a proof of one query against another.
// - Ethos must say `correct`. It says `incomplete` of a proof with unchecked steps.
// - A proof holds only the commands cvc5 writes, and names only its own `@` terms. Nothing else may change what the
//   query's symbols mean by the time its assumptions are compared with it.
// - A `define` is only of the shape cvc5 writes, a nullary `@t<n>`, and nothing else binds its name. Ethos's
//   `define` of a bound name overloads it, and an overload can be selected where the name is used, premises included,
//   so a proof may never give a name a second meaning: not a query's, not the signature's, not its own.

import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const tools =
  process.env.PROOF_TOOLS ?? fileURLToPath(new URL("../.proof-tools", import.meta.url));

const ALLOWED = new Set(["define", "assume", "assume-push", "step", "step-pop"]);

/** The top-level commands of a proof, as `[name, first argument, the rest]`, or why it isn't a list of commands. */
function commands(proof) {
  const out = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < proof.length; i++) {
    const c = proof[i];
    if (c === '"') {
      // A quote inside a string is doubled, which reads as two strings side by side here: harmless.
      i = proof.indexOf('"', i + 1);
      if (i < 0) return { error: "an unterminated string" };
    } else if (c === "|") {
      i = proof.indexOf("|", i + 1);
      if (i < 0) return { error: "an unterminated symbol" };
    } else if (c === ";") {
      const end = proof.indexOf("\n", i);
      i = end < 0 ? proof.length : end;
    } else if (c === "(") {
      if (depth === 0) start = i;
      depth++;
    } else if (c === ")") {
      depth--;
      if (depth < 0) return { error: "an unmatched parenthesis" };
      if (depth === 0) {
        const command = proof.slice(start + 1, i);
        out.push(command.match(/^\s*([^\s()]+)(?:\s+([^\s()]+))?([\s\S]*)$/) ?? [command]);
      }
    } else if (depth === 0 && !/\s/.test(c)) {
      return { error: `text outside a command at offset ${i}` };
    }
  }
  return depth === 0 ? { commands: out } : { error: "an unclosed parenthesis" };
}

/** Why a proof's commands aren't only what cvc5 writes, if they aren't. */
export function forbidden(proof) {
  const parsed = commands(proof);
  if (parsed.error) return parsed.error;
  // cvc5 reuses a step's name in another scope, so only a definition's name must be bound once.
  const defined = new Set();
  const named = new Set();
  for (const [, name, first, rest] of parsed.commands) {
    if (!ALLOWED.has(name)) return `the command ${name}`;
    if (name === "define") {
      if (!(/^@t\d+$/.test(first ?? "") && /^\s*\(\s*\)/.test(rest))) {
        return `a definition of ${first}`;
      }
      if (defined.has(first) || named.has(first)) return `a second binding of ${first}`;
      defined.add(first);
    } else if (first !== undefined) {
      if (defined.has(first)) return `a second binding of ${first}`;
      named.add(first);
    }
  }
  return null;
}

/** `{ ok: true }` if the proof checks against the query, or `{ ok: false, reason }`. */
export function check(queryPath, proof) {
  const query = resolve(queryPath);
  if (query.includes('"')) return { ok: false, reason: "a quote in the query's path" };
  const bad = forbidden(proof);
  if (bad) return { ok: false, reason: `the proof holds ${bad}` };
  const dir = mkdtempSync(join(tmpdir(), "proof-"));
  try {
    const file = join(dir, "proof.cpc");
    writeFileSync(file, `(reference "${query}")\n${proof}`);
    const run = spawnSync(
      join(tools, "ethos"),
      [
        `--include=${join(tools, "signatures/Cpc.eo")}`,
        `--include=${join(tools, "signatures/expert/CpcExpert.eo")}`,
        "--require-proof-of-false",
        file,
      ],
      { encoding: "utf8" },
    );
    if (run.error) return { ok: false, reason: `ethos didn't run: ${run.error.message}` };
    const lines = `${run.stdout}\n${run.stderr}`.split("\n").filter((l) => l.trim());
    const last = lines.at(-1) ?? "no output";
    return run.status === 0 && last.trim() === "correct"
      ? { ok: true }
      : { ok: false, reason: last };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [query, proof] = process.argv.slice(2);
  if (!query || !proof) {
    console.error("usage: check.mjs query.smt2 proof.cpc");
    process.exit(2);
  }
  const result = check(query, readFileSync(proof, "utf8"));
  console.log(result.ok ? "correct" : `rejected: ${result.reason}`);
  process.exit(result.ok ? 0 : 1);
}
