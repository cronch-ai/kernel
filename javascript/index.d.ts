// Protocol objects (`crates/kernel/src/ir.rs`). Values are plain JSON.

export type Value = null | boolean | number | string | Value[] | { [field: string]: Value };

/** A structural type. `Int` conforms to `Float`, wider records to narrower ones, and `Any` both ways. */
export type Type =
  | "Int"
  | "Float"
  | "String"
  | "Bool"
  | "Any"
  | { readonly List: Type }
  | { readonly Record: Readonly<Record<string, Type>> };

export interface Param {
  readonly name: string;
  readonly type: Type;
}

/** A check on an Answer's value. Kernel validators are replayable; `checker` is delegated; `prose` is never checked. */
export type Validator =
  | { readonly kind: "min"; readonly value: number; readonly at?: readonly string[] }
  | { readonly kind: "max"; readonly value: number; readonly at?: readonly string[] }
  | { readonly kind: "nonEmpty"; readonly at?: readonly string[] }
  | { readonly kind: "oneOf"; readonly values: readonly Value[]; readonly at?: readonly string[] }
  | { readonly kind: "minLength"; readonly value: number; readonly at?: readonly string[] }
  | { readonly kind: "maxLength"; readonly value: number; readonly at?: readonly string[] }
  | { readonly kind: "checker"; readonly actor: string; readonly note?: string }
  | { readonly kind: "prose"; readonly text: string };

/** An Ask's signature and contract: a typed hole. */
export interface AskSpec {
  readonly name: string;
  readonly params?: readonly Param[];
  readonly output: Type;
  readonly description?: string;
  readonly validators?: readonly Validator[];
  /** `derivation`: a raw value or an assumption can't answer it. */
  readonly form?: "any" | "derivation";
  /** An effect Ask changes the world: it needs authority, and is never answered from the store. */
  readonly effect?: boolean;
}

/** A closed Ask: a spec with a value for each parameter. */
export interface Ask {
  readonly spec: AskSpec;
  readonly args: Readonly<Record<string, Value>>;
}

/** An input the requester declares, with where it came from. */
export interface Fact {
  readonly value: Value;
  readonly type?: Type;
  readonly source?: string;
}

/** The context an Ask is answered in: information (part of the content key) and capabilities (not). */
export interface World {
  readonly facts?: Readonly<Record<string, Fact>>;
  /** The Actors a responder's holes may be routed to. Empty: any. */
  readonly actors?: readonly string[];
}

/** A Procedure's body. `ask` is a computational hole; `assume`, `opaque` and `fact` are epistemic ones. */
export type Expr =
  | { readonly e: "lit"; readonly v: Value }
  | { readonly e: "var"; readonly name: string }
  | { readonly e: "fact"; readonly name: string }
  | { readonly e: "op"; readonly op: string; readonly args: readonly Expr[] }
  | { readonly e: "ask"; readonly spec: AskSpec; readonly args: Readonly<Record<string, Expr>> }
  | {
      readonly e: "assume";
      readonly v: Value;
      readonly type: Type;
      readonly note?: string;
      readonly source?: string;
    }
  | { readonly e: "opaque"; readonly type: Type; readonly note?: string }
  | { readonly e: "let"; readonly name: string; readonly be: Expr; readonly in: Expr }
  | { readonly e: "if"; readonly cond: Expr; readonly then: Expr; readonly else: Expr }
  | { readonly e: "record"; readonly fields: Readonly<Record<string, Expr>> }
  | { readonly e: "list"; readonly items: readonly Expr[] }
  | { readonly e: "get"; readonly of: Expr; readonly field: string }
  | {
      readonly e: "fold";
      readonly over: Expr;
      readonly init: Expr;
      readonly acc: string;
      readonly item: string;
      readonly body: Expr;
    };

/** What an Actor returns: a raw value, or a Procedure over the Ask's parameters. */
export type Answer = { readonly value: Value } | { readonly procedure: Expr };

// What the kernel says about an Answer (`crates/kernel/src/assess.rs`).

/** A `/`-separated path into a Procedure's body; `""` is the root. */
export type Path = string;

export type State = "illFormed" | "constructed" | "evaluable" | "satisfied" | "failed";

export type OpTypeError =
  | { readonly kind: "arity"; readonly expected: unknown; readonly got: number }
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

