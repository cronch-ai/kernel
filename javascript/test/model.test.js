import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { after, before, test } from "node:test";
import {
  ANTHROPIC_MODEL,
  anthropic,
  model,
  OPENAI_MODEL,
  openai,
  openrouter,
  TOOL_RESULT,
} from "../run.js";

// The model port and its providers, with `fetch` stubbed: nothing here reaches a real API.
const contract = async (example, name) =>
  JSON.parse(
    await readFile(new URL(`../../examples/${example}/${name}.json`, import.meta.url), "utf8"),
  );
const brain = await contract("claude-code", "brain");
const reader = await contract("research", "reader");
const context = { owner: "o", run: "r", origin: [], call: 1 };
const history = [
  { role: "system", content: "Be brief." },
  { role: "user", content: "Read https://a.example/." },
  { role: "assistant", content: '{kind: "net.fetch", url: "https://a.example/"}' },
  { role: "tool", content: "" },
];

/** A `fetch` that answers each request with the next of `replies`, keeping what it was sent. */
function stub(...replies) {
  const sent = [];
  const fetch = async (url, init) => {
    sent.push({ url, headers: init.headers, body: JSON.parse(init.body) });
    const [status, body] = replies.shift();
    return new Response(JSON.stringify(body), { status });
  };
  return { fetch, sent };
}

test("Anthropic: a choice is offered as tools, and a tool call is its case", async () => {
  const { fetch, sent } = stub([
    200,
    {
      stop_reason: "tool_use",
      content: [
        { type: "thinking", thinking: "" },
        { type: "text", text: "Fetching." },
        { type: "tool_use", id: "t", name: "net_fetch", input: { url: "https://b.example/" } },
      ],
    },
  ]);
  const capability = model(anthropic({ apiKey: "k", fetch }), brain);
  const out = await capability.perform("choose", { messages: history }, context);
  assert.deepEqual(out, { ok: true, value: { kind: "net.fetch", url: "https://b.example/" } });
  const [{ url, headers, body }] = sent;
  assert.equal(url, "https://api.anthropic.com/v1/messages");
  assert.equal(headers["x-api-key"], "k");
  assert.equal(headers["anthropic-version"], "2026-10-21");
  assert.equal(body.model, ANTHROPIC_MODEL);
  assert.equal(body.system, "Be brief.");
  assert.deepEqual(body.messages, [
    { role: "user", content: "Read https://a.example/." },
    { role: "assistant", content: '{kind: "net.fetch", url: "https://a.example/"}' },
    { role: "user", content: TOOL_RESULT },
  ]);
  assert.deepEqual(
    body.tools.map((t) => t.name),
    ["fs_read", "fs_write", "shell_run", "shell_online", "net_fetch"],
  );
  assert.deepEqual(body.tools[1].input_schema, {
    type: "object",
    properties: {
      path: { type: "string", description: "path" },
      text: { type: "string", description: "text" },
    },
    required: ["path", "text"],
    additionalProperties: false,
  });
});

test("Anthropic: text alone is an answer, and a refusal or an error fails the call", async () => {
  const { fetch } = stub(
    [200, { stop_reason: "end_turn", content: [{ type: "text", text: "Done." }] }],
    [200, { stop_reason: "refusal", content: [] }],
    [529, { type: "error" }],
  );
  const capability = model(anthropic({ apiKey: "k", fetch, model: "claude-haiku-5-5" }), brain);
  const args = { messages: history.slice(0, 2) };
  assert.deepEqual(await capability.perform("choose", args, context), {
    ok: true,
    value: { kind: "answer", text: "Done." },
  });
  for (const _ of [0, 1]) {
    const out = await capability.perform("choose", args, context);
    assert.equal(out.ok, false);
    assert.equal(out.error.kind, "model");
  }
});

test("a choice that doesn't fit its case, or names no tool offered, fails", async () => {
  const { fetch } = stub(
    [200, { content: [{ type: "tool_use", name: "net_fetch", input: { url: 1 } }] }],
    [200, { content: [{ type: "tool_use", name: "net_fetch", input: { url: "u", x: "y" } }] }],
    [200, { content: [{ type: "tool_use", name: "rm_rf", input: {} }] }],
  );
  const capability = model(anthropic({ apiKey: "k", fetch }), brain);
  const kinds = [];
  for (const _ of [0, 1, 2])
    kinds.push((await capability.perform("choose", { messages: history }, context)).error.kind);
  assert.deepEqual(kinds, ["badChoice", "badChoice", "unknownTool"]);
});

