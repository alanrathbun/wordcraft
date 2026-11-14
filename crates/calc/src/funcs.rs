//! The built-in functions. Each takes already-evaluated arguments and checks their dimensions.

use crate::CalcError;
use craft_units::{Dim, Quantity, UnitError};

/// Every built-in function name.
const BUILTINS: &[&str] = &[
    "sqrt", "abs", "exp", "ln", "log", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "min", "max", "round", "floor",
    "ceil", "mod",
];

/// Whether `name` is a built-in function.
pub(crate) fn is_builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

/// Call the built-in `name` on evaluated arguments, or `None` when there is no such built-in.
///
/// Dimension checks ignore the angle exponent (a plain number and an angle are both
/// "dimensionless"), and a function that yields a number is non-finite-checked, so `ln(0)` and
/// `asin(2)` are errors rather than infinities or NaN.
pub(crate) fn call(name: &str, args: &[Quantity]) -> Option<Result<Quantity, CalcError>> {
    if !is_builtin(name) {
        return None;
    }
    Some(dispatch(name, args))
}

fn dispatch(name: &str, args: &[Quantity]) -> Result<Quantity, CalcError> {
    match name {
        "sqrt" => one(name, args)?.sqrt().map_err(unit),
        "abs" => {
            let q = one(name, args)?;
            number(q.value.abs(), q.dim)
        }
        "exp" => plain1(name, args, f64::exp),
        "ln" => plain1(name, args, f64::ln),
        "sinh" => plain1(name, args, f64::sinh),
        "cosh" => plain1(name, args, f64::cosh),
        "tanh" => plain1(name, args, f64::tanh),
        "sin" => plain1(name, args, f64::sin),
        "cos" => plain1(name, args, f64::cos),
        "tan" => plain1(name, args, f64::tan),
        "asin" => angle1(name, args, f64::asin),
        "acos" => angle1(name, args, f64::acos),
        "atan" => angle1(name, args, f64::atan),
        "floor" => plain1(name, args, f64::floor),
        "ceil" => plain1(name, args, f64::ceil),
        "log" => log(args),
        "atan2" => atan2(args),
        "min" => extreme(name, args, f64::min),
        "max" => extreme(name, args, f64::max),
        "round" => round(args),
        "mod" => modulo(args),
        // `call` only dispatches names in `BUILTINS`.
        _ => Err(CalcError::Undefined(name.to_string())),
    }
}

/// Map a unit error, keeping every non-finite result one uniform error.
pub(crate) fn unit(e: UnitError) -> CalcError {
    match e {
        UnitError::NonFinite => CalcError::NonFinite,
        other => CalcError::Unit(other),
    }
}

fn number(value: f64, dim: Dim) -> Result<Quantity, CalcError> {
    if value.is_finite() { Ok(Quantity::new(value, dim)) } else { Err(CalcError::NonFinite) }
}

/// The only argument, or an arity error naming `want` = 1.
fn one(name: &str, args: &[Quantity]) -> Result<Quantity, CalcError> {
    match args {
        [q] => Ok(*q),
        _ => Err(CalcError::Arity(name.to_string(), 1)),
    }
}

/// A dimensionless argument's value.
fn plain(q: Quantity) -> Result<f64, CalcError> {
    if q.dim.is_dimensionless() { Ok(q.value) } else { Err(CalcError::Unit(UnitError::NotDimensionless(q.dim))) }
}

/// A dimensionless-in, dimensionless-out function of one argument.
fn plain1(name: &str, args: &[Quantity], f: fn(f64) -> f64) -> Result<Quantity, CalcError> {
    let x = plain(one(name, args)?)?;
    number(f(x), Dim::ONE)
}

/// A dimensionless-in, angle-out function of one argument.
fn angle1(name: &str, args: &[Quantity], f: fn(f64) -> f64) -> Result<Quantity, CalcError> {
    let x = plain(one(name, args)?)?;
    number(f(x), Dim::ANGLE)
}

