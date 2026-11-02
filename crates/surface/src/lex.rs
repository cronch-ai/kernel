//! Tokens and where they are.
//!
//! A declaration ends at a newline, except inside brackets, braces and parentheses, which group across lines. A `#`
//! starts a comment that runs to the end of the line.

use crate::{Problem, What};
use serde::Serialize;

/// A place in the source: a line and a column, both counted from 1, the column in characters.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub line: u32,
    pub column: u32,
}

/// The source from `start` up to, not including, `end`.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: Pos,
    pub end: Pos,
}

impl Span {
    /// From the start of this span to the end of that one.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start,
            end: other.end,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Str(String),
    Int(i64),
    Float(f64),
    Sym(&'static str),
    Newline,
    Eof,
}

impl Tok {
    /// How a problem names it.
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Str(_) => "a string".to_string(),
            Tok::Int(_) | Tok::Float(_) => "a number".to_string(),
            Tok::Sym(s) => format!("`{s}`"),
            Tok::Newline => "the end of the line".to_string(),
            Tok::Eof => "the end of the file".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// Longest first, so `==` is never read as two `=`.
const SYMBOLS: [&str; 25] = [
    "->", "=>", "==", "!=", "<=", ">=", "//", "(", ")", "[", "]", "{", "}", ",", ":", ".", "=",
    "<", ">", "+", "-", "*", "/", "%", "|",
];

struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    pos: Pos,
    tokens: Vec<Token>,
    depth: usize,
}

impl Lexer<'_> {
    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        if c == '\n' {
            self.pos.line += 1;
            self.pos.column = 1;
        } else {
            self.pos.column += 1;
        }
        Some(c)
    }

    fn push(&mut self, tok: Tok, start: Pos) {
        self.tokens.push(Token {
            tok,
            span: Span {
                start,
                end: self.pos,
            },
        });
    }

    fn problem(&self, start: Pos, expected: &str, found: &str) -> Problem {
        Problem {
            span: Span {
                start,
                end: self.pos,
            },
            what: What::Syntax {
                expected: expected.to_string(),
                found: found.to_string(),
            },
        }
    }

    fn string(&mut self, start: Pos) -> Result<Tok, Problem> {
        let mut out = String::new();
        loop {
            match self.bump() {
                Some('"') => return Ok(Tok::Str(out)),
                Some('\\') => {
                    let escaped = match self.bump() {
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some('"') => '"',
                        Some('\\') => '\\',
                        other => {
                            let found = other
                                .map_or("the end of the file".to_string(), |c| format!("`\\{c}`"));
                            return Err(self.problem(
                                start,
                                "an escape: \\n \\t \\r \\\" \\\\",
                                &found,
                            ));
                        }
                    };
                    out.push(escaped);
                }
                Some('\n') | None => {
                    return Err(self.problem(
                        start,
                        "`\"` to close the string",
                        "the end of the line",
                    ));
                }
                Some(c) => out.push(c),
            }
        }
    }

    fn number(&mut self, start: Pos, first: char) -> Result<Tok, Problem> {
        let mut text = first.to_string();
        while let Some(c) = self.peek().filter(char::is_ascii_digit) {
            text.push(c);
            self.bump();
        }
        // A `.` makes a float only when a digit follows: `xs.0` isn't a thing, but `1.5` is.
        let mut lookahead = self.chars.clone();
        if lookahead.next() == Some('.') && lookahead.next().is_some_and(|c| c.is_ascii_digit()) {
            text.push('.');
            self.bump();
            while let Some(c) = self.peek().filter(char::is_ascii_digit) {
                text.push(c);
                self.bump();
            }
            return text
                .parse()
                .map(Tok::Float)
                .map_err(|_| self.problem(start, "a number", &text));
        }
        text.parse()
            .map(Tok::Int)
            .map_err(|_| self.problem(start, "an integer that fits in 64 bits", &text))
    }

    fn run(mut self) -> Result<Vec<Token>, Problem> {
        while let Some(c) = self.peek() {
            let start = self.pos;
            match c {
                '\n' => {
                    self.bump();
                    let after_newline = matches!(
                        self.tokens.last(),
                        None | Some(Token {
                            tok: Tok::Newline,
                            ..
                        })
                    );
                    if self.depth == 0 && !after_newline {
                        self.push(Tok::Newline, start);
                    }
                }
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '#' => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                '"' => {
                    self.bump();
                    let tok = self.string(start)?;
                    self.push(tok, start);
                }
                c if c.is_ascii_digit() => {
                    self.bump();
                    let tok = self.number(start, c)?;
                    self.push(tok, start);
                }
                c if c.is_alphabetic() || c == '_' => {
                    let mut name = String::new();
                    while let Some(c) = self.peek().filter(|c| c.is_alphanumeric() || *c == '_') {
                        name.push(c);
                        self.bump();
                    }
                    self.push(Tok::Ident(name), start);
                }
                _ => {
                    let rest: String = self.chars.clone().take(2).collect();
                    let Some(sym) = SYMBOLS.iter().find(|s| rest.starts_with(**s)) else {
                        self.bump();
                        return Err(self.problem(start, "a token", &format!("`{c}`")));
                    };
                    for _ in 0..sym.len() {
                        self.bump();
                    }
                    match *sym {
                        "(" | "[" | "{" => self.depth += 1,
                        ")" | "]" | "}" => self.depth = self.depth.saturating_sub(1),
                        _ => {}
                    }
                    self.push(Tok::Sym(sym), start);
                }
            }
        }
        let start = self.pos;
        if !matches!(
            self.tokens.last(),
            None | Some(Token {
                tok: Tok::Newline,
                ..
            })
        ) {
            self.push(Tok::Newline, start);
        }
        self.push(Tok::Eof, start);
        Ok(self.tokens)
    }
}

/// The tokens of a source, ending with a newline and `Eof`, or the first thing that isn't a token.
pub fn lex(source: &str) -> Result<Vec<Token>, Problem> {
    Lexer {
        chars: source.chars().peekable(),
        pos: Pos { line: 1, column: 1 },
        tokens: Vec::new(),
        depth: 0,
    }
    .run()
}
