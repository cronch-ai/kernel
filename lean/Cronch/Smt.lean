/-! SMT-LIB's operations over Lean's types, as the kernel's exports use them. See `docs/2026-11-06-lean.md`.

An export reads SMT-LIB's `Bool` as `Prop`, `(Seq t)` as `List t`, `String` as `String` and `Int` as `Int`. Each
operation here is named as in SMT-LIB, under `Smt`, and agrees with SMT-LIB wherever an export can use it:

- A refutation never holds a partial operation (the kernel refuses one), so `Smt.div` and `Smt.mod` are only used
  with a divisor that isn't zero, where SMT-LIB's `div` and `mod` are Euclidean, as `Int.ediv` and `Int.emod` are.
- `seq.nth` is only used under `0 ≤ i < seq.len s`. Outside that, SMT-LIB leaves it unspecified, and `Smt.seq.nth`
  gives `default`.

Lemmas tagged `grind` and `simp` let those tactics see through these definitions. They are proved here, so they add
nothing to what a proof rests on. -/

namespace Smt

/-- `div`, Euclidean: the remainder is never negative. -/
def div (x y : Int) : Int := Int.ediv x y

/-- `mod`, Euclidean: the remainder is never negative. -/
def mod (x y : Int) : Int := Int.emod x y

/-- `seq.len` -/
def seq.len (s : List α) : Int := s.length

/-- `seq.nth`, within `0 ≤ i < seq.len s`. -/
def seq.nth [Inhabited α] (s : List α) (i : Int) : α := s.getD i.toNat default

/-- `seq.extract s i n`: the part of `s` from `i` of length `n`, or as much as there is; empty if `i` is outside `s`
or `n` isn't positive. -/
def seq.extract (s : List α) (i n : Int) : List α :=
  if 0 ≤ i ∧ i < s.length ∧ 0 < n then (s.drop i.toNat).take n.toNat else []

/-- `seq.prefixof a b`: `a` begins `b`. -/
def seq.prefixof (a b : List α) : Prop := a <+: b

/-- `seq.contains s t`: `t` is a contiguous part of `s`. -/
def seq.contains (s t : List α) : Prop := t <:+: s

/-- `str.len`, in characters. -/
def str.len (s : String) : Int := s.length

/-- `str.prefixof a b`: `a` begins `b`. -/
def str.prefixof (a b : String) : Prop := a.toList <+: b.toList

/-- `str.contains s t`: `t` is a contiguous part of `s`. -/
def str.contains (s t : String) : Prop := t.toList <:+: s.toList

/-- `str.from_int`: a natural number's decimal digits, and `""` for a negative one. -/
def str.from_int (n : Int) : String := if n < 0 then "" else toString n

/-- Containing a one-element sequence is membership. -/
@[simp, grind =] theorem seq.contains_unit {s : List α} {x : α} : seq.contains s [x] ↔ x ∈ s := by
  constructor
  · rintro ⟨a, b, rfl⟩
    simp
  · intro h
    obtain ⟨a, b, rfl⟩ := List.append_of_mem h
    exact ⟨a, b, by simp⟩

end Smt
