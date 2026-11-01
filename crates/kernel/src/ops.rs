//! The pure operators a Procedure may apply: their types, and their meaning over values.
//!
//! Every operator is total over well-typed inputs except where an error says otherwise (division by zero, integer
//! overflow, a non-finite float). Nothing here touches the world.

use crate::ir::Type;
use crate::types::conforms;
use serde::Serialize;
use serde_json::{Number, Value};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Neg,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    Concat,
    Str,
    Len,
    Round,
}

/// Every operator, by the name a Procedure applies it with.
pub const OPS: [(&str, Op); 20] = [
    ("add", Op::Add),
    ("sub", Op::Sub),
    ("mul", Op::Mul),
    ("div", Op::Div),
    ("idiv", Op::IDiv),
    ("mod", Op::Mod),
    ("neg", Op::Neg),
    ("eq", Op::Eq),
    ("ne", Op::Ne),
    ("lt", Op::Lt),
    ("le", Op::Le),
    ("gt", Op::Gt),
    ("ge", Op::Ge),
    ("and", Op::And),
    ("or", Op::Or),
    ("not", Op::Not),
    ("concat", Op::Concat),
    ("str", Op::Str),
    ("len", Op::Len),
    ("round", Op::Round),
];

impl Op {
    pub fn parse(name: &str) -> Option<Op> {
        OPS.iter().find(|(n, _)| *n == name).map(|(_, op)| *op)
    }
}

/// How many operands an operator takes.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "arity", rename_all = "camelCase")]
pub enum Arity {
    Exactly { n: usize },
    Between { min: usize, max: usize },
    AtLeast { n: usize },
}

