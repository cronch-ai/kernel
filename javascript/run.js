/**
 * A local host: runs a harness against capabilities implemented by JavaScript functions, with no network and no state
 * beyond the run. It keeps the host protocol (`docs/2026-11-06-host-protocol.md`); each duty is marked where it is
 * kept.
 */

import { readFile } from "node:fs/promises";
import { instantiate } from "./index.js";

export {
  ANTHROPIC_MODEL,
  anthropic,
  model,
  OPENAI_MODEL,
  openai,
  TOOL_RESULT,
  toolName,
} from "./model.js";

/** The `prev` of a log's first entry. */
export const GENESIS = `sha256:${"0".repeat(64)}`;

/** The error a call fails with when its implementation answered with a result the kernel withheld. */
export const NO_FIT = "no result that fits";

let shared;

/** The kernel this package ships. */
export function kernel() {
  shared ??= readFile(new URL("./kernel.wasm", import.meta.url)).then(instantiate);
  return shared;
}

/**
 * Runs `harness`, Pata source or an elaborated term, to the end.
 *
 * `capabilities` maps each capability's name, as `uses` names it, to its contract. `handlers` maps an operation, as
 * `prefix.op`, to an async function of its arguments and `{ call, signal, send }`: its result is the call's, and a
 * throw fails the call. `send(channel, value)` delivers a message to the run.
 *
 * Options: `input`, the start's value; `label`, the sources the starting principal is no more trusted than (none: a
 * trusted principal); `kernel`, an instantiated kernel; `interface`, for a term, in place of one installed from
 * `capabilities` with each under its own name.
 *
 * Resolves to the run's `outcome`, its final `system`, every event, and the `log`, hash-chained as `logEntry`s.
 */
export async function run(harness, capabilities, handlers, options = {}) {
  const k = options.kernel ?? (await kernel());
  const { term, iface } = load(k, harness, capabilities, options.interface);

  const ready = k.init({ harness: term, interface: iface });
  if (ready.kind !== "ready") throw new Error(`init: ${JSON.stringify(ready)}`);
  let system = ready.system;
  const events = [];
  const log = [];
  const queue = [];
  const inFlight = new Map();
  let wake = () => {};

  const enqueue = (delivery) => {
    queue.push(delivery);
    wake();
  };

  const perform = (event) => {
    const { action } = event;
    const id = event.call;
    const handler = handlers[action.op];
    const controller = new AbortController();
    const send = (channel, value) => enqueue({ input: { kind: "msg", channel, value } });
    const done = (async () => {
      if (handler === undefined) throw new Error(`no handler for ${action.op}`);
      return handler(action.args, { call: id, signal: controller.signal, send });
    })().then(
      (value) => ({ kind: "ret", call: id, value }),
      (error) => ({ kind: "failed", call: id, error: String(error?.message ?? error) }),
    );
    inFlight.set(key(event.origin, id), controller);
    // H8: every call ends, with its result or `failed`.
    done.then((input) => {
      inFlight.delete(key(event.origin, id));
      enqueue(event.origin.length === 0 ? { input } : { to: event.origin, input });
    });
  };

  const step = (delivery) => {
    // H1, H3: each delivery steps the configuration the last one returned, unchanged, one at a time.
    const out = k.deliver(system, delivery);
    system = out.system;
    events.push(...out.events);
    // H7: every delivery and its events, in order, each entry keyed with the last one's key as `prev`.
    const seq = log.length;
    const prev = seq === 0 ? GENESIS : log[seq - 1].key;
    const entry = { seq, prev, delivery, events: out.events };
    log.push({ ...entry, key: k.key("logEntry", entry) });
    for (const event of out.events) {
      if (event.kind === "out" && event.performed && event.action.kind === "call") {
        // H5: exactly the calls the interpreter performed, each once.
        perform(event);
      } else if (event.kind === "out" && event.performed && event.action.kind === "cancel") {
        // H6: cancellation goes to the implementation, which still answers.
        inFlight.get(key(event.origin, event.action.call))?.abort();
      } else if (event.kind === "withheld" && fits(event.reason) && delivery.input.kind === "ret") {
        // H8: a result that didn't fit leaves the call pending, so it fails instead.
        const input = { kind: "failed", call: delivery.input.call, error: NO_FIT };
        queue.unshift(delivery.to === undefined ? { input } : { to: delivery.to, input });
      }
    }
  };

  // H4: the start comes from the principal, with an honest label; results only from their implementations.
  const start = { kind: "start", value: options.input ?? null };
  if (options.label !== undefined) start.label = options.label;
  step({ input: start });
  for (;;) {
    while (queue.length > 0) step(queue.shift());
    if (inFlight.size === 0) break;
    await new Promise((resolve) => {
      wake = resolve;
    });
  }
  return { outcome: system.root.outcome, system, events, log };
}

/** Whether a withheld result left its call pending: it was the call's, but didn't fit. */
const fits = (reason) => reason.kind === "mismatch" || reason.kind === "postcondition";

const key = (origin, call) => `${origin.join("/")}:${call}`;

function load(k, harness, capabilities, given) {
  if (typeof harness !== "string") {
    return { term: harness, iface: given ?? install(k, Object.keys(capabilities), capabilities) };
  }
  const out = k.elaborate(harness, capabilities);
  if (out.kind !== "elaborated") throw new Error(`elaborate: ${JSON.stringify(out.problems)}`);
  const iface =
    given ??
    install(
      k,
      out.uses.map((u) => [u.name, u.capability]),
      capabilities,
    );
  return { term: out.harness, iface };
}

function install(k, uses, capabilities) {
  const installed = k.install(
    uses.map((u) => {
      const [prefix, name] = Array.isArray(u) ? u : [u, u];
      return { prefix, capability: capabilities[name] };
    }),
  );
  if (installed.kind !== "installed") throw new Error(`install: ${JSON.stringify(installed)}`);
  return installed.interface;
}
