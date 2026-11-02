//! Labelled evaluation: how a harness's expressions run, with every value carrying the untrusted sources it may
//! depend on.
//!
//! A label is a set of source names; the empty set is trusted. Every node of a value has one: a container's label
//! covers its shape (which fields or how many items it has), so reading a field or an item joins the container's
//! label into what is read. Operators join the labels of everything they read. Control flow contributes too: the
//! result of an `if` carries its condition's label, and a `fold` carries its list's, since which branch ran or how
//! many steps ran depends on them. That is the standard program-counter rule, and it rules out implicit flows.
//!
//! Every variable has a value, so an expression either gives a value or a defined error, and `fold` is the only
//! iteration, so evaluation always ends.

use crate::check::Path;
use crate::ir::Expr;
use crate::ops::{self, Op, OpEvalError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The untrusted sources a value may depend on. Empty: trusted.
pub type Label = BTreeSet<String>;

/// A value with a label on every node.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum LValue {
    Scalar {
        #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
        label: Label,
        value: Value,
    },
    List {
        #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
        label: Label,
        items: Vec<LValue>,
    },
    Record {
        #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
        label: Label,
        fields: BTreeMap<String, LValue>,
    },
}

impl LValue {
    /// A value whose every node has this label.
    pub fn new(value: &Value, label: &Label) -> LValue {
        match value {
            Value::Array(items) => LValue::List {
                label: label.clone(),
                items: items.iter().map(|v| LValue::new(v, label)).collect(),
            },
            Value::Object(map) => LValue::Record {
                label: label.clone(),
                fields: map
                    .iter()
                    .map(|(k, v)| (k.clone(), LValue::new(v, label)))
                    .collect(),
            },
            v => LValue::Scalar {
                label: label.clone(),
                value: v.clone(),
            },
        }
    }

    pub fn trusted(value: &Value) -> LValue {
        LValue::new(value, &Label::new())
    }

