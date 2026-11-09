import type {
  Delivery,
  Event,
  Harness,
  Interface,
  Kernel,
  Label,
  Outcome,
  System,
  Value,
} from "./index.js";

/** The `prev` of a log's first entry. */
export const GENESIS: string;

/** The error a call fails with when its implementation answered with a result the kernel withheld. */
export const NO_FIT: string;

/** The kernel this package ships, instantiated once. Node only. */
export function kernel(): Promise<Kernel>;

export interface CallContext {
  /** The call's identifier, as the kernel numbered it. */
  readonly call: number;
  /** Aborted when the run cancels the call. The handler should still answer (H6). */
  readonly signal: AbortSignal;
  /** Deliver a message to the run on `channel`, as `prefix.channel`. */
  send(channel: string, value: Value): void;
}

/** An operation's implementation: its result is the call's, and a throw fails the call. */
export type Handler = (args: Value, context: CallContext) => Promise<Value>;

export interface RunOptions {
  /** The start's value. */
  readonly input?: Value;
  /** The sources the starting principal is no more trusted than. None: a trusted principal. */
  readonly label?: Label;
  readonly kernel?: Kernel;
  /** For a term, in place of the interface installed from `capabilities`, each under its own name. */
  readonly interface?: Interface;
}

export interface LogEntry {
  readonly seq: number;
  readonly prev: string;
  readonly delivery: Delivery;
  readonly events: readonly Event[];
  /** The kernel's `logEntry` key of the entry without it. */
  readonly key: string;
}

/**
 * Run `harness`, Pata source or an elaborated term, to the end, with each operation (`prefix.op`) answered by its
 * handler. A local host that keeps the host protocol: no network, no state beyond the run.
 */
export function run(
  harness: string | Harness,
  capabilities: Readonly<Record<string, Interface>>,
  handlers: Readonly<Record<string, Handler>>,
  options?: RunOptions,
): Promise<{
  readonly outcome: Outcome | undefined;
  readonly system: System;
  readonly events: readonly Event[];
  readonly log: readonly LogEntry[];
}>;

/** What performing a call gave: its result, or why it failed. Either way it returns to the run. */
export type Performed = { ok: true; value: Value } | { ok: false; error: Value };

/** Whom a call is performed for: the run's owner, the run, and the call's identifier in it. */
export interface Context {
  owner: string;
  run: string;
  origin: number[];
  call: number;
}

/**
 * A capability: its contract, and how its operations are performed. Its publisher vouches for its contract: the kernel
 * checks every result against `post` and withholds what breaks it.
 */
export interface Capability {
  /** The contract, before installation under a prefix. Null: there is no such capability. */
  contract: Interface | Promise<Interface | null>;
  /** Perform `op`, named as in the contract, on arguments the kernel has already checked. */
  perform(op: string, args: Value, context: Context): Performed | Promise<Performed>;
}

/** A message as a provider takes it. A harness's `tool` message reaches it as the user's, after `TOOL_RESULT`. */
export interface ModelMessage {
  readonly role: "system" | "user" | "assistant";
  readonly content: string;
}

/** A tool the model is offered: one case of a choosing operation's output, with its fields as `parameters`. */
export interface ModelTool {
  readonly name: string;
  readonly description: string;
  /** A JSON Schema object whose properties are all required. */
  readonly parameters: { readonly [key: string]: unknown };
}

/** What the model said: its text, if any, and the tools it called, first first. */
export interface ModelReply {
  readonly text: string | null;
  readonly calls: readonly { readonly name: string; readonly input: unknown }[];
}

/** The port to a language model. A throw fails the call. */
export interface ModelProvider {
  complete(request: {
    readonly messages: readonly ModelMessage[];
    readonly tools: readonly ModelTool[];
  }): Promise<ModelReply>;
}

/**
 * The model behind `provider`, as a capability under `contract`. An operation whose output is `String` answers with
 * text. One whose output is a variant offers its cases other than `answer` as tools, and answers with the case the
 * model called, its fields the tool's input, or with `answer {text}`.
 */
export function model(provider: ModelProvider, contract: Interface): Capability;

/** What begins a tool's result, sent as the user's. */
export const TOOL_RESULT: string;

/** A tool's name: providers allow only letters, digits, `_` and `-`, so `fs.read` is `fs_read`. */
export function toolName(option: string): string;

export interface ProviderOptions {
  readonly apiKey: string;
  /** The model's identifier: `ANTHROPIC_MODEL` or `OPENAI_MODEL` if not given, or OpenRouter's account default. */
  readonly model?: string;
  /** In place of the provider's own API. */
  readonly baseUrl?: string;
  readonly fetch?: typeof fetch;
}

export const ANTHROPIC_MODEL: string;
export const OPENAI_MODEL: string;

/** Claude, through Anthropic's Messages API, with plain `fetch`. */
export function anthropic(
  options: ProviderOptions & { readonly maxTokens?: number },
): ModelProvider;

/** OpenAI's models, through its Responses API, with plain `fetch`. */
export function openai(options: ProviderOptions): ModelProvider;

/** Any model OpenRouter serves, through its Chat Completions API, with plain `fetch`. */
export function openrouter(options: ProviderOptions): ModelProvider;
