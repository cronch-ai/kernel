//! Lean 4: a refutation's script read as the statement that its assertions can't all hold.
//!
//! This is a second printer over the same `Script`, not a second encoder: what a query says and what its statement
//! says are one piece of code. It writes `abbrev statement : Prop := ∀ <constants>, ¬ (<assertions>)` over the
//! records the script declares, with SMT-LIB's operations as `lean/Cronch/Smt.lean` defines them. SMT-LIB's `Bool`
//! is read as `Prop`. A part with no Lean reading yet (a float, a variant, a recursive function) is refused, never
//! approximated. See `docs/2026-11-06-lean.md`.

use super::NoReading;
use super::script::{Command, Script, Term};

fn refuse(what: impl Into<String>) -> NoReading {
    NoReading::Lean { what: what.into() }
}

/// The statement a refutation's script says, as a Lean module.
pub fn statement(script: &Script) -> Result<String, NoReading> {
    let mut out = String::new();
    for line in &script.preamble {
        out.push_str(&format!("-- {line}\n"));
    }
    // The encoder binds every name a definition may read, whether or not it does.
    out.push_str(
        "import Cronch.Smt\n\nopen Classical\n\nset_option linter.unusedVariables false\n",
    );
    let mut binders = Vec::new();
    let mut held = Vec::new();
    for section in script.sections.iter().filter(|s| !s.entries.is_empty()) {
        let mut text = String::new();
        for entry in &section.entries {
            for line in &entry.comments {
                text.push_str(&format!("-- {line}\n"));
            }
            match &entry.command {
                None | Some(Command::SetLogic(_)) | Some(Command::CheckSat) => {}
                Some(Command::DeclareDatatype {
                    name,
                    constructor,
                    selectors,
                }) => text.push_str(&structure(name, constructor, selectors)?),
                Some(Command::DeclareConst { name, sort: s }) => {
                    binders.push(format!("({} : {})", symbol(name)?, sort(s)?));
                }
                Some(Command::Assert(t)) => held.push(term(t)?),
                Some(other) => {
                    let head = other.to_string();
                    let head = head
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_start_matches('(');
                    return Err(refuse(head));
                }
            }
        }
        if !text.is_empty() {
            out.push_str(&format!("\n-- {}\n{text}", section.heading));
        }
    }
    let held = if held.is_empty() {
        "True".to_string()
    } else {
        held.join(" ∧\n      ")
    };
    let binders = if binders.is_empty() {
        String::new()
    } else {
        format!("∀ {},\n    ", binders.join(" "))
    };
    out.push_str(&format!(
        "\n/-- The obligation holds when this is proved. -/\nabbrev statement : Prop :=\n  {binders}¬ ({held})\n"
    ));
    Ok(out)
}

/// A record: a structure with its constructor and selectors exported under their SMT-LIB names.
fn structure(
    name: &Term,
    constructor: &Term,
    selectors: &[(Term, Term)],
) -> Result<String, NoReading> {
    let name = symbol(name)?;
    let constructor = symbol(constructor)?;
    let mut text = format!("structure {name} where\n  {constructor} ::\n");
    let mut exported = vec![constructor];
    for (selector, s) in selectors {
        let selector = symbol(selector)?;
        text.push_str(&format!("  {selector} : {}\n", sort(s)?));
        exported.push(selector);
    }
    text.push_str("  deriving Inhabited\n");
    text.push_str(&format!("export {name} ({})\n", exported.join(" ")));
    Ok(text)
}

/// A quoted SMT-LIB symbol as a Lean identifier: `|v:args|` is `«v:args»`.
fn symbol(t: &Term) -> Result<String, NoReading> {
    let Term::Atom(text) = t else {
        return Err(refuse(t.to_string()));
    };
    let name = text
        .strip_prefix('|')
        .and_then(|s| s.strip_suffix('|'))
        .ok_or_else(|| refuse(text.clone()))?;
    if name.contains(['«', '»']) || name.chars().any(char::is_control) {
        return Err(NoReading::Name {
            name: name.to_string(),
        });
    }
    Ok(format!("«{name}»"))
}

fn sort(t: &Term) -> Result<String, NoReading> {
    match t {
        Term::Atom(a) if a == "Int" => Ok("Int".into()),
        Term::Atom(a) if a == "Bool" => Ok("Prop".into()),
        Term::Atom(a) if a == "String" => Ok("String".into()),
        Term::Atom(a) if a.starts_with('|') => symbol(t),
        Term::List(items) => match items.as_slice() {
            [Term::Atom(seq), item] if seq == "Seq" => Ok(format!("(List {})", sort(item)?)),
            _ => Err(refuse(t.to_string())),
        },
        _ => Err(refuse(t.to_string())),
    }
}

