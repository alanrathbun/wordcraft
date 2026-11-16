//! The array functions: built-ins that take whole grids rather than applying element by element.
//!
//! - **Sizes:** `length(v)`, `rows(M)`, `cols(M)`; a single value is one element.
//! - **Reductions:** `min`/`max` over every element of every argument, `sum(v)`, `mean(v)`.
//! - **Products:** `dot(a, b)`, `mmul(A, B)` (the matrix product), `transpose(A)`.
//! - **Lookup:** `lookup(xs, ys, x)` is the `ys` value whose `xs` equals `x`.
//! - **Interpolation:** `linterp(xs, ys, x)` (linear) and `spline(xs, ys, x)` (natural cubic
//!   spline), over `xs` strictly increasing.
//!
//! None of them guesses: a lookup that matches no `xs` and an interpolation outside `xs` are
//! errors, never the nearest value or an extrapolation. `x` may be a grid, giving one result per
//! element. An error about an argument names it as written when it is a bare name (`NPS`), else
//! by its role (`xs`). Each function charges its element work to the pass before doing it.

use crate::array::{self, Data, Grid, MAX_ELEMENTS};
use crate::eval::{Pass, Val, picked};
use crate::funcs::{self, unit};
use crate::{CalcError, Expr, render};
use craft_units::{Dim, Formatted, Precision, Quantity, UnitError, UnitSystem, format};

/// How far apart (relative to the larger) two values may be and still count as equal in `lookup`
/// and at the ends and knots of an interpolation: what converting units can leave
/// (`38.1 mm` against `1.5 in`).
const EQUAL: f64 = 1e-12;

/// What an array function needs besides its arguments.
pub(crate) struct Ctx<'a> {
    /// What an array's first element is numbered: 0 or 1.
    pub(crate) origin: u8,
    /// The recalculation's work.
    pub(crate) pass: &'a Pass,
    /// The unit system values in messages are shown in.
    pub(crate) system: UnitSystem,
    /// The decimal places values in messages are shown with.
    pub(crate) precision: Precision,
}

