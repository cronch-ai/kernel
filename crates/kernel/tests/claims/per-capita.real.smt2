; Claim NonNegative about PerCapita.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:210391aea2d2163027c73573e5ecf8512717bb2666aea6c0186cbbb7d617ba06
(set-option :produce-models true)
(set-logic ALL)

; The Asks this Answer rests on, uninterpreted: their Answers are not part of this one.
(declare-fun |ask:GDP:e064ff52| (String) Real)
(declare-fun |ask:Population:027dfe20| (String) Int)

; The Answer under check, as a function of its spec's parameters.
(define-fun answer ((|v:country| String)) Real (/ (|ask:GDP:e064ff52| |v:country|) (to_real (|ask:Population:027dfe20| |v:country|))))

; Where the kernel evaluates it without error: no overflow, no division by zero, finite floats.
(define-fun defined ((|v:country| String)) Bool (and (and (<= (- 9007199254740991) (|ask:Population:027dfe20| |v:country|)) (<= (|ask:Population:027dfe20| |v:country|) 9007199254740991)) (not (= (to_real (|ask:Population:027dfe20| |v:country|)) 0.0))))

; The claim's variables.
(declare-const |v:country| String)

; Only where the claim itself, and every Answer it applies, evaluates.
(assert (defined |v:country|))

; A counterexample: sat exactly where the claim fails.
; The claim is false here.
(assert (not (>= (answer |v:country|) (to_real 0))))
(check-sat)
(get-value (|v:country|))
