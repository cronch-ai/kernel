//! `Json` in SMT-LIB: a recursive datatype of JSON values, with the functions that read a record's fields and say
//! which terms are values the kernel could hold.
//!
//! A record is a chain of fields. The kernel holds each key of a record once, so a term is a value it could hold only
//! when no key is repeated; and a number only when it is finite. `json:ok` says so, and a query asserts it of what it
//! quantifies over, so a model is a value the kernel can be handed back. Keys may come in any order: nothing reads a
//! record's order, and `Json` has no equality reading, so two orders of one record are never told apart. (Requiring
//! key order instead would compare strings, where solvers give up.)

use super::NoReading;
use super::script::{Command, Constructor, Entry, Term};

type Read<T> = Result<T, NoReading>;

pub(super) fn name(n: &str) -> Read<Term> {
    Term::symbol("json:", n)
}

/// The sort of JSON values.
pub(super) fn sort() -> Read<Term> {
    name("Json")
}

fn fields_sort() -> Read<Term> {
    name("Fields")
}

/// `((_ is C) term)` for one of `Json`'s constructors.
pub(super) fn tester(constructor: &str, term: Term) -> Read<Term> {
    let is = Term::List(vec![Term::atom("_"), Term::atom("is"), name(constructor)?]);
    Ok(Term::List(vec![is, term]))
}

/// A selector applied: `json:bool.v`, `json:field.key` and the like.
pub(super) fn select(selector: &str, term: Term) -> Read<Term> {
    Ok(Term::app(name(selector)?, vec![term]))
}

pub(super) fn null() -> Read<Term> {
    name("null")
}

pub(super) fn wrap(constructor: &str, term: Term) -> Read<Term> {
    Ok(Term::app(name(constructor)?, vec![term]))
}

/// A record of these fields, each key once.
pub(super) fn record(fields: Vec<(Term, Term)>) -> Read<Term> {
    let mut chain = name("end")?;
    for (key, value) in fields.into_iter().rev() {
        chain = Term::app(name("field")?, vec![key, value, chain]);
    }
    wrap("record", chain)
}

/// Whether a record's fields include `key`.
pub(super) fn has(fields: Term, key: Term) -> Read<Term> {
    Ok(Term::app(name("has")?, vec![fields, key]))
}

/// The value of a record's field `key`; `null` if it has none.
pub(super) fn get(fields: Term, key: Term) -> Read<Term> {
    Ok(Term::app(name("get")?, vec![fields, key]))
}

/// Whether a term is a JSON value the kernel could hold.
pub(super) fn ok(term: Term) -> Read<Term> {
    Ok(Term::app(name("ok")?, vec![term]))
}

