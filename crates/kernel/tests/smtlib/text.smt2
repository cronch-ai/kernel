; Greeting, answered by a Procedure.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:53fbc326e51f008b0260e7c35c22f817969d560ac766c1a1ad187a31bd568f84
(set-logic ALL)

; The Answer, as a function of the Ask's parameters.
(define-fun answer ((|v:name| String) (|v:times| Int)) String (str.++ "H\u{e9} """ |v:name| """ \u{5c} " (let ((|c:1| |v:times|)) (ite (< |c:1| 0) (str.++ "-" (str.from_int (- |c:1|))) (str.from_int |c:1|))) " " (ite (= (str.len |v:name|) |v:times|) "true" "false") " " (let ((|c:2| (seq.len (seq.++ (seq.unit |v:name|) (seq.unit "b") (seq.unit "c"))))) (ite (< |c:2| 0) (str.++ "-" (str.from_int (- |c:2|))) (str.from_int |c:2|)))))

; What the Ask's replayable validators say of a result.
(define-fun contract ((|v:name| String) (|v:times| Int) (result String)) Bool (and (>= (str.len result) 12) (= result "H\u{e9} ""x"" \u{5c} 1 true 3")))

; The closed Ask.
(define-fun value () String (answer "x" 1))
(define-fun holds () Bool (contract "x" 1 value))
