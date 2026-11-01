//! The Cronch kernel: the trusted checker at every trust boundary.
//!
//! An Actor answers an Ask with a raw value or a Procedure that may contain further Asks. The kernel decides, from the
//! data alone, whether that Answer is a valid construction, how far it evaluates, and which parts of the Ask's contract
//! its value meets. It never decides whom to trust or what to run: that is the requester's policy and the runtime's.

pub mod api;
pub mod assess;
pub mod cbor;
pub mod check;
pub mod claim;
pub mod eval;
pub mod harness;
pub mod ir;
pub mod key;
pub mod labeled;
pub mod ops;
pub mod run;
pub mod smtlib;
pub mod types;
pub mod validate;