test("OpenAI: the Responses API, with function tools and their arguments as JSON", async () => {
  const { fetch, sent } = stub(
    [
      200,
      {
        output: [
          { type: "reasoning", summary: [] },
          {
            type: "function_call",
            call_id: "c",
            name: "net_fetch",
            arguments: '{"url":"https://b.example/"}',
          },
        ],
      },
    ],
    [
      200,
      {
        output: [
          {
            type: "message",
            role: "assistant",
            content: [{ type: "output_text", text: "Noted." }],
          },
        ],
      },
    ],
  );
  const provider = openai({ apiKey: "k", fetch });
  const chose = await model(provider, brain).perform("choose", { messages: history }, context);
  assert.deepEqual(chose, { ok: true, value: { kind: "net.fetch", url: "https://b.example/" } });
  const noted = await model(provider, reader).perform("extract", { messages: history }, context);
  assert.deepEqual(noted, { ok: true, value: "Noted." });
  const [first, second] = sent;
  assert.equal(first.url, "https://api.openai.com/v1/responses");
  assert.equal(first.headers.authorization, "Bearer k");
  assert.equal(first.body.model, OPENAI_MODEL);
  assert.deepEqual(first.body.input.at(-1), { role: "user", content: TOOL_RESULT });
  assert.deepEqual(first.body.tools.at(-1), {
    type: "function",
    name: "net_fetch",
    description: "The net.fetch operation.",
    parameters: {
      type: "object",
      properties: { url: { type: "string", description: "url" } },
      required: ["url"],
      additionalProperties: false,
    },
    strict: true,
  });
  assert.equal(second.body.tools, undefined, "an operation answering with text offers no tools");
});

test("OpenRouter: Chat Completions, with the account's default model unless one is named", async () => {
  const { fetch, sent } = stub(
    [
      200,
      {
        choices: [
          {
            message: {
              role: "assistant",
              content: null,
              tool_calls: [
                {
                  id: "c",
                  type: "function",
                  function: { name: "net_fetch", arguments: '{"url":"https://c.example/"}' },
                },
              ],
            },
          },
        ],
      },
    ],
    [200, { choices: [{ message: { role: "assistant", content: "Noted." } }] }],
    [200, { error: { code: 402, message: "Insufficient credits" } }],
  );
  const chose = await model(openrouter({ apiKey: "k", fetch }), brain).perform(
    "choose",
    { messages: history },
    context,
  );
  assert.deepEqual(chose, { ok: true, value: { kind: "net.fetch", url: "https://c.example/" } });
  const named = openrouter({ apiKey: "k", model: "vendor/some-model", fetch });
  const noted = await model(named, reader).perform("extract", { messages: history }, context);
  assert.deepEqual(noted, { ok: true, value: "Noted." });
  const refused = await model(named, reader).perform("extract", { messages: history }, context);
  assert.equal(refused.error.kind, "model");
  assert.match(refused.error.detail, /Insufficient credits/);
  const [first, second] = sent;
  assert.equal(first.url, "https://openrouter.ai/api/v1/chat/completions");
  assert.equal(first.headers.authorization, "Bearer k");
  assert.equal(first.body.model, undefined, "no model: OpenRouter's account default");
  assert.deepEqual(first.body.messages.at(0), { role: "system", content: "Be brief." });
  assert.deepEqual(first.body.messages.at(-1), { role: "user", content: TOOL_RESULT });
  assert.deepEqual(first.body.tools.at(-1), {
    type: "function",
    function: {
      name: "net_fetch",
      description: "The net.fetch operation.",
      parameters: {
        type: "object",
        properties: { url: { type: "string", description: "url" } },
        required: ["url"],
        additionalProperties: false,
      },
    },
  });
  assert.equal(second.body.model, "vendor/some-model");
  assert.equal(second.body.tools, undefined, "an operation answering with text offers no tools");
});

