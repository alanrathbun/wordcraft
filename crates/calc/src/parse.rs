//! The calculation language parser: a lexer and a Pratt parser with explicit depth limits.
//!
//! Grammar, loosest binding first:
//!
//! ```text
//! statement := name [ "(" params ")" ] ":=" expr [ show ]      define (and optionally show)
//!            | expr show                                       evaluate and show
//! show      := "=" [ "?" ] [ unit text ]                       the rest of the text is the unit
//! expr      := term { ("+" | "-") term }
//! term      := unary { ("*" | "/" | adjacency) unary }
//! unary     := "-" unary | power                               so -x^2 is -(x^2)
//! power     := atom [ "^" unary ]                              right-associative
//! atom      := number | name | name"(" args ")" | "(" expr ")" | "|" expr "|" | "√" atom
//! ```
//!
//! The parser knows nothing about units or variables. `5 kN` is `5 * kN`; the evaluator decides
//! what `kN` is. Hostile input is bounded three ways: the source is at most [`MAX_SOURCE_BYTES`],
//! the token list at most [`MAX_TOKENS`], and both the parser's recursion and the height of the
//! tree it builds at most [`MAX_DEPTH`], so nothing downstream recurses deeply.

use crate::{CalcError, Expr, Op, Show, Stmt};

#[cfg(test)]
mod tests;

/// Largest source text accepted, in bytes.
const MAX_SOURCE_BYTES: usize = 16 * 1024;
/// Most tokens accepted.
const MAX_TOKENS: usize = 10_000;
/// Deepest nesting of expressions (parentheses, calls, `|…|`, prefix and `^` operands), and
/// tallest syntax tree (levels, counting the leaves), accepted: one more is `TooComplex`.
const MAX_DEPTH: usize = 64;

// Binding powers: (left, right) for infix operators; a higher number binds tighter.
const ADD_BP: (u8, u8) = (10, 11);
const MUL_BP: (u8, u8) = (20, 21);
/// Prefix minus: tighter than `*`, looser than `^`, so `-x^2` is `-(x^2)` and `-a*b` is `(-a)*b`.
const NEG_BP: u8 = 30;
/// `^` is right-associative: its right binding power is below its left.
const POW_BP: (u8, u8) = (40, 39);
/// The operand of `√` takes only an atom: `√x^2` is `(√x)^2`.
const SQRT_BP: u8 = 41;

/// Parse one statement.
///
/// # Errors
/// [`CalcError::Syntax`] when the text isn't a valid statement, and [`CalcError::TooComplex`]
/// when it exceeds the size, token or nesting limits.
pub fn parse(src: &str) -> Result<Stmt, CalcError> {
    if src.len() > MAX_SOURCE_BYTES {
        return Err(CalcError::TooComplex);
    }
    let toks = lex(src)?;
    match toks.iter().position(|t| t.kind == Kind::Define) {
        Some(at) => {
            let (name, params) = define_head(toks.get(..at).unwrap_or_default())?;
            let mut p = Parser::new(toks.get(at + 1..).unwrap_or_default());
            let body = p.root()?.expr;
            let show = p.finish(src)?;
            Ok(Stmt::Define { name, params, body, show })
        }
        None => {
            let mut p = Parser::new(&toks);
            let expr = p.root()?.expr;
            match p.finish(src)? {
                Some(show) => Ok(Stmt::Eval { expr, show }),
                None => Err(syntax("end with `=` to show the result, or use `:=` to define a name")),
            }
        }
    }
}

fn syntax(msg: impl Into<String>) -> CalcError {
    CalcError::Syntax(msg.into())
}

// ---------------------------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Num(f64),
    Name(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Bar,
    LParen,
    RParen,
    Comma,
    Sqrt,
    /// `:=` or `≔`
    Define,
    /// A lone `=`; the lexer stops here, and the text after it is the unit.
    Eq,
}

impl Kind {
    fn describe(&self) -> String {
        match self {
            Kind::Num(_) => "a number".into(),
            Kind::Name(n) => format!("`{n}`"),
            Kind::Plus => "`+`".into(),
            Kind::Minus => "`-`".into(),
            Kind::Star => "`*`".into(),
            Kind::Slash => "`/`".into(),
            Kind::Caret => "`^`".into(),
            Kind::Bar => "`|`".into(),
            Kind::LParen => "`(`".into(),
            Kind::RParen => "`)`".into(),
            Kind::Comma => "`,`".into(),
            Kind::Sqrt => "`√`".into(),
            Kind::Define => "`:=`".into(),
            Kind::Eq => "`=`".into(),
        }
    }
}