impl Arity {
    fn admits(&self, got: usize) -> bool {
        match *self {
            Arity::Exactly { n } => got == n,
            Arity::Between { min, max } => (min..=max).contains(&got),
            Arity::AtLeast { n } => got >= n,
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpTypeError {
    Arity {
        expected: Arity,
        got: usize,
    },
    Operand {
        index: usize,
        expected: Type,
        got: Type,
    },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpEvalError {
    DivisionByZero,
    Overflow,
    NotFinite,
    /// An operand that type-checking admitted (through `Any`) turned out not to fit.
    Operand {
        index: usize,
        expected: Type,
        got: Value,
    },
}

fn arity(op: Op) -> Arity {
    match op {
        Op::Neg | Op::Not | Op::Str | Op::Len => Arity::Exactly { n: 1 },
        Op::Round => Arity::Between { min: 1, max: 2 },
        Op::Concat => Arity::AtLeast { n: 1 },
        _ => Arity::Exactly { n: 2 },
    }
}

fn expect(index: usize, got: &Type, expected: Type) -> Result<(), OpTypeError> {
    if conforms(got, &expected) {
        Ok(())
    } else {
        Err(OpTypeError::Operand {
            index,
            expected,
            got: got.clone(),
        })
    }
}

fn numeric(args: &[Type]) -> Result<Type, OpTypeError> {
    for (i, t) in args.iter().enumerate() {
        expect(i, t, Type::Float)?;
    }
    Ok(if args.iter().all(|t| *t == Type::Int) {
        Type::Int
    } else {
        Type::Float
    })
}

/// The operator's result type for operands of these types.
pub fn result_type(op: Op, args: &[Type]) -> Result<Type, OpTypeError> {
    let expected = arity(op);
    if !expected.admits(args.len()) {
        return Err(OpTypeError::Arity {
            expected,
            got: args.len(),
        });
    }
    match op {
        Op::Add | Op::Sub | Op::Mul | Op::Neg => numeric(args),
        Op::Div => numeric(args).map(|_| Type::Float),
        Op::IDiv | Op::Mod => {
            for (i, t) in args.iter().enumerate() {
                expect(i, t, Type::Int)?;
            }
            Ok(Type::Int)
        }
        Op::Eq | Op::Ne => Ok(Type::Bool),
        Op::Lt | Op::Le | Op::Gt | Op::Ge => numeric(args).map(|_| Type::Bool),
        Op::And | Op::Or | Op::Not => {
            for (i, t) in args.iter().enumerate() {
                expect(i, t, Type::Bool)?;
            }
            Ok(Type::Bool)
        }
        Op::Concat => {
            for (i, t) in args.iter().enumerate() {
                expect(i, t, Type::String)?;
            }
            Ok(Type::String)
        }
        Op::Str => Ok(Type::String),
        Op::Len => match &args[0] {
            Type::String | Type::List(_) | Type::Any => Ok(Type::Int),
            got => Err(OpTypeError::Operand {
                index: 0,
                expected: Type::String,
                got: got.clone(),
            }),
        },
        Op::Round => {
            expect(0, &args[0], Type::Float)?;
            match args.get(1) {
                None => Ok(Type::Int),
                Some(digits) => expect(1, digits, Type::Int).map(|_| Type::Float),
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Num {
    I(i64),
    F(f64),
}

fn num(index: usize, v: &Value) -> Result<Num, OpEvalError> {
    let bad = || OpEvalError::Operand {
        index,
        expected: Type::Float,
        got: v.clone(),
    };
    let n = v.as_number().ok_or_else(bad)?;
    Ok(match n.as_i64() {
        Some(i) => Num::I(i),
        None => Num::F(n.as_f64().ok_or_else(bad)?),
    })
}

fn float(f: f64) -> Result<Value, OpEvalError> {
    Number::from_f64(f)
        .map(Value::Number)
        .ok_or(OpEvalError::NotFinite)
}

fn as_f64(n: &Num) -> f64 {
    match *n {
        Num::I(i) => i as f64,
        Num::F(f) => f,
    }
}

/// The largest integer every host carries exactly: JSON's and JavaScript's safe integers, ±(2^53 - 1).
pub const MAX_SAFE_INT: i64 = (1 << 53) - 1;

/// The integer, if it is within ±(2^53 - 1).
fn safe(i: i64) -> Option<i64> {
    (-MAX_SAFE_INT..=MAX_SAFE_INT).contains(&i).then_some(i)
}

fn arith(
    a: &Value,
    b: &Value,
    int: fn(i64, i64) -> Option<i64>,
    flt: fn(f64, f64) -> f64,
) -> Result<Value, OpEvalError> {
    match (num(0, a)?, num(1, b)?) {
        (Num::I(x), Num::I(y)) => int(x, y)
            .and_then(safe)
            .map(Value::from)
            .ok_or(OpEvalError::Overflow),
        (x, y) => float(flt(as_f64(&x), as_f64(&y))),
    }
}

fn int(index: usize, v: &Value) -> Result<i64, OpEvalError> {
    match num(index, v)? {
        Num::I(i) => Ok(i),
        Num::F(f) if f.fract() == 0.0 => Ok(f as i64),
        Num::F(_) => Err(OpEvalError::Operand {
            index,
            expected: Type::Int,
            got: v.clone(),
        }),
    }
}

fn boolean(index: usize, v: &Value) -> Result<bool, OpEvalError> {
    v.as_bool().ok_or_else(|| OpEvalError::Operand {
        index,
        expected: Type::Bool,
        got: v.clone(),
    })
}

fn string(index: usize, v: &Value) -> Result<&str, OpEvalError> {
    v.as_str().ok_or_else(|| OpEvalError::Operand {
        index,
        expected: Type::String,
        got: v.clone(),
    })
}

/// An integer against a finite float, by their exact values: the integer is never rounded to a float first.
fn int_against_float(i: i64, f: f64) -> Ordering {
    // 2^63: every float at or beyond it is beyond every i64.
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if f >= LIMIT {
        return Ordering::Less;
    }
    if f < -LIMIT {
        return Ordering::Greater;
    }
    // Within range, the float's integer part is exact as an i64; its fraction decides a tie.
    let whole = f.trunc();
    i.cmp(&(whole as i64))
        .then_with(|| 0.0.partial_cmp(&(f - whole)).unwrap_or(Ordering::Equal))
}

/// Two numbers by their exact values, so integers beyond 2^53 compare as integers, not as the floats nearest them.
fn exact(a: &Num, b: &Num) -> Ordering {
    match (*a, *b) {
        (Num::I(x), Num::I(y)) => x.cmp(&y),
        (Num::F(x), Num::F(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Num::I(x), Num::F(y)) => int_against_float(x, y),
        (Num::F(x), Num::I(y)) => int_against_float(y, x).reverse(),
    }
}

/// Two numbers by their exact values, or `None` if either isn't a number. Validators compare with it too.
pub fn order(a: &Value, b: &Value) -> Option<Ordering> {
    Some(exact(&num(0, a).ok()?, &num(1, b).ok()?))
}

fn compare(a: &Value, b: &Value, test: fn(Ordering) -> bool) -> Result<Value, OpEvalError> {
    Ok(Value::Bool(test(exact(&num(0, a)?, &num(1, b)?))))
}

/// Numeric equality is of exact values and ignores the integer/float distinction, as the canonical encoding does.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(_), Value::Number(_)) => match (num(0, a), num(1, b)) {
            (Ok(x), Ok(y)) => exact(&x, &y) == Ordering::Equal,
            _ => false,
        },
        _ => a == b,
    }
}

fn length(n: usize) -> Result<Value, OpEvalError> {
    i64::try_from(n)
        .ok()
        .and_then(safe)
        .map(Value::from)
        .ok_or(OpEvalError::Overflow)
}

/// Apply the operator to operand values. Operands were type-checked, but are re-checked here: a value admitted
/// through `Any` must still fit.
pub fn apply(op: Op, args: &[Value]) -> Result<Value, OpEvalError> {
    match op {
        Op::Add => arith(&args[0], &args[1], i64::checked_add, |x, y| x + y),
        Op::Sub => arith(&args[0], &args[1], i64::checked_sub, |x, y| x - y),
        Op::Mul => arith(&args[0], &args[1], i64::checked_mul, |x, y| x * y),
        Op::Div => {
            let (x, y) = (as_f64(&num(0, &args[0])?), as_f64(&num(1, &args[1])?));
            if y == 0.0 {
                Err(OpEvalError::DivisionByZero)
            } else {
                float(x / y)
            }
        }
        Op::IDiv | Op::Mod => {
            let (x, y) = (int(0, &args[0])?, int(1, &args[1])?);
            if y == 0 {
                return Err(OpEvalError::DivisionByZero);
            }
            let r = if op == Op::IDiv {
                x.checked_div_euclid(y)
            } else {
                x.checked_rem_euclid(y)
            };
            r.and_then(safe)
                .map(Value::from)
                .ok_or(OpEvalError::Overflow)
        }
        Op::Neg => match num(0, &args[0])? {
            Num::I(i) => i
                .checked_neg()
                .and_then(safe)
                .map(Value::from)
                .ok_or(OpEvalError::Overflow),
            Num::F(f) => float(-f),
        },
        Op::Eq => Ok(Value::Bool(same(&args[0], &args[1]))),
        Op::Ne => Ok(Value::Bool(!same(&args[0], &args[1]))),
        Op::Lt => compare(&args[0], &args[1], Ordering::is_lt),
        Op::Le => compare(&args[0], &args[1], Ordering::is_le),
        Op::Gt => compare(&args[0], &args[1], Ordering::is_gt),
        Op::Ge => compare(&args[0], &args[1], Ordering::is_ge),
        Op::And => Ok(Value::Bool(boolean(0, &args[0])? && boolean(1, &args[1])?)),
        Op::Or => Ok(Value::Bool(boolean(0, &args[0])? || boolean(1, &args[1])?)),
        Op::Not => Ok(Value::Bool(!boolean(0, &args[0])?)),
        Op::Concat => {
            let mut out = String::new();
            for (i, a) in args.iter().enumerate() {
                out.push_str(string(i, a)?);
            }
            Ok(Value::String(out))
        }
        Op::Str => Ok(Value::String(match &args[0] {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })),
        Op::Len => match &args[0] {
            Value::String(s) => length(s.chars().count()),
            Value::Array(items) => length(items.len()),
            other => Err(OpEvalError::Operand {
                index: 0,
                expected: Type::String,
                got: other.clone(),
            }),
        },
        Op::Round => {
            let x = as_f64(&num(0, &args[0])?);
            match args.get(1) {
                None => safe(x.round() as i64)
                    .map(Value::from)
                    .ok_or(OpEvalError::Overflow),
                Some(d) => {
                    let scale = 10f64.powi(int(1, d)? as i32);
                    float((x * scale).round() / scale)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integer_arithmetic_stays_integral() {
        assert_eq!(result_type(Op::Mul, &[Type::Int, Type::Int]), Ok(Type::Int));
        assert_eq!(apply(Op::Mul, &[json!(3), json!(3)]), Ok(json!(9)));
    }

    #[test]
    fn division_is_float_and_partial() {
        assert_eq!(
            result_type(Op::Div, &[Type::Int, Type::Int]),
            Ok(Type::Float)
        );
        assert_eq!(apply(Op::Div, &[json!(1), json!(4)]), Ok(json!(0.25)));
        assert_eq!(
            apply(Op::Div, &[json!(1), json!(0)]),
            Err(OpEvalError::DivisionByZero)
        );
    }

    #[test]
    fn overflow_is_an_error() {
        assert_eq!(
            apply(Op::Add, &[json!(i64::MAX), json!(1)]),
            Err(OpEvalError::Overflow)
        );
    }

    #[test]
    fn integers_are_bounded_where_every_host_carries_them() {
        let max = json!(MAX_SAFE_INT);
        assert_eq!(
            apply(Op::Add, &[json!(MAX_SAFE_INT - 1), json!(1)]),
            Ok(max.clone())
        );
        assert_eq!(
            apply(Op::Add, &[max.clone(), json!(1)]),
            Err(OpEvalError::Overflow)
        );
        assert_eq!(
            apply(Op::Sub, &[json!(-MAX_SAFE_INT), json!(1)]),
            Err(OpEvalError::Overflow)
        );
        assert_eq!(
            apply(Op::Mul, &[json!(1 << 27), json!(1 << 27)]),
            Err(OpEvalError::Overflow)
        );
        assert_eq!(apply(Op::Neg, &[max]), Ok(json!(-MAX_SAFE_INT)));
        assert_eq!(
            apply(Op::Round, &[json!(1e300)]),
            Err(OpEvalError::Overflow)
        );
    }

    #[test]
    fn arity_and_operand_errors_name_what_was_expected() {
        assert_eq!(
            result_type(Op::Neg, &[Type::Int, Type::Int]),
            Err(OpTypeError::Arity {
                expected: Arity::Exactly { n: 1 },
                got: 2
            })
        );
        assert_eq!(
            result_type(Op::Add, &[Type::Int, Type::String]),
            Err(OpTypeError::Operand {
                index: 1,
                expected: Type::Float,
                got: Type::String
            })
        );
    }

    #[test]
    fn round_to_digits() {
        assert_eq!(apply(Op::Round, &[json!(2.345), json!(1)]), Ok(json!(2.3)));
        assert_eq!(apply(Op::Round, &[json!(2.5)]), Ok(json!(3)));
    }

    #[test]
    fn numbers_compare_by_their_exact_values() {
        // 2^60 + 1 and 2^60 are the same float; as integers they differ.
        let (big, next) = (json!(1_i64 << 60), json!((1_i64 << 60) + 1));
        assert_eq!(apply(Op::Gt, &[next.clone(), big.clone()]), Ok(json!(true)));
        assert_eq!(
            apply(Op::Eq, &[next.clone(), big.clone()]),
            Ok(json!(false))
        );
        // An integer against a float: the integer isn't rounded first, and a fraction breaks a tie.
        assert_eq!(
            apply(Op::Gt, &[next, json!(1_152_921_504_606_846_976.0_f64)]),
            Ok(json!(true))
        );
        assert_eq!(apply(Op::Lt, &[json!(-2), json!(-2.5)]), Ok(json!(false)));
        assert_eq!(
            apply(Op::Lt, &[json!(i64::MAX), json!(1e19)]),
            Ok(json!(true))
        );
        assert_eq!(apply(Op::Eq, &[json!(3), json!(3.0)]), Ok(json!(true)));
        assert_eq!(apply(Op::Le, &[json!(0.5), json!(1)]), Ok(json!(true)));
    }
}
