#!/usr/bin/env node
/**
 * `pata run [dir]`: runs the harness in `dir` on this machine, with the local host (`run.js`), and answers each ask
 * at the terminal. `dir` holds one `.pata` harness and its capabilities' contracts, one `<name>.json` each, as
 * `examples/claude-code` does. Without `dir`, it runs that example, which the package ships.
 *
 * The host's implementations: `net.ask` asks you, `net.fetch` is Node's `fetch`, and `fs` reads and writes under
 * `--root`. `shell` is refused, since nothing here keeps a command off the network. For the research template
 * (`examples/research`), `reader.extract` is a demo that reads a page's title, and `report.write` prints the report to
 * standard error. The model, `brain.choose`, is a module you pass with `--model`, or the scripted demo: it fetches the
 * first URL in the task and answers.
 */

import { existsSync } from "node:fs";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { isAbsolute, join, relative, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { run } from "./run.js";

const USAGE = `usage: pata run [dir] [--input <task>] [--root <dir>] [--model <module.js>]

  dir      a harness and its contracts (default: T1's example, examples/claude-code)
  --input  the task the run starts with (default: a demo task)
  --root   the directory fs reads and writes under (default: the current one)
  --model  a module whose default export answers brain.choose (default: a scripted demo)`;

const DEMO_TASK = "Fetch https://example.com/ and tell me its title.";

/** T1's example: in the package, where `prepack` copies it, or in the repo. */
const EXAMPLE = ["./examples/claude-code", "../examples/claude-code"]
  .map((path) => fileURLToPath(new URL(path, import.meta.url)))
  .find((path) => existsSync(path));

/** Lines read from standard input, one per ask, whether it's a terminal or a pipe. */
function lines() {
  const rl = createInterface({ input: process.stdin, terminal: false });
  const heard = [];
  const waiting = [];
  let ended = false;
  rl.on("line", (line) => (waiting.length > 0 ? waiting.shift()(line) : heard.push(line)));
  rl.on("close", () => {
    ended = true;
    for (const w of waiting.splice(0)) w(null);
  });
  return {
    next: () =>
      heard.length > 0
        ? Promise.resolve(heard.shift())
        : ended
          ? Promise.resolve(null)
          : new Promise((r) => waiting.push(r)),
    close: () => rl.close(),
  };
}

/** The scripted demo model: fetch the first URL in the task, then answer from what came back. */
async function demoModel({ messages }) {
  const task = messages[0]?.content ?? "";
  const last = messages.at(-1);
  if (messages.length === 1) {
    const url = task.match(/https?:\/\/[^\s"'<>]+[^\s"'<>.,;:!?)]/)?.[0];
    return url
      ? { kind: "net.fetch", url }
      : { kind: "answer", text: "There's no URL in the task." };
  }
  // The research template tells its planner only which URL it read: the page went to the report.
  if (last.content.startsWith("read "))
    return { kind: "answer", text: "What I found is in the report." };
  const title = titleOf(last.content);
  if (last.content === "denied")
    return { kind: "answer", text: "You didn't allow it, so I didn't fetch it." };
  return {
    kind: "answer",
    text: title ? `Its title is "${title}".` : `It answered: ${last.content.slice(0, 200)}`,
  };
}

/** A page's title, if it has one. */
function titleOf(page) {
  return page.match(/<title>([^<]*)<\/title>/i)?.[1]?.trim();
}

/** `path` under `root`, or a throw if it would leave it. */
function under(root, path) {
  const full = resolve(root, path);
  const rel = relative(root, full);
  if (rel.startsWith("..") || isAbsolute(rel)) throw new Error(`${path} is outside ${root}`);
  return full;
}

/** The host's handlers, each printing what it does to standard error, and why it failed if it did. */
function handlers(given) {
  const said = {};
  for (const [op, handle] of Object.entries(hosted(given))) {
    said[op] = (args, context) =>
      handle(args, context).catch((error) => {
        process.stderr.write(`${op}: failed: ${error?.message ?? error}\n`);
        throw error;
      });
  }
  return said;
}

function hosted({ root, model, ask }) {
  const say = (text) => process.stderr.write(`${text}\n`);
  const refused = (op) => async () => {
    throw new Error(
      `${op} isn't available: the local runner has no sandbox to keep a command off the network`,
    );
  };
  return {
    "brain.choose": async (args) => {
      const choice = await model(args);
      say(`model: ${choice.kind === "answer" ? "answers" : `chooses ${choice.kind}`}`);
      return choice;
    },
    "net.ask": async ({ url }) => {
      process.stderr.write(`ask: allow ${url}? [y/N] `);
      const line = await ask();
      const answer = /^\s*y(es)?\s*$/i.test(line ?? "") ? "allow" : "deny";
      // A terminal shows what was typed; a pipe doesn't.
      if (!process.stdin.isTTY) say(line ?? "");
      say(`ask: ${answer === "allow" ? "allowed" : "denied"}`);
      return answer;
    },
    "net.fetch": async ({ url }, { signal }) => {
      say(`net.fetch: ${url}`);
      const response = await fetch(url, { signal });
      return await response.text();
    },
    "fs.read": async ({ path }) => {
      say(`fs.read: ${path}`);
      return await readFile(under(root, path), "utf8");
    },
    "fs.write": async ({ path, text }) => {
      say(`fs.write: ${path}`);
      await writeFile(under(root, path), text);
      return "written";
    },
    "reader.extract": async ({ messages }) => {
      const page = messages.at(-1)?.content ?? "";
      say("reader: reads the page");
      const title = titleOf(page);
      return title ? `The page is titled "${title}".` : "The page has no title.";
    },
    "report.write": async ({ text }) => {
      say(`report: ${text}`);
      return "written";
    },
    "shell.run": refused("shell.run"),
    "shell.online": refused("shell.online"),
  };
}

/** The harness and contracts in `dir`. */
async function load(dir) {
  const names = await readdir(dir);
  const harnesses = names.filter((n) => n.endsWith(".pata"));
  if (harnesses.length !== 1)
    throw new Error(`${dir} needs exactly one .pata harness, not ${harnesses.length}`);
  const capabilities = {};
  for (const name of names.filter((n) => n.endsWith(".json"))) {
    capabilities[name.slice(0, -".json".length)] = JSON.parse(
      await readFile(join(dir, name), "utf8"),
    );
  }
  return { source: await readFile(join(dir, harnesses[0]), "utf8"), capabilities };
}

async function main(argv) {
  const { values, positionals } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      input: { type: "string" },
      root: { type: "string" },
      model: { type: "string" },
      help: { type: "boolean", short: "h" },
    },
  });
  if (values.help || positionals[0] !== "run" || positionals.length > 2) {
    console.error(USAGE);
    return values.help ? 0 : 2;
  }
  const { source, capabilities } = await load(positionals[1] ?? EXAMPLE);
  const model = values.model
    ? (await import(pathToFileURL(resolve(values.model)).href)).default
    : demoModel;
  if (typeof model !== "function")
    throw new Error(`${values.model} has no default export to answer brain.choose`);

  const input = values.input ?? DEMO_TASK;
  process.stderr.write(`task: ${input}\n`);
  const stdin = lines();
  try {
    const { outcome } = await run(
      source,
      capabilities,
      handlers({ root: resolve(values.root ?? "."), model, ask: stdin.next }),
      { input },
    );
    console.log(JSON.stringify(outcome));
    return outcome?.kind === "halted" ? 0 : 1;
  } finally {
    stdin.close();
  }
}

main(process.argv.slice(2)).then(
  (code) => process.exit(code),
  (error) => {
    console.error(`pata: ${error?.message ?? error}`);
    process.exit(1);
  },
);
