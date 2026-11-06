/-! SMT-LIB's operations over Lean's types, as SMT-LIB defines them. See `docs/2026-11-06-lean.md`. -/

namespace Smt

/-- `seq.contains s t`: `t` is a contiguous part of `s`. -/
def contains (s t : List α) : Prop := t <:+: s

/-- Containing a one-element sequence is membership. -/
@[simp, grind =] theorem contains_unit {s : List α} {x : α} : contains s [x] ↔ x ∈ s := by
  constructor
  · rintro ⟨a, b, rfl⟩
    simp
  · intro h
    obtain ⟨a, b, rfl⟩ := List.append_of_mem h
    exact ⟨a, b, by simp⟩

end Smt
