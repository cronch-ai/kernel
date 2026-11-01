; Claim Smaller about Half.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:5b9ad9867612d95b38cd1f1995b7bd7a7f030c9d72680c7dd4e19f9016825ea6
(set-option :produce-models true)
(set-logic ALL)

; The Answer under check, as a function of its spec's parameters.
(define-fun answer ((|v:x| Real)) Real (/ |v:x| (to_real 2)))

; Where the kernel evaluates it without error: no overflow, no division by zero, finite floats.
(define-fun defined ((|v:x| Real)) Bool (not (= (to_real 2) 0.0)))

; The claim's variables.
(declare-const |v:x| Real)

; Only where the claim itself, and every Answer it applies, evaluates.
(assert (defined |v:x|))

; A counterexample: sat exactly where the claim fails.
; The claim is false here.
(assert (not (<= (answer |v:x|) |v:x|)))
(check-sat)
(get-value (|v:x|))