export type Problem =
  | { readonly kind: "missingArgument"; readonly ask: string; readonly param: string }
  | {
      readonly kind: "unexpectedArgument";
      readonly at: Path;
      readonly ask: string;
      readonly arg: string;
    }
  | {
      readonly kind: "argumentMismatch";
      readonly ask: string;
      readonly param: string;
      readonly expected: Type;
      readonly got: Value;
    }
  | {
      readonly kind: "duplicateParameter";
      readonly at: Path;
      readonly ask: string;
      readonly param: string;
    }
  | {
      readonly kind: "missingHoleArgument";
      readonly at: Path;
      readonly ask: string;
      readonly param: string;
    }
  | { readonly kind: "unknownVariable"; readonly at: Path; readonly name: string }
  | { readonly kind: "unknownFact"; readonly at: Path; readonly name: string }
  | { readonly kind: "unknownOperator"; readonly at: Path; readonly op: string }
  | {
      readonly kind: "operator";
      readonly at: Path;
      readonly op: string;
      readonly error: OpTypeError;
    }
  | { readonly kind: "mismatch"; readonly at: Path; readonly expected: Type; readonly got: Type }
  | { readonly kind: "notARecord"; readonly at: Path; readonly got: Type }
  | { readonly kind: "notAList"; readonly at: Path; readonly got: Type }
  | { readonly kind: "noSuchField"; readonly at: Path; readonly field: string; readonly of: Type }
  | {
      readonly kind: "assumptionMismatch";
      readonly at: Path;
      readonly expected: Type;
      readonly got: Value;
    }
  | {
      readonly kind: "notDerived";
      readonly at: Path;
      readonly reason: "rawValue" | "assumption" | "opaque";
    };

/** A computational hole: an Ask inside a Procedure. */
export interface Hole {
  readonly at: Path;
  readonly spec: AskSpec;
}

/** An epistemic hole: something the result rests on that Cronch has not checked. */
export type Assumption =
  | { readonly kind: "rawValue"; readonly value: Value }
  | {
      readonly kind: "assumed";
      readonly at: Path;
      readonly type: Type;
      readonly value: Value;
      readonly note?: string;
      readonly source?: string;
    }
  | { readonly kind: "opaque"; readonly at: Path; readonly type: Type; readonly note?: string }
  | {
      readonly kind: "fact";
      readonly name: string;
      readonly value: Value;
      readonly source?: string;
    };

/** A hole evaluation is blocked on. */
export type Open =
  | { readonly kind: "ask"; readonly at: Path; readonly key: string; readonly ask: Ask }
  | { readonly kind: "opaque"; readonly at: Path; readonly type: Type };

export type EvalError =
  | {
      readonly kind: "operator";
      readonly at: Path;
      readonly op: string;
      readonly error: OpEvalError;
    }
  | {
      readonly kind: "resolution";
      readonly at: Path;
      readonly key: string;
      readonly expected: Type;
      readonly got: Value;
    }
  | { readonly kind: "unknownVariable"; readonly at: Path; readonly name: string }
  | { readonly kind: "unknownFact"; readonly at: Path; readonly name: string }
  | { readonly kind: "unknownOperator"; readonly at: Path; readonly op: string }
  | { readonly kind: "notABool"; readonly at: Path; readonly got: Value }
  | { readonly kind: "noSuchField"; readonly at: Path; readonly field: string; readonly of: Value }
  | { readonly kind: "notAList"; readonly at: Path; readonly got: Value };

/** A Checker Actor's verdict on one validator. It counts only if `checker` is the one the validator names. */
export interface Verdict {
  readonly validator: number;
  readonly checker: string;
  readonly verdict: boolean;
}

export type Subject =
  | { readonly on: "type"; readonly expected: Type }
  | { readonly on: "validator"; readonly index: number; readonly validator: Validator };

export type Failure =
  | { readonly kind: "notInhabited" }
  | { readonly kind: "noSuchField"; readonly at: readonly string[] }
  | { readonly kind: "notANumber"; readonly got: Value }
  | { readonly kind: "notSized"; readonly got: Value }
  | { readonly kind: "belowMin"; readonly min: number; readonly got: number }
  | { readonly kind: "aboveMax"; readonly max: number; readonly got: number }
  | { readonly kind: "empty" }
  | { readonly kind: "notOneOf"; readonly got: Value }
  | { readonly kind: "tooShort"; readonly min: number; readonly got: number }
  | { readonly kind: "tooLong"; readonly max: number; readonly got: number }
  | { readonly kind: "rejected"; readonly checker: string };

export type Outcome =
  | { readonly result: "pass" }
  | { readonly result: "attested"; readonly checker: string }
  | { readonly result: "fail"; readonly failure: Failure }
  | { readonly result: "delegated"; readonly checker: string }
  | { readonly result: "unchecked" };

export interface Check {
  readonly subject: Subject;
  readonly outcome: Outcome;
}

export interface Report {
  readonly key: string;
  readonly state: State;
  readonly problems: readonly Problem[];
  readonly holes: readonly Hole[];
  readonly assumptions: readonly Assumption[];
  readonly open: readonly Open[];
  readonly value?: Value;
  readonly checks: readonly Check[];
  readonly error?: EvalError;
}

