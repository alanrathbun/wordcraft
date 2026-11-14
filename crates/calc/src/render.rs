//! The displayed form of a calculation: its linear-format (UnicodeMath) text and math tree.
//!
//! The display is generated from the parsed statement, never copied from the source text, so the
//! structure on screen is the structure that was evaluated: wherever UnicodeMath would group the
//! text differently from the calculation parser, the display carries explicit parentheses
//! (`√x²` parses as `(√x)²` and shows as `(√x)^2`). Units are upright text, quoted in the linear
//! form (`1500 "lbf"`). The result follows: `=600 "psi"` when current, a grey `≈600 "psi"` when
//! stale, and a red ` ⚠` instead of any number when the calculation has an error.

use std::collections::HashMap;

use craft_units::{Formatted, lookup};
use wordcraft_doc::Rgb;
use wordcraft_doc::calc::CalcState;
use wordcraft_doc::math::{Arg, MNode, MRun, MSty, parse_linear};
use wordcraft_doc::math_linear::map_args;

use crate::{CalcError, Expr, Op, Stmt, parse};

/// Colour of the error marker.
pub const ERROR_RED: Rgb = Rgb(0xC0, 0x00, 0x00);
/// Colour of a stale result.
pub const STALE_GREY: Rgb = Rgb(0x80, 0x80, 0x80);
/// The marker shown after a calculation with an error, in place of its result.
const ERROR_MARK: &str = "⚠";
/// Longest source text shown for a calculation that doesn't parse, in characters.
const MAX_SHOWN_SOURCE: usize = 2000;
/// First private-use character standing for a unit while the linear text is parsed into a tree.
const SENTINEL_BASE: u32 = 0xE000;
/// How many distinct units one display can stand in for (the private-use area's size).
const MAX_SENTINELS: usize = 6400;
/// Separates a number from the unit after it in the math tree (spaces in linear text are dropped).
const THIN_SPACE: char = '\u{2009}';

/// The linear-format text shown for the calculation `stmt_src`, with its result `shown` (when
/// the statement shows one) in the given state. A source that doesn't parse is shown as typed,
/// followed by the error marker.
pub fn display(stmt_src: &str, shown: Option<&Formatted>, state: &CalcState) -> String {
    rendered(&parse(stmt_src), stmt_src, shown, state).linear
}

/// A display: the linear text and the math tree to store on the equation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Rendered {
    pub linear: String,
    pub math: Arg,
}

/// Whether a statement shows a value after `=`.
pub(crate) fn shows_value(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Eval { .. } => true,
        Stmt::Define { show, .. } => show.is_some(),
    }
}

/// The display of an already parsed statement (`src` is used only when it didn't parse).
pub(crate) fn rendered(stmt: &Result<Stmt, CalcError>, src: &str, shown: Option<&Formatted>, state: &CalcState) -> Rendered {
    let mut out = Out::default();
    let shows = match stmt {
        Ok(stmt) => {
            write_stmt(stmt, &mut out);
            shows_value(stmt)
        }
        Err(_) => {
            let text: String = src.chars().take(MAX_SHOWN_SOURCE).collect();
            let ellipsis = if src.chars().nth(MAX_SHOWN_SOURCE).is_some() { "…" } else { "" };
            out.text(&text);
            out.text(ellipsis);
            false
        }
    };
    let mut linear = out.linear;
    let mut math = restore_units(parse_linear(&out.parsed), &out.units);
    if stmt.is_err() || matches!(state, CalcState::Error(_)) {
        linear.push(' ');
        linear.push_str(ERROR_MARK);
        math.push(MNode::Run(MRun { text: ERROR_MARK.to_string(), sty: Some(MSty::Plain), color: Some(ERROR_RED), ..MRun::default() }));
    } else if shows {
        let stale = !matches!(state, CalcState::Fresh);
        let rel = if stale { "≈" } else { "=" };
        let number = shown.map_or_else(|| "?".to_string(), |f| linear_number(&f.number));
        let unit = shown.map_or("", |f| f.unit.as_str());
        let mut result: Arg = vec![MNode::Run(MRun::new(rel))];
        result.extend(parse_linear(&number));
        linear.push_str(rel);
        linear.push_str(&number);
        if !unit.is_empty() {
            linear.push_str(&format!(" \"{unit}\""));
            result.push(MNode::Run(unit_run(format!("{THIN_SPACE}{unit}"))));
        }
        if stale {
            result = paint(result, STALE_GREY);
        }
        math.extend(result);
    }
    Rendered { linear, math }
}