/// The datatypes and functions, given the sort numbers have and what makes one finite (`None`: every one is).
pub(super) fn declarations(number: Term, finite: Option<fn(Term) -> Term>) -> Read<Vec<Entry>> {
    let (json, fields) = (sort()?, fields_sort()?);
    let seq = Term::call("Seq", vec![json.clone()]);
    let ctor = |n: &str, selectors: Vec<(&str, Term)>| -> Read<Constructor> {
        let mut out = Vec::new();
        for (s, sort) in selectors {
            out.push((name(s)?, sort));
        }
        Ok((name(n)?, out))
    };
    let datatypes = Command::DeclareDatatypes {
        datatypes: vec![
            (
                json.clone(),
                vec![
                    ctor("null", vec![])?,
                    ctor("bool", vec![("bool.v", Term::atom("Bool"))])?,
                    ctor("number", vec![("number.v", number)])?,
                    ctor("string", vec![("string.v", Term::atom("String"))])?,
                    ctor("list", vec![("list.v", seq.clone())])?,
                    ctor("record", vec![("record.v", fields.clone())])?,
                ],
            ),
            (
                fields.clone(),
                vec![
                    ctor("end", vec![])?,
                    ctor(
                        "field",
                        vec![
                            ("field.key", Term::atom("String")),
                            ("field.value", json.clone()),
                            ("field.rest", fields.clone()),
                        ],
                    )?,
                ],
            ),
        ],
    };

    let (f, k, j, s) = (
        Term::atom("f"),
        Term::atom("k"),
        Term::atom("j"),
        Term::atom("s"),
    );
    let ite = |c: Term, t: Term, e: Term| Term::call("ite", vec![c, t, e]);
    let is_field = |t: Term| tester("field", t);
    let key = |t: Term| select("field.key", t);
    let value = |t: Term| select("field.value", t);
    let rest = |t: Term| select("field.rest", t);
    let lookup = Command::DefineFunsRec {
        functions: vec![
            (
                name("has")?,
                vec![
                    (f.clone(), fields.clone()),
                    (k.clone(), Term::atom("String")),
                ],
                Term::atom("Bool"),
                ite(
                    is_field(f.clone())?,
                    Term::call(
                        "or",
                        vec![
                            Term::call("=", vec![key(f.clone())?, k.clone()]),
                            has(rest(f.clone())?, k.clone())?,
                        ],
                    ),
                    Term::atom("false"),
                ),
            ),
            (
                name("get")?,
                vec![
                    (f.clone(), fields.clone()),
                    (k.clone(), Term::atom("String")),
                ],
                json.clone(),
                ite(
                    is_field(f.clone())?,
                    ite(
                        Term::call("=", vec![key(f.clone())?, k.clone()]),
                        value(f.clone())?,
                        get(rest(f.clone())?, k.clone())?,
                    ),
                    null()?,
                ),
            ),
        ],
    };

    let number_ok = match finite {
        Some(finite) => ite(
            tester("number", j.clone())?,
            finite(select("number.v", j.clone())?),
            Term::atom("true"),
        ),
        None => Term::atom("true"),
    };
    let fields_ok = |t: Term| -> Read<Term> { Ok(Term::app(name("ok.fields")?, vec![t])) };
    let len = Term::call("seq.len", vec![s.clone()]);
    let ok_fns = Command::DefineFunsRec {
        functions: vec![
            (
                name("ok")?,
                vec![(j.clone(), json.clone())],
                Term::atom("Bool"),
                Term::call(
                    "and",
                    vec![
                        number_ok,
                        ite(
                            tester("list", j.clone())?,
                            Term::app(name("ok.items")?, vec![select("list.v", j.clone())?]),
                            Term::atom("true"),
                        ),
                        ite(
                            tester("record", j.clone())?,
                            fields_ok(select("record.v", j.clone())?)?,
                            Term::atom("true"),
                        ),
                    ],
                ),
            ),
            (
                name("ok.items")?,
                vec![(s.clone(), seq)],
                Term::atom("Bool"),
                ite(
                    Term::call("=", vec![len.clone(), Term::atom("0")]),
                    Term::atom("true"),
                    Term::call(
                        "and",
                        vec![
                            ok(Term::call("seq.nth", vec![s.clone(), Term::atom("0")]))?,
                            Term::app(
                                name("ok.items")?,
                                vec![Term::call(
                                    "seq.extract",
                                    vec![
                                        s.clone(),
                                        Term::atom("1"),
                                        Term::call("-", vec![len, Term::atom("1")]),
                                    ],
                                )],
                            ),
                        ],
                    ),
                ),
            ),
            (
                name("ok.fields")?,
                vec![(f.clone(), fields)],
                Term::atom("Bool"),
                ite(
                    is_field(f.clone())?,
                    Term::call(
                        "and",
                        vec![
                            ok(value(f.clone())?)?,
                            Term::call("not", vec![has(rest(f.clone())?, key(f.clone())?)?]),
                            fields_ok(rest(f.clone())?)?,
                        ],
                    ),
                    Term::atom("true"),
                ),
            ),
        ],
    };
    let entry = |comments: &[&str], command| Entry {
        comments: comments.iter().map(|c| c.to_string()).collect(),
        command: Some(command),
    };
    Ok(vec![
        entry(&["JSON values: a record is a chain of fields."], datatypes),
        entry(
            &["Whether a record has a field, and its value (null if it has none)."],
            lookup,
        ),
        entry(
            &[
                "Whether a term is a value the kernel could hold: finite numbers, and each record's keys once.",
            ],
            ok_fns,
        ),
    ])
}
