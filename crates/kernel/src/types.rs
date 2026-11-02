//! Structural typing: which types conform to which, and which values inhabit a type.

use crate::ir::Type;
use serde_json::Value;
use std::collections::BTreeMap;

/// `actual` is acceptable where `expected` is required.
///
/// * `Any` on either side conforms (gradual: unchecked shapes pass here and are reported elsewhere).
/// * `Int` conforms to `Float`.
/// * Lists are covariant; a record conforms when it has every expected field, each conforming (width subtyping).
/// * Every type conforms to `Json`, and `Json` to nothing else: every value is one, but what one holds is known only
///   once it is tested with `is`.
pub fn conforms(actual: &Type, expected: &Type) -> bool {
    match (actual, expected) {
        (Type::Any, _) | (_, Type::Any) => true,
        (_, Type::Json) => true,
        (Type::Int, Type::Float) => true,
        (Type::List(a), Type::List(e)) => conforms(a, e),
        (Type::Record(a), Type::Record(e)) => e
            .iter()
            .all(|(name, et)| a.get(name).is_some_and(|at| conforms(at, et))),
        // Fewer cases is narrower.
        (Type::Variant { tag: ta, cases: ca }, Type::Variant { tag: te, cases: ce }) => {
            ta == te
                && ca
                    .iter()
                    .all(|(name, at)| ce.get(name).is_some_and(|et| conforms(at, et)))
        }
        // A variant is a record with its tag and what every case has.
        (Type::Variant { .. }, Type::Record(_)) => conforms(&record_view(actual), expected),
        (a, e) => a == e,
    }
}

/// What every value of a variant has, as a record: its tag, and each field every case has, at their join.
pub fn record_view(ty: &Type) -> Type {
    match ty {
        Type::Variant { tag, cases } => {
            let mut common: Option<BTreeMap<String, Type>> = None;
            for case in cases.values() {
                let Type::Record(fields) = record_view(case) else {
                    return Type::Record(BTreeMap::from([(tag.clone(), Type::String)]));
                };
                common = Some(match common {
                    None => fields,
                    Some(c) => c
                        .into_iter()
                        .filter_map(|(n, t)| Some((n.clone(), join(&t, fields.get(&n)?)?)))
                        .collect(),
                });
            }
            let mut fields = common.unwrap_or_default();
            fields.insert(tag.clone(), Type::String);
            Type::Record(fields)
        }
        other => other.clone(),
    }
}

/// The least type both conform to, if there is one other than giving up on checking.
///
/// A record keeps the fields both have, each at their join, and drops a common field whose types have none: the
/// result is still a supertype of both under width subtyping. `Any` absorbs everything; two unrelated scalars have
/// no join.
pub fn join(a: &Type, b: &Type) -> Option<Type> {
    match (a, b) {
        (Type::Any, _) | (_, Type::Any) => Some(Type::Any),
        _ if conforms(a, b) && conforms(b, a) => Some(a.clone()),
        _ if conforms(a, b) => Some(b.clone()),
        _ if conforms(b, a) => Some(a.clone()),
        (Type::List(x), Type::List(y)) => join(x, y).map(|t| Type::List(Box::new(t))),
        (Type::Variant { tag: tx, cases: cx }, Type::Variant { tag: ty, cases: cy })
            if tx == ty =>
        {
            let mut cases = cx.clone();
            for (name, t) in cy {
                let joined = match cases.get(name) {
                    Some(existing) => join(existing, t)?,
                    None => t.clone(),
                };
                cases.insert(name.clone(), joined);
            }
            Some(Type::Variant {
                tag: tx.clone(),
                cases,
            })
        }
        (Type::Variant { .. }, _) | (_, Type::Variant { .. }) => {
            join(&record_view(a), &record_view(b))
        }
        (Type::Record(x), Type::Record(y)) => Some(Type::Record(
            x.iter()
                .filter_map(|(name, t)| Some((name.clone(), join(t, y.get(name)?)?)))
                .collect(),
        )),
        _ => None,
    }
}

