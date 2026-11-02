// Types and expressions (`crates/kernel/src/ir.rs`). Values are plain JSON.

export type Value = null | boolean | number | string | Value[] | { [field: string]: Value };

/** A structural type. `Int` conforms to `Float`, wider records to narrower ones, and `Any` both ways. */
export type Type =
  | "Int"
  | "Float"
  | "String"
  | "Bool"
  | "Any"
  | { readonly List: Type }
  | { readonly Record: Readonly<Record<string, Type>> }
  /** A record whose `tag` field names its case, and which has that case's type. */
  | { readonly Variant: { readonly tag: string; readonly cases: Readonly<Record<string, Type>> } };

export interface Param {
  readonly name: string;
  readonly type: Type;
}

/** A pure expression. `fold` is the only iteration, so evaluation always ends. */
export type Expr =
  | { readonly e: "lit"; readonly v: Value }
  | { readonly e: "var"; readonly name: string }
  | { readonly e: "op"; readonly op: string; readonly args: readonly Expr[] }
  | { readonly e: "let"; readonly name: string; readonly be: Expr; readonly in: Expr }
  | { readonly e: "if"; readonly cond: Expr; readonly then: Expr; readonly else: Expr }
  | { readonly e: "record"; readonly fields: Readonly<Record<string, Expr>> }
  | { readonly e: "list"; readonly items: readonly Expr[] }
  | { readonly e: "get"; readonly of: Expr; readonly field: string }
  /** The untrusted sources a value may depend on, as a sorted list. Empty: trusted. */
  | { readonly e: "label"; readonly of: Expr }
  /** The case of a variant, read off its `on` field (`kind` by default), with `bind` holding it at that case's type. */
  | {
      readonly e: "match";
      readonly of: Expr;
      readonly on?: string;
      readonly cases: Readonly<Record<string, { readonly bind: string; readonly body: Expr }>>;
      readonly otherwise?: Expr;
    }
  | {
      readonly e: "fold";
      readonly over: Expr;
      readonly init: Expr;
      readonly acc: string;
      readonly item: string;
      readonly body: Expr;
    };

/** A `/`-separated path into an expression; `""` is the root. */
export type Path = string;

export type Arity =
  | { readonly arity: "exactly"; readonly n: number }
  | { readonly arity: "between"; readonly min: number; readonly max: number }
  | { readonly arity: "atLeast"; readonly n: number };

export type OpTypeError =
  | { readonly kind: "arity"; readonly expected: Arity; readonly got: number }
  | {
      readonly kind: "operand";
      readonly index: number;
      readonly expected: Type;
      readonly got: Type;
    };

export type OpEvalError =
  | { readonly kind: "divisionByZero" }
  | { readonly kind: "overflow" }
  | { readonly kind: "notFinite" }
  | {
      readonly kind: "operand";
      readonly index: number;
      readonly expected: Type;
      readonly got: Value;
    };

/** What makes an expression ill-typed (`crates/kernel/src/check.rs`). */
export type Problem =
  | { readonly kind: "unknownVariable"; readonly at: Path; readonly name: string }
  | { readonly kind: "unknownOperator"; readonly at: Path; readonly op: string }
  | {
      readonly kind: "operator";
      readonly at: Path;
      readonly op: string;
      readonly error: OpTypeError;
    }
  | { readonly kind: "mismatch"; readonly at: Path; readonly expected: Type; readonly got: Type }
  | { readonly kind: "notARecord"; readonly at: Path; readonly got: Type }
  | { readonly kind: "noSuchField"; readonly at: Path; readonly field: string; readonly of: Type }
  | { readonly kind: "notAList"; readonly at: Path; readonly got: Type }
  | { readonly kind: "notAVariant"; readonly at: Path; readonly on: string; readonly got: Type }
  | { readonly kind: "noSuchCase"; readonly at: Path; readonly case: string }
  | { readonly kind: "notExhaustive"; readonly at: Path; readonly missing: readonly string[] };

/** Why an expression failed to evaluate (`crates/kernel/src/labeled.rs`). */
export type EvalError =
  | {
      readonly kind: "operator";
      readonly at: Path;
      readonly op: string;
      readonly error: OpEvalError;
    }
  | { readonly kind: "unknownVariable"; readonly at: Path; readonly name: string }
  | { readonly kind: "unknownOperator"; readonly at: Path; readonly op: string }
  | { readonly kind: "notABool"; readonly at: Path; readonly got: Value }
  | { readonly kind: "noSuchField"; readonly at: Path; readonly field: string; readonly of: Value }
  | { readonly kind: "notAList"; readonly at: Path; readonly got: Value }
  | { readonly kind: "noCase"; readonly at: Path; readonly got: Value };

