# A refund agent that reasons under a policy

Constrained reasoning: a harness acts on a conclusion only if a checker has accepted a proof of it, from a fixed theory and from facts that trusted results gave. Here the action is a refund, and the theory is the refund policy.

- [orders.json](orders.json) looks an order up. Its trusted result adds the order to the grant's `late` and `premium` facts if they hold.
- [carrier.json](carrier.json) asks the carrier about damage. Its trusted result adds the order to `damaged`.
- [policy.json](policy.json) stands for the checker. `prove_refundable` takes an order and a proof, and every fact in the grant: its precondition requires the call's facts to be exactly the grant's, so no fact is left out or made up. Its result is trusted, since the checker is vouched for, and `ok` adds the order to `refundable`.
- [refund.json](refund.json) refunds an order only if it is `refundable` and not yet `refunded`. It counts as refunded when the call is made.

Each predicate is a grant field of its own, a list of strings, so the preconditions read as SMT-LIB and Lean read them.

## The claim

**`refund.issue` can't happen for an order until `policy.prove_refundable` has accepted a proof that it is refundable.** It is a rule, approved by `policy.prove_refundable` when its result is `ok` and its order is the refund's. Its obligations are the initial grant, the checker's update and the refund's own update. The capabilities that give facts are framed: they write nothing a refund reads, so adding one re-proves nothing.

[proofs/reasoning.test.mjs](../../proofs/reasoning.test.mjs) proves the obligations with cvc5 and checks the proofs with Ethos, proves them again in Lean with `grind`, and breaks the contracts to show the proofs depend on them. [crates/kernel/tests/reasoning.rs](../../crates/kernel/tests/reasoning.rs) runs it: a refund before any derivation, a call that cites a fact the grant doesn't have or leaves one out, a refused proof, a refund of another order and a second refund are all refused, and untrusted results change no grant.

What the checker accepts, a Lean proof of `Gamma -> facts -> refundable(order)`, is the platform's: see [Constrained reasoning](https://linear.app/cronch-ai/document/constrained-reasoning-acting-only-on-what-follows-from-a-fixed-theory-79e80245ee7f).
