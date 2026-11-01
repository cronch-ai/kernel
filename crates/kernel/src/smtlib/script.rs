//! SMT-LIB 2 as data: terms, the commands an export uses, and the one printer that writes them as text.
//!
//! Encoding builds a `Script`; nothing else writes SMT-LIB text, so every symbol and literal is quoted and escaped in
//! one place.

use super::NoReading;
use std::fmt::{self, Display, Formatter};

/// An S-expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Atom(String),
    List(Vec<Term>),
}

impl Term {
    pub fn atom(text: impl Into<String>) -> Term {
        Term::Atom(text.into())
    }

    /// `head` applied to `args`; a nullary function stands alone.
    pub fn app(head: Term, args: Vec<Term>) -> Term {
        if args.is_empty() {
            head
        } else {
            Term::List(std::iter::once(head).chain(args).collect())
        }
    }

    pub fn call(name: &str, args: Vec<Term>) -> Term {
        Term::app(Term::atom(name), args)
    }

    /// `args` under an associative function, or its unit when there are none.
    pub fn fold(name: &str, unit: Term, args: Vec<Term>) -> Term {
        match args.len() {
            0 => unit,
            1 => args.into_iter().next().unwrap_or(unit),
            _ => Term::call(name, args),
        }
    }

    pub fn bind(var: Term, bound: Term, body: Term) -> Term {
        Term::List(vec![
            Term::atom("let"),
            Term::List(vec![Term::List(vec![var, bound])]),
            body,
        ])
    }

    /// A quoted symbol. Every symbol an export makes carries a prefix for its kind, so no Cronch name can collide with
    /// an SMT-LIB keyword or with a name of another kind.
    pub fn symbol(prefix: &str, name: &str) -> Result<Term, NoReading> {
        if name
            .chars()
            .any(|c| c == '|' || c == '\\' || (c.is_control() && !c.is_whitespace()))
        {
            return Err(NoReading::Name {
                name: name.to_string(),
            });
        }
        Ok(Term::Atom(format!("|{prefix}{name}|")))
    }

    pub fn int(negative: bool, digits: String) -> Term {
        let n = Term::Atom(digits);
        if negative {
            Term::call("-", vec![n])
        } else {
            n
        }
    }

    /// A 64-bit float, exactly: the shortest decimal that reads back as `f`, rounded to nearest.
    pub fn float(f: f64) -> Term {
        if f == 0.0 && f.is_sign_negative() {
            return Term::List(vec![
                Term::atom("_"),
                Term::atom("-zero"),
                Term::atom("11"),
                Term::atom("53"),
            ]);
        }
        let mut digits = format!("{}", f.abs());
        if !digits.contains('.') {
            digits.push_str(".0");
        }
        Term::to_float(Term::int(f < 0.0, digits))
    }

    /// A 64-bit float as a real: the shortest decimal that reads back as `f`, which may differ from it in the last
    /// digit's worth. Only search reads floats as reals.
    pub fn real(f: f64) -> Term {
        let mut digits = format!("{}", f.abs());
        if !digits.contains('.') {
            digits.push_str(".0");
        }
        Term::int(f < 0.0, digits)
    }

    /// A real or integer term as the nearest 64-bit float.
    pub fn to_float(real: Term) -> Term {
        let to_fp = Term::List(vec![
            Term::atom("_"),
            Term::atom("to_fp"),
            Term::atom("11"),
            Term::atom("53"),
        ]);
        Term::List(vec![to_fp, Term::atom("RNE"), real])
    }

    pub fn string(s: &str) -> Result<Term, NoReading> {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => out.push_str("\"\""),
                '\\' => out.push_str("\\u{5c}"),
                ' '..='~' => out.push(c),
                c if (c as u32) <= 0x2FFFF => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
                c => return Err(NoReading::Character { code: c as u32 }),
            }
        }
        out.push('"');
        Ok(Term::Atom(out))
    }
}

