; Claim NonNegative about Abs.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:12e6dc6d262ec408a01f985fd1d6ffe107bf44765c11f720cd010abb7e190587
(set-option :produce-models true)
(set-logic ALL)

; The Answer under check, as a function of its spec's parameters.
; The absolute value.
(define-fun answer ((|v:n| Int)) Int (ite (< |v:n| 0) |v:n| |v:n|))

; The claim's variables.
(declare-const |v:n| Int)

; Only values Cronch has: integers within ±(2^53 - 1), finite floats.
(assert (and (<= (- 9007199254740991) |v:n|) (<= |v:n| 9007199254740991)))

; A counterexample: sat exactly where the claim fails.
; The claim is false here.
(assert (not (>= (answer |v:n|) 0)))
(check-sat)
(get-value (|v:n|))
