// Fails when a git-tracked text file holds a character outside ASCII, unless the file is exempt below.
// Binary files (any with a NUL byte) are skipped.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

// What may hold non-ASCII, and why. "dir/" exempts a directory, "*.ext" an extension, anything else one file.
const EXEMPT = [
  ["*.lean", "Lean sources use Lean's notation: forall, not, and, guillemets for names"],
  ["crates/kernel/src/smtlib/lean.rs", "writes Lean, so its code and tests hold Lean's notation"],
  ["proofs/lean.test.mjs", "its tests embed Lean sources"],
  ["docs/2026-11-06-lean.md", "shows the Lean the kernel writes"],
  ["docs/2026-11-06-claims.md", "its table gives the logic symbols for the property names"],
];

const exempt = (path) =>
  EXEMPT.some(([pattern]) =>
    pattern.endsWith("/")
      ? path.startsWith(pattern)
      : pattern.startsWith("*")
        ? path.endsWith(pattern.slice(1))
        : path === pattern,
  );

const files = execFileSync("git", ["ls-files", "-z"], { encoding: "utf8" })
  .split("\0")
  .filter(Boolean);
let bad = 0;
for (const path of files) {
  if (exempt(path)) continue;
  let bytes;
  try {
    bytes = readFileSync(path);
  } catch {
    continue; // Deleted in the working tree.
  }
  if (bytes.includes(0)) continue;
  if (bytes.every((b) => b < 0x80)) continue;
  const lines = bytes.toString("utf8").split("\n");
  for (const [i, line] of lines.entries()) {
    let column = 0;
    for (const char of line) {
      column++;
      const code = char.codePointAt(0);
      if (code < 0x80) continue;
      const hex = code.toString(16).toUpperCase().padStart(4, "0");
      console.error(`${path}:${i + 1}:${column}: U+${hex} ${JSON.stringify(char)}`);
      bad++;
    }
  }
}
if (bad > 0) {
  console.error(
    `\n${bad} non-ASCII character(s). Use ASCII, or exempt the file in scripts/ascii.mjs.`,
  );
  process.exit(1);
}
