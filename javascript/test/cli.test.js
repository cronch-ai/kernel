import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";

// `pata run` on T1's example, with the scripted model, answering its ask through standard input.
const cli = new URL("../cli.js", import.meta.url).pathname;
const example = new URL("../../examples/claude-code", import.meta.url).pathname;

let server;
let url;
const fetched = [];
before(async () => {
  server = createServer((request, response) => {
    fetched.push(request.url);
    response.end("<html><title>Local page</title></html>");
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  url = `http://127.0.0.1:${server.address().port}/page`;
});
after(() => server.close());

function pata(args, stdin = "", command = cli) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [command, ...args]);
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("close", (code) => resolve({ code, stdout, stderr }));
    child.stdin.end(stdin);
  });
}

test("an allowed URL is fetched, after the ask", async () => {
  fetched.length = 0;
  const { code, stdout, stderr } = await pata(["run", example, "--input", `Read ${url}.`], "y\n");
  assert.equal(code, 0, stderr);
  assert.match(
    stderr,
    new RegExp(`ask: allow ${url}\\? \\[y/N\\] y\\nask: allowed\\nnet.fetch: ${url}`),
  );
  assert.deepEqual(JSON.parse(stdout), {
    kind: "halted",
    label: ["model"],
    value: 'Its title is "Local page".',
  });
  assert.deepEqual(fetched, ["/page"]);
});

test("a denied URL, or no answer at all, isn't fetched", async () => {
  fetched.length = 0;
  for (const answer of ["n\n", ""]) {
    const { code, stdout, stderr } = await pata(
      ["run", example, "--input", `Read ${url}.`],
      answer,
    );
    assert.equal(code, 0, stderr);
    assert.match(stderr, /ask: denied/);
    assert.doesNotMatch(stderr, /^net\.fetch:/m);
    assert.equal(JSON.parse(stdout).value, "You didn't allow it, so I didn't fetch it.");
  }
  assert.deepEqual(fetched, []);
});

test("a model plugged in uses fs under --root, and nowhere else; shell is refused", async () => {
  const root = await mkdtemp(join(tmpdir(), "pata-root-"));
  await writeFile(join(root, "in.txt"), "hello");
  const model = join(root, "..", `pata-model-${process.pid}.mjs`);
  await writeFile(
    model,
    `const turns = [
      { kind: "fs.read", path: "in.txt" },
      { kind: "fs.write", path: "out.txt", text: "copied" },
      { kind: "fs.read", path: "../escape.txt" },
      { kind: "shell.run", command: "true" },
    ];
    export default async ({ messages }) =>
      turns[(messages.length - 1) / 2] ?? { kind: "answer", text: messages.filter((m) => m.role === "tool").map((m) => m.content).join(" | ") };`,
  );
  const { code, stdout, stderr } = await pata(["run", example, "--root", root, "--model", model]);
  assert.equal(code, 0, stderr);
  const [read, written, escaped, shell] = JSON.parse(stdout).value.split(" | ");
  assert.equal(read, "hello");
  assert.equal(written, "written");
  // The model hears that each failed; the terminal shows why.
  assert.match(escaped, /^failed/);
  assert.match(shell, /^failed/);
  assert.match(stderr, /fs\.read: failed: \.\.\/escape\.txt is outside/);
  assert.match(stderr, /shell\.run: failed: shell\.run isn't available/);
  assert.equal(await readFile(join(root, "out.txt"), "utf8"), "copied");
  assert.doesNotMatch(stderr, /^ask:/m, "fs needs no ask");
});

test("says how to use it", async () => {
  const { code, stderr } = await pata(["help"]);
  assert.equal(code, 2);
  assert.match(stderr, /usage: pata run \[dir\]/);
});

// What `npx @cronch/kernel run` does on a machine with nothing but Node: the packed package, installed into an
// empty directory, runs T1's example, which it ships, with no directory given.
test("the package, installed, runs T1's example by itself", async () => {
  const dir = await mkdtemp(join(tmpdir(), "pata-npx-"));
  const javascript = new URL("..", import.meta.url).pathname;
  const [{ filename }] = JSON.parse(
    execFileSync("npm", ["pack", "--json", "--pack-destination", dir], { cwd: javascript }),
  );
  execFileSync("npm", ["install", "--no-audit", "--no-fund", join(dir, filename)], { cwd: dir });
  const bin = join(dir, "node_modules/@cronch/kernel/cli.js");
  fetched.length = 0;
  const { code, stdout, stderr } = await pata(["run", "--input", `Read ${url}.`], "y\n", bin);
  assert.equal(code, 0, stderr);
  assert.equal(JSON.parse(stdout).value, 'Its title is "Local page".');
  assert.deepEqual(fetched, ["/page"]);
});
