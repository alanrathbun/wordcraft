//! Evaluating calculations: variables, functions, and unit-checked arithmetic.
//!
//! Source is hostile input, so evaluation is bounded three ways: the depth of recursion (shared
//! across user-function calls, so a deep body inside deep calls can't overflow the stack), the
//! depth of user-function calls, and the total number of expression nodes evaluated (so a chain of
//! functions that each call the previous twice can't run for hours).

use crate::funcs::{self, unit};
use crate::{CalcError, Expr, Op, Show, Stmt};
use craft_units::{Quantity, Scale, UnitError, lookup, parse_unit};
use std::cell::Cell;
use std::collections::BTreeMap;

/// How deep user functions may call each other.
const MAX_CALL_DEPTH: usize = 32;
/// How deep evaluation may recurse in total. The parser caps an expression's height at 1024, so
/// this leaves room for a function body called from the bottom of another expression.
const MAX_EVAL_DEPTH: usize = 2048;
/// How many expression nodes one statement may evaluate.
const MAX_STEPS: u32 = 1_000_000;

/// What a name means in an [`Env`].
#[derive(Debug, Clone, PartialEq)]
enum Entry {
    Var(Quantity),
    Func {
        params: Vec<String>,
        body: Expr,
    },
    /// A calculation that defines this name has an error.
    Failed,
}

/// The names defined so far: variables, functions, and names whose calculation failed.
///
/// A name has one meaning: defining it again replaces whatever it was. Whether a second
/// definition is an error is decided by whoever sees the whole document, not here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Env {
    entries: BTreeMap<String, Entry>,
}

impl Env {
    /// An empty environment.
    pub fn new() -> Env {
        Env::default()
    }

    /// Define a variable.
    pub fn define_var(&mut self, name: &str, q: Quantity) {
        self.entries.insert(name.to_string(), Entry::Var(q));
    }

    /// Define a function of `params`.
    pub fn define_fn(&mut self, name: &str, params: Vec<String>, body: Expr) {
        self.entries.insert(name.to_string(), Entry::Func { params, body });
    }

    /// Record that the calculation defining `name` has an error: using the name gives
    /// [`CalcError::Upstream`].
    pub fn mark_failed(&mut self, name: &str) {
        self.entries.insert(name.to_string(), Entry::Failed);
    }

    /// Every defined name (variables, functions and failed names), sorted.
    pub fn names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }
}

/// Refuse a name that is a unit (with or without an SI prefix), a built-in function, or one of
/// the constants `π`, `pi` and `e`.
pub fn check_name(name: &str) -> Result<(), CalcError> {
    if lookup(name).is_some() || funcs::is_builtin(name) || is_constant(name) {
        return Err(CalcError::ReservedName(name.to_string()));
    }
    Ok(())
}

fn constant(name: &str) -> Option<f64> {
    match name {
        "π" | "pi" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        _ => None,
    }
}

fn is_constant(name: &str) -> bool {
    constant(name).is_some()
}

/// Evaluate an expression against the defined names.
pub fn eval_expr(env: &Env, e: &Expr) -> Result<Quantity, CalcError> {
    let cx = Cx { env, steps: Cell::new(0) };
    eval(&cx, &Frame::GLOBAL, e, 0)
}

/// Run one statement: a definition adds to `env` and an evaluation does not. Returns the value
/// when the statement shows one.
///
/// Nothing is defined unless the whole statement succeeds, including a `= ? unit` that does not
/// match the value's dimension. The unit itself is applied by whoever formats the value.
pub fn run_stmt(env: &mut Env, s: &Stmt) -> Result<Option<Quantity>, CalcError> {
    match s {
        Stmt::Eval { expr, show } => {
            let q = eval_expr(env, expr)?;
            check_show(show, q)?;
            Ok(Some(q))
        }
        Stmt::Define { name, params, body, show } => {
            check_name(name)?;
            params.iter().try_for_each(|p| check_name(p))?;
            if !params.is_empty() {
                if show.is_some() {
                    return Err(CalcError::Syntax("a function has no single value to show — call it, like f(2) =".to_string()));
                }
                env.define_fn(name, params.clone(), body.clone());
                return Ok(None);
            }
            let q = eval_expr(env, body)?;
            if let Some(show) = show {
                check_show(show, q)?;
            }
            env.define_var(name, q);
            Ok(show.as_ref().map(|_| q))
        }
    }
}

