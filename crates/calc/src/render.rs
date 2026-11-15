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
use wordcraft_doc::math::{Arg, FUNCTION_NAMES, MNode, MRun, MSty, Math, parse_linear};
use wordcraft_doc::math_linear::map_args;

use crate::{CalcError, Expr, Op, Stmt, funcs, parse};

/// Colour of the error marker.
pub const ERROR_RED: Rgb = Rgb(0xC0, 0x00, 0x00);
/// Colour of a stale result.
pub const STALE_GREY: Rgb = Rgb(0x80, 0x80, 0x80);
/// Colour of the mark after a calculation edited outside WordCraft.
pub const EDITED_AMBER: Rgb = Rgb(0xC0, 0x60, 0x00);
/// The marker shown after a calculation with an error, in place of its result.
const ERROR_MARK: &str = "⚠";
/// The mark after a calculation edited outside WordCraft (amber, where an error's is red).
const EDITED_MARK: &str = "⚠";
/// Longest source text shown for a calculation that doesn't parse, in characters.
const MAX_SHOWN_SOURCE: usize = 2000;
/// First private-use character standing for a unit while the linear text is parsed into a tree.
const SENTINEL_BASE: u32 = 0xE000;
/// How many distinct units and names one display can stand in for (the private-use area's size).
const MAX_SENTINELS: usize = 6400;
/// UnicodeMath's invisible brackets: a group that shows no brackets.
const INVISIBLE_OPEN: &str = "〖";
const INVISIBLE_CLOSE: &str = "〗";
/// The base of an exponential number, grouped so its power is on both digits.
const TEN: &str = "〖10〗";
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
    let mut math = restore_pieces(parse_linear(&out.parsed), &out.pieces);
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
        result.extend(restore_pieces(parse_linear(&number), &[]));
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

/// A short red note after a display, upright (`⚠ not calculated here`).
pub(crate) fn push_note(display: &mut Rendered, note: &str) {
    display.linear.push_str(&format!(" \"{note}\""));
    display.math.push(MNode::Run(MRun { text: format!("{THIN_SPACE}{note}"), sty: Some(MSty::Plain), color: Some(ERROR_RED), ..MRun::default() }));
}

/// The mark run after an equation edited outside WordCraft.
fn edited_run() -> MNode {
    MNode::Run(MRun { text: EDITED_MARK.to_string(), sty: Some(MSty::Plain), color: Some(EDITED_AMBER), ..MRun::default() })
}

fn is_edited_run(n: &MNode) -> bool {
    matches!(n, MNode::Run(r) if r.text == EDITED_MARK && r.color == Some(EDITED_AMBER))
}

/// Whether the equation's nodes end in the edited mark.
pub fn has_edited_mark(math: &Math) -> bool {
    math.nodes.last().is_some_and(is_edited_run)
}

/// The display of an equation edited outside WordCraft (`linear`, `math` as Word left them) with
/// the edited mark after it; `None` when it has the mark already. Nothing of Word's equation is
/// changed: its tree (read from its linear text when it has none yet) gains one run.
pub(crate) fn mark_edited(linear: &str, math: &Math) -> Option<Rendered> {
    if has_edited_mark(math) {
        return None;
    }
    let mut nodes = if math.nodes.is_empty() { parse_linear(linear) } else { math.nodes.clone() };
    nodes.push(edited_run());
    Some(Rendered { linear: format!("{linear} {EDITED_MARK}"), math: nodes })
}

/// Takes the edited mark ([`mark_edited`]) off an equation again: before it is saved for Word,
/// and when the author keeps Word's equation. An equation without it is left as it is.
pub fn strip_edited_mark(linear: &mut String, math: &mut Math) {
    if !has_edited_mark(math) {
        return;
    }
    math.nodes.pop();
    if let Some(rest) = linear.strip_suffix(&format!(" {EDITED_MARK}")) {
        *linear = rest.to_string();
    }
    math.omml.clear();
}

