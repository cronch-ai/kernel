// What `check-lean.mjs` accepts and refuses, over T1's `net.fetch` obligation for `net.ask.onResult`: a proof, and the
// attacks on the tie between a proof and its statement.
// Run with `npm run test:proofs` after `scripts/install-lean.sh` and `npm run build`. Comparator needs `bwrap`.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { instantiate } from "../javascript/index.js";
import { checkLean } from "./check-lean.mjs";

const kernel = await instantiate(
  readFileSync(new URL("../javascript/kernel.wasm", import.meta.url)),
);
const example = (name) =>
  JSON.parse(readFileSync(new URL(`../examples/claude-code/${name}.json`, import.meta.url)));
const installed = kernel.install(
  ["brain", "fs", "shell", "net", "session"].map((prefix) => ({
    prefix,
    capability: example(prefix),
  })),
);
const lit = (v) => ({ e: "lit", v });
const v = (name) => ({ e: "var", name });
const get = (of, field) => ({ e: "get", of, field });
const op = (name, ...args) => ({ e: "op", op: name, args });
const out = kernel.ruleObligations({
  interface: installed.interface,
  rule: {
    op: "net.fetch",
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
  },
  format: "lean",
});
assert.equal(out.kind, "obligations", JSON.stringify(out));
const statement = out.obligations.find((o) => o.name === "net.ask.onResult").text;

/** The statement as the solution restates it, with `before` ahead of its definition, and then `proof`. */
const solution = (proof, before = "") => {
  const at = statement.indexOf("/-- The obligation holds");
  return `${statement.slice(0, at)}${before}${statement.slice(at)}\n${proof}\n`;
};

const refused = (text, why) => {
  const checked = checkLean(statement, text);
  assert.equal(checked.ok, false, `accepted: ${text}`);
  if (why) assert.match(checked.error, why);
};

test("a proof of the statement is accepted", () => {
  assert.deepEqual(
    checkLean(statement, solution("theorem obligation : statement := by\n  grind")),
    { ok: true },
  );
});

// Each of these builds: what refuses it is Comparator's comparison with the challenge.
const differs = /Const does not match between challenge and target/;

test("a proof of another statement is refused", () => {
  const other = `import Cronch.Smt\n\nabbrev statement : Prop := True\n\ntheorem obligation : statement := trivial\n`;
  refused(other, differs);
});

test("sorry and admit are refused", () => {
  refused(solution("theorem obligation : statement := sorry"), /Illegal axiom detected: .sorryAx./);
  refused(
    solution("theorem obligation : statement := by\n  admit"),
    /Illegal axiom detected: .sorryAx./,
  );
});

test("a new axiom is refused", () => {
  refused(
    solution("axiom cheat : statement\n\ntheorem obligation : statement := cheat"),
    /Illegal axiom detected: .cheat./,
  );
});

test("native_decide is refused", () => {
  refused(
    solution(
      "theorem shortcut : 2 + 2 = 4 := by native_decide\n\ntheorem obligation : statement := by\n  have := shortcut\n  grind",
    ),
    /Illegal axiom detected/,
  );
});

test("a solution that gives a name from Cronch/Smt.lean another meaning is refused", () => {
  // Without the library, the solution defines `seq.contains` itself: never true, so the statement is trivial.
  const own =
    "namespace Smt\ndef seq.contains (_s _t : List α) : Prop := False\nend Smt\n\nopen Classical\n\n";
  const text = solution(
    "theorem obligation : statement := by\n  simp [statement, Smt.seq.contains]",
  ).replace("import Cronch.Smt\n", "");
  refused(text.replace("open Classical\n", own), differs);
});

test("a solution that changes a notation or an instance first is refused", () => {
  // `¬ p` is `True`, so the statement holds.
  refused(
    solution(
      "theorem obligation : statement := by\n  simp",
      "macro_rules | `(¬ $p) => `(True)\n\n",
    ),
    differs,
  );
  // `a ++ b` is `[]`, so no result is one of `["allow"] ++ ["deny"]`, and the statement holds.
  refused(
    solution(
      "theorem obligation : statement := by\n  simp [statement, cheat, HAppend.hAppend]",
      "local instance (priority := high) cheat : HAppend (List String) (List String) (List String) :=\n  ⟨fun _ _ => []⟩\n\n",
    ),
    differs,
  );
});
