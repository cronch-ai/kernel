; Rule: net.fetch needs an approval. Obligation: initial.
; unsat is what the rule needs; sat gives a counterexample. Unsat is not the kernel's verdict: check a proof against this query.
(set-logic ALL)

; Records and variants.
; {url: String}
(declare-datatype |record:8742a65a| ((|record:8742a65a#make| (|record:8742a65a.url| String))))
; {netAllow: List(String)}
(declare-datatype |record:2e5c7a65| ((|record:2e5c7a65#make| (|record:2e5c7a65.netAllow| (Seq String)))))

; The query.
(declare-const |v:args| |record:8742a65a|)
(declare-const |v:grant| |record:2e5c7a65|)
(assert (= |v:grant| (|record:2e5c7a65#make| (as seq.empty (Seq String)))))
(assert (seq.contains (|record:2e5c7a65.netAllow| |v:grant|) (seq.unit (|record:8742a65a.url| |v:args|))))
(check-sat)