/** Untrusted sources. Empty: trusted. */
export type Label = readonly string[];

/** A value with a label on every node. */
export type LValue =
  | { readonly scalar: { readonly label?: Label; readonly value: Value } }
  | { readonly list: { readonly label?: Label; readonly items: readonly LValue[] } }
  | {
      readonly record: {
        readonly label?: Label;
        readonly fields: Readonly<Record<string, LValue>>;
      };
    };

// Harnesses and interfaces (`crates/kernel/src/harness.rs`).

/** `⟨A, S, s₀, δ⟩`. `step` is over `state` and `input`, and gives `{ state, actions }`. */
export interface Harness {
  readonly arg: Type;
  readonly state: Type;
  readonly init: Value;
  readonly step: Expr;
}

/** An operation and its contract. Every expression is over `args`, and as named: `grant`, `result`. */
export interface Operation {
  readonly input: Type;
  readonly output: Type;
  readonly mode?: "single" | "stream";
  /** Over `args` and `grant`: the call is performed only if it holds. */
  readonly pre?: Expr;
  /** Over `args` and `result`: results that break it are withheld. */
  readonly post?: Expr;
  /** Over `args`: the sources of what the call delivers. Absent: the arguments' labels. */
  readonly ret?: Expr;
  readonly raise?: Label;
  readonly effect?: boolean;
  /** Over `args`: what the call may touch, recorded on the trace. */
  readonly footprint?: Expr;
  /** Over `args` and `grant`: the next grant state, when the call is performed. */
  readonly onCall?: Expr;
  /** Over `args`, `result` and `grant`: the next grant state, when a trusted result arrives. */
  readonly onResult?: Expr;
}

export interface Channel {
  readonly type: Type;
  readonly label?: Label;
  /** Over `value`: messages that break it are withheld. */
  readonly inv?: Expr;
  /** Over `value` and `grant`: the next grant state, on a trusted channel. */
  readonly onMsg?: Expr;
}

export interface Interface {
  readonly operations?: Readonly<Record<string, Operation>>;
  readonly channels?: Readonly<Record<string, Channel>>;
}

export interface Installation {
  readonly prefix?: string;
  readonly capability: Interface;
}

/** Narrowing only ever removes, strengthens or distrusts: there is no way to widen. */
export type Narrowing =
  | { readonly kind: "drop"; readonly op: string }
  | { readonly kind: "only"; readonly ops: readonly string[] }
  | { readonly kind: "dropChannel"; readonly channel: string }
  | { readonly kind: "restrict"; readonly op: string; readonly pre: Expr }
  | { readonly kind: "raise"; readonly op: string; readonly label: Label };

export type HarnessPlace =
  | { readonly in: "step" }
  | { readonly in: "operation"; readonly name: string; readonly part: string }
  | { readonly in: "channel"; readonly name: string; readonly part: string };

export type HarnessProblem =
  | { readonly kind: "expr"; readonly place: HarnessPlace; readonly problem: Problem }
  | { readonly kind: "init"; readonly expected: Type; readonly got: Value }
  | { readonly kind: "step"; readonly expected: Type; readonly got: Type }
  | {
      readonly kind: "part";
      readonly place: HarnessPlace;
      readonly expected: Type;
      readonly got: Type;
    };

// Elaboration (`crates/surface`).

/** A place in a source: line and column, both from 1, the column in characters. */
export interface Pos {
  readonly line: number;
  readonly column: number;
}

/** The source from `start` up to, not including, `end`. */
export interface Span {
  readonly start: Pos;
  readonly end: Pos;
}

/** A capability a harness uses, and the prefix it is installed under for every run of the harness. */
export interface Binding {
  readonly name: string;
  readonly capability: string;
}

/** An invariant of an elaborated harness's state, over the variable `state`. */
export interface Invariant {
  readonly name: string;
  readonly expr: Expr;
}

/** Where each named thing of a source is. */
export interface SourceMap {
  readonly nodes: Readonly<Record<string, Span>>;
  readonly properties: Readonly<Record<string, Span>>;
  readonly invariants: Readonly<Record<string, Span>>;
}

/** A registered harness a harness spawns, by the name it uses for it. */
export interface HarnessBinding {
  readonly name: string;
  readonly harness: string;
}

