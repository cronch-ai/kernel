; Rule: net.fetch needs an approval. Obligation: net.ask.onResult.
; unsat is what the rule needs; sat gives a counterexample. Unsat is not the kernel's verdict: check a proof against this query.
(set-logic ALL)

; Records and variants.
; {url: String}
(declare-datatype |record:8742a65a| ((|record:8742a65a#make| (|record:8742a65a.url| String))))
; {netAllow: List(String)}
(declare-datatype |record:2e5c7a65| ((|record:2e5c7a65#make| (|record:2e5c7a65.netAllow| (Seq String)))))

; The query.
(declare-const |v:rule:call| |record:8742a65a|)
(declare-const |v:rule:approved| Bool)
(declare-const |v:rule:grant| |record:2e5c7a65|)
(declare-const |v:rule:args| |record:8742a65a|)
(declare-const |v:rule:result| String)
(assert (and (and (or (not (let ((|v:args| |v:rule:call|)) (let ((|v:grant| |v:rule:grant|)) (seq.contains (|record:2e5c7a65.netAllow| |v:grant|) (seq.unit (|record:8742a65a.url| |v:args|)))))) |v:rule:approved|) (let ((|v:result| |v:rule:result|)) (let ((|v:args| |v:rule:args|)) (let ((|v:grant| |v:rule:grant|)) (seq.contains (seq.++ (seq.unit "allow") (seq.unit "deny")) (seq.unit |v:result|)))))) (not (or (not (let ((|v:args| |v:rule:call|)) (let ((|v:grant| (let ((|v:result| |v:rule:result|)) (let ((|v:args| |v:rule:args|)) (let ((|v:grant| |v:rule:grant|)) (ite (= |v:result| "allow") (|record:2e5c7a65#make| (seq.++ (|record:2e5c7a65.netAllow| |v:grant|) (seq.unit (|record:8742a65a.url| |v:args|)))) |v:grant|)))))) (seq.contains (|record:2e5c7a65.netAllow| |v:grant|) (seq.unit (|record:8742a65a.url| |v:args|)))))) (or |v:rule:approved| (let ((|v:result| |v:rule:result|)) (let ((|v:approval| |v:rule:args|)) (let ((|v:args| |v:rule:call|)) (and (= |v:result| "allow") (= (|record:8742a65a.url| |v:approval|) (|record:8742a65a.url| |v:args|)))))))))))
(check-sat)