    pub fn value(&self) -> Value {
        match self {
            LValue::Scalar { value, .. } => value.clone(),
            LValue::List { items, .. } => Value::Array(items.iter().map(LValue::value).collect()),
            LValue::Record { fields, .. } => Value::Object(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.value()))
                    .collect::<Map<_, _>>(),
            ),
        }
    }

    /// The label of this node alone.
    pub fn top(&self) -> &Label {
        match self {
            LValue::Scalar { label, .. }
            | LValue::List { label, .. }
            | LValue::Record { label, .. } => label,
        }
    }

    fn top_mut(&mut self) -> &mut Label {
        match self {
            LValue::Scalar { label, .. }
            | LValue::List { label, .. }
            | LValue::Record { label, .. } => label,
        }
    }

    /// The join of every node's label: what reading all of the value depends on.
    pub fn label(&self) -> Label {
        let mut out = self.top().clone();
        match self {
            LValue::Scalar { .. } => {}
            LValue::List { items, .. } => items.iter().for_each(|i| out.extend(i.label())),
            LValue::Record { fields, .. } => fields.values().for_each(|f| out.extend(f.label())),
        }
        out
    }

    /// The same value, its top node raised by `by`. Everything below is reached through the top, so it is raised too.
    pub fn raised(mut self, by: &Label) -> LValue {
        self.top_mut().extend(by.iter().cloned());
        self
    }

    /// The labels as a value of the same shape: each scalar replaced by the sources it depends on, including those of
    /// every container above it. What properties read to say which data reached where.
    pub fn labels(&self) -> Value {
        self.labels_under(&Label::new())
    }

    fn labels_under(&self, above: &Label) -> Value {
        let mut here = above.clone();
        here.extend(self.top().iter().cloned());
        match self {
            LValue::Scalar { .. } => Value::Array(here.into_iter().map(Value::String).collect()),
            LValue::List { items, .. } => {
                Value::Array(items.iter().map(|i| i.labels_under(&here)).collect())
            }
            LValue::Record { fields, .. } => Value::Object(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.labels_under(&here)))
                    .collect::<Map<_, _>>(),
            ),
        }
    }

    /// A field, carrying the record's label; `None` if this isn't a record with that field.
    pub fn field(&self, name: &str) -> Option<LValue> {
        match self {
            LValue::Record { label, fields } => fields.get(name).map(|f| f.clone().raised(label)),
            _ => None,
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EvalError {
    Operator {
        at: Path,
        op: String,
        error: Box<OpEvalError>,
    },
    UnknownVariable {
        at: Path,
        name: String,
    },
    UnknownOperator {
        at: Path,
        op: String,
    },
    NotABool {
        at: Path,
        got: Value,
    },
    NoSuchField {
        at: Path,
        field: String,
        of: Value,
    },
    NotAList {
        at: Path,
        got: Value,
    },
    /// A `match` on a value whose tag no case and no `otherwise` covers.
    NoCase {
        at: Path,
        got: Value,
    },
}

fn child(at: &str, segment: &str) -> Path {
    format!("{at}/{segment}")
}

/// Evaluate an expression with these variables bound, innermost last.
pub fn eval(expr: &Expr, scope: Vec<(String, LValue)>) -> Result<LValue, EvalError> {
    Evaluator { scope }.eval(expr, "", &Label::new())
}

struct Evaluator {
    scope: Vec<(String, LValue)>,
}

impl Evaluator {
    fn bind<T>(&mut self, names: Vec<(String, LValue)>, f: impl FnOnce(&mut Self) -> T) -> T {
        let n = names.len();
        self.scope.extend(names);
        let out = f(self);
        self.scope.truncate(self.scope.len() - n);
        out
    }

    /// `pc` is the label of the control flow that reached this expression; what it builds carries it.
    fn eval(&mut self, expr: &Expr, at: &str, pc: &Label) -> Result<LValue, EvalError> {
        match expr {
            Expr::Lit { v } => Ok(LValue::new(v, pc)),
            Expr::Var { name } => self
                .scope
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| EvalError::UnknownVariable {
                    at: at.to_string(),
                    name: name.clone(),
                }),
            Expr::Op { op, args } => {
                let parsed = Op::parse(op).ok_or_else(|| EvalError::UnknownOperator {
                    at: at.to_string(),
                    op: op.clone(),
                })?;
                let operands = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| self.eval(a, &child(at, &format!("args/{i}")), pc))
                    .collect::<Result<Vec<_>, _>>()?;
                // Appending keeps each item's own label, so a history of values from different sources stays precise.
                if parsed == Op::Append
                    && let [LValue::List { label, items }, item] = operands.as_slice()
                {
                    let mut items = items.clone();
                    items.push(item.clone());
                    let mut label = label.clone();
                    label.extend(pc.iter().cloned());
                    return Ok(LValue::List { label, items });
                }
                let mut label = pc.clone();
                operands.iter().for_each(|o| label.extend(o.label()));
                let values: Vec<Value> = operands.iter().map(LValue::value).collect();
                ops::apply(parsed, &values)
                    .map(|v| LValue::new(&v, &label))
                    .map_err(|error| EvalError::Operator {
                        at: at.to_string(),
                        op: op.clone(),
                        error: Box::new(error),
                    })
            }
            Expr::Label { of } => {
                let of = self.eval(of, &child(at, "of"), pc)?;
                let sources = of.label().into_iter().map(Value::String).collect();
                Ok(LValue::new(&Value::Array(sources), pc))
            }
            // Annotations are for checking; evaluation never needs them.
            Expr::Let { name, be, body, .. } => {
                let bound = self.eval(be, &child(at, "be"), pc)?;
                self.bind(vec![(name.clone(), bound)], |e| {
                    e.eval(body, &child(at, "in"), pc)
                })
            }
            Expr::If {
                cond,
                then,
                otherwise,
            } => {
                let cond_at = child(at, "cond");
                let c = self.eval(cond, &cond_at, pc)?;
                let mut inner = pc.clone();
                inner.extend(c.label());
                let taken = match c.value() {
                    Value::Bool(true) => self.eval(then, &child(at, "then"), &inner)?,
                    Value::Bool(false) => self.eval(otherwise, &child(at, "else"), &inner)?,
                    got => return Err(EvalError::NotABool { at: cond_at, got }),
                };
                Ok(taken.raised(&inner))
            }
            Expr::Record { fields } => {
                let fields = fields
                    .iter()
                    .map(|(n, e)| {
                        self.eval(e, &child(at, &format!("fields/{n}")), pc)
                            .map(|v| (n.clone(), v))
                    })
                    .collect::<Result<_, _>>()?;
                Ok(LValue::Record {
                    label: pc.clone(),
                    fields,
                })
            }
            Expr::List { items } => {
                let items = items
                    .iter()
                    .enumerate()
                    .map(|(i, e)| self.eval(e, &child(at, &format!("items/{i}")), pc))
                    .collect::<Result<_, _>>()?;
                Ok(LValue::List {
                    label: pc.clone(),
                    items,
                })
            }
            Expr::Get { of, field } => {
                let v = self.eval(of, &child(at, "of"), pc)?;
                v.field(field).ok_or_else(|| EvalError::NoSuchField {
                    at: at.to_string(),
                    field: field.clone(),
                    of: v.value(),
                })
            }
            Expr::Match {
                of,
                on,
                cases,
                otherwise,
            } => {
                let v = self.eval(of, &child(at, "of"), pc)?;
                // Which case runs depends on the tag, and on the shape that holds it.
                let tag = v.field(on);
                let mut inner = pc.clone();
                inner.extend(v.top().iter().cloned());
                if let Some(t) = &tag {
                    inner.extend(t.label());
                }
                let name = tag
                    .as_ref()
                    .and_then(|t| t.value().as_str().map(str::to_string));
                let taken = match name.as_ref().and_then(|n| cases.get(n).map(|c| (n, c))) {
                    Some((n, case)) => {
                        let body_at = child(at, &format!("cases/{n}/body"));
                        self.bind(vec![(case.bind.clone(), v)], |e| {
                            e.eval(&case.body, &body_at, &inner)
                        })?
                    }
                    None => match otherwise {
                        Some(o) => self.eval(o, &child(at, "otherwise"), &inner)?,
                        None => {
                            return Err(EvalError::NoCase {
                                at: at.to_string(),
                                got: v.value(),
                            });
                        }
                    },
                };
                Ok(taken.raised(&inner))
            }
            Expr::Fold {
                over,
                init,
                acc,
                item,
                body,
                ..
            } => {
                let over_at = child(at, "over");
                let (outer, items) = match self.eval(over, &over_at, pc)? {
                    LValue::List { label, items } => (label, items),
                    other => {
                        return Err(EvalError::NotAList {
                            at: over_at,
                            got: other.value(),
                        });
                    }
                };
                // How many steps run depends on the list's length, so every step runs under its label.
                let mut inner = pc.clone();
                inner.extend(outer.iter().cloned());
                let mut value = self.eval(init, &child(at, "init"), pc)?;
                let body_at = child(at, "body");
                for x in items {
                    let names = vec![(acc.clone(), value), (item.clone(), x.raised(&outer))];
                    value = self.bind(names, |e| e.eval(body, &body_at, &inner))?;
                }
                Ok(value.raised(&inner))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn expr(v: Value) -> Expr {
        serde_json::from_value(v).unwrap()
    }

    fn web() -> Label {
        Label::from(["web".to_string()])
    }

    fn run(e: Value, scope: Vec<(&str, LValue)>) -> LValue {
        let scope = scope.into_iter().map(|(n, v)| (n.to_string(), v)).collect();
        eval(&expr(e), scope).unwrap()
    }

    #[test]
    fn operators_join_what_they_read() {
        let page = LValue::new(&json!("<html>"), &web());
        let out = run(
            json!({ "e": "op", "op": "concat", "args": [{ "e": "lit", "v": "fetched: " }, { "e": "var", "name": "page" }] }),
            vec![("page", page)],
        );
        assert_eq!(out.value(), json!("fetched: <html>"));
        assert_eq!(out.label(), web());
    }

    #[test]
    fn fields_keep_their_own_labels() {
        let state = LValue::Record {
            label: Label::new(),
            fields: BTreeMap::from([
                ("plan".to_string(), LValue::trusted(&json!("send"))),
                ("page".to_string(), LValue::new(&json!("x"), &web())),
            ]),
        };
        let plan = run(
            json!({ "e": "get", "of": { "e": "var", "name": "s" }, "field": "plan" }),
            vec![("s", state)],
        );
        assert!(plan.label().is_empty());
    }

    #[test]
    fn branches_carry_their_conditions_label() {
        // An implicit flow: no untrusted value is copied, but which constant comes out depends on one.
        let secret = LValue::new(&json!(true), &web());
        let out = run(
            json!({ "e": "if", "cond": { "e": "var", "name": "c" }, "then": { "e": "lit", "v": 1 }, "else": { "e": "lit", "v": 2 } }),
            vec![("c", secret)],
        );
        assert_eq!(out.value(), json!(1));
        assert_eq!(out.label(), web());
    }

    #[test]
    fn appending_keeps_items_apart_and_folds_carry_their_lists_label() {
        let history = run(
            json!({ "e": "op", "op": "append", "args": [{ "e": "list", "items": [{ "e": "lit", "v": "task" }] }, { "e": "var", "name": "page" }] }),
            vec![("page", LValue::new(&json!("page"), &web()))],
        );
        let LValue::List { items, .. } = &history else {
            panic!()
        };
        assert!(items[0].label().is_empty());
        assert_eq!(items[1].label(), web());

        let untrusted_length = LValue::List {
            label: web(),
            items: vec![LValue::trusted(&json!(1))],
        };
        let count = run(
            json!({ "e": "fold", "over": { "e": "var", "name": "xs" }, "init": { "e": "lit", "v": 0 }, "acc": "n", "item": "x",
                     "body": { "e": "op", "op": "add", "args": [{ "e": "var", "name": "n" }, { "e": "lit", "v": 1 }] } }),
            vec![("xs", untrusted_length)],
        );
        assert_eq!(count.value(), json!(1));
        assert_eq!(count.label(), web());
    }

    #[test]
    fn a_match_carries_its_tags_label() {
        // Only the tag is untrusted: which case runs depends on it, so the constant chosen carries its label.
        let v = LValue::Record {
            label: Label::new(),
            fields: BTreeMap::from([
                ("kind".to_string(), LValue::new(&json!("a"), &web())),
                ("n".to_string(), LValue::trusted(&json!(1))),
            ]),
        };
        let m = json!({ "e": "match", "of": { "e": "var", "name": "v" }, "cases": {
            "a": { "bind": "x", "body": { "e": "lit", "v": "chose a" } },
            "b": { "bind": "x", "body": { "e": "lit", "v": "chose b" } },
        }});
        let out = run(m, vec![("v", v)]);
        assert_eq!(out.value(), json!("chose a"));
        assert_eq!(out.label(), web());
    }

    #[test]
    fn labels_are_readable() {
        let out = run(
            json!({ "e": "label", "of": { "e": "var", "name": "x" } }),
            vec![("x", LValue::new(&json!(1), &web()))],
        );
        assert_eq!(out.value(), json!(["web"]));
    }

    #[test]
    fn folds_run_once_per_item_in_order() {
        let xs = LValue::trusted(&json!([7, 8, 9]));
        let fold = |body: Value| json!({ "e": "fold", "over": { "e": "var", "name": "xs" }, "init": { "e": "lit", "v": 0 }, "acc": "a", "item": "x", "body": body });
        let sum = fold(
            json!({ "e": "op", "op": "add", "args": [{ "e": "var", "name": "a" }, { "e": "var", "name": "x" }] }),
        );
        assert_eq!(run(sum, vec![("xs", xs.clone())]).value(), json!(24));
        // The last item seen is the one that stays.
        assert_eq!(
            run(fold(json!({ "e": "var", "name": "x" })), vec![("xs", xs)]).value(),
            json!(9)
        );
        let not_a_list = json!({ "e": "fold", "over": { "e": "lit", "v": 3 }, "init": { "e": "lit", "v": 0 }, "acc": "a", "item": "x", "body": { "e": "var", "name": "a" } });
        assert_eq!(
            eval(&expr(not_a_list), vec![]),
            Err(EvalError::NotAList {
                at: "/over".into(),
                got: json!(3)
            })
        );
    }
}
