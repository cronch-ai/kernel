; Summary, answered by a Procedure.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:405d5f4e9d8dcd1be078b4d0a18bd0fb53a64ab27566c90a033d030d25d7c2fd
(set-logic ALL)

; Records.
; {name: String, price: Float, qty: Int}
(declare-datatype |record:13030103| ((|record:13030103| (|record:13030103.name| String) (|record:13030103.price| Float64) (|record:13030103.qty| Int))))
; {bulk: Bool, label: String, total: Float}
(declare-datatype |record:2074176b| ((|record:2074176b| (|record:2074176b.bulk| Bool) (|record:2074176b.label| String) (|record:2074176b.total| Float64))))
; {bulk: Bool, label: String, note: String, total: Float}
(declare-datatype |record:ee0968bb| ((|record:ee0968bb| (|record:ee0968bb.bulk| Bool) (|record:ee0968bb.label| String) (|record:ee0968bb.note| String) (|record:ee0968bb.total| Float64))))

; The Answer, as a function of the Ask's parameters.
(define-fun answer ((|v:item| |record:13030103|)) |record:2074176b| (let ((|c:2| (let ((|v:qty| (|record:13030103.qty| |v:item|))) (|record:ee0968bb| (>= |v:qty| 10) (str.++ (|record:13030103.name| |v:item|) " x" (let ((|c:1| |v:qty|)) (ite (< |c:1| 0) (str.++ "-" (str.from_int (- |c:1|))) (str.from_int |c:1|)))) "dropped: the output type doesn't declare it" (ite (> |v:qty| 0) (fp.mul RNE (|record:13030103.price| |v:item|) ((_ to_fp 11 53) RNE (to_real |v:qty|))) ((_ to_fp 11 53) RNE (to_real 0))))))) (|record:2074176b| (|record:ee0968bb.bulk| |c:2|) (|record:ee0968bb.label| |c:2|) (|record:ee0968bb.total| |c:2|))))

; What the Ask's replayable validators say of a result.
(define-fun contract ((|v:item| |record:13030103|) (result |record:2074176b|)) Bool (and (> (str.len (|record:2074176b.label| result)) 0) (<= (str.len (|record:2074176b.label| result)) 12) (fp.geq (|record:2074176b.total| result) ((_ to_fp 11 53) RNE 0.0)) (= (|record:2074176b.bulk| result) true)))

; The closed Ask.
(define-fun value () |record:2074176b| (answer (|record:13030103| "pen" ((_ to_fp 11 53) RNE 1.25) 12)))
(define-fun holds () Bool (contract (|record:13030103| "pen" ((_ to_fp 11 53) RNE 1.25) 12) value))