/// A `= ? unit` must be a unit of the value's dimension.
fn check_show(show: &Show, q: Quantity) -> Result<(), CalcError> {
    let Some(text) = &show.unit else {
        return Ok(());
    };
    let scale = parse_unit(text).map_err(unit)?;
    if q.dim.same_ignoring_angle(scale.dim) { Ok(()) } else { Err(CalcError::Unit(UnitError::Mismatch { left: q.dim, right: scale.dim })) }
}

/// Shared state of one evaluation.
struct Cx<'a> {
    env: &'a Env,
    /// Expression nodes evaluated so far.
    steps: Cell<u32>,
}

/// The parameters in scope, and how many user functions deep we are.
struct Frame<'a> {
    names: &'a [String],
    values: &'a [Quantity],
    calls: usize,
}

impl Frame<'_> {
    const GLOBAL: Frame<'static> = Frame { names: &[], values: &[], calls: 0 };

    fn param(&self, name: &str) -> Option<Quantity> {
        self.names.iter().position(|n| n == name).and_then(|i| self.values.get(i).copied())
    }
}

fn eval(cx: &Cx<'_>, fr: &Frame<'_>, e: &Expr, depth: usize) -> Result<Quantity, CalcError> {
    if depth > MAX_EVAL_DEPTH {
        return Err(CalcError::TooComplex);
    }
    let steps = cx.steps.get() + 1;
    if steps > MAX_STEPS {
        return Err(CalcError::TooComplex);
    }
    cx.steps.set(steps);
    match e {
        Expr::Num(v) => finite(*v),
        Expr::Name(n) => resolve(cx, fr, n),
        Expr::Quantity(v, text) => {
            let scale = parse_unit(text).map_err(unit)?;
            from_scale(*v, &scale)
        }
        Expr::Unary(Op::Neg, a) => {
            refuse_offset_operand(cx, fr, &[a])?;
            eval(cx, fr, a, depth + 1)?.neg().map_err(unit)
        }
        Expr::Unary(..) => Err(CalcError::Syntax("unsupported operator".to_string())),
        Expr::Abs(a) => {
            let q = eval(cx, fr, a, depth + 1)?;
            finite_q(Quantity::new(q.value.abs(), q.dim))
        }
        Expr::Binary(op, a, b) => {
            if *op == Op::Mul
                && let Some(q) = offset_literal(cx, fr, a, b)
            {
                return q;
            }
            // Offset literals may be added and subtracted (a difference of two absolute
            // temperatures is an interval) but not scaled: `5 °F/s` would be a wrong number that
            // looks right.
            if !matches!(op, Op::Add | Op::Sub) {
                refuse_offset_operand(cx, fr, &[a, b])?;
            }
            let x = eval(cx, fr, a, depth + 1)?;
            let y = eval(cx, fr, b, depth + 1)?;
            apply(*op, x, y)
        }
        Expr::Call(name, args) => call(cx, fr, name, args, depth),
    }
}

fn finite(v: f64) -> Result<Quantity, CalcError> {
    finite_q(Quantity::dimensionless(v))
}

fn finite_q(q: Quantity) -> Result<Quantity, CalcError> {
    if q.value.is_finite() { Ok(q) } else { Err(CalcError::NonFinite) }
}

/// The SI quantity for `v` in `scale`; an offset unit gives the absolute temperature.
fn from_scale(v: f64, scale: &Scale) -> Result<Quantity, CalcError> {
    finite_q(Quantity::new(scale.to_si(v), scale.dim))
}

#[inline(never)]
fn apply(op: Op, x: Quantity, y: Quantity) -> Result<Quantity, CalcError> {
    match op {
        Op::Add => x.add(y),
        Op::Sub => x.sub(y),
        Op::Mul => x.mul(y),
        Op::Div => x.div(y),
        Op::Pow => {
            if !y.dim.is_dimensionless() {
                return Err(CalcError::Unit(UnitError::NotDimensionless(y.dim)));
            }
            x.powf(y.value)
        }
        Op::Neg => return Err(CalcError::Syntax("unsupported operator".to_string())),
    }
    .map_err(unit)
}

/// `5 °F`: a number times an offset unit that no variable or constant shadows is an absolute
/// temperature. Anything else involving an offset unit is refused in [`resolve`].
fn offset_literal(cx: &Cx<'_>, fr: &Frame<'_>, num: &Expr, unit_name: &Expr) -> Option<Result<Quantity, CalcError>> {
    let v = match num {
        Expr::Num(v) => *v,
        Expr::Unary(Op::Neg, inner) => match &**inner {
            Expr::Num(v) => -*v,
            _ => return None,
        },
        _ => return None,
    };
    let Expr::Name(n) = unit_name else {
        return None;
    };
    if fr.param(n).is_some() || cx.env.entries.contains_key(n.as_str()) || is_constant(n) {
        return None;
    }
    let scale = lookup(n).filter(|s| s.offset != 0.0)?;
    Some(from_scale(v, &scale))
}