/// Call the array function `name` on evaluated `args`, written as `exprs`; `None` when `name`
/// isn't one, and for `min`/`max` of single values only (those are scalar built-ins).
pub(crate) fn call_array(name: &str, args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Option<Result<Data, CalcError>> {
    let result = match name {
        "min" | "max" if args.iter().all(|a| matches!(a, Data::One(_))) => return None,
        "min" | "max" => extreme(name, args, cx),
        "length" | "rows" | "cols" => size(name, args),
        "sum" | "mean" => total(name, args, cx),
        "dot" => dot(args, exprs, cx),
        "mmul" => mmul(args, exprs, cx),
        "transpose" => transpose(args, cx),
        "lookup" => lookup(args, exprs, cx),
        "linterp" | "spline" => interpolate(name, args, exprs, cx),
        _ => return None,
    };
    Some(result)
}

/// The elements of `d`: a single value is one.
fn items(d: &Data) -> &[Val] {
    match d {
        Data::One(v) => std::slice::from_ref(v),
        Data::Grid(g) => &g.items,
    }
}

/// The rows and columns of `d`: a single value is 1 × 1.
fn shape(d: &Data) -> (usize, usize) {
    d.shape().unwrap_or((1, 1))
}

/// How argument `i` is named in messages: its name when it is a bare name, else `role`.
fn arg_name<'a>(exprs: &'a [Expr], i: usize, role: &'a str) -> &'a str {
    match exprs.get(i) {
        Some(Expr::Name(n)) => n,
        _ => role,
    }
}

/// Count `n` elements' work against the pass.
fn charge(cx: &Ctx<'_>, n: usize) -> Result<(), CalcError> {
    cx.pass.charge(u64::try_from(n).unwrap_or(u64::MAX))
}

fn plain_count(n: usize) -> Data {
    // A count is at most `MAX_ELEMENTS`, so the conversion is exact.
    Data::One(Val::plain(Quantity::dimensionless(n as f64)))
}

fn finite(q: Quantity) -> Result<Quantity, CalcError> {
    if q.value.is_finite() { Ok(q) } else { Err(CalcError::NonFinite) }
}

/// `q` as the document shows values: `1.6 in` in a US customary document. A value too large for
/// the document's units shows in SI units, and never as a number without its unit.
fn shown(q: Quantity, cx: &Ctx<'_>) -> String {
    let text =
        |f: Formatted| if f.unit.is_empty() { render::text_number(&f.number) } else { format!("{} {}", render::text_number(&f.number), f.unit) };
    match format(q, None, cx.system, cx.precision).or_else(|_| format(q, None, UnitSystem::Si, cx.precision)) {
        Ok(f) => text(f),
        // Only a non-finite value fails in SI units, and values here are finite; still, keep a unit.
        Err(_) if q.dim == Dim::ONE => q.value.to_string(),
        Err(_) => format!("{} {}", q.value, q.dim),
    }
}

/// `length`, `rows` or `cols` of the one argument.
fn size(name: &str, args: &[Data]) -> Result<Data, CalcError> {
    let [a] = args else {
        return Err(CalcError::Arity(name.to_string(), 1));
    };
    let (rows, cols) = shape(a);
    Ok(plain_count(match name {
        "rows" => rows,
        "cols" => cols,
        _ => rows.saturating_mul(cols),
    }))
}

/// `min` or `max` over every element of every argument, of one dimension. The smallest or largest
/// of temperatures stands for what they do, as for single values.
fn extreme(name: &str, args: &[Data], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    charge(cx, args.iter().map(|a| items(a).len()).fold(0, usize::saturating_add))?;
    let values: Vec<Val> = args.iter().flat_map(|a| items(a).iter().copied()).collect();
    let quantities: Vec<Quantity> = values.iter().map(|v| v.q).collect();
    let q = funcs::call(name, &quantities).unwrap_or_else(|| Err(CalcError::Undefined(name.to_string())))?;
    Ok(Data::One(Val { q, t: picked(&values) }))
}

/// `sum` or `mean` of the one argument's elements, of one dimension. A sum isn't a temperature;
/// the mean of temperatures stands for what they do.
fn total(name: &str, args: &[Data], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [a] = args else {
        return Err(CalcError::Arity(name.to_string(), 1));
    };
    let values = items(a);
    charge(cx, values.len())?;
    let Some((first, rest)) = values.split_first() else {
        return Err(CalcError::Syntax("an array needs at least one element".to_string()));
    };
    let sum = rest.iter().try_fold(first.q, |acc, v| acc.add(v.q)).map_err(unit)?;
    if name == "sum" {
        return Ok(Data::One(Val::plain(sum)));
    }
    let mean = sum.div(Quantity::dimensionless(values.len() as f64)).map_err(unit)?;
    Ok(Data::One(Val { q: mean, t: picked(values) }))
}

/// The elements of `d`, which `f` needs to be a vector (a single value is one element); an error
/// naming it as `label` for a matrix.
fn vector<'a>(f: &str, d: &'a Data, label: &str) -> Result<&'a [Val], CalcError> {
    match d {
        Data::Grid(g) if g.rows != 1 && g.cols != 1 => Err(CalcError::Syntax(format!("{f} needs {label} to be a vector, not a matrix"))),
        _ => Ok(items(d)),
    }
}

/// `f`'s two vectors `a` and `b` must have the same length.
fn same_length(f: &str, a: (&str, usize), b: (&str, usize)) -> Result<(), CalcError> {
    if a.1 == b.1 {
        return Ok(());
    }
    Err(CalcError::Syntax(format!("{f} needs {} and {} of the same length ({} and {})", a.0, b.0, a.1, b.1)))
}

