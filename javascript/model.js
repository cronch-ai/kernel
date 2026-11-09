/**
 * Models as capabilities. A `ModelProvider` is the port: it takes messages and the tools on offer, and answers with
 * text or tool calls. `model(provider, contract)` makes it a `Capability`, the shape the platform's capabilities have
 * too, by reading the contract:
 *
 * - An operation whose output is `String`, such as the research template's `reader.extract`, answers with text.
 * - An operation whose output is a variant chooses. It answers with `answer {text}`, or with one of its other cases,
 *   which the model is offered as tools. A choice carries the tool's input as the case's fields, as T1's
 *   `brain.choose` does.
 *
 * A choice that doesn't fit its case fails, rather than leaving the call pending: the kernel would withhold it.
 *
 * `anthropic` and `openai` are providers over plain `fetch`, with no SDK.
 */

/** What begins a tool's result, sent as the user's: a harness's `tool` message has no tool call id. */
export const TOOL_RESULT = "Tool result:\n";

/** A tool's name: providers allow only letters, digits, `_` and `-`, so `fs.read` is `fs_read`. */
export const toolName = (option) => option.replace(/[^A-Za-z0-9_-]/g, "_");

/** JSON Schema's name for each Pata type a tool's field may have. */
const SCHEMA = { String: "string", Int: "integer", Float: "number", Bool: "boolean" };

/** The model behind `provider`, as a capability under `contract`. */
export function model(provider, contract) {
  return {
    contract,
    async perform(op, args) {
      const output = contract.operations?.[op]?.output;
      if (output === undefined) return failed("badCall", { op });
      const messages = (args?.messages ?? []).map(provided);
      const variant = output?.Variant;
      if (variant === undefined) {
        const reply = await ask(provider, { messages, tools: [] });
        if (!reply.ok) return reply;
        return typeof reply.value.text === "string"
          ? { ok: true, value: reply.value.text }
          : failed("noText");
      }
      const tools = [];
      for (const [option, type] of Object.entries(variant.cases)) {
        if (option === "answer") continue;
        const record = type?.Record;
        const fields = record !== null && typeof record === "object" ? Object.entries(record) : [];
        if (fields.length === 0 || !fields.every(([, t]) => SCHEMA[t] !== undefined))
          return failed("unsupportedTool", { tool: option });
        const name = toolName(option);
        if (tools.some((t) => t.name === name)) return failed("toolNames", { tool: name });
        tools.push({ name, option, fields });
      }
      const reply = await ask(provider, { messages, tools: tools.map(offer) });
      if (!reply.ok) return reply;
      const [called] = reply.value.calls ?? [];
      if (called !== undefined) return chosen(variant.tag, tools, called);
      if (variant.cases.answer === undefined) return failed("noChoice");
      if (typeof reply.value.text !== "string") return failed("noText");
      return { ok: true, value: { [variant.tag]: "answer", text: reply.value.text } };
    },
  };
}

async function ask(provider, request) {
  try {
    return { ok: true, value: await provider.complete(request) };
  } catch (error) {
    return failed("model", { detail: String(error?.message ?? error) });
  }
}

const failed = (kind, fields = {}) => ({ ok: false, error: { kind, ...fields } });

function provided(message) {
  return message.role === "tool"
    ? { role: "user", content: TOOL_RESULT + message.content }
    : { role: message.role, content: message.content };
}

function offer({ name, option, fields }) {
  return {
    name,
    description: `The ${option} operation.`,
    parameters: {
      type: "object",
      properties: Object.fromEntries(
        fields.map(([field, t]) => [field, { type: SCHEMA[t], description: field }]),
      ),
      required: fields.map(([field]) => field),
      additionalProperties: false,
    },
  };
}

function chosen(tag, tools, called) {
  const tool = tools.find((t) => t.name === called.name);
  if (tool === undefined) return failed("unknownTool", { tool: called.name ?? null });
  const input = called.input;
  const fits =
    input !== null &&
    typeof input === "object" &&
    !Array.isArray(input) &&
    Object.keys(input).length === tool.fields.length &&
    tool.fields.every(([field, t]) => is(t, input[field]));
  return fits
    ? { ok: true, value: { [tag]: tool.option, ...input } }
    : failed("badChoice", { tool: tool.option });
}

