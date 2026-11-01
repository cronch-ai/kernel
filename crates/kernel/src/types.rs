//! Structural typing: which types conform to which, and which values inhabit a type.

use crate::ir::Type;
use serde_json::Value;

/// `actual` is acceptable where `expected` is required.
///
/// * `Any` on either side conforms (gradual: unchecked shapes pass here and are reported elsewhere).
/// * `Int` conforms to `Float`.
/// * Lists are covariant; a record conforms when it has every expected field, each conforming (width subtyping).
pub fn conforms(actual: &Type, expected: &Type) -> bool {
    match (actual, expected) {
        (Type::Any, _) | (_, Type::Any) => true,
        (Type::Int, Type::Float) => true,
        (Type::List(a), Type::List(e)) => conforms(a, e),
        (Type::Record(a), Type::Record(e)) => e
            .iter()
            .all(|(name, et)| a.get(name).is_some_and(|at| conforms(at, et))),
        (a, e) => a == e,
    }
}

/// The value inhabits the type.
pub fn inhabits(value: &Value, ty: &Type) -> bool {
    match (ty, value) {
        (Type::Any, _) => true,
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
