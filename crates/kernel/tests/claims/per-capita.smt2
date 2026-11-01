; Claim NonNegative about PerCapita.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:210391aea2d2163027c73573e5ecf8512717bb2666aea6c0186cbbb7d617ba06
(set-option :produce-models true)
(set-logic ALL)

; The Asks this Answer rests on, uninterpreted: their Answers are not part of this one.
(declare-fun |ask:GDP:e064ff52| (String) Float64)
(declare-fun |ask:Population:027dfe20| (String) Int)

; The Answer under check, as a function of its spec's parameters.
(define-fun answer ((|v:country| String)) Float64 (fp.div RNE (|ask:GDP:e064ff52| |v:country|) ((_ to_fp 11 53) RNE (to_real (|ask:Population:027dfe20| |v:country|)))))

; Where the kernel evaluates it without error: no overflow, no division by zero, finite floats.
(define-fun defined ((|v:country| String)) Bool (and (not (or (fp.isNaN (|ask:GDP:e064ff52| |v:country|)) (fp.isInfinite (|ask:GDP:e064ff52| |v:country|)))) (and (<= (- 9007199254740991) (|ask:Population:027dfe20| |v:country|)) (<= (|ask:Population:027dfe20| |v:country|) 9007199254740991)) (not (fp.isZero ((_ to_fp 11 53) RNE (to_real (|ask:Population:027dfe20| |v:country|))))) (not (or (fp.isNaN (fp.div RNE (|ask:GDP:e064ff52| |v:country|) ((_ to_fp 11 53) RNE (to_real (|ask:Population:027dfe20| |v:country|))))) (fp.isInfinite (fp.div RNE (|ask:GDP:e064ff52| |v:country|) ((_ to_fp 11 53) RNE (to_real (|ask:Population:027dfe20| |v:country|)))))))))

; The claim's variables.
(declare-const |v:country| String)

; Only where the claim itself, and every Answer it applies, evaluates.
(assert (defined |v:country|))

; A counterexample: sat exactly where the claim fails.
; The claim is false here.
(assert (not (>= (fp.to_real (answer |v:country|)) (to_real 0))))
(check-sat)
(get-value (|v:country|))