/// A token and the byte range of the source it came from.
#[derive(Debug, Clone, PartialEq)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
}

/// A name may start with a letter (any script, so Greek works) or one of the unit marks
/// (`°F`, `Δ°F`), so unit names lex as names.
fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || matches!(c, '°' | '∆' | 'Δ')
}

/// A name continues with letters, digits, `_` (the subscript) and the unit marks.
fn is_name_continue(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '°' | '∆' | 'Δ' | 'µ' | 'μ')
}

fn lex(src: &str) -> Result<Vec<Tok>, CalcError> {
    let mut toks: Vec<Tok> = Vec::new();
    let mut chars = src.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if toks.len() >= MAX_TOKENS {
            return Err(CalcError::TooComplex);
        }
        let mut end = start + c.len_utf8();
        let kind = match c {
            '+' => Kind::Plus,
            '-' | '\u{2212}' => Kind::Minus,
            '*' | '·' | '×' | '\u{22C5}' => Kind::Star,
            '/' | '÷' => Kind::Slash,
            '^' => Kind::Caret,
            '|' => Kind::Bar,
            '(' => Kind::LParen,
            ')' => Kind::RParen,
            ',' => Kind::Comma,
            '√' => Kind::Sqrt,
            '≔' => Kind::Define,
            ':' => match chars.next_if(|&(_, n)| n == '=') {
                Some((i, n)) => {
                    end = i + n.len_utf8();
                    Kind::Define
                }
                None => return Err(syntax("unexpected character `:` (did you mean `:=`?)")),
            },
            '=' => {
                toks.push(Tok { kind: Kind::Eq, start, end });
                return Ok(toks);
            }
            c if c.is_ascii_digit() || (c == '.' && chars.peek().is_some_and(|&(_, n)| n.is_ascii_digit())) => {
                end = lex_number(src, start, &mut chars);
                let text = src.get(start..end).unwrap_or_default();
                if chars.peek().is_some_and(|&(_, n)| n == '.') {
                    return Err(syntax(format!("`{text}.` isn't a valid number")));
                }
                match text.parse::<f64>() {
                    Ok(v) if v.is_finite() => Kind::Num(v),
                    _ => return Err(syntax(format!("`{text}` isn't a valid number"))),
                }
            }
            c if is_name_start(c) => {
                while let Some((i, n)) = chars.next_if(|&(_, n)| is_name_continue(n)) {
                    end = i + n.len_utf8();
                }
                Kind::Name(src.get(start..end).unwrap_or_default().to_string())
            }
            other => return Err(syntax(format!("unexpected character `{}`", other.escape_debug()))),
        };
        toks.push(Tok { kind, start, end });
    }
    Ok(toks)
}

type Chars<'a> = std::iter::Peekable<std::str::CharIndices<'a>>;

/// Consume ASCII digits, advancing `end` past them.
fn eat_digits(chars: &mut Chars<'_>, end: &mut usize) {
    while let Some((i, d)) = chars.next_if(|&(_, n)| n.is_ascii_digit()) {
        *end = i + d.len_utf8();
    }
}

/// Consume the rest of a number that starts at `start` (a digit, or a `.` before a digit) and
/// return its end offset: digits, an optional fraction, an optional exponent. An exponent marker
/// only counts when digits follow it, so `2eV` is `2` then the name `eV`.
fn lex_number(src: &str, start: usize, chars: &mut Chars<'_>) -> usize {
    let mut end = start + 1;
    eat_digits(chars, &mut end);
    if chars.next_if(|&(_, n)| n == '.').is_some() {
        end += 1;
        eat_digits(chars, &mut end);
    }
    // Look ahead in the source for a complete exponent before consuming any of it.
    let mut look = src.get(end..).unwrap_or_default().chars();
    if matches!(look.next(), Some('e' | 'E')) {
        let digit_follows = match look.next() {
            Some('+' | '-') => look.next().is_some_and(|d| d.is_ascii_digit()),
            Some(d) => d.is_ascii_digit(),
            None => false,
        };
        if digit_follows {
            chars.next(); // the `e`
            end += 1;
            if let Some((i, sign)) = chars.next_if(|&(_, n)| n == '+' || n == '-') {
                end = i + sign.len_utf8();
            }
            eat_digits(chars, &mut end);
        }
    }
    end
}