/// SMT-LIB's functions, with the Lean function each is.
const FUNCTIONS: &[(&str, &str)] = &[
    ("div", "Smt.div"),
    ("mod", "Smt.mod"),
    ("seq.len", "Smt.seq.len"),
    ("seq.nth", "Smt.seq.nth"),
    ("seq.extract", "Smt.seq.extract"),
    ("seq.prefixof", "Smt.seq.prefixof"),
    ("seq.contains", "Smt.seq.contains"),
    ("str.len", "Smt.str.len"),
    ("str.prefixof", "Smt.str.prefixof"),
    ("str.contains", "Smt.str.contains"),
    ("str.from_int", "Smt.str.from_int"),
];

/// SMT-LIB's infix operators, each with its Lean operator.
const INFIX: &[(&str, &str)] = &[
    ("and", "∧"),
    ("or", "∨"),
    ("=>", "→"),
    ("+", "+"),
    ("*", "*"),
    ("str.++", "++"),
    ("seq.++", "++"),
    ("<", "<"),
    ("<=", "≤"),
    (">", ">"),
    (">=", "≥"),
];

fn term(t: &Term) -> Result<String, NoReading> {
    match t {
        Term::Atom(a) => atom(a),
        Term::List(items) => {
            let [head, args @ ..] = items.as_slice() else {
                return Err(refuse("()"));
            };
            let Term::Atom(head) = head else {
                return Err(refuse(t.to_string()));
            };
            let all = || args.iter().map(term).collect::<Result<Vec<_>, _>>();
            match head.as_str() {
                "not" => match args {
                    [a] => Ok(format!("(¬ {})", term(a)?)),
                    _ => Err(refuse(t.to_string())),
                },
                "-" => match args {
                    [a] => Ok(format!("(-{})", term(a)?)),
                    _ => Ok(format!("({})", all()?.join(" - "))),
                },
                "=" => match args {
                    [a, b] => Ok(format!("({} = {})", term(a)?, term(b)?)),
                    _ => Err(refuse(t.to_string())),
                },
                "ite" => match args {
                    [c, a, b] => Ok(format!(
                        "(if {} then {} else {})",
                        term(c)?,
                        term(a)?,
                        term(b)?
                    )),
                    _ => Err(refuse(t.to_string())),
                },
                "let" => match args {
                    [Term::List(bindings), body] => match bindings.as_slice() {
                        // SMT-LIB's bindings are parallel; the encoder makes one per `let`.
                        [Term::List(pair)] => match pair.as_slice() {
                            [var, bound] => Ok(format!(
                                "(let {} := {}; {})",
                                symbol(var)?,
                                term(bound)?,
                                term(body)?
                            )),
                            _ => Err(refuse(t.to_string())),
                        },
                        _ => Err(refuse("let with more than one binding")),
                    },
                    _ => Err(refuse(t.to_string())),
                },
                "forall" => match args {
                    [Term::List(bindings), body] => {
                        let mut binders = Vec::new();
                        for b in bindings {
                            let Term::List(pair) = b else {
                                return Err(refuse(t.to_string()));
                            };
                            let [var, s] = pair.as_slice() else {
                                return Err(refuse(t.to_string()));
                            };
                            binders.push(format!("({} : {})", bound(var)?, sort(s)?));
                        }
                        Ok(format!("(∀ {}, {})", binders.join(" "), term(body)?))
                    }
                    _ => Err(refuse(t.to_string())),
                },
                "seq.unit" => match args {
                    [a] => Ok(format!("[{}]", term(a)?)),
                    _ => Err(refuse(t.to_string())),
                },
                "as" => match args {
                    [Term::Atom(empty), s] if empty == "seq.empty" => {
                        Ok(format!("([] : {})", sort(s)?))
                    }
                    _ => Err(refuse(t.to_string())),
                },
                h if h.starts_with('|') => {
                    let mut parts = vec![symbol(&Term::atom(h))?];
                    parts.extend(all()?);
                    Ok(format!("({})", parts.join(" ")))
                }
                h => {
                    if let Some((_, op)) = INFIX.iter().find(|(s, _)| *s == h) {
                        if args.len() < 2 {
                            return Err(refuse(t.to_string()));
                        }
                        // Lean's `→` is right-associative, as SMT-LIB's `=>` is; the others chain from the left.
                        return Ok(format!("({})", all()?.join(&format!(" {op} "))));
                    }
                    if let Some((_, f)) = FUNCTIONS.iter().find(|(s, _)| *s == h) {
                        return Ok(format!("({f} {})", all()?.join(" ")));
                    }
                    Err(refuse(h))
                }
            }
        }
    }
}

