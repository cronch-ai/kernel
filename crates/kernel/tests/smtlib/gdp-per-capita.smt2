; GDPPerCapita, answered by a Procedure.
; Exported from Cronch. Its identity is the content key of its deterministic CBOR, not this text:
; sha256:8c1c4928701f80bd7ad8e405ab2e83556ebc535f4cc38f6009cd5e26459f1c6d
(set-logic ALL)

; The requester's declared world.
; from the requester
(define-fun |fact:year| () Int 2023)

; The Asks this Answer rests on, uninterpreted: their Answers are not part of this one.
; Nominal GDP in current US dollars.
(declare-fun |ask:GDP:900a98e2| (String Int) Float64)
(declare-fun |ask:Population:2c2e147c| (String Int) Int)

; Values the responder assumes, which nobody checked:
; /be: 1 : Float (no revision since publication), from World Bank

; The Answer, as a function of the Ask's parameters.
; GDP per capita for the country, in current US dollars.
(define-fun answer ((|v:country| String)) Float64 (let ((|v:adjustment| ((_ to_fp 11 53) RNE 1.0))) (fp.mul RNE |v:adjustment| (fp.div RNE (|ask:GDP:900a98e2| |v:country| |fact:year|) ((_ to_fp 11 53) RNE (to_real (|ask:Population:2c2e147c| |v:country| |fact:year|)))))))

; What the Ask's replayable validators say of a result.
; unchecked: compatible GDP/population basis
; delegated to auditor: same year for both figures
(define-fun contract ((|v:country| String) (result Float64)) Bool (fp.geq result ((_ to_fp 11 53) RNE 0.0)))

; The closed Ask.
(define-fun value () Float64 (answer "Kenya"))
(define-fun holds () Bool (contract "Kenya" value))
