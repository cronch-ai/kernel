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
