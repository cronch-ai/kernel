; Claim LongEnough about Label.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:978cce2ea405c0a42635118fdbfc555c67d9078146d36915fa38302ea32e6047
(set-option :produce-models true)
(set-logic ALL)

; Records.
; {name: String, qty: Int}
(declare-datatype |record:dcaf65c9| ((|record:dcaf65c9| (|record:dcaf65c9.name| String) (|record:dcaf65c9.qty| Int))))

; The Answer under check, as a function of its spec's parameters.
(define-fun answer ((|v:item| |record:dcaf65c9|)) String (str.++ (|record:dcaf65c9.name| |v:item|) " x" (let ((|c:1| (|record:dcaf65c9.qty| |v:item|))) (ite (< |c:1| 0) (str.++ "-" (str.from_int (- |c:1|))) (str.from_int |c:1|)))))

; The claim's variables.
(declare-const |v:item| |record:dcaf65c9|)

; Only values Cronch has: integers within ±(2^53 - 1), finite floats.
(assert (and (<= (- 9007199254740991) (|record:dcaf65c9.qty| |v:item|)) (<= (|record:dcaf65c9.qty| |v:item|) 9007199254740991)))

; A counterexample: sat exactly where the claim fails.
; The claim is false here.
(assert (not (> (str.len (answer |v:item|)) 3)))
(check-sat)
(get-value (|v:item|))