// `pata run --model anthropic`, `--model openai` and `--model openrouter`, against a local stand-in for each API.
const cli = new URL("../cli.js", import.meta.url).pathname;
const examples = new URL("../../examples/", import.meta.url).pathname;
let server;
let base;
let page;
const requests = [];
before(async () => {
  server = createServer(async (request, response) => {
    let body = "";
    for await (const chunk of request) body += chunk;
    if (request.url === "/page") return response.end("<html><title>Local page</title></html>");
    const sent = JSON.parse(body);
    requests.push({ path: request.url, body: sent });
    const turn = (sent.messages ?? sent.input).filter((m) => m.role !== "system").length;
    const tools = sent.tools !== undefined;
    response.setHeader("content-type", "application/json");
    if (request.url === "/v1/messages") {
      const content =
        tools && turn === 1
          ? [{ type: "tool_use", id: "t", name: "net_fetch", input: { url: page } }]
          : [{ type: "text", text: tools ? "It's titled Local page." : "A note." }];
      return response.end(JSON.stringify({ stop_reason: "end_turn", content }));
    }
    if (request.url === "/v1/chat/completions") {
      const message =
        tools && turn === 1
          ? {
              role: "assistant",
              content: null,
              tool_calls: [
                {
                  id: "c",
                  type: "function",
                  function: { name: "net_fetch", arguments: JSON.stringify({ url: page }) },
                },
              ],
            }
          : { role: "assistant", content: tools ? "It's titled Local page." : "A note." };
      return response.end(JSON.stringify({ choices: [{ message }] }));
    }
    const output =
      tools && turn === 1
        ? [{ type: "function_call", name: "net_fetch", arguments: JSON.stringify({ url: page }) }]
        : [
            {
              type: "message",
              content: [{ type: "output_text", text: tools ? "It's in the report." : "A note." }],
            },
          ];
    response.end(JSON.stringify({ output }));
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  base = `http://127.0.0.1:${server.address().port}`;
  page = `${base}/page`;
});
after(() => server.close());

function pata(args, env, stdin = "") {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [cli, ...args], { env: { ...process.env, ...env } });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("close", (code) => resolve({ code, stdout, stderr }));
    child.stdin.end(stdin);
  });
}

test("--model anthropic runs T1 against the model, and the ask still happens", async () => {
  requests.length = 0;
  const env = { ANTHROPIC_API_KEY: "k", ANTHROPIC_BASE_URL: base };
  const args = ["run", `${examples}claude-code`, "--model", "anthropic:claude-sonnet-5-5"];
  const { code, stdout, stderr } = await pata([...args, "--input", "Read the page."], env, "y\n");
  assert.equal(code, 0, stderr);
  assert.match(stderr, new RegExp(`ask: allow ${page}\\? \\[y/N\\] y\\nask: allowed\\nnet.fetch:`));
  assert.deepEqual(JSON.parse(stdout), {
    kind: "halted",
    label: ["model"],
    value: "It's titled Local page.",
  });
  assert.deepEqual(
    requests.map((r) => r.body.model),
    ["claude-sonnet-5-5", "claude-sonnet-5-5"],
  );
  assert.match(requests[1].body.messages.at(-1).content, /^Tool result:\n<html>/);
});

test("--model openai runs the research template, whose reader is the model too", async () => {
  requests.length = 0;
  const env = { OPENAI_API_KEY: "k", OPENAI_BASE_URL: base };
  const args = ["run", `${examples}research`, "--model", "openai", "--input", "Research it."];
  const { code, stdout, stderr } = await pata(args, env);
  assert.equal(code, 0, stderr);
  assert.match(stderr, /report: A note\./);
  assert.equal(JSON.parse(stdout).value, "It's in the report.");
  assert.deepEqual(
    requests.map((r) => r.body.tools?.length ?? 0),
    [1, 0, 1],
    "planner, reader, planner",
  );
});

test("--model openrouter runs T1 against the model it names, and the ask still happens", async () => {
  requests.length = 0;
  const env = { OPENROUTER_API_KEY: "k", OPENROUTER_BASE_URL: base };
  const args = ["run", `${examples}claude-code`, "--model", "openrouter:vendor/some-model"];
  const { code, stdout, stderr } = await pata([...args, "--input", "Read the page."], env, "y\n");
  assert.equal(code, 0, stderr);
  assert.match(stderr, new RegExp(`ask: allow ${page}\\? \\[y/N\\] y\\nask: allowed\\nnet.fetch:`));
  assert.equal(JSON.parse(stdout).value, "It's titled Local page.");
  assert.deepEqual(
    requests.map((r) => [r.path, r.body.model]),
    [
      ["/v1/chat/completions", "vendor/some-model"],
      ["/v1/chat/completions", "vendor/some-model"],
    ],
  );
});

test("--model anthropic with no key says what to set", async () => {
  const { code, stderr } = await pata(["run", "--model", "anthropic"], { ANTHROPIC_API_KEY: "" });
  assert.equal(code, 1);
  assert.match(stderr, /needs ANTHROPIC_API_KEY set/);
});
