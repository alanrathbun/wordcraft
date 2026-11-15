//! The calculation language parser: a lexer and a Pratt parser with explicit depth limits.
//!
//! Grammar, loosest binding first:
//!
//! ```text
//! statement := head ":=" body [ show ]                          define (and optionally show)
//!            | expr show                                       evaluate and show
//! head      := name | name "(" params ")" | name"[" name "]"   variable, function, indexed
//! body      := range | expr
//! range     := expr [ "," expr ] ".." expr                    first, second..last; `..` loosest
//! show      := "=" [ "?" ] [ unit text ]                       the rest of the text is the unit
//! expr      := term { ("+" | "-") term }
//! term      := unary { ("*" | "/" | adjacency) unary }
//! unary     := "-" unary | power                               so -x^2 is -(x^2)
//! power     := atom [ "^" unary ]                              right-associative
//! atom      := number | name | name"(" args ")" | name"[" args "]" | "(" expr ")"
//!            | "[" rows "]" | "|" expr "|" | "√" atom
//! rows      := items { ";" items }                             every row the same length
//! items     := expr { "," expr }
//! postfix   := atom { "²" | "³" | "⁻¹" ... }                   one superscript digit, like `^`
//! ```
//!
//! A `(` or `[` right after a name, with no space, is a call or an index (`f(x)`, `D[2]`); after
//! a space it starts a factor (`D [2]` is `D` times a one-element literal). A literal with no `;`
//! is a column (`[1, 2, 3]` is 3 rows × 1). A range (`0..5`, `1, 1.5..3`) can only be the whole
//! right side of a definition, so its commas are never confused with list or argument commas.
//!
//! Adjacency (implicit multiplication) groups exactly like `*`, left to right, so `1/2 x` is
//! `(1/2) x` and not `1/(2 x)` (the Mathcad convention). A number directly after a number, name
//! or closing bracket (`1 500`, `x 2`) is an error rather than a product.
//!
//! The parser knows nothing about units or variables. `5 kN` is `5 * kN`; the evaluator decides
//! what `kN` is. Hostile input is bounded three ways: the source is at most [`MAX_SOURCE_BYTES`],
//! the token list at most [`MAX_TOKENS`], the parser's recursion (nesting) at most [`MAX_DEPTH`] and
//! the height of the tree it builds at most [`MAX_HEIGHT`], so nothing downstream recurses deeply.

use crate::{CalcError, Expr, Op, Show, Stmt};

#[cfg(test)]
mod tests;

/// Largest source text accepted, in bytes.
const MAX_SOURCE_BYTES: usize = 16 * 1024;
/// Most tokens accepted.
const MAX_TOKENS: usize = 10_000;
/// Deepest nesting of expressions (parentheses, calls, literals, indexes, `|…|`, prefix signs and
/// `^` operands) accepted: one more is `TooComplex`.
const MAX_DEPTH: usize = 64;
/// Tallest syntax tree (levels, counting the leaves) accepted. A flat sum of `n` terms is `n`
/// levels tall, so this also bounds how long a sum or product can be.
const MAX_HEIGHT: usize = 1024;

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
            let (name, index, params) = define_head(toks.get(..at).unwrap_or_default())?;
            let mut p = Parser::new(toks.get(at + 1..).unwrap_or_default());
            let body = p.body()?.expr;
            let show = p.finish(src)?;
            Ok(Stmt::Define { name, index, params, body, show })
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

/// A `..` anywhere but between the ends of a definition's whole right side.
fn misplaced_range() -> CalcError {
    syntax("a range like 0..5 can only be the whole right side of a definition")
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
    LBracket,
    RBracket,
    Comma,
    /// Separates the rows of a literal.
    Semi,
    /// `..`, between the ends of a range.
    DotDot,
    Sqrt,
    /// A superscript exponent such as `²` or `⁻¹`, already decoded.
    Sup(i32),
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
            Kind::LBracket => "`[`".into(),
            Kind::RBracket => "`]`".into(),
            Kind::Comma => "`,`".into(),
            Kind::Semi => "`;`".into(),
            Kind::DotDot => "`..`".into(),
            Kind::Sqrt => "`√`".into(),
            Kind::Sup(_) => "a superscript".into(),
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
    (c.is_alphanumeric() && superscript_digit(c).is_none()) || matches!(c, '_' | '°' | '∆' | 'Δ' | 'µ' | 'μ')
}

