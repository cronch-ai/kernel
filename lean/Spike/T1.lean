import Cronch.Smt

/-! The T1 obligations, written by hand from the queries beside this file. Each theorem is the negation of its
query's assertions: the query is `unsat` exactly when the theorem holds. -/
open Smt

/-- `{url: String}` -/
structure «record:8742a65a» where
  url : String

/-- `{netAllow: List(String)}` -/
structure «record:2e5c7a65» where
  netAllow : List String

/-- `{command: String, url: String}` -/
structure «record:7dc8022b» where
  command : String
  url : String

/-- `net.fetch.initial.smt2` -/
theorem net_fetch_initial :
    ∀ (args : «record:8742a65a») (grant : «record:2e5c7a65»),
      ¬ (grant = ⟨[]⟩ ∧ contains grant.netAllow [args.url]) := by
  grind

/-- `net.fetch.net.ask.onResult.smt2` -/
theorem net_fetch_on_result :
    ∀ (call : «record:8742a65a») (approved : Bool) (grant : «record:2e5c7a65»)
      (args : «record:8742a65a») (result : String),
      ¬ (((¬ contains grant.netAllow [call.url] ∨ approved = true)
          ∧ contains ["allow", "deny"] [result])
        ∧ ¬ (¬ contains
                (if result = "allow" then («record:2e5c7a65».mk (grant.netAllow ++ [args.url])) else grant).netAllow
                [call.url]
              ∨ (approved = true ∨ (result = "allow" ∧ args.url = call.url)))) := by
  grind

/-- `shell.online.initial.smt2` -/
theorem shell_online_initial :
    ∀ (args : «record:7dc8022b») (grant : «record:2e5c7a65»),
      ¬ (grant = ⟨[]⟩ ∧ contains grant.netAllow [args.url]) := by
  grind

/-- `shell.online.net.ask.onResult.smt2` -/
theorem shell_online_on_result :
    ∀ (call : «record:7dc8022b») (approved : Bool) (grant : «record:2e5c7a65»)
      (args : «record:8742a65a») (result : String),
      ¬ (((¬ contains grant.netAllow [call.url] ∨ approved = true)
          ∧ contains ["allow", "deny"] [result])
        ∧ ¬ (¬ contains
                (if result = "allow" then («record:2e5c7a65».mk (grant.netAllow ++ [args.url])) else grant).netAllow
                [call.url]
              ∨ (approved = true ∨ (result = "allow" ∧ args.url = call.url)))) := by
  grind

/-! Each proof should use only Lean's standard axioms. -/
#print axioms net_fetch_initial
#print axioms net_fetch_on_result
#print axioms shell_online_initial
#print axioms shell_online_on_result
