//! A solver's `get-value` reply, read back as values of the types a query asked for.
//!
//! The reply is untrusted, so reading is bounded and fails closed: nesting is capped, and a solver's `let` sharing is
//! expanded within a budget of nodes, since a few lines of sharing can describe a value exponentially larger than its
//! text. A value read here is still only a proposal: the kernel re-checks the counterexample it is part of.

use super::encode::{case_fields, record_names, variant_names};
use crate::ir::Type;
use serde::Serialize;
use serde_json::{Number, Value};

const MAX_DEPTH: usize = 256;
const MAX_NODES: usize = 100_000;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Unreadable {
    /// Not S-expressions, or not the `((term value) ...)` shape of a `get-value` reply.
    Syntax { detail: String },
    /// A different number of values than were asked for.
    Count { expected: usize, got: usize },
    /// Not a value of the type asked for, as the export writes one.
    Value {
        #[serde(rename = "type")]
        ty: Type,
        got: String,
    },
    /// An infinity or NaN, which no Cronch value is.
    NotFinite,
    /// Deeper or larger than the reader follows.
    TooLarge,
}

#[derive(Clone, Debug, PartialEq)]
enum Sexp {
    /// A symbol (quoted or not: SMT-LIB reads `|x|` as `x`), numeral, decimal or bit-vector literal.
    Atom(String),
    Str(String),
    List(Vec<Sexp>),
}

impl std::fmt::Display for Sexp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Sexp::Atom(a) => f.write_str(a),
            Sexp::Str(s) => write!(f, "{s:?}"),
            Sexp::List(items) => {
                f.write_str("(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str(")")
            }
        }
    }
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

fn syntax(detail: &str) -> Unreadable {
    Unreadable::Syntax {
        detail: detail.to_string(),
    }
}

impl Parser<'_> {
    fn skip(&mut self) {
        while let Some(&c) = self.chars.peek() {
            if c == ';' {
                while self.chars.next().is_some_and(|c| c != '\n') {}
            } else if c.is_whitespace() {
                self.chars.next();
            } else {
                break;
            }
        }
    }

    fn sexp(&mut self, depth: usize) -> Result<Sexp, Unreadable> {
        if depth > MAX_DEPTH {
            return Err(Unreadable::TooLarge);
        }
        self.skip();
        match self.chars.next() {
            None => Err(syntax("unexpected end")),
            Some('(') => {
                let mut items = Vec::new();
                loop {
                    self.skip();
                    if self.chars.peek() == Some(&')') {
                        self.chars.next();
                        return Ok(Sexp::List(items));
                    }
                    items.push(self.sexp(depth + 1)?);
                }
            }
            Some(')') => Err(syntax("unexpected )")),
            Some('|') => {
                let mut text = String::new();
                loop {
                    match self.chars.next() {
                        None => return Err(syntax("unterminated symbol")),
                        Some('|') => return Ok(Sexp::Atom(text)),
                        Some(c) => text.push(c),
                    }
                }
            }
            Some('"') => {
                let mut raw = String::new();
                loop {
                    match self.chars.next() {
                        None => return Err(syntax("unterminated string")),
                        Some('"') if self.chars.peek() == Some(&'"') => {
                            self.chars.next();
                            raw.push('"');
                        }
                        Some('"') => return Ok(Sexp::Str(unescape(&raw)?)),
                        Some(c) => raw.push(c),
                    }
                }
            }
            Some(c) => {
                let mut text = String::from(c);
                while let Some(&c) = self.chars.peek() {
                    if c.is_whitespace() || c == '(' || c == ')' || c == '"' || c == '|' || c == ';'
                    {
                        break;
                    }
                    text.push(c);
                    self.chars.next();
                }
                Ok(Sexp::Atom(text))
            }
        }
    }
}