// ---------------------------------------------------------------------------------------------
// Statement head
// ---------------------------------------------------------------------------------------------

/// The tokens before `:=` must be `name` or `name(param, ...)`.
fn define_head(head: &[Tok]) -> Result<(String, Vec<String>), CalcError> {
    let bad = || syntax("`:=` must follow a name, or a function like f(x, y)");
    let mut it = head.iter();
    let Some(Tok { kind: Kind::Name(name), .. }) = it.next() else {
        return Err(bad());
    };
    let mut params: Vec<String> = Vec::new();
    match it.next() {
        None => return Ok((name.clone(), params)),
        Some(Tok { kind: Kind::LParen, .. }) => {}
        Some(_) => return Err(bad()),
    }
    loop {
        match it.next() {
            Some(Tok { kind: Kind::Name(p), .. }) => {
                if params.contains(p) {
                    return Err(syntax(format!("parameter `{p}` is repeated")));
                }
                params.push(p.clone());
            }
            _ => return Err(syntax("a function's parameters must be names, like f(x, y)")),
        }
        match it.next() {
            Some(Tok { kind: Kind::Comma, .. }) => {}
            Some(Tok { kind: Kind::RParen, .. }) => break,
            _ => return Err(syntax("a function's parameters must be names separated by commas")),
        }
    }
    match it.next() {
        None => Ok((name.clone(), params)),
        Some(_) => Err(bad()),
    }
}

// ---------------------------------------------------------------------------------------------
// Expression parser
// ---------------------------------------------------------------------------------------------

/// An expression with the height of its tree.
struct Node {
    expr: Expr,
    height: usize,
}

impl Node {
    fn leaf(expr: Expr) -> Self {
        Node { expr, height: 1 }
    }

    /// A node over `children`, rejecting a tree taller than [`MAX_DEPTH`].
    fn over(children_height: usize, expr: Expr) -> Result<Self, CalcError> {
        let height = children_height + 1;
        if height > MAX_DEPTH { Err(CalcError::TooComplex) } else { Ok(Node { expr, height }) }
    }
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
    /// Current recursion depth of [`Parser::expr`].
    depth: usize,
    /// Whether a `|` ends the current absolute value instead of starting a new factor.
    bar_closes: bool,
}

impl<'a> Parser<'a> {
    fn new(toks: &'a [Tok]) -> Self {
        Parser { toks, pos: 0, depth: 0, bar_closes: false }
    }