/// `dot(a, b)`: the sum of the products of matching elements of two vectors of the same length.
fn dot(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [a, b] = args else {
        return Err(CalcError::Arity("dot".to_string(), 2));
    };
    let (a_name, b_name) = (arg_name(exprs, 0, "a"), arg_name(exprs, 1, "b"));
    let (x, y) = (vector("dot", a, a_name)?, vector("dot", b, b_name)?);
    same_length("dot", (a_name, x.len()), (b_name, y.len()))?;
    charge(cx, x.len())?;
    let mut sum: Option<Quantity> = None;
    for (p, q) in x.iter().zip(y) {
        let product = p.q.mul(q.q).map_err(unit)?;
        sum = Some(match sum {
            None => product,
            Some(s) => s.add(product).map_err(unit)?,
        });
    }
    let sum = sum.ok_or_else(|| CalcError::Syntax("an array needs at least one element".to_string()))?;
    Ok(Data::One(Val::plain(sum)))
}

/// `mmul(A, B)`: the matrix product of `A` (r × n) and `B` (n × c), a single value being 1 × 1.
/// The result's size is checked, and its `r · c · n` products charged, before any is worked out.
fn mmul(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [a, b] = args else {
        return Err(CalcError::Arity("mmul".to_string(), 2));
    };
    let ((rows, inner), (inner_b, cols)) = (shape(a), shape(b));
    if inner != inner_b {
        let (a_name, b_name) = (arg_name(exprs, 0, "A"), arg_name(exprs, 1, "B"));
        return Err(CalcError::Syntax(format!("mmul needs {a_name}'s columns to match {b_name}'s rows ({inner} and {inner_b})")));
    }
    let n = rows.checked_mul(cols).filter(|&n| n <= MAX_ELEMENTS).ok_or(CalcError::TooLarge)?;
    charge(cx, n.saturating_mul(inner))?;
    let (x, y) = (items(a), items(b));
    let at = |v: &[Val], k: usize| v.get(k).map(|v| v.q).ok_or_else(|| CalcError::Syntax("an array is missing an element".to_string()));
    let mut out = Vec::with_capacity(n);
    for i in 0..rows {
        for j in 0..cols {
            let cell = (0..inner).try_fold(None, |sum: Option<Quantity>, m| {
                let product = at(x, i * inner + m)?.mul(at(y, m * cols + j)?).map_err(unit)?;
                Ok::<_, CalcError>(Some(match sum {
                    None => product,
                    Some(s) => s.add(product).map_err(unit)?,
                }))
            });
            let k = i * cols + j;
            let q = cell
                .and_then(|q| q.ok_or_else(|| CalcError::Syntax("an array needs at least one element".to_string())))
                .map_err(|e| array::element_error(rows, cols, k, cx.origin, e))?;
            out.push(Val::plain(q));
        }
    }
    Grid::new(rows, cols, out).map(Data::Grid)
}

/// `transpose(A)`: rows become columns. A single value is itself.
fn transpose(args: &[Data], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [a] = args else {
        return Err(CalcError::Arity("transpose".to_string(), 1));
    };
    let Data::Grid(g) = a else {
        return Ok(a.clone());
    };
    charge(cx, g.items.len())?;
    let items = (0..g.cols)
        .flat_map(|j| (0..g.rows).map(move |i| g.items.get(i * g.cols + j).copied()))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| CalcError::Syntax("an array is missing an element".to_string()))?;
    Grid::new(g.cols, g.rows, items).map(Data::Grid)
}

/// Whether `a` and `b` are equal to within [`EQUAL`] of the larger.
fn equal(a: f64, b: f64) -> bool {
    (a - b).abs() <= EQUAL * a.abs().max(b.abs())
}

