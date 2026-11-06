//! Structural typing: which types conform to which, and which values inhabit a type.

use crate::ir::Type;
use serde_json::Value;
use std::collections::BTreeMap;

/// `actual` is acceptable where `expected` is required.
///
/// * `Int` conforms to `Float`.
/// * Lists are covariant; a record conforms when it has every expected field, each conforming (width subtyping).
/// * A variant conforms to one with more cases, and to the record of what every case has.
pub fn conforms(actual: &Type, expected: &Type) -> bool {
    match (actual, expected) {
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

/// What every value of a variant has, as a record: its tag, and each field every case has at a type they all conform
/// to. A field whose cases have no such type is read only after `match`.
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
                        .filter_map(|(n, t)| {
                            let other = fields.get(&n)?;
                            Some((n.clone(), join(&t, other)?))
                        })
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

/// The least type both conform to, if there is one.
///
/// A record keeps the fields both have, each at their join, and drops a common field whose types have none: the
/// result is still a supertype of both under width subtyping. Two unrelated scalars have no join.
pub fn join(a: &Type, b: &Type) -> Option<Type> {
    match (a, b) {
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

/// The value inhabits the type and has no field the type doesn't declare, at any depth. This is how a value from the
/// world is checked on arrival: what doesn't fit doesn't cross, and an undeclared field doesn't fit.
///
/// `inhabits` reads only the declared fields, as width subtyping does for the values a harness makes itself. A
/// variant's value holds its tag beside its case's fields, so the tags of every variant enclosing a record are
/// allowed at that record's level.
pub fn inhabits_exactly(value: &Value, ty: &Type) -> bool {
    exactly(value, ty, &[])
}

fn exactly(value: &Value, ty: &Type, tags: &[&str]) -> bool {
    match (ty, value) {
        (Type::List(t), Value::Array(items)) => items.iter().all(|i| exactly(i, t, &[])),
        (Type::Record(fields), Value::Object(map)) => {
            map.keys()
                .all(|k| fields.contains_key(k) || tags.contains(&k.as_str()))
                && fields
                    .iter()
                    .all(|(name, t)| map.get(name).is_some_and(|v| exactly(v, t, &[])))
        }
        (Type::Variant { tag, cases }, Value::Object(map)) => {
            let mut enclosing = tags.to_vec();
            enclosing.push(tag);
            map.get(tag)
                .and_then(Value::as_str)
                .and_then(|case| cases.get(case))
                .is_some_and(|t| exactly(value, t, &enclosing))
        }
        _ => inhabits(value, ty),
    }
}

/// The most precise type of a value, used for literals. `null` has none, and nor does an empty list: it takes the type
/// expected where it appears. A list has the join of its items' types, and none if an item doesn't inhabit that.
pub fn of_value(value: &Value) -> Option<Type> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(_) => Type::Bool,
        Value::Number(n) if n.is_i64() || n.is_u64() => Type::Int,
        Value::Number(_) => Type::Float,
        Value::String(_) => Type::String,
        Value::Array(items) => {
            let mut element = of_value(items.first()?)?;
            for t in items.iter().filter_map(of_value) {
                element = join(&element, &t)?;
            }
            if !items.iter().all(|i| inhabits(i, &element)) {
                return None;
            }
            Type::List(Box::new(element))
        }
        Value::Object(map) => Type::Record(
            map.iter()
                .map(|(k, v)| Some((k.clone(), of_value(v)?)))
                .collect::<Option<_>>()?,
        ),
    })
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
        // What every case has: the tag. `value` isn't a field of the view: Int and String have no join.
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