// The SMT-LIB 2 export of an Ask and its Answer (`crates/kernel/src/smtlib`).

export interface SmtlibInput {
  readonly ask: Ask;
  readonly world?: World;
  readonly answer: Answer;
}

/** Where a part with no SMT-LIB reading is. */
export type SmtlibPlace =
  | { readonly in: "spec" }
  | { readonly in: "argument"; readonly param: string }
  | { readonly in: "fact"; readonly name: string }
  | { readonly in: "body"; readonly at: Path }
  | { readonly in: "validator"; readonly index: number };

/** Why a part has no faithful SMT-LIB reading. */
export type NoReading =
  | { readonly kind: "any" }
  | { readonly kind: "operator"; readonly op: string }
  | { readonly kind: "listConversion"; readonly from: Type; readonly to: Type }
  | { readonly kind: "equality"; readonly left: Type; readonly right: Type }
  | { readonly kind: "text"; readonly of: Type }
  | { readonly kind: "fold" }
  | { readonly kind: "roundingDigits" }
  | { readonly kind: "recordSize" }
  | { readonly kind: "undeclaredField"; readonly field: string }
  | { readonly kind: "name"; readonly name: string }
  | { readonly kind: "character"; readonly code: number }
  | { readonly kind: "value"; readonly expected: Type; readonly got: Value };

export type SmtlibError =
  | { readonly kind: "illFormed"; readonly problems: readonly Problem[] }
  | { readonly kind: "noReading"; readonly place: SmtlibPlace; readonly reason: NoReading };

export type SmtlibExport =
  | { readonly kind: "exported"; readonly text: string }
  | { readonly kind: "refused"; readonly error: SmtlibError };

// Claims (`crates/kernel/src/claim.rs`).

/** A claim is wanted of the Answers; an assumption is something their author is committed to. */
export type Stance = "claim" | "assumption";

export interface Claim {
  readonly name: string;
  /** `claim` when absent. */
  readonly stance?: Stance;
  /** The spec whose Answers the claim is about. Inside `holds`, an Ask of it is the Answer under check. */
  readonly about: AskSpec;
  readonly vars?: readonly Param[];
  /** A Bool over `vars`, true for every value of them if the claim holds. */
  readonly holds: Expr;
  readonly description?: string;
}

/** A value for one of the Answer's nested Asks, closed at the arguments its evaluation reached it with. */
export interface HoleValue {
  readonly ask: Ask;
  readonly value: Value;
}

/** A proposed counterexample: values for the claim's variables, and for the nested Asks evaluation reaches. */
export interface Witness {
  readonly vars?: Readonly<Record<string, Value>>;
  readonly holes?: readonly HoleValue[];
}

export type ClaimProblem =
  | { readonly kind: "body"; readonly problem: Problem }
  | { readonly kind: "foreignAsk"; readonly at: Path; readonly ask: string };

/** Why a counterexample can't be read against the Answer at all. */
export type InvalidWitness =
  | { readonly kind: "illFormedClaim"; readonly problems: readonly ClaimProblem[] }
  | { readonly kind: "illFormedAnswer"; readonly problems: readonly Problem[] }
  | { readonly kind: "unexpectedVar"; readonly name: string }
  | {
      readonly kind: "varMismatch";
      readonly name: string;
      readonly expected: Type;
      readonly got: Value;
    }
  | { readonly kind: "holeMismatch"; readonly ask: string; readonly problems: readonly Problem[] }
  | { readonly kind: "opaque"; readonly at: Path }
  | { readonly kind: "tooDeep" };

export type ClaimVerdict =
  /** The claim is false at the witness. */
  | { readonly kind: "refutes" }
  /** The assumption is false at the witness: the Answer contradicts it. */
  | { readonly kind: "inconsistent" }
  /** `holds` is true at the witness: a proposed counterexample that isn't one. */
  | { readonly kind: "notRefuted" }
  /** The Answer or the claim has no value at the witness: it neither holds nor fails there. */
  | { readonly kind: "undefined"; readonly error: EvalError }
  | { readonly kind: "invalid"; readonly reason: InvalidWitness };

export type ClaimAssessment =
  | {
      readonly kind: "assessed";
      /** The claim's content key. */
      readonly claim: string;
      readonly verdict: ClaimVerdict;
      /** For a refutation: an SMT-LIB script, every value fixed, that any solver confirms with `sat`. */
      readonly validation?: string | null;
    }
  /** The witness lacks values the evaluation needs. */
  | {
      readonly kind: "incomplete";
      readonly vars: readonly Param[];
      readonly holes: readonly Open[];
    };