export type SourceProblem = { readonly span: Span } & (
  | { readonly kind: "syntax"; readonly expected: string; readonly found: string }
  | { readonly kind: "duplicate"; readonly name: string }
  | { readonly kind: "unknownName"; readonly name: string }
  | { readonly kind: "unknownType"; readonly name: string }
  | { readonly kind: "recursiveType"; readonly name: string }
  | { readonly kind: "noReturn" }
  | { readonly kind: "notAValue"; readonly name: string }
  | { readonly kind: "unknownFunction"; readonly name: string }
  | { readonly kind: "unknownCapability"; readonly name: string; readonly capability: string }
  | { readonly kind: "unknownOperation"; readonly op: string }
  | { readonly kind: "unknownChannel"; readonly channel: string }
  | { readonly kind: "unknownHarness"; readonly name: string; readonly harness: string }
  | { readonly kind: "notAHarness"; readonly name: string }
  | { readonly kind: "child"; readonly name: string; readonly problems: readonly HarnessProblem[] }
  | { readonly kind: "until"; readonly name: string }
  | { readonly kind: "filterReadsNode"; readonly node: string }
  | { readonly kind: "nestedCall"; readonly op: string }
  | { readonly kind: "argument"; readonly expected: string }
  | { readonly kind: "guardOnValue"; readonly node: string }
  | { readonly kind: "cycle"; readonly nodes: readonly string[] }
  | { readonly kind: "mayBeSkipped"; readonly node: string; readonly via: string }
  | { readonly kind: "uncovered"; readonly node: string; readonly outcome: string }
  | { readonly kind: "noSuchOutcome"; readonly outcome: string }
  | { readonly kind: "mixedPatterns" }
  | { readonly kind: "noDefault" }
  | { readonly kind: "propertyOnly"; readonly construct: string }
  | { readonly kind: "notASlot"; readonly node: string }
  | { readonly kind: "helper"; readonly name: string; readonly expected: string }
  | { readonly kind: "nestedAlways" }
  | { readonly kind: "otherwiseReadsNode"; readonly node: string }
  | { readonly kind: "type"; readonly problem: Problem }
  | { readonly kind: "mismatch"; readonly expected: Type; readonly got: Type }
  /** The kernel rejected the elaborated term: a bug in the elaborator, not the source. */
  | { readonly kind: "internal"; readonly problems: readonly HarnessProblem[] }
);

// Runs (`crates/kernel/src/run.rs`).

export type WorldInput =
  | { readonly kind: "start"; readonly value: Value; readonly label?: Label }
  | { readonly kind: "ret"; readonly call: number; readonly value: Value }
  | { readonly kind: "end"; readonly call: number }
  | { readonly kind: "failed"; readonly call: number; readonly error: Value }
  | { readonly kind: "msg"; readonly channel: string; readonly value: Value };

/** An input from the world, to the run at `to` (a path of spawn identifiers; `[]` is the root). */
export interface Delivery {
  readonly to?: readonly number[];
  readonly input: WorldInput;
}

export type Withheld =
  | { readonly kind: "noSuchRun" }
  | { readonly kind: "ended" }
  | { readonly kind: "notStarted" }
  | { readonly kind: "alreadyStarted" }
  | { readonly kind: "notPending"; readonly call: number }
  | { readonly kind: "notAStream"; readonly call: number }
  | { readonly kind: "noSuchChannel"; readonly channel: string }
  | { readonly kind: "mismatch"; readonly expected: Type; readonly got: Value }
  | { readonly kind: "postcondition"; readonly call: number }
  | { readonly kind: "invariant"; readonly channel: string }
  | { readonly kind: "returnLabel"; readonly call: number };

export type StepError =
  | { readonly kind: "eval"; readonly error: EvalError }
  | { readonly kind: "notAStep"; readonly got: Value }
  | { readonly kind: "badAction"; readonly action: Value }
  | { readonly kind: "runaway" };

export type Outcome =
  | { readonly kind: "halted"; readonly value: Value; readonly label?: Label }
  | { readonly kind: "failed"; readonly error: StepError };

export interface Pending {
  readonly call: number;
  readonly op: string;
  readonly args: LValue;
}

