//! A recursive-descent parser. It stops at the first thing it can't read, with that thing's span.
//!
//! ```text
//! file    = "harness" name "(" name ":" type ")" "->" type NL decl*
//! decl    = "uses" use ("," use)* NL
//!         | "type" name "=" (variant | type) NL
//!         | "return" expr NL
//!         | "otherwise" "return" expr NL
//!         | ("property" | "invariant") name ":" NL? expr NL
//!         | name "=" (expr | receive | spawn | each) ("when" expr)? ("until" name)? NL
//! receive = "receive" name? "from" path
//! spawn   = "spawn" name "(" expr ")" ("only" path ("," path)*)? ("without" path ("," path)*)?
//! each    = "for" name "in" expr ":" expr
//! path    = label ("." label)*
//! use     = name ("=" "harness"? string)?
//! type    = "List" "[" type "]" | "{" (name ":" type),* "}" | name
//! variant = case ("|" case)*          case = name ("{" (name ":" type),* "}")?
//! expr    = "if" expr "then" expr "else" expr | "fold" or "from" expr "with" "(" name "," name ")" "=>" expr | implies
//! implies = since ("implies" implies)?  since = or ("since" or)*
//! or      = and ("or" and)*           and = not ("and" not)*        not = "not" not | compare
//! compare = sum (("==" | "!=" | "<" | "<=" | ">" | ">=" | "in" | "not" "in") sum)?
//! sum     = term (("+" | "-") term)*  term = unary (("*" | "/" | "//" | "%") unary)*
//! unary   = "-" unary | postfix       postfix = primary ("." name | "(" args ")")*
//! primary = literal | name | "(" expr ")" | "[" expr,* "]" | "{" (name ":" expr),* "}" | "match" expr "{" arm* "}"
//!         | ("exists" | "forall") name "in" or ":" expr
//! arm     = ("_" | name ("(" name ")")? | literal ("|" literal)*) "=>" expr ","?
//! ```

use crate::lex::{Span, Tok, Token};
use crate::syntax::*;
use crate::{Problem, What};
use serde_json::{Value, json};

/// Words that can't name a node, a parameter or a binding. A field may still be called any of them.
pub const KEYWORDS: [&str; 28] = [
    "harness",
    "uses",
    "type",
    "return",
    "otherwise",
    "when",
    "until",
    "receive",
    "spawn",
    "for",
    "property",
    "invariant",
    "implies",
    "since",
    "exists",
    "forall",
    "if",
    "then",
    "else",
    "match",
    "fold",
    "from",
    "with",
    "and",
    "or",
    "not",
    "in",
    "true",
];

const FALSE: &str = "false";

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