/// SMT-LIB's string escapes, `\u{d}` to `\u{ddddd}` and `\udddd`; any other backslash is itself.
fn unescape(raw: &str) -> Result<String, Unreadable> {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(i) = rest.find("\\u") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let (hex, len) = if let Some(braced) = after.strip_prefix('{') {
            match braced.find('}') {
                Some(end) if (1..=5).contains(&end) => (&braced[..end], end + 2),
                _ => ("", 0),
            }
        } else if after.len() >= 4 && after.is_char_boundary(4) {
            (&after[..4], 4)
        } else {
            ("", 0)
        };
        match u32::from_str_radix(hex, 16)
            .ok()
            .filter(|_| !hex.is_empty())
            .and_then(char::from_u32)
        {
            Some(c) if hex.chars().all(|h| h.is_ascii_hexdigit()) => {
                out.push(c);
                rest = &after[len..];
            }
            _ => {
                out.push_str("\\u");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn parse(text: &str) -> Result<Sexp, Unreadable> {
    let mut parser = Parser {
        chars: text.chars().peekable(),
    };
    let sexp = parser.sexp(0)?;
    parser.skip();
    if parser.chars.next().is_some() {
        return Err(syntax("trailing text"));
    }
    Ok(sexp)
}

/// `let` bindings in scope: each name's expression, with the scope it was bound in.
#[derive(Clone)]
struct Env<'s> {
    bindings: Vec<(&'s str, &'s Sexp, usize)>,
}

struct Reader<'s> {
    budget: usize,
    /// Every binding scope seen, so a binding is read in the scope it was made in.
    scopes: Vec<Env<'s>>,
}

/// A real, as a solver writes one where floats are read as reals (`1.5`, `(- 2.0)`, `(/ 1.0 3.0)`), as the
/// nearest float: a candidate, re-checked like any other.
fn real(s: &Sexp) -> Option<f64> {
    match s {
        Sexp::Atom(a) if !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit() || b == b'.') => {
            a.parse().ok()
        }
        Sexp::List(items) => match items.as_slice() {
            [Sexp::Atom(minus), x] if minus == "-" => real(x).map(|x| -x),
            [Sexp::Atom(slash), x, y] if slash == "/" => Some(real(x)? / real(y)?),
            _ => None,
        },
        _ => None,
    }
}

fn bits(literal: &str) -> Option<(u64, u32)> {
    if let Some(b) = literal.strip_prefix("#b") {
        let width = b.len() as u32;
        (width <= 64).then(|| u64::from_str_radix(b, 2).ok().map(|v| (v, width)))?
    } else if let Some(x) = literal.strip_prefix("#x") {
        let width = x.len() as u32 * 4;
        (width <= 64).then(|| u64::from_str_radix(x, 16).ok().map(|v| (v, width)))?
    } else {
        None
    }
}

impl<'s> Reader<'s> {
    fn charge(&mut self) -> Result<(), Unreadable> {
        self.budget = self.budget.checked_sub(1).ok_or(Unreadable::TooLarge)?;
        Ok(())
    }

    fn mismatch(ty: &Type, got: &Sexp) -> Unreadable {
        let mut text = got.to_string();
        text.truncate(200);
        Unreadable::Value {
            ty: ty.clone(),
            got: text,
        }
    }