/// The one dimension of `values` (named `label` in `f`'s messages); an error naming the first
/// element of another.
fn one_dim(values: &[Val], label: &str, origin: u8) -> Result<Dim, CalcError> {
    let dim = values.first().map_or(Dim::ONE, |v| v.q.dim);
    match values.iter().position(|v| !v.q.dim.same_ignoring_angle(dim)) {
        None => Ok(dim),
        Some(k) => Err(CalcError::Element {
            at: format!("{} of {label}", k.saturating_add(usize::from(origin))),
            error: Box::new(CalcError::Unit(UnitError::Mismatch { left: dim, right: values.get(k).map_or(dim, |v| v.q.dim) })),
        }),
    }
}

/// `x` must be of the dimension of the `xs` it is looked up in.
fn fits(x: Quantity, dim: Dim) -> Result<(), CalcError> {
    if x.dim.same_ignoring_angle(dim) { Ok(()) } else { Err(CalcError::Unit(UnitError::Mismatch { left: x.dim, right: dim })) }
}

/// `lookup(xs, ys, x)`: the `ys` value whose `xs` equals `x` (to within [`EQUAL`], after unit
/// conversion), for each element of `x`. No match is an error, never the nearest value, and so is
/// more than one (repeated `xs`), never the first.
fn lookup(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [xs, ys, x] = args else {
        return Err(CalcError::Arity("lookup".to_string(), 3));
    };
    let (xs_name, ys_name) = (arg_name(exprs, 0, "xs"), arg_name(exprs, 1, "ys"));
    let (xs, ys) = (vector("lookup", xs, xs_name)?, vector("lookup", ys, ys_name)?);
    same_length("lookup", (xs_name, xs.len()), (ys_name, ys.len()))?;
    // Every element of `x` is compared with every element of `xs`.
    charge(cx, items(x).len().saturating_add(1).saturating_mul(xs.len()))?;
    let dim = one_dim(xs, xs_name, cx.origin)?;
    array::map(x, cx.origin, |x| {
        fits(x.q, dim)?;
        let mut matches = xs.iter().enumerate().filter(|(_, xi)| equal(xi.q.value, x.q.value)).map(|(i, _)| i);
        match (matches.next(), matches.next()) {
            (Some(i), None) => ys.get(i).copied().ok_or_else(|| CalcError::Syntax(format!("{ys_name} has no element {i}"))),
            (Some(_), Some(_)) => Err(CalcError::Syntax(format!("{} matches more than one value in {xs_name}", shown(x.q, cx)))),
            (None, _) => Err(CalcError::Syntax(format!("{} isn't in {xs_name}", shown(x.q, cx)))),
        }
    })
}

/// `linterp(xs, ys, x)` or `spline(xs, ys, x)`: the value at each element of `x` of the line
/// through neighbouring points, or of the natural cubic spline through all of them. `xs` must be
/// strictly increasing, of one dimension, and as long as `ys` (at least 2 points); an `x` outside
/// `xs` is an error, never an extrapolation.
fn interpolate(name: &str, args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [xs, ys, x] = args else {
        return Err(CalcError::Arity(name.to_string(), 3));
    };
    let (xs_name, ys_name) = (arg_name(exprs, 0, "xs"), arg_name(exprs, 1, "ys"));
    let (xs, ys) = (vector(name, xs, xs_name)?, vector(name, ys, ys_name)?);
    same_length(name, (xs_name, xs.len()), (ys_name, ys.len()))?;
    let (Some(lo), Some(hi)) = (xs.first().map(|v| v.q), xs.last().map(|v| v.q)) else {
        return Err(CalcError::Syntax("an array needs at least one element".to_string()));
    };
    if xs.len() < 2 {
        return Err(CalcError::Syntax(format!("{name} needs at least 2 points in {xs_name}")));
    }
    // Checking the points (and solving for the spline) is linear in them; each `x` is a search.
    charge(cx, xs.len().saturating_add(items(x).len()))?;
    let x_dim = one_dim(xs, xs_name, cx.origin)?;
    let y_dim = one_dim(ys, ys_name, cx.origin)?;
    let o = usize::from(cx.origin);
    if let Some(k) = xs.windows(2).position(|w| matches!(w, [a, b] if b.q.value <= a.q.value)) {
        return Err(CalcError::Syntax(format!(
            "{name} needs {xs_name} strictly increasing — element {} isn't more than element {}",
            k.saturating_add(o).saturating_add(1),
            k.saturating_add(o)
        )));
    }
    let xv: Vec<f64> = xs.iter().map(|v| v.q.value).collect();
    let yv: Vec<f64> = ys.iter().map(|v| v.q.value).collect();
    let curve = if name == "spline" { second_derivatives(&xv, &yv) } else { Vec::new() };
    let t = picked(ys);
    array::map(x, cx.origin, |x| {
        fits(x.q, x_dim)?;
        let v = x.q.value;
        // The first point at or past `v`: `v` is between it and the one before.
        let p = xv.partition_point(|&xi| xi < v);
        let knot = [p.checked_sub(1), Some(p)].into_iter().flatten().find(|&k| xv.get(k).is_some_and(|&xk| equal(xk, v)));
        let value = match knot {
            Some(k) => yv.get(k).copied(),
            None if p == 0 || p >= xv.len() => None,
            None => between(&xv, &yv, &curve, p - 1, v),
        };
        let Some(value) = value else {
            return Err(CalcError::Syntax(format!("{} is outside {xs_name} ({} to {})", shown(x.q, cx), shown(lo, cx), shown(hi, cx))));
        };
        Ok(Val { q: finite(Quantity::new(value, y_dim))?, t })
    })
}