export interface Run {
  readonly harness: Harness;
  readonly interface: Interface;
  readonly state: LValue;
  readonly pending: readonly Pending[];
  readonly next: number;
  readonly queue: readonly { readonly input: Value; readonly label?: Label }[];
  readonly started: boolean;
  readonly outcome?: Outcome;
  /** What this run's preconditions read: the root principal's grant at the root, a copy of its parent's in a child. */
  readonly grant: Value;
  readonly seen?: Label;
  readonly children?: readonly { readonly call: number; readonly run: Run }[];
}

/** A configuration. Hosts keep it between deliveries and hand it back unchanged. */
export interface System {
  readonly root: Run;
}

export type Event =
  | {
      readonly kind: "in";
      readonly origin: readonly number[];
      readonly input: Value;
      readonly label?: Label;
      readonly state: Value;
    }
  | {
      readonly kind: "out";
      readonly origin: readonly number[];
      readonly call?: number;
      readonly action: Value;
      readonly performed: boolean;
      readonly effect?: boolean;
      readonly footprint?: Value;
      /** The action's labels, shaped like the action. */
      readonly labels: Value;
      readonly state: Value;
    }
  | {
      readonly kind: "withheld";
      readonly origin: readonly number[];
      readonly input: WorldInput;
      readonly reason: Withheld;
    }
  | {
      readonly kind: "failed";
      readonly origin: readonly number[];
      readonly input: Value;
      readonly error: StepError;
    };

// Properties (`crates/kernel/src/property.rs`).

export type Formula =
  /** An expression over `event` and bound variables. One that fails to evaluate is false. */
  | { readonly f: "atom"; readonly holds: Expr }
  | { readonly f: "not"; readonly of: Formula }
  | { readonly f: "and"; readonly of: readonly Formula[] }
  | { readonly f: "or"; readonly of: readonly Formula[] }
  | { readonly f: "implies"; readonly if: Formula; readonly then: Formula }
  /** `var` ranges over the values `of` gave at the events so far. */
  | { readonly f: "exists"; readonly var: string; readonly of: Expr; readonly body: Formula }
  | { readonly f: "forall"; readonly var: string; readonly of: Expr; readonly body: Formula }
  | { readonly f: "prev"; readonly of: Formula }
  | { readonly f: "since"; readonly left: Formula; readonly right: Formula }
  | { readonly f: "once"; readonly of: Formula }
  | { readonly f: "historically"; readonly of: Formula };

/** `□always`: at every event of every run. */
export interface Property {
  readonly name: string;
  readonly always: Formula;
}

export interface PropertyProblem {
  readonly at: string;
  readonly problem: Problem;
}

export type Monitored =
  | { readonly verdict: "holds" }
  | { readonly verdict: "violated"; readonly at: number };

/** The incremental monitor's state: each subformula's value at the last event. Hosts hand it back unchanged. */
export interface MonitorState {
  readonly last?: readonly boolean[];
}

export type MonitorError =
  /** Quantifiers need the values seen so far: such properties are checked on whole traces. */
  | { readonly kind: "quantified" }
  /** The state isn't one this property's monitor could be in. */
  | { readonly kind: "state"; readonly expected: number; readonly got: number };

// Evidence (`crates/kernel/src/evidence.rs`).

export interface Subject {
  readonly harness: Harness;
  readonly interface?: Interface;
  readonly grant?: Value;
  readonly property: Property;
}

export type Template =
  | { readonly kind: "start"; readonly value: Value; readonly label?: Label }
  | { readonly kind: "ret"; readonly value: Value; readonly op?: string }
  | { readonly kind: "end"; readonly op?: string }
  | { readonly kind: "failed"; readonly error: Value; readonly op?: string }
  | { readonly kind: "msg"; readonly channel: string; readonly value: Value };

export interface Choice {
  readonly to?: readonly number[];
  readonly input: Template;
}

/** A configuration as a certificate lists it. */
export interface Configuration {
  readonly system: System;
  readonly monitor?: { readonly last?: readonly boolean[] };
}

export type Method =
  | { readonly kind: "counterexample"; readonly deliveries: readonly Delivery[] }
  | { readonly kind: "bounded"; readonly depth: number; readonly alphabet: readonly Choice[] }
  | { readonly kind: "exhaustive"; readonly alphabet: readonly Choice[]; readonly limit?: number }
  /** An inductive invariant over `config`, with the configurations it holds of. */
  | {
      readonly kind: "invariant";
      readonly invariant: Expr;
      readonly configurations: readonly Configuration[];
      readonly alphabet: readonly Choice[];
    };

