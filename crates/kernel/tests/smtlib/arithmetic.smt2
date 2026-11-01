; Score, answered by a Procedure.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:13668b7d4b6cc9a3b3c9b36c6ac6daf319fbc46c2c25e68496a91f950d252cb7
(set-logic ALL)

; The Answer, as a function of the Ask's parameters.
(define-fun answer ((|v:n| Int) (|v:x| Float64)) Float64 (let ((|v:s| (* |v:n| |v:n|))) (ite (> |v:s| 10) (fp.add RNE (fp.div RNE (fp.roundToIntegral RNA (fp.mul RNE (fp.div RNE |v:x| ((_ to_fp 11 53) RNE (to_real 3))) ((_ to_fp 11 53) RNE 100.0))) ((_ to_fp 11 53) RNE 100.0)) ((_ to_fp 11 53) RNE (to_real (mod |v:s| 7)))) (fp.sub RNE (fp.neg |v:x|) ((_ to_fp 11 53) RNE (to_real (div |v:n| (- 2))))))))

; What the Ask's replayable validators say of a result.
(define-fun contract ((|v:n| Int) (|v:x| Float64) (result Float64)) Bool (and (fp.geq result ((_ to_fp 11 53) RNE (- 100.0))) (fp.leq result ((_ to_fp 11 53) RNE 1000.0))))

; The closed Ask.
(define-fun value () Float64 (answer 4 ((_ to_fp 11 53) RNE 0.1)))
(define-fun holds () Bool (contract 4 ((_ to_fp 11 53) RNE 0.1) value))