/// The value at `v` between points `i` and `i + 1`: on the line through them, or with `curve`
/// (the spline's second derivatives at every point) on the spline.
fn between(xv: &[f64], yv: &[f64], curve: &[f64], i: usize, v: f64) -> Option<f64> {
    let (x0, x1, y0, y1) = (*xv.get(i)?, *xv.get(i + 1)?, *yv.get(i)?, *yv.get(i + 1)?);
    let h = x1 - x0;
    let (a, b) = ((x1 - v) / h, (v - x0) / h);
    let linear = a * y0 + b * y1;
    match (curve.get(i), curve.get(i + 1)) {
        (Some(m0), Some(m1)) => Some(linear + ((a * a * a - a) * m0 + (b * b * b - b) * m1) * h * h / 6.0),
        _ => Some(linear),
    }
}

/// The second derivatives at every point of the natural cubic spline through (`x`, `y`) (`x`
/// strictly increasing): zero at both ends, the rest solving the spline's tridiagonal system
/// (Thomas algorithm, stable here because the system is diagonally dominant).
fn second_derivatives(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len();
    // Each interval's width and slope.
    let h: Vec<f64> = x.windows(2).map(|w| if let [a, b] = w { b - a } else { 0.0 }).collect();
    let slope: Vec<f64> = y.windows(2).zip(&h).map(|(w, h)| if let [a, b] = w { (b - a) / h } else { 0.0 }).collect();
    // Forward sweep over the inner points: for point i, h[i-1]·m[i-1] + 2(h[i-1] + h[i])·m[i] +
    // h[i]·m[i+1] = 6(slope[i] - slope[i-1]).
    let mut upper = Vec::with_capacity(n);
    let mut rhs = Vec::with_capacity(n);
    let (mut c, mut r) = (0.0, 0.0);
    for (hw, sw) in h.windows(2).zip(slope.windows(2)) {
        let ([h0, h1], [s0, s1]) = (hw, sw) else { continue };
        let w = 2.0 * (h0 + h1) - h0 * c;
        r = (6.0 * (s1 - s0) - h0 * r) / w;
        c = h1 / w;
        upper.push(c);
        rhs.push(r);
    }
    // Back substitution, from the last inner point; the ends stay zero.
    let mut m = vec![0.0; n];
    let mut next = 0.0;
    for (k, (c, r)) in upper.iter().zip(&rhs).enumerate().rev() {
        next = r - c * next;
        if let Some(slot) = m.get_mut(k + 1) {
            *slot = next;
        }
    }
    m
}