export type Verdict =
  | {
      readonly verdict: "refuted";
      readonly counterexample: readonly Delivery[];
      readonly at: number;
    }
  | { readonly verdict: "holdsUpTo"; readonly depth: number }
  | { readonly verdict: "holds" };

export type Assumption =
  | { readonly kind: "contracts"; readonly interface: string }
  | { readonly kind: "checker"; readonly kernel: string }
  | { readonly kind: "environment"; readonly alphabet: string }
  | { readonly kind: "runtime" };

export type Evidence = {
  readonly harness: string;
  readonly interface: string;
  readonly grant: string;
  readonly property: string;
  readonly method: Method;
  readonly assumptions: readonly Assumption[];
} & Verdict;

export type ProveError =
  | { readonly kind: "init"; readonly error: InitError }
  | { readonly kind: "property"; readonly problems: readonly PropertyProblem[] }
  | { readonly kind: "inadmissible"; readonly index: number; readonly reason: Withheld }
  | { readonly kind: "notViolated" }
  | { readonly kind: "quantified" }
  | { readonly kind: "inconclusive"; readonly explored: number }
  | { readonly kind: "refuted"; readonly counterexample: readonly Delivery[] }
  | { readonly kind: "initiation" }
  | { readonly kind: "notInvariant"; readonly index: number }
  | { readonly kind: "notClosed"; readonly index: number; readonly delivery: Delivery }
  | { readonly kind: "unsafe"; readonly index: number; readonly delivery: Delivery };

export type InitError =
  | { readonly kind: "harness"; readonly problems: readonly HarnessProblem[] }
  | { readonly kind: "interface"; readonly problems: readonly HarnessProblem[] };

/**
 * `logEntry`: an entry of a run's log, `{ seq, prev, delivery, events }`.
 * `module`: a capability's code, `{ main, modules: { [name]: source } }`, which the kernel never reads.
 */
export type Kind =
  | "harness"
  | "interface"
  | "property"
  | "grant"
  | "alphabet"
  | "evidence"
  | "logEntry"
  | "module";

// SMT-LIB (`crates/kernel/src/smtlib`).

export interface Definition {
  readonly name: string;
  readonly params?: readonly Param[];
  readonly body: Expr;
  readonly description?: string;
}

export type Floats = "exact" | "real";

export type NoReading =
  | { readonly kind: "any" }
  | { readonly kind: "operator"; readonly op: string }
  | { readonly kind: "listConversion"; readonly from: Type; readonly to: Type }
  | { readonly kind: "equality"; readonly left: Type; readonly right: Type }
  | { readonly kind: "text"; readonly of: Type }
  | { readonly kind: "fold" }
  | { readonly kind: "label" }
  | { readonly kind: "emptyVariant" }
  | { readonly kind: "roundingDigits" }
  | { readonly kind: "recordSize" }
  | { readonly kind: "undeclaredField"; readonly field: string }
  | { readonly kind: "name"; readonly name: string }
  | { readonly kind: "character"; readonly code: number }
  | { readonly kind: "value"; readonly expected: Type; readonly got: Value };

export type SmtlibRefusal = { readonly definition: string } & (
  | { readonly kind: "illFormed"; readonly problems: readonly Problem[] }
  | {
      readonly kind: "noReading";
      readonly place:
        | { readonly in: "param"; readonly name: string }
        | { readonly in: "body"; readonly at: Path };
      readonly reason: NoReading;
    }
);

export type Unreadable =
  | { readonly kind: "syntax"; readonly detail: string }
  | { readonly kind: "count"; readonly expected: number; readonly got: number }
  | { readonly kind: "value"; readonly type: Type; readonly got: string }
  | { readonly kind: "notFinite" }
  | { readonly kind: "tooLarge" };

type Refusable<K extends string, T> =
  | ({ readonly kind: K } & T)
  | { readonly kind: "refused"; readonly error: unknown };

