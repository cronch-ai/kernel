; Assumption Grows about Double.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:0d8732ee260d901a9a7550acf4d4c68fa8824df466682d275269261761f07651
(set-option :produce-models true)
(set-logic ALL)

; The Answer under check, as a function of its spec's parameters.
(define-fun answer ((|v:n| Int)) Int (+ |v:n| |v:n|))

; Where the kernel evaluates it without error: no overflow, no division by zero, finite floats.
(define-fun defined ((|v:n| Int)) Bool (and (<= (- 9007199254740991) (+ |v:n| |v:n|)) (<= (+ |v:n| |v:n|) 9007199254740991)))

; The claim's variables.
(declare-const |v:n| Int)

; Only values Cronch has: integers within ±(2^53 - 1), finite floats.
(assert (and (<= (- 9007199254740991) |v:n|) (<= |v:n| 9007199254740991)))

; Only where the claim itself, and every Answer it applies, evaluates.
(assert (defined |v:n|))

; A counterexample: sat exactly where the claim fails.
; The claim is false here.
(assert (not (> (answer |v:n|) |v:n|)))
(check-sat)
(get-value (|v:n|))