type Parsed<T> = Result<T, Problem>;

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.at].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.tokens[(self.at + n).min(self.tokens.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.tokens[self.at].span
    }

    /// The span of the token just consumed.
    fn last(&self) -> Span {
        self.tokens[self.at.saturating_sub(1)].span
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens[self.at].clone();
        if self.at + 1 < self.tokens.len() {
            self.at += 1;
        }
        token
    }

    fn fail<T>(&self, expected: &str) -> Parsed<T> {
        Err(Problem {
            span: self.span(),
            what: What::Syntax {
                expected: expected.to_string(),
                found: self.peek().describe(),
            },
        })
    }

    fn is_sym(&self, sym: &str) -> bool {
        matches!(self.peek(), Tok::Sym(s) if *s == sym)
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(self.peek(), Tok::Ident(w) if w == word)
    }

    fn eat_sym(&mut self, sym: &str) -> bool {
        let found = self.is_sym(sym);
        if found {
            self.advance();
        }
        found
    }

    fn eat_word(&mut self, word: &str) -> bool {
        let found = self.is_word(word);
        if found {
            self.advance();
        }
        found
    }

    fn sym(&mut self, sym: &str) -> Parsed<Span> {
        if self.eat_sym(sym) {
            Ok(self.last())
        } else {
            self.fail(&format!("`{sym}`"))
        }
    }

    fn word(&mut self, word: &str) -> Parsed<Span> {
        if self.eat_word(word) {
            Ok(self.last())
        } else {
            self.fail(&format!("`{word}`"))
        }
    }

    fn newline(&mut self) -> Parsed<()> {
        match self.peek() {
            Tok::Newline => {
                self.advance();
                Ok(())
            }
            _ => self.fail("the end of the line"),
        }
    }

    /// Any identifier, keywords included: a field, a case, a capability's operation.
    fn label(&mut self) -> Parsed<Name> {
        match self.peek().clone() {
            Tok::Ident(text) => {
                let span = self.advance().span;
                Ok(Name { text, span })
            }
            _ => self.fail("a name"),
        }
    }

    /// An identifier that binds something, so isn't a keyword.
    fn name(&mut self) -> Parsed<Name> {
        match self.peek() {
            Tok::Ident(w) if KEYWORDS.contains(&w.as_str()) || w == FALSE => self.fail("a name"),
            _ => self.label(),
        }
    }

    fn string(&mut self) -> Parsed<(String, Span)> {
        match self.peek().clone() {
            Tok::Str(s) => Ok((s, self.advance().span)),
            _ => self.fail("a string"),
        }
    }

    fn comma_separated<T>(
        &mut self,
        close: &str,
        mut item: impl FnMut(&mut Self) -> Parsed<T>,
    ) -> Parsed<Vec<T>> {
        let mut items = Vec::new();
        while !self.is_sym(close) {
            items.push(item(self)?);
            if !self.eat_sym(",") {
                break;
            }
        }
        self.sym(close)?;
        Ok(items)
    }

    fn file(&mut self) -> Parsed<File> {
        while *self.peek() == Tok::Newline {
            self.advance();
        }
        self.word("harness")?;
        let name = self.name()?;
        self.sym("(")?;
        let param = self.name()?;
        self.sym(":")?;
        let param_type = self.ty()?;
        self.sym(")")?;
        self.sym("->")?;
        let result_type = self.ty()?;
        self.newline()?;
        let mut file = File {
            name,
            param,
            param_type,
            result_type,
            uses: Vec::new(),
            types: Vec::new(),
            nodes: Vec::new(),
            returns: Vec::new(),
            otherwise: Vec::new(),
            properties: Vec::new(),
            invariants: Vec::new(),
        };
        while *self.peek() != Tok::Eof {
            self.decl(&mut file)?;
        }
        Ok(file)
    }

    fn decl(&mut self, file: &mut File) -> Parsed<()> {
        if self.eat_word("uses") {
            loop {
                let name = self.name()?;
                let (capability, harness) = if self.eat_sym("=") {
                    let harness = self.eat_word("harness");
                    (Some(self.string()?), harness)
                } else {
                    (None, false)
                };
                file.uses.push(Use {
                    name,
                    capability,
                    harness,
                });
                if !self.eat_sym(",") {
                    break;
                }
            }
        } else if self.eat_word("type") {
            let name = self.name()?;
            self.sym("=")?;
            let is_variant = matches!(self.peek(), Tok::Ident(_))
                && matches!(self.peek_at(1), Tok::Sym("|") | Tok::Sym("{"));
            let ty = if is_variant {
                self.variant()?
            } else {
                self.ty()?
            };
            file.types.push((name, ty));
        } else if self.eat_word("return") {
            file.returns.push(self.expr()?);
        } else if self.is_word("property") || self.is_word("invariant") {
            let property = self.eat_word("property");
            if !property {
                self.word("invariant")?;
            }
            let name = self.name()?;
            self.sym(":")?;
            if *self.peek() == Tok::Newline {
                self.advance();
            }
            let body = self.expr()?;
            if property {
                file.properties.push((name, body));
            } else {
                file.invariants.push((name, body));
            }
        } else if self.eat_word("otherwise") {
            self.word("return")?;
            file.otherwise.push(self.expr()?);
        } else if matches!(self.peek(), Tok::Ident(_)) && *self.peek_at(1) == Tok::Sym("=") {
            let name = self.name()?;
            self.sym("=")?;
            let value = if self.is_word("receive") {
                self.receive()?
            } else if self.is_word("spawn") {
                self.spawn()?
            } else if self.is_word("for") {
                self.each()?
            } else {
                self.expr()?
            };
            let when = if self.eat_word("when") {
                Some(self.expr()?)
            } else {
                None
            };
            let until = if self.eat_word("until") {
                Some(self.name()?)
            } else {
                None
            };
            file.nodes.push(Node {
                name,
                value,
                when,
                until,
            });
        } else {
            return self
                .fail("a declaration: `uses`, `type`, `return`, `otherwise` or `name = ...`");
        }
        self.newline()
    }

    fn receive(&mut self) -> Parsed<Expr> {
        let start = self.word("receive")?;
        let binder = if self.is_word("from") {
            None
        } else {
            Some(self.name()?)
        };
        self.word("from")?;
        let channel = self.path()?;
        let span = start.to(channel.span);
        Ok(Expr {
            kind: ExprKind::Receive { binder, channel },
            span,
        })
    }

    /// A dotted name, as one: a channel or an operation.
    fn path(&mut self) -> Parsed<Name> {
        let mut path = self.label()?;
        while self.eat_sym(".") {
            let part = self.label()?;
            path.text = format!("{}.{}", path.text, part.text);
            path.span = path.span.to(part.span);
        }
        Ok(path)
    }

    fn paths(&mut self) -> Parsed<Vec<Name>> {
        let mut paths = vec![self.path()?];
        while self.eat_sym(",") {
            paths.push(self.path()?);
        }
        Ok(paths)
    }

    fn spawn(&mut self) -> Parsed<Expr> {
        let start = self.word("spawn")?;
        let harness = self.name()?;
        self.sym("(")?;
        let arg = self.expr()?;
        self.sym(")")?;
        let only = if self.eat_word("only") {
            Some(self.paths()?)
        } else {
            None
        };
        let without = if self.eat_word("without") {
            self.paths()?
        } else {
            Vec::new()
        };
        Ok(Expr {
            kind: ExprKind::Spawn {
                harness,
                arg: Box::new(arg),
                only,
                without,
            },
            span: start.to(self.last()),
        })
    }

    fn each(&mut self) -> Parsed<Expr> {
        let start = self.word("for")?;
        let item = self.name()?;
        self.word("in")?;
        let over = self.expr()?;
        self.sym(":")?;
        let call = self.expr()?;
        let span = start.to(call.span);
        Ok(Expr {
            kind: ExprKind::ForEach {
                item,
                over: Box::new(over),
                call: Box::new(call),
            },
            span,
        })
    }

    fn fields_type(&mut self) -> Parsed<Vec<(Name, TypeExpr)>> {
        self.comma_separated("}", |p| {
            let name = p.label()?;
            p.sym(":")?;
            Ok((name, p.ty()?))
        })
    }

    fn ty(&mut self) -> Parsed<TypeExpr> {
        let start = self.span();
        if self.eat_sym("{") {
            let fields = self.fields_type()?;
            return Ok(TypeExpr::Record(fields, start.to(self.last())));
        }
        let name = self.name()?;
        if name.text == "List" && self.eat_sym("[") {
            let item = self.ty()?;
            self.sym("]")?;
            return Ok(TypeExpr::List(Box::new(item), start.to(self.last())));
        }
        Ok(TypeExpr::Named(name))
    }

    fn variant(&mut self) -> Parsed<TypeExpr> {
        let start = self.span();
        let mut cases = Vec::new();
        loop {
            let case = self.label()?;
            let fields = if self.is_sym("{") {
                let open = self.advance().span;
                let fields = self.fields_type()?;
                Some(TypeExpr::Record(fields, open.to(self.last())))
            } else {
                None
            };
            cases.push((case, fields));
            if !self.eat_sym("|") {
                break;
            }
        }
        Ok(TypeExpr::Variant(cases, start.to(self.last())))
    }

    fn expr(&mut self) -> Parsed<Expr> {
        let start = self.span();
        if self.eat_word("if") {
            let cond = self.expr()?;
            self.word("then")?;
            let then = self.expr()?;
            self.word("else")?;
            let otherwise = self.expr()?;
            let span = start.to(otherwise.span);
            return Ok(Expr {
                kind: ExprKind::If(Box::new(cond), Box::new(then), Box::new(otherwise)),
                span,
            });
        }
        if self.eat_word("fold") {
            let over = self.or()?;
            self.word("from")?;
            let init = self.expr()?;
            self.word("with")?;
            self.sym("(")?;
            let acc = self.name()?;
            self.sym(",")?;
            let item = self.name()?;
            self.sym(")")?;
            self.sym("=>")?;
            let body = self.expr()?;
            let span = start.to(body.span);
            return Ok(Expr {
                kind: ExprKind::Fold {
                    over: Box::new(over),
                    init: Box::new(init),
                    acc,
                    item,
                    body: Box::new(body),
                },
                span,
            });
        }
        self.implies()
    }

    fn implies(&mut self) -> Parsed<Expr> {
        let e = self.since()?;
        if self.eat_word("implies") {
            let rhs = self.implies()?;
            return Ok(Self::binary(Binary::Implies, e, rhs));
        }
        Ok(e)
    }

    fn since(&mut self) -> Parsed<Expr> {
        let mut e = self.or()?;
        while self.eat_word("since") {
            e = Self::binary(Binary::Since, e, self.or()?);
        }
        Ok(e)
    }

    fn binary(op: Binary, a: Expr, b: Expr) -> Expr {
        let span = a.span.to(b.span);
        Expr {
            kind: ExprKind::Binary(op, Box::new(a), Box::new(b)),
            span,
        }
    }

    fn or(&mut self) -> Parsed<Expr> {
        let mut e = self.and()?;
        while self.eat_word("or") {
            e = Self::binary(Binary::Or, e, self.and()?);
        }
        Ok(e)
    }

    fn and(&mut self) -> Parsed<Expr> {
        let mut e = self.not()?;
        while self.eat_word("and") {
            e = Self::binary(Binary::And, e, self.not()?);
        }
        Ok(e)
    }

    fn not(&mut self) -> Parsed<Expr> {
        let start = self.span();
        if self.eat_word("not") {
            let of = self.not()?;
            let span = start.to(of.span);
            return Ok(Expr {
                kind: ExprKind::Unary(Unary::Not, Box::new(of)),
                span,
            });
        }
        self.compare()
    }

    fn compare(&mut self) -> Parsed<Expr> {
        let e = self.sum()?;
        let op = match self.peek() {
            Tok::Sym("==") => Binary::Eq,
            Tok::Sym("!=") => Binary::Ne,
            Tok::Sym("<") => Binary::Lt,
            Tok::Sym("<=") => Binary::Le,
            Tok::Sym(">") => Binary::Gt,
            Tok::Sym(">=") => Binary::Ge,
            Tok::Ident(w) if w == "in" => Binary::In,
            Tok::Ident(w)
                if w == "not" && matches!(self.peek_at(1), Tok::Ident(n) if n == "in") =>
            {
                self.advance();
                Binary::NotIn
            }
            _ => return Ok(e),
        };
        self.advance();
        let rhs = self.sum()?;
        Ok(Self::binary(op, e, rhs))
    }

    fn sum(&mut self) -> Parsed<Expr> {
        let mut e = self.term()?;
        loop {
            let op = match self.peek() {
                Tok::Sym("+") => Binary::Add,
                Tok::Sym("-") => Binary::Sub,
                _ => return Ok(e),
            };
            self.advance();
            e = Self::binary(op, e, self.term()?);
        }
    }

    fn term(&mut self) -> Parsed<Expr> {
        let mut e = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Sym("*") => Binary::Mul,
                Tok::Sym("/") => Binary::Div,
                Tok::Sym("//") => Binary::IDiv,
                Tok::Sym("%") => Binary::Mod,
                _ => return Ok(e),
            };
            self.advance();
            e = Self::binary(op, e, self.unary()?);
        }
    }

    fn unary(&mut self) -> Parsed<Expr> {
        let start = self.span();
        if self.eat_sym("-") {
            let of = self.unary()?;
            let span = start.to(of.span);
            return Ok(Expr {
                kind: ExprKind::Unary(Unary::Neg, Box::new(of)),
                span,
            });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Parsed<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.eat_sym(".") {
                let field = self.label()?;
                let span = e.span.to(field.span);
                e = Expr {
                    kind: ExprKind::Field(Box::new(e), field),
                    span,
                };
            } else if self.eat_sym("(") {
                let args = self.comma_separated(")", |p| {
                    let name =
                        if matches!(p.peek(), Tok::Ident(_)) && *p.peek_at(1) == Tok::Sym(":") {
                            let name = p.label()?;
                            p.advance();
                            Some(name)
                        } else {
                            None
                        };
                    Ok(Arg {
                        name,
                        value: p.expr()?,
                    })
                })?;
                let span = e.span.to(self.last());
                e = Expr {
                    kind: ExprKind::Call(Box::new(e), args),
                    span,
                };
            } else {
                return Ok(e);
            }
        }
    }

    fn literal(&mut self) -> Option<Value> {
        let v = match self.peek() {
            Tok::Str(s) => json!(s),
            Tok::Int(i) => json!(i),
            Tok::Float(f) => json!(f),
            Tok::Ident(w) if w == "true" => json!(true),
            Tok::Ident(w) if w == FALSE => json!(false),
            _ => return None,
        };
        self.advance();
        Some(v)
    }

    fn primary(&mut self) -> Parsed<Expr> {
        let start = self.span();
        let at = |kind, p: &Self| Expr {
            kind,
            span: start.to(p.last()),
        };
        if let Some(v) = self.literal() {
            return Ok(at(ExprKind::Lit(v), self));
        }
        if self.eat_sym("(") {
            let e = self.expr()?;
            self.sym(")")?;
            return Ok(Expr {
                kind: e.kind,
                span: start.to(self.last()),
            });
        }
        if self.eat_sym("[") {
            let items = self.comma_separated("]", Self::expr)?;
            return Ok(at(ExprKind::List(items), self));
        }
        if self.eat_sym("{") {
            let fields = self.comma_separated("}", |p| {
                let name = p.label()?;
                p.sym(":")?;
                Ok((name, p.expr()?))
            })?;
            return Ok(at(ExprKind::Record(fields), self));
        }
        if self.is_word("exists") || self.is_word("forall") {
            let exists = self.eat_word("exists");
            if !exists {
                self.word("forall")?;
            }
            let var = self.name()?;
            self.word("in")?;
            let over = self.or()?;
            self.sym(":")?;
            let body = self.expr()?;
            let span = start.to(body.span);
            return Ok(Expr {
                kind: ExprKind::Quantified {
                    exists,
                    var,
                    over: Box::new(over),
                    body: Box::new(body),
                },
                span,
            });
        }
        if self.eat_word("match") {
            let of = self.expr()?;
            self.sym("{")?;
            let mut arms = Vec::new();
            while !self.eat_sym("}") {
                let pattern = self.pattern()?;
                self.sym("=>")?;
                let body = self.expr()?;
                arms.push(Arm { pattern, body });
                self.eat_sym(",");
            }
            return Ok(at(ExprKind::Match(Box::new(of), arms), self));
        }
        let name = self.name()?;
        Ok(Expr {
            kind: ExprKind::Name(name.text),
            span: name.span,
        })
    }

    fn pattern(&mut self) -> Parsed<Pattern> {
        let start = self.span();
        if let Some(first) = self.literal() {
            let mut values = vec![first];
            while self.eat_sym("|") {
                match self.literal() {
                    Some(v) => values.push(v),
                    None => return self.fail("a literal"),
                }
            }
            return Ok(Pattern::Literals(values, start.to(self.last())));
        }
        if self.is_word("_") {
            self.advance();
            return Ok(Pattern::Wild(start));
        }
        let case = self.label()?;
        let bind = if self.eat_sym("(") {
            let bind = self.name()?;
            self.sym(")")?;
            Some(bind)
        } else {
            None
        };
        Ok(Pattern::Case { case, bind })
    }
}

pub fn parse(tokens: Vec<Token>) -> Result<File, Problem> {
    Parser { tokens, at: 0 }.file()
}
