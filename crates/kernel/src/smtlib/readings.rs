//! The registries: what each Cronch type and operator reads as in SMT-LIB, declared as data.
//!
//! Encoding consults these and nothing else for meaning: adding an operator to the kernel means registering its
//! reading here, and one with no registered reading is refused, never guessed.

use crate::ir::Type;
use crate::ops::Op;

/// The sorts of the types that have one name.
pub const SORTS: [(Type, &str); 4] = [
    (Type::Int, "Int"),
    (Type::Float, "Float64"),
    (Type::String, "String"),
    (Type::Bool, "Bool"),
];

/// How an operator reads, given its operands' types.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reading {
    /// The integer function when every operand is an integer, else the float one, operands converted to float.
    Numeric {
        int: &'static str,
        float: &'static str,
        rounded: bool,
    },
    /// A float function of operands converted to float. `rounded` takes the rounding mode, nearest-even.
    Float { name: &'static str, rounded: bool },
    /// The same function, of the operands as they are.
    Same(&'static str),
    /// A comparison of exact values, as the kernel's: of integers, of floats, or of both read as reals.
    Compare {
        int: &'static str,
        float: &'static str,
    },
    /// The kernel's equality: numbers by exact value, other values as SMT-LIB values where their type says all of them.
    Equal { negated: bool },
    /// String concatenation, of one or more operands.
    Concat,
    /// The JSON text of a value: strings as they are, integers and booleans as JSON writes them.
    Text,
    /// The length of a string in code points, or of a list.
    Length,
    /// Rounding half away from zero, to an integer or to a literal number of decimal digits.
    Round,
    /// One string or list is a prefix of another, by the kernel's equality of items.
    Prefix,
    /// A list with one more item at its end.
    Append,
    /// A list has an item, or a string a substring.
    Contains,
}

/// Every operator's reading.
pub const OPERATORS: [(Op, Reading); 23] = [
    (
        Op::Add,
        Reading::Numeric {
            int: "+",
            float: "fp.add",
            rounded: true,
        },
    ),
    (
        Op::Sub,
        Reading::Numeric {
            int: "-",
            float: "fp.sub",
            rounded: true,
        },
    ),
    (
        Op::Mul,
        Reading::Numeric {
            int: "*",
            float: "fp.mul",
            rounded: true,
        },
    ),
    (
        Op::Div,
        Reading::Float {
            name: "fp.div",
            rounded: true,
        },
    ),
    // Euclidean, in SMT-LIB as in the kernel.
    (Op::IDiv, Reading::Same("div")),
    (Op::Mod, Reading::Same("mod")),
    (
        Op::Neg,
        Reading::Numeric {
            int: "-",
            float: "fp.neg",
            rounded: false,
        },
    ),
    (Op::Eq, Reading::Equal { negated: false }),
    (Op::Ne, Reading::Equal { negated: true }),
    // The kernel compares numbers by their exact values, so integers compare as integers.
    (
        Op::Lt,
        Reading::Compare {
            int: "<",
            float: "fp.lt",
        },
    ),
    (
        Op::Le,
        Reading::Compare {
            int: "<=",
            float: "fp.leq",
        },
    ),
    (
        Op::Gt,
        Reading::Compare {
            int: ">",
            float: "fp.gt",
        },
    ),
    (
        Op::Ge,
        Reading::Compare {
            int: ">=",
            float: "fp.geq",
        },
    ),
    (Op::And, Reading::Same("and")),
    (Op::Or, Reading::Same("or")),
    (Op::Not, Reading::Same("not")),
    (Op::Concat, Reading::Concat),
    (Op::Str, Reading::Text),
    (Op::Len, Reading::Length),
    (Op::Round, Reading::Round),
    (Op::Prefix, Reading::Prefix),
    (Op::Append, Reading::Append),
    (Op::Contains, Reading::Contains),
];

pub fn operator(op: Op) -> Option<Reading> {
    OPERATORS
        .iter()
        .find(|(o, _)| *o == op)
        .map(|(_, reading)| *reading)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::OPS;

    #[test]
    fn every_operator_has_a_reading() {
        for (name, op) in OPS {
            assert!(operator(op).is_some(), "{name} has no SMT-LIB reading");
        }
    }
}