/// Whether `e` is exactly a `number offset-unit` literal.
fn is_offset_literal(cx: &Cx<'_>, fr: &Frame<'_>, e: &Expr) -> bool {
    matches!(e, Expr::Binary(Op::Mul, a, b) if offset_literal(cx, fr, a, b).is_some())
}

/// An absolute temperature can't be multiplied, divided or raised to a power.
fn refuse_offset_operand(cx: &Cx<'_>, fr: &Frame<'_>, operands: &[&Expr]) -> Result<(), CalcError> {
    if operands.iter().any(|e| is_offset_literal(cx, fr, e)) { Err(CalcError::Unit(UnitError::AffineTemperature)) } else { Ok(()) }
}

fn not_a_value(name: &str) -> CalcError {
    CalcError::Syntax(format!("{name} is a function — call it like {name}(x)"))
}

/// A name as a value: parameter, variable, constant, unit.
fn resolve(cx: &Cx<'_>, fr: &Frame<'_>, name: &str) -> Result<Quantity, CalcError> {
    if let Some(q) = fr.param(name) {
        return Ok(q);
    }
    match cx.env.entries.get(name) {
        Some(Entry::Var(q)) => return Ok(*q),
        Some(Entry::Failed) => return Err(CalcError::Upstream),
        Some(Entry::Func { .. }) => return Err(not_a_value(name)),
        None => {}
    }
    if let Some(v) = constant(name) {
        return Ok(Quantity::dimensionless(v));
    }
    // A unit wins over a built-in of the same spelling: `5 min` is minutes, `min(1, 2)` a call.
    if let Some(scale) = lookup(name) {
        if scale.offset != 0.0 {
            return Err(CalcError::Unit(UnitError::AffineTemperature));
        }
        return finite_q(Quantity::new(scale.factor, scale.dim));
    }
    if funcs::is_builtin(name) {
        return Err(not_a_value(name));
    }
    Err(CalcError::Undefined(name.to_string()))
}

#[inline(never)]
fn call(cx: &Cx<'_>, fr: &Frame<'_>, name: &str, args: &[Expr], depth: usize) -> Result<Quantity, CalcError> {
    // Resolve what is being called before evaluating the arguments, so a missing name is reported
    // as missing rather than after unrelated errors in its arguments.
    let user = if fr.param(name).is_some() { None } else { cx.env.entries.get(name) };
    match user {
        Some(Entry::Failed) => return Err(CalcError::Upstream),
        Some(Entry::Var(_)) => return Err(not_callable(name)),
        Some(Entry::Func { params, body }) => return call_user(cx, fr, name, params, body, args, depth),
        None => {}
    }
    if fr.param(name).is_some() {
        return Err(not_callable(name));
    }
    if !funcs::is_builtin(name) {
        return if is_constant(name) || lookup(name).is_some() { Err(not_callable(name)) } else { Err(CalcError::Undefined(name.to_string())) };
    }
    let values = eval_args(cx, fr, args, depth)?;
    funcs::call(name, &values).unwrap_or_else(|| Err(CalcError::Undefined(name.to_string())))
}

fn not_callable(name: &str) -> CalcError {
    CalcError::Syntax(format!("{name} isn't a function — it can't be called with ( )"))
}

fn eval_args(cx: &Cx<'_>, fr: &Frame<'_>, args: &[Expr], depth: usize) -> Result<Vec<Quantity>, CalcError> {
    args.iter().map(|a| eval(cx, fr, a, depth + 1)).collect()
}

/// Call a user function in a scope holding only its own parameters (and the global names).
fn call_user(cx: &Cx<'_>, fr: &Frame<'_>, name: &str, params: &[String], body: &Expr, args: &[Expr], depth: usize) -> Result<Quantity, CalcError> {
    if args.len() != params.len() {
        return Err(CalcError::Arity(name.to_string(), params.len()));
    }
    if fr.calls >= MAX_CALL_DEPTH {
        return Err(CalcError::TooComplex);
    }
    let values = eval_args(cx, fr, args, depth)?;
    let inner = Frame { names: params, values: &values, calls: fr.calls + 1 };
    eval(cx, &inner, body, depth + 1)
}

#[cfg(test)]
mod tests;