/** How a claim query reads Floats: exactly, as IEEE doubles, or as reals, to search fast. */
export type Floats = "exact" | "real";

/** A value a query asks the solver for, in the order of its `get-value`. */
export type ValueRead =
  | { readonly of: "var"; readonly name: string; readonly type: Type }
  | { readonly of: "hole"; readonly key: string; readonly ask: Ask; readonly type: Type };

export interface ClaimInput {
  readonly claim: Claim;
  readonly answer: Answer;
  readonly witness?: Witness;
}

export type ClaimQuery =
  | Extract<ClaimAssessment, { readonly kind: "assessed" }>
  | { readonly kind: "query"; readonly script: string; readonly reads: readonly ValueRead[] }
  | { readonly kind: "refused"; readonly error: SmtlibError };

export type Unreadable =
  | { readonly kind: "syntax"; readonly detail: string }
  | { readonly kind: "count"; readonly expected: number; readonly got: number }
  | { readonly kind: "value"; readonly type: Type; readonly got: string }
  | { readonly kind: "notFinite" }
  | { readonly kind: "tooLarge" };

export type ReadValues =
  | { readonly kind: "values"; readonly values: readonly Value[] }
  | { readonly kind: "unreadable"; readonly error: Unreadable };

// Deterministic CBOR (`cbor.js`).

export type DecodeError =
  | { readonly kind: "truncated"; readonly offset: number }
  | { readonly kind: "unsupported"; readonly offset: number; readonly initial: number }
  | { readonly kind: "invalidUtf8"; readonly offset: number }
  | { readonly kind: "nonTextKey"; readonly offset: number }
  | { readonly kind: "duplicateKey"; readonly offset: number }
  | { readonly kind: "notFinite"; readonly offset: number }
  /** An integer JavaScript can't represent exactly. */
  | { readonly kind: "outOfRange"; readonly offset: number }
  | { readonly kind: "tooDeep"; readonly offset: number }
  | { readonly kind: "trailingBytes"; readonly offset: number }
  | { readonly kind: "nonCanonical" };

export type Decoded =
  | { readonly ok: true; readonly value: Value }
  | { readonly ok: false; readonly error: DecodeError };

/** Thrown when encoding something outside the data model: a non-finite number, a function, a symbol. */
export class EncodeError extends Error {
  readonly name: "EncodeError";
  readonly found: string;
  constructor(found: string);
}

/** Encode any JSON-shaped value. */
export function encode(value: unknown): Uint8Array<ArrayBuffer>;
export function decode(bytes: Uint8Array): Decoded;

// The kernel.

export interface AssessInput {
  readonly ask: Ask;
  readonly world?: World;
  readonly answer: Answer;
  readonly resolved?: Readonly<Record<string, Value>>;
  readonly evidence?: readonly Verdict[];
}

/** Whether a value is an Answer the kernel can read: the Answer itself, or why not. */
export type AnswerReading =
  | { readonly kind: "readable"; readonly answer: Answer }
  | { readonly kind: "unreadable"; readonly detail: string };

/** The kernel refused a request: it was not one it understands. A bug in the caller, never a verdict. */
export class KernelError extends Error {
  readonly name: "KernelError";
  readonly detail: unknown;
  constructor(detail: unknown);
}

/** The trusted checker. Synchronous and deterministic: the same input gives the same report on every host. */
export interface Kernel {
  key(ask: Ask, world?: World): string;
  specKey(spec: AskSpec): string;
  checkAsk(ask: Ask): readonly Problem[];
  /** Never throws: an Answer from outside is read here before it becomes an event. */
  readAnswer(value: unknown): AnswerReading;
  assess(input: AssessInput): Report;
  /** The Ask, its world and an Answer as an SMT-LIB 2 script, or why part of them has no faithful reading. */
  smtlib(input: SmtlibInput): SmtlibExport;
  claimKey(claim: Claim): string;
  checkClaim(claim: Claim): readonly ClaimProblem[];
  /** Re-check a counterexample to a claim about an Answer: a verdict, or the values it still lacks. */
  assessClaim(input: ClaimInput): ClaimAssessment;
  /** The verdict if the witness is complete; otherwise the SMT-LIB query that asks a solver for what it lacks. */
  claimQuery(input: ClaimInput & { readonly floats?: Floats }): ClaimQuery;
  /** A solver's `get-value` reply to a claim query, as the values its reads ask for. */
  readValues(reply: string, reads: readonly ValueRead[]): ReadValues;
}

/** Instantiate the kernel from its bytes, or from a compiled module where bytes can't be compiled (a Worker). */
export function instantiate(source: BufferSource | WebAssembly.Module): Promise<Kernel>;