/// The result shown in a display made by [`rendered`] (`…=600 "psi"`), read back from its
/// linear text; `None` when the text doesn't end in a result.
pub(crate) fn shown_result(linear: &str) -> Option<Formatted> {
    let (_, tail) = linear.rsplit_once('=')?;
    let (number, unit) = match tail.strip_suffix('"') {
        Some(rest) => {
            let (number, unit) = rest.rsplit_once(" \"")?;
            (number, unit.to_string())
        }
        None => (tail, String::new()),
    };
    let number = number.replace("^(-", "^-").replace(')', "");
    let numeric = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '×' | '^'));
    (numeric && number.chars().any(|c| c.is_ascii_digit())).then_some(Formatted { number, unit })
}

/// A formatted number as linear text: a negative exponent is parenthesised (`10^(-5)`), so the
/// whole exponent is raised.
fn linear_number(number: &str) -> String {
    match number.split_once("^-") {
        Some((mantissa, exp)) => format!("{mantissa}^(-{exp})"),
        None => number.to_string(),
    }
}

/// An upright run of unit text.
fn unit_run(text: String) -> MRun {
    MRun { text, sty: Some(MSty::Plain), ..MRun::default() }
}

/// Every run in `arg`, at any depth, in `colour`.
fn paint(arg: Arg, colour: Rgb) -> Arg {
    arg.into_iter()
        .map(|n| match n {
            MNode::Run(mut r) => {
                r.color = Some(colour);
                MNode::Run(r)
            }
            other => map_args(other, &|a| paint(a, colour)),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Statement → linear text
// ---------------------------------------------------------------------------------------------

/// The text being built: `linear` is what is stored and shown; `parsed` is the same text with
/// each unit replaced by a private-use sentinel, which [`parse_linear`] keeps as one atom so a
/// unit like `m/s²` or `lbf·in` never becomes a fraction or product.
#[derive(Default)]
struct Out {
    linear: String,
    parsed: String,
    units: Vec<String>,
    sentinels: HashMap<String, char>,
}

impl Out {
    fn text(&mut self, s: &str) {
        self.linear.push_str(s);
        self.parsed.push_str(s);
    }

    fn unit(&mut self, unit: &str) {
        self.linear.push_str(&format!("\"{unit}\""));
        if let Some(&c) = self.sentinels.get(unit) {
            self.parsed.push(c);
            return;
        }
        let sentinel = (self.units.len() < MAX_SENTINELS).then(|| char::from_u32(SENTINEL_BASE + self.units.len() as u32)).flatten();
        match sentinel {
            Some(c) => {
                self.sentinels.insert(unit.to_string(), c);
                self.units.push(unit.to_string());
                self.parsed.push(c);
            }
            None => self.parsed.push_str(&format!("\"{unit}\"")),
        }
    }
}

/// One step of writing an expression, so deep trees are written without recursion.
enum Step<'a> {
    Expr(&'a Expr),
    Text(&'static str),
}

fn write_stmt(stmt: &Stmt, out: &mut Out) {
    match stmt {
        Stmt::Define { name, params, body, .. } => {
            out.text(&name_text(name));
            if !params.is_empty() {
                let params: Vec<String> = params.iter().map(|p| name_text(p)).collect();
                out.text(&format!("({})", params.join(",")));
            }
            out.text("≔");
            write_expr(body, out);
        }
        Stmt::Eval { expr, .. } => write_expr(expr, out),
    }
}

/// Binding strength: sums 1, products 2, negation 3, powers 4, atoms 5.
fn prec(e: &Expr) -> u8 {
    match e {
        Expr::Binary(Op::Add | Op::Sub, ..) => 1,
        Expr::Binary(Op::Mul | Op::Div, ..) | Expr::Quantity(..) => 2,
        Expr::Unary(..) => 3,
        Expr::Binary(Op::Pow, ..) => 4,
        Expr::Binary(Op::Neg, ..) => 1,
        Expr::Num(_) | Expr::Name(_) | Expr::Call(..) | Expr::Abs(_) => 5,
    }
}

fn is_sqrt(e: &Expr) -> bool {
    matches!(e, Expr::Call(n, args) if n == "sqrt" && args.len() == 1)
}

/// A name the evaluator reads as a unit. A variable or parameter can never have a unit's name
/// (`check_name` refuses it), so this needs no knowledge of the document.
fn is_unit(name: &str) -> bool {
    lookup(name).is_some()
}

/// A unit, or a power of one (`in^2`): written after a number with a space, not `·`.
fn is_unitish(e: &Expr) -> bool {
    match e {
        Expr::Name(n) => is_unit(n),
        Expr::Binary(Op::Pow, base, _) => matches!(&**base, Expr::Name(n) if is_unit(n)),
        _ => false,
    }
}

/// The last factor of a product.
fn last_factor(mut e: &Expr) -> &Expr {
    while let Expr::Binary(Op::Mul, _, b) = e {
        e = b;
    }
    e
}

/// A number written without an exponent.
fn plain_num(e: &Expr) -> bool {
    matches!(e, Expr::Num(v) if !exponential(*v))
}

/// Needs no parentheses as the operand of `√`.
fn sqrt_simple(e: &Expr) -> bool {
    plain_num(e) || matches!(e, Expr::Name(n) if !n.contains('_'))
}

/// Needs no parentheses as the base of `^`.
fn base_simple(e: &Expr) -> bool {
    plain_num(e) || matches!(e, Expr::Name(_) | Expr::Abs(_)) || (matches!(e, Expr::Call(..)) && !is_sqrt(e))
}

/// Needs no parentheses as an exponent: a whole number or a name without a subscript.
fn exponent_simple(e: &Expr) -> bool {
    match e {
        Expr::Num(v) => !exponential(*v) && v.fract() == 0.0 && *v >= 0.0,
        Expr::Name(n) => !n.contains('_'),
        _ => false,
    }
}

/// Needs no parentheses as the numerator or denominator of `/`.
fn fraction_simple(e: &Expr) -> bool {
    plain_num(e) || matches!(e, Expr::Name(_) | Expr::Abs(_)) || (matches!(e, Expr::Call(..)) && !is_sqrt(e))
}

/// Push `e`, in parentheses when `parens`.
fn wrap<'a>(todo: &mut Vec<Step<'a>>, e: &'a Expr, parens: bool) {
    if parens {
        todo.push(Step::Text(")"));
        todo.push(Step::Expr(e));
        todo.push(Step::Text("("));
    } else {
        todo.push(Step::Expr(e));
    }
}

fn write_expr(root: &Expr, out: &mut Out) {
    // Steps are pushed last first, so they come off the stack in writing order.
    let mut todo: Vec<Step<'_>> = vec![Step::Expr(root)];
    while let Some(step) = todo.pop() {
        let e = match step {
            Step::Text(t) => {
                out.text(t);
                continue;
            }
            Step::Expr(e) => e,
        };
        match e {
            Expr::Num(v) => out.text(&num_text(*v)),
            Expr::Name(n) if is_unit(n) => out.unit(n),
            Expr::Name(n) => out.text(&name_text(n)),
            Expr::Quantity(v, unit) => {
                out.text(&num_text(*v));
                out.text(" ");
                out.unit(unit);
            }
            Expr::Unary(_, a) => {
                // Only `-` is a prefix operator; a product or sum under it keeps its parentheses.
                wrap(&mut todo, a, prec(a) <= 3);
                todo.push(Step::Text("-"));
            }
            Expr::Abs(a) => {
                todo.push(Step::Text("|"));
                todo.push(Step::Expr(a));
                todo.push(Step::Text("|"));
            }
            Expr::Call(_, args) if is_sqrt(e) => {
                if let Some(a) = args.first() {
                    wrap(&mut todo, a, !sqrt_simple(a));
                }
                todo.push(Step::Text("√"));
            }
            Expr::Call(name, args) => {
                todo.push(Step::Text(")"));
                for (i, a) in args.iter().enumerate().rev() {
                    todo.push(Step::Expr(a));
                    if i > 0 {
                        todo.push(Step::Text(","));
                    }
                }
                todo.push(Step::Text("("));
                out.text(&name_text(name));
            }
            Expr::Binary(op, a, b) => match op {
                Op::Add | Op::Sub | Op::Neg => {
                    wrap(&mut todo, b, prec(b) <= 1 || matches!(&**b, Expr::Unary(..)));
                    todo.push(Step::Text(if *op == Op::Add { "+" } else { "-" }));
                    todo.push(Step::Expr(a));
                }
                Op::Mul => {
                    // A quotient on the right needs no parentheses: UnicodeMath's numerator stops
                    // at the `·`, so `c·a/b` reads as c·(a/b), as it was evaluated.
                    let right_parens = prec(b) < 2 || matches!(&**b, Expr::Binary(Op::Mul, ..) | Expr::Quantity(..) | Expr::Unary(..));
                    wrap(&mut todo, b, right_parens);
                    let spaced = is_unitish(b) && !is_unitish(last_factor(a)) && !right_parens;
                    todo.push(Step::Text(if spaced { " " } else { "·" }));
                    wrap(&mut todo, a, prec(a) < 2);
                }
                // The parentheses around a fraction's operands are dropped when it is built up.
                Op::Div => {
                    wrap(&mut todo, b, !fraction_simple(b));
                    todo.push(Step::Text("/"));
                    wrap(&mut todo, a, !fraction_simple(a));
                }
                Op::Pow => {
                    wrap(&mut todo, b, !exponent_simple(b));
                    todo.push(Step::Text("^"));
                    wrap(&mut todo, a, !base_simple(a));
                }
            },
        }
    }
}

/// Whether a number is written as `m×10^e` (too long otherwise).
fn exponential(v: f64) -> bool {
    let a = v.abs();
    a != 0.0 && (a >= 1e15 || a < 1e-6)
}

/// A number as the source would write it: shortest round-trip digits, `m×10^e` when very large
/// or small.
fn num_text(v: f64) -> String {
    if !exponential(v) {
        return format!("{v}");
    }
    let repr = format!("{v:e}");
    match repr.split_once('e') {
        Some((mantissa, exp)) if exp.starts_with('-') => format!("{mantissa}×10^({exp})"),
        Some((mantissa, exp)) => format!("{mantissa}×10^{exp}"),
        None => repr,
    }
}

/// A name with its subscript: `σ_allow` stays `σ_allow`; a subscript holding another `_` is
/// parenthesised so all of it is lowered.
fn name_text(name: &str) -> String {
    match name.split_once('_') {
        Some((base, sub)) if !base.is_empty() && sub.contains('_') => format!("{base}_({sub})"),
        _ => name.to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// Sentinels → upright unit runs
// ---------------------------------------------------------------------------------------------

fn sentinel_unit(c: char, units: &[String]) -> Option<&str> {
    let k = (c as u32).checked_sub(SENTINEL_BASE)? as usize;
    units.get(k).map(String::as_str)
}

/// Whether a unit after `prev` needs a thin space to stand apart from it (`1500 lbf`).
fn needs_space(prev: Option<char>) -> bool {
    prev.is_some_and(|c| c.is_alphanumeric() || c == ')')
}

/// Replace each unit sentinel in the tree with an upright run of the unit's text. The tree comes
/// from [`parse_linear`], which caps its depth, so recursing over it is bounded.
fn restore_units(arg: Arg, units: &[String]) -> Arg {
    if units.is_empty() {
        return arg;
    }
    let mut out: Arg = Vec::with_capacity(arg.len());
    for node in arg {
        match node {
            MNode::Run(r) if r.text.chars().any(|c| sentinel_unit(c, units).is_some()) => {
                let mut buf = String::new();
                let mut prev: Option<char> = None;
                for c in r.text.chars() {
                    match sentinel_unit(c, units) {
                        Some(unit) => {
                            if !buf.is_empty() {
                                out.push(MNode::Run(MRun { text: std::mem::take(&mut buf), ..r.clone() }));
                            }
                            let space = if needs_space(prev) { THIN_SPACE.to_string() } else { String::new() };
                            out.push(MNode::Run(MRun { color: r.color, ..unit_run(format!("{space}{unit}")) }));
                        }
                        None => buf.push(c),
                    }
                    prev = Some(c);
                }
                if !buf.is_empty() {
                    out.push(MNode::Run(MRun { text: buf, ..r }));
                }
            }
            MNode::Script { kind, base, sub, sup } => {
                // A unit raised to a power right after a number: `2 in^2`.
                let unit_base =
                    matches!(base.first(), Some(MNode::Run(r)) if r.text.chars().next().is_some_and(|c| sentinel_unit(c, units).is_some()));
                let prev = match out.last() {
                    Some(MNode::Run(r)) => r.text.chars().last(),
                    _ => None,
                };
                let mut base = restore_units(base, units);
                if unit_base
                    && needs_space(prev)
                    && let Some(MNode::Run(r)) = base.first_mut()
                {
                    r.text.insert(0, THIN_SPACE);
                }
                out.push(MNode::Script { kind, base, sub: restore_units(sub, units), sup: restore_units(sup, units) });
            }
            other => out.push(map_args(other, &|a| restore_units(a, units))),
        }
    }
    out
}