    fn peek(&self) -> Option<&'a Tok> {
        self.toks.get(self.pos)
    }

    fn peek_kind(&self) -> Option<&'a Kind> {
        self.peek().map(|t| &t.kind)
    }

    fn bump(&mut self) -> Option<&'a Tok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// After the expression: the end of the statement, or `=` with the unit text after it.
    fn finish(&mut self, src: &str) -> Result<Option<Show>, CalcError> {
        match self.bump() {
            None => Ok(None),
            Some(Tok { kind: Kind::Eq, end, .. }) => {
                let rest = src.get(*end..).unwrap_or_default().trim();
                let unit = rest.strip_prefix('?').unwrap_or(rest).trim();
                Ok(Some(Show { unit: if unit.is_empty() { None } else { Some(unit.to_string()) } }))
            }
            Some(t) => Err(syntax(format!("unexpected {}", t.kind.describe()))),
        }
    }

    fn expect(&mut self, want: &Kind) -> Result<(), CalcError> {
        match self.bump() {
            Some(t) if t.kind == *want => Ok(()),
            Some(t) => Err(syntax(format!("expected {} but found {}", want.describe(), t.kind.describe()))),
            None => Err(syntax(format!("expected {}", want.describe()))),
        }
    }

    /// Parse the whole expression of a statement. It isn't counted as nesting.
    fn root(&mut self) -> Result<Node, CalcError> {
        self.expr_inner(0)
    }

    /// Parse an expression whose operators all bind at least as tightly as `min_bp`.
    fn expr(&mut self, min_bp: u8) -> Result<Node, CalcError> {
        self.depth += 1;
        let r = if self.depth > MAX_DEPTH { Err(CalcError::TooComplex) } else { self.expr_inner(min_bp) };
        self.depth -= 1;
        r
    }

    fn expr_inner(&mut self, min_bp: u8) -> Result<Node, CalcError> {
        let mut lhs = self.prefix()?;
        loop {
            let (op, (left_bp, right_bp), implicit) = match self.peek_kind() {
                Some(Kind::Plus) => (Op::Add, ADD_BP, false),
                Some(Kind::Minus) => (Op::Sub, ADD_BP, false),
                Some(Kind::Star) => (Op::Mul, MUL_BP, false),
                Some(Kind::Slash) => (Op::Div, MUL_BP, false),
                Some(Kind::Caret) => (Op::Pow, POW_BP, false),
                Some(Kind::Num(_) | Kind::Name(_) | Kind::LParen | Kind::Sqrt) => (Op::Mul, MUL_BP, true),
                Some(Kind::Bar) if !self.bar_closes => (Op::Mul, MUL_BP, true),
                _ => break,
            };
            if left_bp < min_bp {
                break;
            }
            if !implicit {
                self.pos += 1;
            }
            let rhs = self.expr(right_bp)?;
            lhs = Node::over(lhs.height.max(rhs.height), Expr::Binary(op, Box::new(lhs.expr), Box::new(rhs.expr)))?;
        }
        Ok(lhs)
    }

    fn prefix(&mut self) -> Result<Node, CalcError> {
        let Some(tok) = self.bump() else {
            return Err(syntax("the expression ends too soon"));
        };
        match &tok.kind {
            Kind::Num(v) => Ok(Node::leaf(Expr::Num(*v))),
            Kind::Name(n) => {
                // `f(` with no space is a call; `f (` is `f` times a parenthesised factor.
                match self.peek() {
                    Some(next) if next.kind == Kind::LParen && next.start == tok.end => {
                        self.pos += 1;
                        self.call(n.clone())
                    }
                    _ => Ok(Node::leaf(Expr::Name(n.clone()))),
                }
            }
            Kind::LParen => {
                let outer = std::mem::replace(&mut self.bar_closes, false);
                let inner = self.expr(0);
                self.bar_closes = outer;
                let inner = inner?;
                self.expect(&Kind::RParen)?;
                Ok(inner)
            }
            Kind::Bar => {
                let outer = std::mem::replace(&mut self.bar_closes, true);
                let inner = self.expr(0);
                self.bar_closes = outer;
                let inner = inner?;
                self.expect(&Kind::Bar)?;
                Node::over(inner.height, Expr::Abs(Box::new(inner.expr)))
            }
            Kind::Minus => {
                let operand = self.expr(NEG_BP)?;
                Node::over(operand.height, Expr::Unary(Op::Neg, Box::new(operand.expr)))
            }
            Kind::Plus => self.expr(NEG_BP),
            Kind::Sqrt => {
                let operand = self.expr(SQRT_BP)?;
                Node::over(operand.height, Expr::Call("sqrt".to_string(), vec![operand.expr]))
            }
            other => Err(syntax(format!("expected a value but found {}", other.describe()))),
        }
    }

    /// The arguments of a call, after its `(`.
    fn call(&mut self, name: String) -> Result<Node, CalcError> {
        let outer = std::mem::replace(&mut self.bar_closes, false);
        let args = self.args();
        self.bar_closes = outer;
        let (args, height) = args?;
        Node::over(height, Expr::Call(name, args))
    }

    fn args(&mut self) -> Result<(Vec<Expr>, usize), CalcError> {
        let mut args = Vec::new();
        let mut height = 0;
        if self.peek_kind() == Some(&Kind::RParen) {
            self.pos += 1;
            return Ok((args, height));
        }
        loop {
            let arg = self.expr(0)?;
            height = height.max(arg.height);
            args.push(arg.expr);
            match self.bump() {
                Some(Tok { kind: Kind::Comma, .. }) => {}
                Some(Tok { kind: Kind::RParen, .. }) => return Ok((args, height)),
                Some(t) => return Err(syntax(format!("expected `,` or `)` but found {}", t.kind.describe()))),
                None => return Err(syntax("expected `)`")),
            }
        }
    }
}