impl Display for Term {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Term::Atom(text) => f.write_str(text),
            Term::List(items) => {
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

/// The commands an export is made of.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    SetLogic(&'static str),
    /// A record: one constructor, named as its sort, with a selector per field.
    DeclareDatatype {
        name: Term,
        selectors: Vec<(Term, Term)>,
    },
    DeclareFun {
        name: Term,
        params: Vec<Term>,
        sort: Term,
    },
    DeclareConst {
        name: Term,
        sort: Term,
    },
    DefineFun {
        name: Term,
        params: Vec<(Term, Term)>,
        sort: Term,
        body: Term,
    },
    Assert(Term),
    CheckSat,
    GetValue(Vec<Term>),
}

fn pairs(items: &[(Term, Term)]) -> Term {
    Term::List(
        items
            .iter()
            .map(|(a, b)| Term::List(vec![a.clone(), b.clone()]))
            .collect(),
    )
}

impl Display for Command {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Command::SetLogic(logic) => write!(f, "(set-logic {logic})"),
            Command::DeclareDatatype { name, selectors } => {
                let constructor = Term::List(
                    std::iter::once(name.clone())
                        .chain(pairs(selectors).items())
                        .collect(),
                );
                write!(f, "(declare-datatype {name} ({constructor}))")
            }
            Command::DeclareFun { name, params, sort } => {
                write!(
                    f,
                    "(declare-fun {name} {} {sort})",
                    Term::List(params.clone())
                )
            }
            Command::DeclareConst { name, sort } => write!(f, "(declare-const {name} {sort})"),
            Command::DefineFun {
                name,
                params,
                sort,
                body,
            } => {
                write!(f, "(define-fun {name} {} {sort} {body})", pairs(params))
            }
            Command::Assert(term) => write!(f, "(assert {term})"),
            Command::CheckSat => f.write_str("(check-sat)"),
            Command::GetValue(terms) => write!(f, "(get-value {})", Term::List(terms.clone())),
        }
    }
}

impl From<u64> for Term {
    fn from(n: u64) -> Term {
        Term::atom(n.to_string())
    }
}

impl Term {
    fn items(self) -> Vec<Term> {
        match self {
            Term::List(items) => items,
            atom => vec![atom],
        }
    }
}

/// A command with the comments that explain it, or comments alone.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Entry {
    pub comments: Vec<String>,
    pub command: Option<Command>,
}

/// A titled group of entries. An empty section is left out.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub heading: &'static str,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    pub preamble: Vec<String>,
    /// `set-option`s, which come before the logic is set.
    pub options: Vec<(&'static str, &'static str)>,
    pub logic: &'static str,
    pub sections: Vec<Section>,
}

fn comment(f: &mut Formatter<'_>, text: &str) -> fmt::Result {
    text.lines().try_for_each(|line| writeln!(f, "; {line}"))
}

impl Display for Script {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.preamble.iter().try_for_each(|line| comment(f, line))?;
        self.options
            .iter()
            .try_for_each(|(option, value)| writeln!(f, "(set-option {option} {value})"))?;
        writeln!(f, "{}", Command::SetLogic(self.logic))?;
        for section in self.sections.iter().filter(|s| !s.entries.is_empty()) {
            writeln!(f)?;
            comment(f, section.heading)?;
            for entry in &section.entries {
                entry
                    .comments
                    .iter()
                    .try_for_each(|line| comment(f, line))?;
                if let Some(command) = &entry.command {
                    writeln!(f, "{command}")?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_are_exact() {
        assert_eq!(Term::float(0.1).to_string(), "((_ to_fp 11 53) RNE 0.1)");
        assert_eq!(
            Term::float(-2.0).to_string(),
            "((_ to_fp 11 53) RNE (- 2.0))"
        );
        assert_eq!(Term::float(-0.0).to_string(), "(_ -zero 11 53)");
    }

    #[test]
    fn strings_escape_what_smtlib_reads_specially() {
        assert_eq!(
            Term::string("a\"b\\é").unwrap().to_string(),
            "\"a\"\"b\\u{5c}\\u{e9}\""
        );
        assert_eq!(
            Term::string("\u{30000}"),
            Err(NoReading::Character { code: 0x30000 })
        );
    }

    #[test]
    fn symbols_refuse_what_quoting_cannot_hold() {
        assert_eq!(Term::symbol("v:", "a b").unwrap().to_string(), "|v:a b|");
        assert_eq!(
            Term::symbol("v:", "a|b"),
            Err(NoReading::Name { name: "a|b".into() })
        );
    }
}