/// The value of a superscript digit (`⁰`..`⁹`). These are not name characters: `x²` is `x^2`.
fn superscript_digit(c: char) -> Option<i32> {
    match c {
        '⁰' => Some(0),
        '¹' => Some(1),
        '²' => Some(2),
        '³' => Some(3),
        '⁴' => Some(4),
        '⁵' => Some(5),
        '⁶' => Some(6),
        '⁷' => Some(7),
        '⁸' => Some(8),
        '⁹' => Some(9),
        _ => None,
    }
}

/// A run of superscript characters is one optional `⁻` and exactly one digit.
fn decode_superscript(run: &[char]) -> Result<i32, CalcError> {
    let (negative, digits) = match run.split_first() {
        Some(('⁻', rest)) => (true, rest),
        _ => (false, run),
    };
    match digits {
        [d] => match superscript_digit(*d) {
            Some(v) => Ok(if negative { -v } else { v }),
            None => Err(syntax("a superscript needs a digit")),
        },
        _ => Err(syntax("a superscript exponent must be a single digit (use `^` for others)")),
    }
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
            '-' | '\u{2212}' | '\u{2013}' => Kind::Minus,
            '*' | '·' | '×' | '\u{22C5}' => Kind::Star,
            '/' | '÷' => Kind::Slash,
            '^' => Kind::Caret,
            '|' => Kind::Bar,
            '(' => Kind::LParen,
            ')' => Kind::RParen,
            '[' => Kind::LBracket,
            ']' => Kind::RBracket,
            ',' => Kind::Comma,
            ';' => Kind::Semi,
            '√' => Kind::Sqrt,
            '≔' => Kind::Define,
            ':' => match chars.next_if(|&(_, n)| n == '=') {
                Some((i, n)) => {
                    end = i + n.len_utf8();
                    Kind::Define
                }
                None => return Err(syntax("unexpected character `:` (did you mean `:=`?)")),
            },
            c if superscript_digit(c).is_some() || c == '⁻' => {
                let mut run = vec![c];
                while let Some((i, n)) = chars.next_if(|&(_, n)| superscript_digit(n).is_some() || n == '⁻') {
                    end = i + n.len_utf8();
                    run.push(n);
                }
                Kind::Sup(decode_superscript(&run)?)
            }
            '=' => {
                toks.push(Tok { kind: Kind::Eq, start, end });
                return Ok(toks);
            }
            '.' if chars.peek().is_some_and(|&(_, n)| n == '.') => {
                chars.next();
                end += 1;
                if chars.peek().is_some_and(|&(_, n)| n == '.') {
                    return Err(syntax("`...` isn't valid; a range is written like 0..5"));
                }
                Kind::DotDot
            }
            c if c.is_ascii_digit() || (c == '.' && chars.peek().is_some_and(|&(_, n)| n.is_ascii_digit())) => {
                end = lex_number(src, start, &mut chars);
                let text = src.get(start..end).unwrap_or_default();
                // A second `.` is an error, unless it starts a range's `..` (`1.5..3`).
                if chars.peek().is_some_and(|&(_, n)| n == '.') && !starts_range(src, end) {
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

/// Whether the source at `at` is a range's `..`.
fn starts_range(src: &str, at: usize) -> bool {
    src.get(at..).is_some_and(|rest| rest.starts_with(".."))
}

/// Consume ASCII digits, advancing `end` past them.
fn eat_digits(chars: &mut Chars<'_>, end: &mut usize) {
    while let Some((i, d)) = chars.next_if(|&(_, n)| n.is_ascii_digit()) {
        *end = i + d.len_utf8();
    }
}

/// Consume the rest of a number that starts at `start` (a digit, or a `.` before a digit) and
/// return its end offset: digits, an optional fraction, an optional exponent. An exponent marker
/// only counts when digits follow it, so `2eV` is `2` then the name `eV`. A number stops before a
/// range's `..`, so `0..5` is `0`, `..`, `5` and never `0.` and `.5`.
fn lex_number(src: &str, start: usize, chars: &mut Chars<'_>) -> usize {
    let mut end = start + 1;
    eat_digits(chars, &mut end);
    if !starts_range(src, end) && chars.next_if(|&(_, n)| n == '.').is_some() {
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

/// The tokens before `:=` must be `name`, `name(param, ...)` or `name[i]`: the name, the range
/// variable of an indexed definition, and the parameters.
fn define_head(head: &[Tok]) -> Result<(String, Option<String>, Vec<String>), CalcError> {
    let bad = || syntax("`:=` must follow a name, or a function like f(x, y)");
    let mut it = head.iter();
    let Some(Tok { kind: Kind::Name(name), end: name_end, .. }) = it.next() else {
        return Err(bad());
    };
    let mut params: Vec<String> = Vec::new();
    match it.next() {
        None => return Ok((name.clone(), None, params)),
        Some(Tok { kind: Kind::LParen, .. }) => {}
        Some(Tok { kind: Kind::LBracket, start, .. }) if start == name_end => {
            return match (it.next(), it.next(), it.next()) {
                (Some(Tok { kind: Kind::Name(i), .. }), Some(Tok { kind: Kind::RBracket, .. }), None) => Ok((name.clone(), Some(i.clone()), params)),
                _ => Err(syntax("an indexed definition takes one range variable, like t[i] := 2·D[i]")),
            };
        }
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
        None => Ok((name.clone(), None, params)),
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

    /// A node over `children`, rejecting a tree taller than [`MAX_HEIGHT`].
    fn over(children_height: usize, expr: Expr) -> Result<Self, CalcError> {
        let height = children_height + 1;
        if height > MAX_HEIGHT { Err(CalcError::TooComplex) } else { Ok(Node { expr, height }) }
    }
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
    /// Current recursion depth of [`Parser::expr`].
    depth: usize,
    /// Whether a `|` ends the current absolute value instead of starting a new factor.
    bar_closes: bool,
    /// Whether a `..` ends the current expression, as it ends a value of a range at the top of
    /// a definition's right side; anywhere else it is an error.
    in_range_end: bool,
}

impl<'a> Parser<'a> {
    fn new(toks: &'a [Tok]) -> Self {
        Parser { toks, pos: 0, depth: 0, bar_closes: false, in_range_end: false }
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

    /// Parse the right side of a definition: a range, or else an ordinary expression. `..` is the
    /// loosest operator and is only accepted here, at the top: each value of a range is a whole
    /// expression (`0..n-1`, `1, 1+0.5..n`).
    fn body(&mut self) -> Result<Node, CalcError> {
        let first = self.range_value()?;
        let second = match self.peek_kind() {
            Some(Kind::Comma) => {
                self.pos += 1;
                let second = self.range_value()?;
                if self.peek_kind() != Some(&Kind::DotDot) {
                    return Err(syntax("a list needs brackets, like [1, 2]; a range with a step is written like 1, 1.5..3"));
                }
                Some(second)
            }
            Some(Kind::DotDot) => None,
            _ => return Ok(first),
        };
        self.pos += 1; // the `..`
        self.range_rhs(first, second)
    }

    /// The rest of a range after its `..`: the last value, then the end of the statement or its
    /// `=`, since a range is the whole right side.
    fn range_rhs(&mut self, first: Node, second: Option<Node>) -> Result<Node, CalcError> {
        let last = self.range_value()?;
        if self.peek_kind() == Some(&Kind::DotDot) {
            return Err(misplaced_range());
        }
        let height = first.height.max(last.height).max(second.as_ref().map_or(0, |s| s.height));
        let range = Expr::Range { first: Box::new(first.expr), second: second.map(|s| Box::new(s.expr)), last: Box::new(last.expr) };
        Node::over(height, range)
    }

    /// One value of a range: a whole expression ended by a `..` at its top level (not inside a
    /// bracket). Like the whole expression of a statement, it isn't counted as nesting; with no
    /// `..` after it, it is that expression.
    fn range_value(&mut self) -> Result<Node, CalcError> {
        let outer = std::mem::replace(&mut self.in_range_end, true);
        let r = self.expr_inner(0);
        self.in_range_end = outer;
        r
    }

    /// Run `f` inside a bracket, where a `|` ends an absolute value only when `bar_closes` and a
    /// `..` never ends a range's value.
    fn bracketed<T>(&mut self, bar_closes: bool, f: impl FnOnce(&mut Self) -> Result<T, CalcError>) -> Result<T, CalcError> {
        let outer_bar = std::mem::replace(&mut self.bar_closes, bar_closes);
        let outer_range = std::mem::replace(&mut self.in_range_end, false);
        let r = f(self);
        self.bar_closes = outer_bar;
        self.in_range_end = outer_range;
        r
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
            if let Some(&Tok { kind: Kind::Sup(exp), start, .. }) = self.peek() {
                // A superscript applies to the factor just before it, like `^`, and must touch it.
                let touches = self.pos.checked_sub(1).and_then(|i| self.toks.get(i)).is_some_and(|prev| prev.end == start);
                if !touches || POW_BP.0 < min_bp {
                    break;
                }
                self.pos += 1;
                let magnitude = Expr::Num(f64::from(exp.unsigned_abs()));
                let exponent = if exp < 0 { Node::over(1, Expr::Unary(Op::Neg, Box::new(magnitude)))? } else { Node::leaf(magnitude) };
                lhs = Node::over(lhs.height.max(exponent.height), Expr::Binary(Op::Pow, Box::new(lhs.expr), Box::new(exponent.expr)))?;
                continue;
            }
            let (op, (left_bp, right_bp), implicit) = match self.peek_kind() {
                Some(Kind::Plus) => (Op::Add, ADD_BP, false),
                Some(Kind::Minus) => (Op::Sub, ADD_BP, false),
                Some(Kind::Star) => (Op::Mul, MUL_BP, false),
                Some(Kind::Slash) => (Op::Div, MUL_BP, false),
                Some(Kind::Caret) => (Op::Pow, POW_BP, false),
                Some(Kind::DotDot) if !self.in_range_end => return Err(misplaced_range()),
                _ if self.starts_factor() => (Op::Mul, MUL_BP, true),
                _ => break,
            };
            if left_bp < min_bp {
                break;
            }
            if implicit {
                self.check_adjacent()?;
            } else {
                self.pos += 1;
            }
            let rhs = self.expr(right_bp)?;
            lhs = Node::over(lhs.height.max(rhs.height), Expr::Binary(op, Box::new(lhs.expr), Box::new(rhs.expr)))?;
        }
        Ok(lhs)
    }

    /// The token just consumed.
    fn prev(&self) -> Option<&'a Tok> {
        self.pos.checked_sub(1).and_then(|i| self.toks.get(i))
    }

    /// Whether the token just consumed ends a value: a number, name, closing bracket or bar, or a
    /// superscript.
    fn after_value(&self) -> bool {
        self.prev().is_some_and(|t| matches!(t.kind, Kind::Num(_) | Kind::Name(_) | Kind::RParen | Kind::RBracket | Kind::Bar | Kind::Sup(_)))
    }

    /// Whether the next token starts a factor, which after a value is an implied product.
    fn starts_factor(&self) -> bool {
        match self.peek_kind() {
            Some(Kind::Num(_) | Kind::Name(_) | Kind::LParen | Kind::LBracket | Kind::Sqrt) => true,
            Some(Kind::Bar) => !self.bar_closes,
            _ => false,
        }
    }

    /// Before the next factor of an implied product: a number straight after a value is refused,
    /// and so is a `[` touching anything but a name (a name took it as an index already).
    fn check_adjacent(&self) -> Result<(), CalcError> {
        match self.peek() {
            Some(Tok { kind: Kind::Num(_), .. }) if self.after_value() => Err(syntax("two values in a row \u{2014} did you mean `*`?")),
            Some(Tok { kind: Kind::LBracket, start, .. }) if self.prev().is_some_and(|p| p.end == *start) => {
                Err(syntax("only a name can be indexed, like D[2]"))
            }
            _ => Ok(()),
        }
    }

    fn prefix(&mut self) -> Result<Node, CalcError> {
        let Some(tok) = self.bump() else {
            return Err(syntax("the expression ends too soon"));
        };
        match &tok.kind {
            Kind::Num(v) => Ok(Node::leaf(Expr::Num(*v))),
            Kind::Name(n) => {
                // `f(` with no space is a call and `D[` an index; `f (` is `f` times a
                // parenthesised factor and `D [` is `D` times a literal.
                match self.peek() {
                    Some(next) if next.kind == Kind::LParen && next.start == tok.end => {
                        self.pos += 1;
                        self.call(n.clone())
                    }
                    Some(next) if next.kind == Kind::LBracket && next.start == tok.end => {
                        self.pos += 1;
                        self.index(n.clone())
                    }
                    _ => Ok(Node::leaf(Expr::Name(n.clone()))),
                }
            }
            Kind::LParen => {
                let inner = self.bracketed(false, |p| p.expr(0))?;
                self.expect(&Kind::RParen)?;
                Ok(inner)
            }
            Kind::LBracket => self.bracketed(false, Self::literal),
            Kind::Bar => {
                let inner = self.bracketed(true, |p| p.expr(0))?;
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
        let (args, height) = self.bracketed(false, |p| p.list(&Kind::RParen))?;
        Node::over(height, Expr::Call(name, args))
    }

    /// The indexes of `name[…]`, after its `[`: one, or a row and a column.
    fn index(&mut self, name: String) -> Result<Node, CalcError> {
        let (at, height) = self.bracketed(false, |p| p.list(&Kind::RBracket))?;
        if !(1..=2).contains(&at.len()) {
            return Err(syntax("an index is one number, or a row and a column, like D[2] or M[1, 0]"));
        }
        Node::over(height, Expr::Index(name, at))
    }

    /// Expressions separated by commas up to `close`, which is consumed, and their tallest height.
    fn list(&mut self, close: &Kind) -> Result<(Vec<Expr>, usize), CalcError> {
        let mut items = Vec::new();
        let mut height = 0;
        if self.peek_kind() == Some(close) {
            self.pos += 1;
            return Ok((items, height));
        }
        loop {
            let item = self.expr(0)?;
            height = height.max(item.height);
            items.push(item.expr);
            match self.bump() {
                Some(Tok { kind: Kind::Comma, .. }) => {}
                Some(t) if t.kind == *close => return Ok((items, height)),
                Some(t) => return Err(syntax(format!("expected `,` or {} but found {}", close.describe(), t.kind.describe()))),
                None => return Err(syntax(format!("expected {}", close.describe()))),
            }
        }
    }

    /// A vector or matrix literal, after its `[`: elements separated by `,`, rows by `;`, every
    /// row as long as the first. A single row is a column vector.
    fn literal(&mut self) -> Result<Node, CalcError> {
        if self.peek_kind() == Some(&Kind::RBracket) {
            return Err(syntax("a matrix needs at least one element, like [1, 2]"));
        }
        let mut items = Vec::new();
        let mut height = 0;
        let mut rows = 0;
        let mut cols = 0;
        let mut in_row = 0;
        loop {
            let item = self.expr(0)?;
            if matches!(item.expr, Expr::Matrix { .. }) {
                return Err(syntax("a matrix element can't be another matrix; use `;` to start a new row"));
            }
            height = height.max(item.height);
            items.push(item.expr);
            in_row += 1;
            let closes = match self.bump() {
                Some(Tok { kind: Kind::Comma, .. }) => continue,
                Some(Tok { kind: Kind::Semi, .. }) => false,
                Some(Tok { kind: Kind::RBracket, .. }) => true,
                Some(t) => return Err(syntax(format!("expected `,`, `;` or `]` but found {}", t.kind.describe()))),
                None => return Err(syntax("expected `]`")),
            };
            rows += 1;
            if rows == 1 {
                cols = in_row;
            } else if in_row != cols {
                let noun = if in_row == 1 { "element" } else { "elements" };
                return Err(syntax(format!("row {rows} has {in_row} {noun}; row 1 has {cols}")));
            }
            in_row = 0;
            if closes {
                break;
            }
        }
        let (rows, cols) = if rows == 1 { (cols, 1) } else { (rows, cols) };
        Node::over(height, Expr::Matrix { rows, cols, items })
    }
}
