; Claim Smaller about Half.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:5b9ad9867612d95b38cd1f1995b7bd7a7f030c9d72680c7dd4e19f9016825ea6
(set-option :produce-models true)
(set-logic ALL)

; The Answer under check, as a function of its spec's parameters.
(define-fun answer ((|v:x| Float64)) Float64 (fp.div RNE |v:x| ((_ to_fp 11 53) RNE (to_real 2))))

; Where the kernel evaluates it without error: no overflow, no division by zero, finite floats.
(define-fun defined ((|v:x| Float64)) Bool (and (not (fp.isZero ((_ to_fp 11 53) RNE (to_real 2)))) (not (or (fp.isNaN (fp.div RNE |v:x| ((_ to_fp 11 53) RNE (to_real 2)))) (fp.isInfinite (fp.div RNE |v:x| ((_ to_fp 11 53) RNE (to_real 2))))))))

; The claim's variables.
(declare-const |v:x| Float64)

; Only values Cronch has: integers within ±(2^53 - 1), finite floats.
(assert (not (or (fp.isNaN |v:x|) (fp.isInfinite |v:x|))))

; Only where the claim itself, and every Answer it applies, evaluates.
(assert (defined |v:x|))

; A counterexample: sat exactly where the claim fails.
; The claim is false here.
(assert (not (fp.leq (answer |v:x|) |v:x|)))
(check-sat)
(get-value (|v:x|))