    /// `s` in scope `scope`, read as a value of `ty`.
    fn value(
        &mut self,
        s: &'s Sexp,
        ty: &Type,
        scope: usize,
        depth: usize,
    ) -> Result<Value, Unreadable> {
        self.charge()?;
        if depth > MAX_DEPTH {
            return Err(Unreadable::TooLarge);
        }
        if let Sexp::Atom(name) = s {
            let bound = self.scopes[scope]
                .bindings
                .iter()
                .rev()
                .find(|(n, _, _)| n == name)
                .copied();
            if let Some((_, expr, at)) = bound {
                return self.value(expr, ty, at, depth + 1);
            }
        }
        if let Sexp::List(items) = s {
            match items.as_slice() {
                [Sexp::Atom(kw), inner, _] if kw == "as" => {
                    return self.value(inner, ty, scope, depth + 1);
                }
                [Sexp::Atom(kw), Sexp::List(binds), body] if kw == "let" => {
                    // Bindings are parallel: each expression is read in the enclosing scope.
                    let mut env = self.scopes[scope].clone();
                    for bind in binds {
                        match bind {
                            Sexp::List(pair) => match pair.as_slice() {
                                [Sexp::Atom(name), expr] => env.bindings.push((name, expr, scope)),
                                _ => return Err(syntax("malformed let binding")),
                            },
                            _ => return Err(syntax("malformed let binding")),
                        }
                    }
                    self.scopes.push(env);
                    let inner = self.scopes.len() - 1;
                    return self.value(body, ty, inner, depth + 1);
                }
                _ => {}
            }
        }
        let wrong = || Self::mismatch(ty, s);
        match ty {
            Type::Any | Type::Json => Err(wrong()),
            Type::Bool => match s {
                Sexp::Atom(a) if a == "true" => Ok(Value::Bool(true)),
                Sexp::Atom(a) if a == "false" => Ok(Value::Bool(false)),
                _ => Err(wrong()),
            },
            Type::Int => {
                let (negative, digits) = match s {
                    Sexp::Atom(a) => (false, a),
                    Sexp::List(items) => match items.as_slice() {
                        [Sexp::Atom(minus), Sexp::Atom(a)] if minus == "-" => (true, a),
                        _ => return Err(wrong()),
                    },
                    Sexp::Str(_) => return Err(wrong()),
                };
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(wrong());
                }
                let n: i128 = digits.parse().map_err(|_| Unreadable::TooLarge)?;
                let n = if negative { -n } else { n };
                i64::try_from(n)
                    .map(Value::from)
                    .map_err(|_| Unreadable::TooLarge)
            }
            Type::Float => {
                if let Some(real) = real(s) {
                    return if real.is_finite() {
                        Number::from_f64(real)
                            .map(Value::Number)
                            .ok_or(Unreadable::NotFinite)
                    } else {
                        Err(Unreadable::NotFinite)
                    };
                }
                let float = match s {
                    Sexp::List(items) => match items.as_slice() {
                        [
                            Sexp::Atom(fp),
                            Sexp::Atom(sign),
                            Sexp::Atom(exp),
                            Sexp::Atom(sig),
                        ] if fp == "fp" => match (bits(sign), bits(exp), bits(sig)) {
                            (Some((s, 1)), Some((e, 11)), Some((m, 52))) => {
                                f64::from_bits(s << 63 | e << 52 | m)
                            }
                            _ => return Err(wrong()),
                        },
                        [
                            Sexp::Atom(u),
                            Sexp::Atom(special),
                            Sexp::Atom(eb),
                            Sexp::Atom(sb),
                        ] if u == "_" && eb == "11" && sb == "53" => match special.as_str() {
                            "+zero" => 0.0,
                            "-zero" => -0.0,
                            "+oo" | "-oo" | "NaN" => return Err(Unreadable::NotFinite),
                            _ => return Err(wrong()),
                        },
                        _ => return Err(wrong()),
                    },
                    _ => return Err(wrong()),
                };
                if !float.is_finite() {
                    return Err(Unreadable::NotFinite);
                }
                Number::from_f64(float)
                    .map(Value::Number)
                    .ok_or(Unreadable::NotFinite)
            }
            Type::String => match s {
                Sexp::Str(text) => Ok(Value::String(text.clone())),
                _ => Err(wrong()),
            },
            Type::List(element) => match s {
                Sexp::Atom(a) if a == "seq.empty" => Ok(Value::Array(Vec::new())),
                Sexp::List(items) => match items.split_first() {
                    Some((Sexp::Atom(unit), [item])) if unit == "seq.unit" => Ok(Value::Array(
                        vec![self.value(item, element, scope, depth + 1)?],
                    )),
                    Some((Sexp::Atom(concat), parts)) if concat == "seq.++" => {
                        let mut out = Vec::new();
                        for part in parts {
                            match self.value(part, ty, scope, depth + 1)? {
                                Value::Array(items) => out.extend(items),
                                _ => return Err(wrong()),
                            }
                        }
                        Ok(Value::Array(out))
                    }
                    _ => Err(wrong()),
                },
                Sexp::Str(_) | Sexp::Atom(_) => Err(wrong()),
            },
            Type::Record(fields) => {
                let name = record_names(fields);
                let (head, args): (&Sexp, &[Sexp]) = match s {
                    Sexp::List(items) if !items.is_empty() => (&items[0], &items[1..]),
                    atom => (atom, &[]),
                };
                if *head != Sexp::Atom(name) || args.len() != fields.len() {
                    return Err(wrong());
                }
                let mut out = serde_json::Map::new();
                for ((field, t), arg) in fields.iter().zip(args) {
                    out.insert(field.clone(), self.value(arg, t, scope, depth + 1)?);
                }
                Ok(Value::Object(out))
            }
            Type::Variant { tag, cases } => {
                let name = variant_names(tag, cases);
                let (head, args): (&Sexp, &[Sexp]) = match s {
                    Sexp::List(items) if !items.is_empty() => (&items[0], &items[1..]),
                    atom => (atom, &[]),
                };
                let Sexp::Atom(head) = head else {
                    return Err(wrong());
                };
                let case = head.strip_prefix(&format!("{name}#")).ok_or_else(wrong)?;
                let case_ty = cases.get(case).ok_or_else(wrong)?;
                let fields = case_fields(case_ty).map_err(|_| wrong())?;
                if args.len() != fields.len() {
                    return Err(wrong());
                }
                let mut out = serde_json::Map::new();
                for ((field, t), arg) in fields.iter().zip(args) {
                    let v = self.value(arg, t, scope, depth + 1)?;
                    if field.is_empty() {
                        let Value::Object(inner) = v else {
                            return Err(wrong());
                        };
                        out.extend(inner);
                    } else {
                        out.insert(field.clone(), v);
                    }
                }
                out.insert(tag.clone(), Value::String(case.to_string()));
                Ok(Value::Object(out))
            }
        }
    }
}