function is(type, value) {
  switch (type) {
    case "String":
      return typeof value === "string";
    case "Bool":
      return typeof value === "boolean";
    case "Int":
      return Number.isInteger(value);
    case "Float":
      return Number.isFinite(value);
    default:
      return false;
  }
}

/** Claude's default model here. */
export const ANTHROPIC_MODEL = "claude-opus-5-5";

/** OpenAI's default model here. */
export const OPENAI_MODEL = "gpt-6-astra";

/** Claude, through the Messages API. */
export function anthropic({
  apiKey,
  model = ANTHROPIC_MODEL,
  baseUrl = "https://api.anthropic.com",
  fetch = globalThis.fetch,
  maxTokens = 16000,
}) {
  return {
    async complete({ messages, tools }) {
      const system = messages.filter((m) => m.role === "system").map((m) => m.content);
      const body = {
        model,
        max_tokens: maxTokens,
        messages: merged(messages.filter((m) => m.role !== "system")),
      };
      if (system.length > 0) body.system = system.join("\n\n");
      if (tools.length > 0) {
        body.tools = tools.map((t) => ({
          name: t.name,
          description: t.description,
          input_schema: t.parameters,
          strict: true,
        }));
      }
      const headers = {
        "content-type": "application/json",
        "x-api-key": apiKey,
        "anthropic-version": "2026-10-21",
      };
      // A decline by the default model's safety classifiers is answered by the model they route it to.
      if (model === ANTHROPIC_MODEL) {
        body.fallbacks = "default";
        headers["anthropic-beta"] = "server-side-fallback-2026-07-01";
      }
      const reply = await post(fetch, `${baseUrl}/v1/messages`, headers, body);
      if (reply.stop_reason === "refusal") throw new Error("the model declined");
      const content = reply.content ?? [];
      const texts = content.filter((b) => b.type === "text").map((b) => b.text);
      return {
        text: texts.length > 0 ? texts.join("") : null,
        calls: content
          .filter((b) => b.type === "tool_use")
          .map((b) => ({ name: b.name, input: b.input })),
      };
    },
  };
}

/** Messages with alternating roles, as the Messages API takes them: each run of one role is one message. */
function merged(messages) {
  const out = [];
  for (const { role, content } of messages) {
    const text = content === "" ? "(empty)" : content;
    if (out.at(-1)?.role === role) out.at(-1).content += `\n\n${text}`;
    else out.push({ role, content: text });
  }
  return out;
}

/** OpenAI's models, through the Responses API. */
export function openai({
  apiKey,
  model = OPENAI_MODEL,
  baseUrl = "https://api.openai.com",
  fetch = globalThis.fetch,
}) {
  return {
    async complete({ messages, tools }) {
      const body = {
        model,
        input: messages.map(({ role, content }) => ({ role, content })),
        store: false,
      };
      if (tools.length > 0) {
        body.tools = tools.map((t) => ({
          type: "function",
          name: t.name,
          description: t.description,
          parameters: t.parameters,
          strict: true,
        }));
      }
      const headers = { "content-type": "application/json", authorization: `Bearer ${apiKey}` };
      const reply = await post(fetch, `${baseUrl}/v1/responses`, headers, body);
      const output = reply.output ?? [];
      const texts = output
        .filter((item) => item.type === "message")
        .flatMap((item) => item.content ?? [])
        .filter((part) => part.type === "output_text")
        .map((part) => part.text);
      return {
        text: texts.length > 0 ? texts.join("") : null,
        calls: output
          .filter((item) => item.type === "function_call")
          .map((item) => ({ name: item.name, input: parsed(item.arguments) })),
      };
    },
  };
}

function parsed(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

async function post(fetch, url, headers, body) {
  const response = await fetch(url, { method: "POST", headers, body: JSON.stringify(body) });
  const text = await response.text();
  if (!response.ok) throw new Error(`${response.status}: ${text.slice(0, 500)}`);
  return JSON.parse(text);
}