/// The value inhabits the type.
pub fn inhabits(value: &Value, ty: &Type) -> bool {
    match (ty, value) {
        (Type::Any | Type::Json, _) => true,
        (Type::Int, Value::Number(n)) => {
            n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.0)
        }
        (Type::Float, Value::Number(_)) => true,
        (Type::String, Value::String(_)) => true,
        (Type::Bool, Value::Bool(_)) => true,
        (Type::List(t), Value::Array(items)) => items.iter().all(|i| inhabits(i, t)),
        (Type::Record(fields), Value::Object(map)) => fields
            .iter()
            .all(|(name, t)| map.get(name).is_some_and(|v| inhabits(v, t))),
        (Type::Variant { tag, cases }, Value::Object(map)) => map
            .get(tag)
            .and_then(Value::as_str)
            .and_then(|case| cases.get(case))
            .is_some_and(|t| inhabits(value, t)),
        _ => false,
    }
}

/// The most precise type of a value, used for literals. Empty lists are `List(Any)`.
pub fn of_value(value: &Value) -> Type {
    match value {
        Value::Null => Type::Any,
        Value::Bool(_) => Type::Bool,
        Value::Number(n) if n.is_i64() || n.is_u64() => Type::Int,
        Value::Number(_) => Type::Float,
        Value::String(_) => Type::String,
        Value::Array(items) => {
            Type::List(Box::new(items.first().map(of_value).unwrap_or(Type::Any)))
        }
        Value::Object(map) => {
            Type::Record(map.iter().map(|(k, v)| (k.clone(), of_value(v))).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn record(fields: &[(&str, Type)]) -> Type {
        Type::Record(
            fields
                .iter()
                .map(|(k, t)| (k.to_string(), t.clone()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn int_conforms_to_float_not_back() {
        assert!(conforms(&Type::Int, &Type::Float));
        assert!(!conforms(&Type::Float, &Type::Int));
    }

    #[test]
    fn records_allow_extra_fields() {
        let wide = record(&[("value", Type::Float), ("year", Type::Int)]);
        let narrow = record(&[("value", Type::Float)]);
        assert!(conforms(&wide, &narrow));
        assert!(!conforms(&narrow, &wide));
    }

    #[test]
    fn joins_keep_what_both_have() {
        let rec = |fields: &[(&str, Type)]| {
            Type::Record(
                fields
                    .iter()
                    .map(|(n, t)| (n.to_string(), t.clone()))
                    .collect(),
            )
        };
        let call = rec(&[("kind", Type::String), ("op", Type::String)]);
        let halt = rec(&[("kind", Type::String), ("value", Type::Int)]);
        assert_eq!(join(&call, &halt), Some(rec(&[("kind", Type::String)])));
        assert_eq!(join(&Type::Int, &Type::Float), Some(Type::Float));
        assert_eq!(join(&Type::Int, &Type::String), None);
        let mixed = rec(&[("kind", Type::Int)]);
        assert_eq!(join(&call, &mixed), Some(rec(&[])));
    }

    #[test]
    fn variants_are_read_off_their_tag() {
        let rec = |fields: &[(&str, Type)]| {
            Type::Record(
                fields
                    .iter()
                    .map(|(n, t)| (n.to_string(), t.clone()))
                    .collect(),
            )
        };
        let input = Type::Variant {
            tag: "kind".into(),
            cases: BTreeMap::from([
                ("start".into(), rec(&[("value", Type::Int)])),
                (
                    "msg".into(),
                    rec(&[("value", Type::String), ("channel", Type::String)]),
                ),
            ]),
        };
        assert!(inhabits(&json!({ "kind": "start", "value": 3 }), &input));
        assert!(!inhabits(&json!({ "kind": "start", "value": "3" }), &input));
        assert!(!inhabits(&json!({ "kind": "stop" }), &input));
        // What every case has: the tag, and `value` at the join of Int and String, which there isn't.
        assert_eq!(record_view(&input), rec(&[("kind", Type::String)]));
        assert!(conforms(&input, &rec(&[("kind", Type::String)])));
        let fewer = Type::Variant {
            tag: "kind".into(),
            cases: BTreeMap::from([("start".into(), rec(&[("value", Type::Int)]))]),
        };
        assert!(conforms(&fewer, &input));
        assert!(!conforms(&input, &fewer));
        assert_eq!(join(&fewer, &input), Some(input.clone()));
    }

    #[test]
    fn values_inhabit_types() {
        assert!(inhabits(&json!(3), &Type::Float));
        assert!(inhabits(&json!(3.0), &Type::Int));
        assert!(!inhabits(&json!(3.5), &Type::Int));
        assert!(inhabits(
            &json!({"value": 1.5, "year": 2023}),
            &record(&[("value", Type::Float)])
        ));
        assert!(!inhabits(
            &json!({"year": 2023}),
            &record(&[("value", Type::Float)])
        ));
    }
}