/// The result shown in a display made by [`rendered`] (`…=600 "psi"`), read back from its
/// linear text; `None` when the text doesn't end in a result.
pub(crate) fn shown_result(linear: &str) -> Option<Formatted> {
    let (_, tail) = linear.rsplit_once('=')?;
    let tail = tail.replace(TEN, "10");
    let tail = tail.as_str();
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

/// A formatted number as linear text: the base `10` is in invisible brackets (`〖10〗^6`) so the
/// power is on all of it, and a negative exponent is parenthesised (`^(-5)`), so the whole
/// exponent is raised.
fn linear_number(number: &str) -> String {
    let number = number.replace("×10^", &format!("×{TEN}^"));
    match number.split_once("^-") {
        Some((mantissa, exp)) => format!("{mantissa}^(-{exp})"),
        None => number,
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

/// What a sentinel stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    /// A unit: upright, set apart from a number before it.
    Unit,
    /// A name or part of one, in the default math style (letters italic).
    Name,
    /// A built-in function name or a multi-letter subscript: upright.
    Upright,
}

/// A piece of text a sentinel stands for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Piece {
    text: String,
    kind: Kind,
}

/// The text being built: `linear` is what is stored and shown; `parsed` is the same text with
/// each unit and each multi-letter name replaced by a private-use sentinel, which
/// [`parse_linear`] keeps as one atom. So a unit like `m/s²` or `lbf·in` never becomes a fraction
/// or product, and a name like `asin`, `xmax` or `f_max` never has a function name (`sin`, `max`)
/// found inside it and applied to what follows.
#[derive(Default)]
struct Out {
    linear: String,
    parsed: String,
    pieces: Vec<Piece>,
    sentinels: HashMap<Piece, char>,
}

impl Out {
    fn text(&mut self, s: &str) {
        self.linear.push_str(s);
        self.parsed.push_str(s);
    }

    fn unit(&mut self, unit: &str) {
        self.linear.push_str(&format!("\"{unit}\""));
        self.sentinel(Piece { text: unit.to_string(), kind: Kind::Unit }, &format!("\"{unit}\""));
    }

    /// Name text: a sentinel when UnicodeMath could read part of it as a function name.
    fn name_piece(&mut self, text: &str, kind: Kind) {
        self.linear.push_str(text);
        if text.chars().filter(char::is_ascii_alphabetic).count() >= 2 {
            self.sentinel(Piece { text: text.to_string(), kind }, text);
        } else {
            self.parsed.push_str(text);
        }
    }

    /// Put `piece`'s sentinel into the parsed text, or `fallback` when the sentinels run out.
    fn sentinel(&mut self, piece: Piece, fallback: &str) {
        if let Some(&c) = self.sentinels.get(&piece) {
            self.parsed.push(c);
            return;
        }
        let k = self.pieces.len();
        let sentinel = (k < MAX_SENTINELS).then(|| u32::try_from(k).ok().and_then(|k| char::from_u32(SENTINEL_BASE + k))).flatten();
        match sentinel {
            Some(c) => {
                self.sentinels.insert(piece.clone(), c);
                self.pieces.push(piece);
                self.parsed.push(c);
            }
            None => self.parsed.push_str(fallback),
        }
    }

    /// A name with its subscripts: `σ_allow` is σ with subscript allow; each further `_`
    /// lowers the rest again, in parentheses (`a_(b_(c))`) so all of it is lowered.
    ///
    /// An empty part (`a__b`, `x_`) shows as nothing: no `_` is left without a subscript after
    /// it, which UnicodeMath would hang on whatever follows (`x_≔3` lowering `≔3`).
    fn name(&mut self, name: &str) {
        let mut parts = name.split('_');
        let base = parts.next().unwrap_or_default();
        self.name_piece(base, Kind::Name);
        let subs: Vec<&str> = parts.filter(|p| !p.is_empty()).collect();
        match subs.as_slice() {
            [] => {}
            [sub] => {
                self.text("_");
                self.name_piece(sub, sub_kind(sub));
            }
            _ => {
                for sub in &subs {
                    self.text("_(");
                    self.name_piece(sub, sub_kind(sub));
                }
                for _ in &subs {
                    self.text(")");
                }
            }
        }
    }
}

/// A subscript of two or more characters is upright (`σ_max`); a one-letter one is a symbol.
fn sub_kind(sub: &str) -> Kind {
    if sub.chars().count() >= 2 { Kind::Upright } else { Kind::Name }
}

/// One step of writing an expression, so deep trees are written without recursion.
enum Step<'a> {
    Expr(&'a Expr),
    Text(&'static str),
}

fn write_stmt(stmt: &Stmt, out: &mut Out) {
    match stmt {
        Stmt::Define { name, params, body, .. } => {
            out.name(name);
            if !params.is_empty() {
                out.text("(");
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        out.text(",");
                    }
                    out.name(p);
                }
                out.text(")");
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

/// Needs no parentheses as the base of `^`. A call does: `sin(θ)^2` would raise the argument
/// (UnicodeMath applies the script to a function's operand) and `f(x)^2` only the `(x)`.
fn base_simple(e: &Expr) -> bool {
    plain_num(e) || matches!(e, Expr::Name(_) | Expr::Abs(_))
}

/// Needs no parentheses as an exponent: a whole number or a name without a subscript.
fn exponent_simple(e: &Expr) -> bool {
    match e {
        Expr::Num(v) => !exponential(*v) && v.fract() == 0.0 && *v >= 0.0,
        Expr::Name(n) => !n.contains('_'),
        _ => false,
    }
}

/// Needs no parentheses as the numerator or denominator of `/` (a call gets them, which the
/// fraction drops, so its whole application is the operand).
fn fraction_simple(e: &Expr) -> bool {
    plain_num(e) || matches!(e, Expr::Name(_) | Expr::Abs(_))
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
            Expr::Name(n) => out.name(n),
            Expr::Quantity(v, unit) => {
                out.text(&num_text(*v));
                out.text(" ");
                out.unit(unit);
            }
            Expr::Unary(Op::Neg, a) => {
                // A product or sum under `-` keeps its parentheses.
                wrap(&mut todo, a, prec(a) <= 3);
                todo.push(Step::Text("-"));
            }
            // No other prefix operator exists (the evaluator refuses one); show the operand only.
            Expr::Unary(_, a) => wrap(&mut todo, a, true),
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
                // UnicodeMath builds `sin(…)` into a function applied to its operand and drops
                // the operand's parentheses (shown as `sin θ`), so anything but a lone atom gets a
                // second pair: `sin((θ+1))` shows sin(θ+1), not (sin θ)+1. Other names are
                // sentinels, so their parentheses stay as written.
                let applied = FUNCTION_NAMES.contains(&name.as_str());
                let lone = matches!(args.as_slice(), [a] if plain_num(a) || matches!(a, Expr::Name(n) if !n.contains('_') && !is_unit(n)));
                let doubled = applied && !lone;
                todo.push(Step::Text(if doubled { "))" } else { ")" }));
                for (i, a) in args.iter().enumerate().rev() {
                    todo.push(Step::Expr(a));
                    if i > 0 {
                        todo.push(Step::Text(","));
                    }
                }
                todo.push(Step::Text(if doubled { "((" } else { "(" }));
                if applied {
                    out.text(name);
                } else if funcs::is_builtin(name) {
                    out.name_piece(name, Kind::Upright);
                } else {
                    out.name(name);
                }
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
                    if base_simple(a) && matches!(&**a, Expr::Num(_)) && num_text_len(a) > 1 {
                        // UnicodeMath raises only a number's last digit (`25^2` is 2·5²); invisible
                        // brackets make the whole number the base.
                        todo.push(Step::Text(INVISIBLE_CLOSE));
                        todo.push(Step::Expr(a));
                        todo.push(Step::Text(INVISIBLE_OPEN));
                    } else {
                        wrap(&mut todo, a, !base_simple(a));
                    }
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
        Some((mantissa, exp)) if exp.starts_with('-') => format!("{mantissa}×{TEN}^({exp})"),
        Some((mantissa, exp)) => format!("{mantissa}×{TEN}^{exp}"),
        None => repr,
    }
}

/// Characters in a number's text.
fn num_text_len(e: &Expr) -> usize {
    match e {
        Expr::Num(v) => num_text(*v).chars().count(),
        _ => 0,
    }
}

// ---------------------------------------------------------------------------------------------
// Sentinels → runs
// ---------------------------------------------------------------------------------------------

fn sentinel_piece(c: char, pieces: &[Piece]) -> Option<&Piece> {
    let k = usize::try_from((c as u32).checked_sub(SENTINEL_BASE)?).ok()?;
    pieces.get(k)
}

fn is_unit_sentinel(c: char, pieces: &[Piece]) -> bool {
    sentinel_piece(c, pieces).is_some_and(|p| p.kind == Kind::Unit)
}

/// Whether a unit after `prev` needs a thin space to stand apart from it (`1500 lbf`, `len m`).
fn needs_space(prev: Option<char>, pieces: &[Piece]) -> bool {
    prev.is_some_and(|c| c.is_alphanumeric() || c == ')' || sentinel_piece(c, pieces).is_some_and(|p| p.kind != Kind::Unit))
}

/// The run a sentinel turns back into, after `prev` (in the same run), coloured like its run.
fn piece_run(piece: &Piece, prev: Option<char>, pieces: &[Piece], template: &MRun) -> MRun {
    match piece.kind {
        Kind::Unit => {
            let space = if needs_space(prev, pieces) { THIN_SPACE.to_string() } else { String::new() };
            MRun { color: template.color, ..unit_run(format!("{space}{}", piece.text)) }
        }
        Kind::Name => MRun { text: piece.text.clone(), ..template.clone() },
        Kind::Upright => MRun { text: piece.text.clone(), sty: Some(MSty::Plain), ..template.clone() },
    }
}

/// Replace each sentinel in the tree with a run of its text, and take a script's base out of the
/// invisible brackets that kept it whole. The tree comes from
/// [`parse_linear`], which caps its depth, so recursing over it is bounded.
fn restore_pieces(arg: Arg, pieces: &[Piece]) -> Arg {
    let mut out: Arg = Vec::with_capacity(arg.len());
    for node in arg {
        match node {
            MNode::Run(r) if r.text.chars().any(|c| sentinel_piece(c, pieces).is_some()) => {
                let mut buf = String::new();
                let mut prev: Option<char> = None;
                for c in r.text.chars() {
                    match sentinel_piece(c, pieces) {
                        Some(piece) => {
                            if !buf.is_empty() {
                                out.push(MNode::Run(MRun { text: std::mem::take(&mut buf), ..r.clone() }));
                            }
                            out.push(MNode::Run(piece_run(piece, prev, pieces, &r)));
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
                let unit_base = matches!(base.first(), Some(MNode::Run(r)) if r.text.chars().next().is_some_and(|c| is_unit_sentinel(c, pieces)));
                let prev = match out.last() {
                    Some(MNode::Run(r)) => r.text.chars().last(),
                    _ => None,
                };
                let mut base = restore_pieces(base, pieces);
                // A base in invisible brackets (`〖25〗^2`) is the bracketed content itself.
                if let [MNode::Delim { beg: None, end: None, elems, .. }] = base.as_mut_slice()
                    && elems.len() == 1
                    && let Some(inner) = elems.pop()
                {
                    base = inner;
                }
                if unit_base
                    && needs_space(prev, pieces)
                    && let Some(MNode::Run(r)) = base.first_mut()
                {
                    r.text.insert(0, THIN_SPACE);
                }
                out.push(MNode::Script { kind, base, sub: restore_pieces(sub, pieces), sup: restore_pieces(sup, pieces) });
            }
            other => out.push(map_args(other, &|a| restore_pieces(a, pieces))),
        }
    }
    out
}