/// A variable bound by a quantifier: the encoder names them plainly, or quoted.
fn bound(t: &Term) -> Result<String, NoReading> {
    match t {
        Term::Atom(a) if a.starts_with('|') => symbol(t),
        Term::Atom(a)
            if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
        {
            Ok(format!("«{a}»"))
        }
        _ => Err(refuse(t.to_string())),
    }
}

fn atom(a: &str) -> Result<String, NoReading> {
    match a {
        "true" => Ok("True".into()),
        "false" => Ok("False".into()),
        _ if a.starts_with('|') => symbol(&Term::atom(a)),
        _ if a.starts_with('"') => string(a),
        _ if !a.is_empty() && a.chars().all(|c| c.is_ascii_digit()) => Ok(format!("({a} : Int)")),
        _ if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
            Ok(format!("«{a}»"))
        }
        _ => Err(refuse(a)),
    }
}

/// An SMT-LIB string literal, as `script::Term::string` writes one, as a Lean string literal.
fn string(literal: &str) -> Result<String, NoReading> {
    let bad = || refuse(literal);
    let inner = literal
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(bad)?;
    let mut chars = inner.chars().peekable();
    let mut out = String::from("\"");
    while let Some(c) = chars.next() {
        let c = match c {
            '"' if chars.peek() == Some(&'"') => {
                chars.next();
                '"'
            }
            '\\' => {
                if chars.next() != Some('u') || chars.next() != Some('{') {
                    return Err(bad());
                }
                let hex: String = chars.by_ref().take_while(|c| *c != '}').collect();
                u32::from_str_radix(&hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(bad)?
            }
            c => c,
        };
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() && (c as u32) < 0x80 => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<String, NoReading> {
        term(&parse(text))
    }

    /// Enough of an S-expression reader for these tests: atoms, lists and strings without spaces.
    fn parse(text: &str) -> Term {
        fn go(tokens: &mut std::iter::Peekable<std::vec::IntoIter<String>>) -> Term {
            let t = tokens.next().unwrap();
            if t == "(" {
                let mut items = Vec::new();
                while tokens.peek().map(String::as_str) != Some(")") {
                    items.push(go(tokens));
                }
                tokens.next();
                Term::List(items)
            } else {
                Term::Atom(t)
            }
        }
        let spaced = text.replace('(', " ( ").replace(')', " ) ");
        let tokens: Vec<String> = spaced.split_whitespace().map(str::to_string).collect();
        go(&mut tokens.into_iter().peekable())
    }

    #[test]
    fn terms_read_as_lean() {
        assert_eq!(
            read("(seq.contains (|r.xs| |v:g|) (seq.unit (|r.url| |v:a|)))").unwrap(),
            "(Smt.seq.contains («r.xs» «v:g») [(«r.url» «v:a»)])"
        );
        assert_eq!(
            read("(and (<= (- 9) |v:n|) (not true))").unwrap(),
            "(((-(9 : Int)) ≤ «v:n») ∧ (¬ True))"
        );
        assert_eq!(
            read("(let ((|v:x| 1)) (= |v:x| 1))").unwrap(),
            "(let «v:x» := (1 : Int); («v:x» = (1 : Int)))"
        );
        assert_eq!(
            read("(forall ((x0 Int)) (=> (< x0 (seq.len |v:s|)) true))").unwrap(),
            "(∀ («x0» : Int), ((«x0» < (Smt.seq.len «v:s»)) → True))"
        );
        assert_eq!(
            read("(as seq.empty (Seq String))").unwrap(),
            "([] : (List String))"
        );
    }

    #[test]
    fn strings_unescape_from_smtlib() {
        let smt = Term::string("a\"b\\é\n").unwrap().to_string();
        assert_eq!(string(&smt).unwrap(), "\"a\\\"b\\\\é\\n\"");
    }

    #[test]
    fn what_has_no_lean_reading_is_refused() {
        assert_eq!(
            read("((_ to_fp 11 53) RNE 0.5)"),
            Err(refuse("((_ to_fp 11 53) RNE 0.5)"))
        );
        assert_eq!(read("(fp.isNaN |v:f|)"), Err(refuse("fp.isNaN")));
        assert_eq!(read("0.5"), Err(refuse("0.5")));
        assert_eq!(sort(&parse("Float64")), Err(refuse("Float64")));
        assert_eq!(
            symbol(&Term::atom("|a»b|")),
            Err(NoReading::Name {
                name: "a»b".into()
            })
        );
    }
}