/// The values of a `get-value` reply, `((term value) ...)`, read in order at `types`.
pub fn read(reply: &str, types: &[&Type]) -> Result<Vec<Value>, Unreadable> {
    let Sexp::List(pairs) = parse(reply)? else {
        return Err(syntax("expected a list of (term value) pairs"));
    };
    if pairs.len() != types.len() {
        return Err(Unreadable::Count {
            expected: types.len(),
            got: pairs.len(),
        });
    }
    let mut reader = Reader {
        budget: MAX_NODES,
        scopes: vec![Env {
            bindings: Vec::new(),
        }],
    };
    pairs
        .iter()
        .zip(types)
        .map(|(pair, ty)| match pair {
            Sexp::List(kv) if kv.len() == 2 => reader.value(&kv[1], ty, 0, 0),
            _ => Err(syntax("expected a (term value) pair")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn one(reply: &str, ty: Type) -> Result<Value, Unreadable> {
        read(&format!("((x {reply}))"), &[&ty]).map(|mut v| v.remove(0))
    }

    #[test]
    fn values_read_as_the_export_writes_them() {
        assert_eq!(one("(- 7)", Type::Int), Ok(json!(-7)));
        assert_eq!(one("true", Type::Bool), Ok(json!(true)));
        assert_eq!(
            one("\"a\"\"b\\u{e9}\\u{5c}\"", Type::String),
            Ok(json!("a\"bé\\"))
        );
        assert_eq!(
            one("(fp #b0 #b01111111111 #x8000000000000)", Type::Float),
            Ok(json!(1.5))
        );
        assert_eq!(
            one(
                "(fp #b1 #b10000000000 #b0000000000000000000000000000000000000000000000000000)",
                Type::Float
            ),
            Ok(json!(-2.0))
        );
        assert_eq!(one("(_ -zero 11 53)", Type::Float), Ok(json!(-0.0)));
        assert_eq!(
            one("(_ NaN 11 53)", Type::Float),
            Err(Unreadable::NotFinite)
        );
        assert_eq!(one("(/ (- 3.0) 4.0)", Type::Float), Ok(json!(-0.75)));
        assert_eq!(one("2", Type::Float), Ok(json!(2.0)));
        let ints = Type::List(Box::new(Type::Int));
        assert_eq!(
            one("(seq.++ (seq.unit 1) (seq.unit 2))", ints.clone()),
            Ok(json!([1, 2]))
        );
        assert_eq!(one("(as seq.empty (Seq Int))", ints), Ok(json!([])));
        let fields = BTreeMap::from([("a".to_string(), Type::Int), ("b".to_string(), Type::Bool)]);
        let name = record_names(&fields);
        assert_eq!(
            one(&format!("(|{name}| 3 false)"), Type::Record(fields)),
            Ok(json!({ "a": 3, "b": false }))
        );
    }

    #[test]
    fn sharing_is_expanded_within_a_budget() {
        let ints = Type::List(Box::new(Type::Int));
        assert_eq!(
            one("(let ((a!1 (seq.unit 4))) (seq.++ a!1 a!1))", ints.clone()),
            Ok(json!([4, 4]))
        );
        // Bindings are parallel: `b` sees the outer `a`.
        assert_eq!(
            one("(let ((a 1)) (let ((a 2) (b a)) b))", Type::Int),
            Ok(json!(1))
        );
        let mut text = String::from("(let ((x0 (seq.unit 0))) ");
        for i in 1..=40 {
            text += &format!("(let ((x{i} (seq.++ x{} x{}))) ", i - 1, i - 1);
        }
        text += "x40";
        text += &")".repeat(41);
        assert_eq!(one(&text, ints), Err(Unreadable::TooLarge));
    }

    #[test]
    fn what_is_not_a_value_of_the_type_is_refused() {
        assert!(matches!(
            one("\"3\"", Type::Int),
            Err(Unreadable::Value { .. })
        ));
        assert!(matches!(
            one("99999999999999999999", Type::Int),
            Err(Unreadable::TooLarge)
        ));
        assert!(matches!(
            read("((x 1) (y 2))", &[&Type::Int]),
            Err(Unreadable::Count {
                expected: 1,
                got: 2
            })
        ));
        assert!(matches!(
            read("((x 1)", &[&Type::Int]),
            Err(Unreadable::Syntax { .. })
        ));
        assert!(matches!(one("1", Type::Any), Err(Unreadable::Value { .. })));
    }
}
