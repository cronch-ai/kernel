; Rule: shell.online needs an approval. Obligation: initial.
; unsat is what the rule needs; sat gives a counterexample. Unsat is not the kernel's verdict: check a proof against this query.
(set-logic ALL)

; Records and variants.
; {command: String, url: String}
(declare-datatype |record:7dc8022b| ((|record:7dc8022b#make| (|record:7dc8022b.command| String) (|record:7dc8022b.url| String))))
; {netAllow: List(String)}
(declare-datatype |record:2e5c7a65| ((|record:2e5c7a65#make| (|record:2e5c7a65.netAllow| (Seq String)))))

; The query.
(declare-const |v:args| |record:7dc8022b|)
(declare-const |v:grant| |record:2e5c7a65|)
(assert (= |v:grant| (|record:2e5c7a65#make| (as seq.empty (Seq String)))))
(assert (seq.contains (|record:2e5c7a65.netAllow| |v:grant|) (seq.unit (|record:7dc8022b.url| |v:args|))))
(check-sat)