export interface Kernel {
  infer(
    expr: Expr,
    vars?: Readonly<Record<string, Type>>,
  ): { readonly type: Type | null; readonly problems: readonly Problem[] };
  evaluate(
    expr: Expr,
    vars?: Readonly<Record<string, Value>>,
  ):
    | { readonly kind: "value"; readonly value: Value }
    | { readonly kind: "failed"; readonly error: EvalError };
  /** Against an interface, the harness's input is typed by `inputType`; without one, it is `Any`. */
  checkHarness(harness: Harness, iface?: Interface): readonly HarnessProblem[];
  inputType(harness: Harness, iface: Interface): Type;
  checkInterface(iface: Interface): readonly HarnessProblem[];
  install(
    capabilities: readonly Installation[],
  ): Refusable<"installed", { readonly interface: Interface }>;
  narrow(
    iface: Interface,
    narrowings: readonly Narrowing[],
  ): Refusable<"narrowed", { readonly interface: Interface }>;
  init(input: {
    readonly harness: Harness;
    readonly interface?: Interface;
    readonly grant?: Value;
  }): Refusable<"ready", { readonly system: System }>;
  /** Deliver one input: the next configuration, and what happened. */
  deliver(
    system: System,
    delivery: Delivery,
  ): { readonly system: System; readonly events: readonly Event[] };
  checkProperty(property: Property): readonly PropertyProblem[];
  monitor(property: Property, trace: readonly Event[]): Monitored;
  /** The incremental monitor before any event, for a quantifier-free property. */
  monitorStart(
    property: Property,
  ):
    | { readonly kind: "started"; readonly state: MonitorState }
    | { readonly kind: "refused"; readonly error: MonitorError };
  /** The monitor over more events. On a violation it stops there, with `at` counted from the first of `events`. */
  monitorStep(
    property: Property,
    state: MonitorState,
    events: readonly Event[],
  ):
    | ({ readonly kind: "stepped"; readonly state: MonitorState } & Monitored)
    | { readonly kind: "refused"; readonly error: MonitorError };
  prove(
    subject: Subject,
    method: Method,
  ):
    | { readonly kind: "evidence"; readonly key: string; readonly evidence: Evidence }
    | { readonly kind: "refused"; readonly error: ProveError };
  verify(
    subject: Subject,
    evidence: Evidence,
  ): { readonly valid: true } | { readonly valid: false; readonly error: unknown };
  /** The configurations an `invariant` certificate lists, found by search. */
  certificate(
    subject: Subject,
    alphabet: readonly Choice[],
    limit?: number,
  ):
    | { readonly kind: "certificate"; readonly configurations: readonly Configuration[] }
    | { readonly kind: "refused"; readonly error: ProveError };
  /** Initiation, decided, and consecution as an SMT-LIB query whose models are counterexamples to induction. */
  obligation(input: {
    readonly harness: Harness;
    readonly interface?: Interface;
    readonly invariant: Expr;
    readonly floats?: Floats;
  }):
    | {
        readonly kind: "obligation";
        readonly initiation: boolean;
        readonly text: string;
        readonly excluded: readonly string[];
        /** The types to read a model's state and input at. */
        readonly reads: readonly [Type, Type];
      }
    | { readonly kind: "refused"; readonly error: unknown };
  /** The kernel's check of a solver's counterexample to induction. */
  checkInduction(input: {
    readonly harness: Harness;
    readonly interface?: Interface;
    readonly invariant: Expr;
    readonly state: Value;
    readonly input: Value;
  }):
    | { readonly kind: "preserved"; readonly next: Value }
    | { readonly kind: "notInductive"; readonly next: Value }
    | {
        readonly kind: "refused";
        readonly error: { readonly kind: "invariant" | "state" | "input" | "notHeld" | "step" };
      };
  /**
   * A harness from source, against the contracts of the capabilities its `uses` header names, and the terms of the
   * harnesses it spawns, each by identifier.
   */
  elaborate(
    source: string,
    capabilities: Readonly<Record<string, Interface>>,
    harnesses?: Readonly<Record<string, Harness>>,
  ):
    | {
        readonly kind: "elaborated";
        readonly harness: Harness;
        readonly uses: readonly Binding[];
        readonly harnesses: readonly HarnessBinding[];
        readonly properties: readonly Property[];
        readonly invariants: readonly Invariant[];
        readonly sourceMap: SourceMap;
      }
    | { readonly kind: "problems"; readonly problems: readonly SourceProblem[] };
  key(kind: Kind, object: unknown): string;
  smtlib(
    definitions: readonly Definition[],
    floats?: Floats,
  ):
    | { readonly kind: "exported"; readonly text: string }
    | { readonly kind: "refused"; readonly error: SmtlibRefusal };
  readValues(
    reply: string,
    types: readonly Type[],
  ):
    | { readonly kind: "values"; readonly values: readonly Value[] }
    | { readonly kind: "unreadable"; readonly error: Unreadable };
}

/** Instantiate the kernel from its bytes, or from a compiled module where bytes can't be compiled (a Worker). */
export function instantiate(source: BufferSource | WebAssembly.Module): Promise<Kernel>;