/// `log(x)` is base 10, `log(x, b)` is base `b`. Bases 10 and 2 use the exact library routines so
/// `log(1000, 10)` is 3, not 2.9999999999999996.
fn log(args: &[Quantity]) -> Result<Quantity, CalcError> {
    match args {
        [x] => number(plain(*x)?.log10(), Dim::ONE),
        [x, b] => {
            let (x, b) = (plain(*x)?, plain(*b)?);
            let v = if b == 10.0 {
                x.log10()
            } else if b == 2.0 {
                x.log2()
            } else {
                x.ln() / b.ln()
            };
            number(v, Dim::ONE)
        }
        _ => Err(CalcError::Arity("log".to_string(), if args.is_empty() { 1 } else { 2 })),
    }
}

fn atan2(args: &[Quantity]) -> Result<Quantity, CalcError> {
    let [y, x] = args else {
        return Err(CalcError::Arity("atan2".to_string(), 2));
    };
    same(*y, *x)?;
    number(y.value.atan2(x.value), Dim::ANGLE)
}

/// The two quantities must agree apart from angle.
fn same(a: Quantity, b: Quantity) -> Result<(), CalcError> {
    if a.dim.same_ignoring_angle(b.dim) { Ok(()) } else { Err(CalcError::Unit(UnitError::Mismatch { left: a.dim, right: b.dim })) }
}

/// The dimension of a value that may come from either operand: the angle exponent is the shared
/// one, else the non-zero one, else zero (the same rule as the units crate's sum).
fn merge(a: Dim, b: Dim) -> Dim {
    // The angle exponent is the one `Dim::ANGLE` sets.
    let Some(i) = Dim::ANGLE.exp.iter().position(|&e| e != 0) else {
        return a;
    };
    let (x, y) = (a.exp[i], b.exp[i]);
    let mut d = a;
    d.exp[i] = if x == y || y == 0 {
        x
    } else if x == 0 {
        y
    } else {
        0
    };
    d
}

/// `min` or `max` of one or more arguments of one dimension.
fn extreme(name: &str, args: &[Quantity], pick: fn(f64, f64) -> f64) -> Result<Quantity, CalcError> {
    let Some((first, rest)) = args.split_first() else {
        return Err(CalcError::Arity(name.to_string(), 1));
    };
    let mut acc = *first;
    for q in rest {
        same(acc, *q)?;
        acc = Quantity::new(pick(acc.value, q.value), merge(acc.dim, q.dim));
    }
    number(acc.value, acc.dim)
}

/// `round(x)` to a whole number, `round(x, n)` to `n` decimal places (`n` from -15 to 15), halves
/// away from zero.
fn round(args: &[Quantity]) -> Result<Quantity, CalcError> {
    match args {
        [x] => number(plain(*x)?.round(), Dim::ONE),
        [x, n] => {
            let (x, n) = (plain(*x)?, plain(*n)?);
            if n.fract() != 0.0 {
                return Err(CalcError::Syntax("round's number of places must be a whole number".to_string()));
            }
            if !(-15.0..=15.0).contains(&n) {
                return Err(CalcError::Syntax("round's number of places must be from -15 to 15".to_string()));
            }
            let scale = 10f64.powi(n as i32);
            number((x * scale).round() / scale, Dim::ONE)
        }
        _ => Err(CalcError::Arity("round".to_string(), if args.is_empty() { 1 } else { 2 })),
    }
}

/// `mod(a, b)`: the remainder of `a` over `b`, taking the sign of `b`, so `mod(-1, 3)` is 2.
fn modulo(args: &[Quantity]) -> Result<Quantity, CalcError> {
    let [a, b] = args else {
        return Err(CalcError::Arity("mod".to_string(), 2));
    };
    same(*a, *b)?;
    let mut r = a.value % b.value;
    if r != 0.0 && (r < 0.0) != (b.value < 0.0) {
        r += b.value;
    }
    number(r, merge(a.dim, b.dim))
}
