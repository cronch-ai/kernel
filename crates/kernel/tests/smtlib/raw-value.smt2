; Primes, answered by a raw value.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:8ad6b793566cb5b9f99ce0ba7504132e0b92fa407257972ffd3f54fce40c7988
(set-logic ALL)

; The Answer, as a function of the Ask's parameters.
(define-fun answer () (Seq Int) (seq.++ (seq.unit 2) (seq.unit 3) (seq.unit 5)))

; What the Ask's replayable validators say of a result.
(define-fun contract ((result (Seq Int))) Bool (and (> (seq.len result) 0) (<= (seq.len result) 2)))

; The closed Ask.
(define-fun value () (Seq Int) answer)
(define-fun holds () Bool (contract value))
