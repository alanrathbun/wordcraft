//! Evaluating calculations: variables, functions, and unit-checked arithmetic.
//!
//! Source is hostile input, so evaluation is bounded three ways: the depth of recursion (shared
//! across user-function calls, so a deep body inside deep calls can't overflow the stack), the
//! depth of user-function calls, and the total number of expression nodes evaluated (so a chain of
//! functions that each call the previous twice can't run for hours).
//!
//! A value is one quantity or a grid of them (a vector or matrix, see [`crate::array`]). Operators
//! and built-in functions work on grids element by element, and every element counts as a step of
//! the recalculation's work. A grid is written as a literal (`[1, 2; 3, 4]`), as a range
//! (`i := 0..5`, which may only define a variable), or element by element in an indexed definition
//! (`t[i] := 2·D[i]`); an element is read by index (`D[2]`, `M[1, 0]`), counted from the
//! document's origin. The array functions (`length`, `sum`, `mmul`, `lookup`, `linterp`…, see
//! [`crate::array_funcs`]) take whole grids rather than applying element by element.

use crate::array::{self, Data, Grid, MAX_ELEMENTS};
use crate::array_funcs;
use crate::funcs::{self, unit};
use crate::{CalcError, Expr, Op, Show, Stmt, Value};
use craft_units::{Dim, Precision, Quantity, Scale, UnitError, UnitSystem, lookup, parse_unit};
use std::cell::Cell;
use std::collections::BTreeMap;

/// How deep user functions may call each other.
const MAX_CALL_DEPTH: usize = 32;
/// How deep evaluation may recurse in total. The parser caps an expression's height at 1024, so
/// this leaves room for a function body called from the bottom of another expression.
const MAX_EVAL_DEPTH: usize = 2048;
/// How many expression nodes one statement may evaluate.
const MAX_STEPS: u32 = 1_000_000;
/// How many expression nodes one recalculation of a whole document may evaluate: what keeps a
/// hostile document from costing seconds on every edit.
pub(crate) const MAX_PASS_STEPS: u64 = 5_000_000;

/// What a value stands for as a temperature, so that it is shown on an offset scale (`°C`, `°F`)
/// only when that means something: `20 Δ°C` or `T_2 - T_1` shown in `°C` would be a wrong number
/// that looks right (−253.15 °C).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Temp {
    /// Not known to be an absolute temperature: a difference, a scaled temperature, a computed
    /// one, or not a temperature at all.
    #[default]
    Plain,
    /// An absolute temperature: a `°C` or `°F` literal, or one plus or minus a difference.
    Absolute,
    /// A bare `K` or `°R` literal (`300 K`): an absolute temperature or a difference, as it is used.
    Either,
}

/// What an absolute temperature combined with another by `+` or `-` stands for: plus or minus a
/// difference it stays absolute; two absolute temperatures differ by a difference (plain) and
/// have no meaningful sum. Anything else is plain.
fn sum_temp(op: Op, a: Temp, b: Temp) -> Temp {
    match (op, a, b) {
        (_, Temp::Absolute, Temp::Absolute) => Temp::Plain,
        (Op::Add, Temp::Absolute, _) | (Op::Add, _, Temp::Absolute) | (Op::Sub, Temp::Absolute, _) => Temp::Absolute,
        _ => Temp::Plain,
    }
}

/// An evaluated value with what it stands for as a temperature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Val {
    pub(crate) q: Quantity,
    pub(crate) t: Temp,
}

impl Val {
    pub(crate) fn plain(q: Quantity) -> Val {
        Val { q, t: Temp::Plain }
    }
}

/// What a name means in an [`Env`].
#[derive(Debug, Clone, PartialEq)]
enum Entry {
    Var(Data),
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
    /// `g` is standard gravity (Mathcad's convention) rather than the gram; the gram is then `gm`.
    g_is_gravity: bool,
    /// What an array's first element is numbered: 0 (Mathcad's convention) or 1.
    origin: u8,
    /// The unit system values in messages are shown in (`1.6 in is outside NPS …`).
    system: UnitSystem,
    /// The decimal places values in messages are shown with.
    precision: Precision,
}

impl Env {
    /// An empty environment (`g` is the gram until [`Env::set_g_is_gravity`]).
    pub fn new() -> Env {
        Env::default()
    }

    /// Make `g` standard gravity (Mathcad's convention), or the gram.
    pub fn set_g_is_gravity(&mut self, on: bool) {
        self.g_is_gravity = on;
    }

    /// A built-in constant as this environment reads it: `g` is gravity when the document says so.
    fn constant(&self, name: &str) -> Option<Quantity> {
        if self.g_is_gravity && name == "g" { constant("g_0") } else { constant(name) }
    }

    /// When `g` is gravity, a display unit spelled with `g` (`= ? g`, `kg/g`) would read as
    /// gravity: refused, naming `gm`.
    pub(crate) fn check_display_unit(&self, text: &str) -> Result<(), CalcError> {
        let gram = text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '°')).any(|w| w == "g");
        if self.g_is_gravity && gram {
            return Err(CalcError::Syntax("g is gravity in this document — write gm for grams".into()));
        }
        Ok(())
    }

    /// Number array elements from `origin`: 0 (Mathcad's convention, the default) or 1; anything
    /// else is 0.
    pub fn set_origin(&mut self, origin: u8) {
        self.origin = if origin == 1 { 1 } else { 0 };
    }

    /// What an array's first element is numbered: 0 or 1.
    pub(crate) fn origin(&self) -> u8 {
        self.origin
    }

    /// Show values in messages as the document shows results: in `system`'s units, to
    /// `precision` decimal places (US customary to 3 places until this is called).
    pub fn set_display(&mut self, system: UnitSystem, precision: Precision) {
        self.system = system;
        self.precision = precision.clamped();
    }

    /// Define a variable (not an absolute temperature: see [`Temp`]).
    pub fn define_var(&mut self, name: &str, q: Quantity) {
        self.entries.insert(name.to_string(), Entry::Var(Data::One(Val::plain(q))));
    }

    /// Define an array variable of `rows` × `cols` values given row by row (none of them an
    /// absolute temperature: see [`Temp`]). [`CalcError::TooLarge`] beyond 10,000 elements, and an
    /// error when `items` doesn't fill the shape; nothing is defined then.
    pub fn define_array(&mut self, name: &str, rows: usize, cols: usize, items: Vec<Quantity>) -> Result<(), CalcError> {
        array::check_size(rows, cols, items.len())?;
        let grid = Grid::new(rows, cols, items.into_iter().map(Val::plain).collect())?;
        self.entries.insert(name.to_string(), Entry::Var(Data::Grid(grid)));
        Ok(())
    }

    /// The value of the variable `name`, with what each value stands for as a temperature.
    pub(crate) fn data(&self, name: &str) -> Option<&Data> {
        match self.entries.get(name) {
            Some(Entry::Var(d)) => Some(d),
            _ => None,
        }
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
/// one of the built-in constants (`π`, `pi`, `e`, `g_0`, `c_0`, `R_u`); and a name with an empty subscript part (`a__b`, `x_`),
/// which would display like a different name (`a_b`, `x`).
pub fn check_name(name: &str) -> Result<(), CalcError> {
    if name.contains('_') && name.split('_').any(str::is_empty) {
        return Err(CalcError::Syntax(format!("{name} has an empty subscript — write a name like a_b")));
    }
    if lookup(name).is_some() || funcs::is_builtin(name) || is_constant(name) {
        return Err(CalcError::ReservedName(name.to_string()));
    }
    Ok(())
}

/// The built-in constants, with their units: π and e, standard gravity `g_0`, the speed of light
/// `c_0` and the molar gas constant `R_u` (exact SI values). `g` alone is the gram, so gravity
/// carries a subscript.
fn constant(name: &str) -> Option<Quantity> {
    let with = |value: f64, unit: &str| parse_unit(unit).ok().map(|s| Quantity::new(value * s.factor, s.dim));
    match name {
        "π" | "pi" => Some(Quantity::dimensionless(std::f64::consts::PI)),
        "e" => Some(Quantity::dimensionless(std::f64::consts::E)),
        "g_0" => with(9.80665, "m/s^2"),
        "c_0" => with(299_792_458.0, "m/s"),
        "R_u" => with(8.314_462_618_153_24, "J/(mol·K)"),
        _ => None,
    }
}

fn is_constant(name: &str) -> bool {
    constant(name).is_some()
}

/// Evaluate an expression against the defined names.
pub fn eval_expr(env: &Env, e: &Expr) -> Result<Value, CalcError> {
    let pass = Pass::new(u64::MAX);
    eval_data(env, e, &pass).map(|d| d.value())
}

/// The work a whole recalculation may do, shared by every statement it runs.
pub(crate) struct Pass {
    spent: Cell<u64>,
    limit: u64,
}

impl Pass {
    pub(crate) fn new(limit: u64) -> Pass {
        Pass { spent: Cell::new(0), limit }
    }

    /// Count `n` steps of work: [`CalcError::TooMuchWork`], with nothing counted, once that would
    /// pass the limit.
    pub(crate) fn charge(&self, n: u64) -> Result<(), CalcError> {
        let spent = self.spent.get().saturating_add(n);
        if spent > self.limit {
            return Err(CalcError::TooMuchWork);
        }
        self.spent.set(spent);
        Ok(())
    }
}

fn eval_data(env: &Env, e: &Expr, pass: &Pass) -> Result<Data, CalcError> {
    let cx = Cx { env, steps: Cell::new(0), pass };
    eval(&cx, &Frame::GLOBAL, e, 0)
}

/// Run one statement: a definition adds to `env` and an evaluation does not. Returns the value
/// when the statement shows one.
///
/// Nothing is defined unless the whole statement succeeds, including a `= ? unit` that does not
/// match the value's dimension. The unit itself is applied by whoever formats the value.
pub fn run_stmt(env: &mut Env, s: &Stmt) -> Result<Option<Value>, CalcError> {
    Ok(run_stmt_in(env, s, &Pass::new(u64::MAX))?.map(|d| d.value()))
}

/// [`run_stmt`], counting its work against `pass`: once the pass has done its limit, every
/// statement is [`CalcError::TooMuchWork`].
pub(crate) fn run_stmt_in(env: &mut Env, s: &Stmt, pass: &Pass) -> Result<Option<Data>, CalcError> {
    match s {
        Stmt::Eval { expr, show } => {
            let v = eval_data(env, expr, pass)?;
            check_show(env, show, &v)?;
            Ok(Some(v))
        }
        Stmt::Define { name, params, body, show, index } => {
            check_name(name)?;
            params.iter().try_for_each(|p| check_name(p))?;
            let is_range = matches!(body, Expr::Range { .. });
            if is_range && (index.is_some() || !params.is_empty()) {
                return Err(range_only_defines_a_variable());
            }
            if index.is_some() {
                // The parser never gives an indexed definition parameters.
                if !params.is_empty() {
                    return Err(CalcError::Syntax("an indexed definition takes one range variable, like t[i] := 2·D[i]".to_string()));
                }
            } else if !params.is_empty() {
                if show.is_some() {
                    return Err(CalcError::Syntax("a function has no single value to show — call it, like f(2) =".to_string()));
                }
                env.define_fn(name, params.clone(), body.clone());
                return Ok(None);
            }
            let v = match (index, body) {
                (Some(i), _) => indexed(env, name, i, body, pass)?,
                (None, Expr::Range { first, second, last }) => {
                    let cx = Cx { env, steps: Cell::new(0), pass };
                    range(&cx, first, second.as_deref(), last)?
                }
                (None, _) => eval_data(env, body, pass)?,
            };
            if let Some(show) = show {
                check_show(env, show, &v)?;
            }
            let shown = show.as_ref().map(|_| v.clone());
            env.entries.insert(name.to_string(), Entry::Var(v));
            Ok(shown)
        }
    }
}

/// A `= ? unit` must be a unit of the value's dimension (of every element's, for a grid).
fn check_show(env: &Env, show: &Show, v: &Data) -> Result<(), CalcError> {
    match &show.unit {
        Some(text) => check_display(env, text, v),
        None => Ok(()),
    }
}

/// Whether `v` can be shown in the unit `text` ([`check_unit_for`]); for a grid, every element,
/// the first that can't named in the error.
pub(crate) fn check_display(env: &Env, text: &str, v: &Data) -> Result<(), CalcError> {
    env.check_display_unit(text)?;
    match v {
        Data::One(v) => check_unit_for(*v, text),
        Data::Grid(g) => {
            let scale = parse_unit(text).map_err(unit)?;
            g.items
                .iter()
                .enumerate()
                .try_for_each(|(k, v)| fits(*v, &scale, text).map_err(|e| array::element_error(g.rows, g.cols, k, env.origin, e)))
        }
    }
}

/// Whether `v` can be shown in the unit `text`: one of its dimension and, for an offset scale
/// (`°C`, `°F`), an absolute temperature ([`Temp`]).
pub(crate) fn check_unit_for(v: Val, text: &str) -> Result<(), CalcError> {
    let scale = parse_unit(text).map_err(unit)?;
    fits(v, &scale, text)
}

/// [`check_unit_for`] with the unit `text` already parsed as `scale`.
fn fits(v: Val, scale: &Scale, text: &str) -> Result<(), CalcError> {
    if !v.q.dim.same_ignoring_angle(scale.dim) {
        return Err(CalcError::Unit(UnitError::Mismatch { left: v.q.dim, right: scale.dim }));
    }
    if scale.offset != 0.0 && v.t == Temp::Plain {
        return Err(CalcError::NotAbsolute(text.trim().to_string()));
    }
    Ok(())
}

/// Shared state of one evaluation.
struct Cx<'a> {
    env: &'a Env,
    /// Expression nodes evaluated so far.
    steps: Cell<u32>,
    /// The whole recalculation's work.
    pass: &'a Pass,
}

/// The parameters in scope, and how many user functions deep we are.
struct Frame<'a> {
    names: &'a [String],
    values: &'a [Data],
    calls: usize,
}

impl Frame<'_> {
    const GLOBAL: Frame<'static> = Frame { names: &[], values: &[], calls: 0 };

    fn param(&self, name: &str) -> Option<&Data> {
        self.names.iter().position(|n| n == name).and_then(|i| self.values.get(i))
    }
}

/// Count one expression node against the statement's and the pass's limits.
fn tick(cx: &Cx<'_>) -> Result<(), CalcError> {
    let steps = cx.steps.get() + 1;
    if steps > MAX_STEPS {
        return Err(CalcError::TooComplex);
    }
    cx.steps.set(steps);
    cx.pass.charge(1)
}

fn eval(cx: &Cx<'_>, fr: &Frame<'_>, e: &Expr, depth: usize) -> Result<Data, CalcError> {
    if depth > MAX_EVAL_DEPTH {
        return Err(CalcError::TooComplex);
    }
    tick(cx)?;
    match e {
        Expr::Num(v) => finite(*v).map(|q| Data::One(Val::plain(q))),
        Expr::Name(n) => resolve(cx, fr, n),
        Expr::Quantity(v, text) => {
            let scale = parse_unit(text).map_err(unit)?;
            let t = literal_temp(text, &scale);
            from_scale(*v, &scale).map(|q| Data::One(Val { q, t }))
        }
        Expr::Unary(Op::Neg, a) => {
            refuse_offset_operand(cx, fr, &[a])?;
            let x = eval(cx, fr, a, depth + 1)?;
            each(cx, &x, |v| v.q.neg().map(Val::plain).map_err(unit))
        }
        Expr::Unary(..) => Err(CalcError::Syntax("unsupported operator".to_string())),
        Expr::Abs(a) => {
            let x = eval(cx, fr, a, depth + 1)?;
            each(cx, &x, |v| finite_q(Quantity::new(v.q.value.abs(), v.q.dim)).map(Val::plain))
        }
        Expr::Binary(op, a, b) => {
            if *op == Op::Mul
                && let Some(q) = offset_literal(cx, fr, a, b)
            {
                return q.map(|q| Data::One(Val { q, t: Temp::Absolute }));
            }
            // Offset literals may be combined with an interval but not scaled (`5 °F/s` would be a
            // wrong number that looks right) or combined with each other.
            if matches!(op, Op::Add | Op::Sub) {
                // An absolute temperature literal may only be combined with a temperature
                // interval literal (`100 °C + 5 K`); `100 °C + 5 °C`, `T + 5 °C` and `100 °C + T`
                // have no meaning, and a variable can't be told apart from an absolute value.
                let (abs_a, abs_b) = (is_offset_literal(cx, fr, a), is_offset_literal(cx, fr, b));
                // Subtracting an absolute temperature from anything (`5 K - 100 °C`) is meaningless.
                let sub_abs = *op == Op::Sub && abs_b;
                if sub_abs || (abs_a && !is_non_offset_temperature_literal(cx, fr, b)) || (abs_b && !is_non_offset_temperature_literal(cx, fr, a)) {
                    return Err(CalcError::Unit(UnitError::AffineTemperature));
                }
            } else {
                refuse_offset_operand(cx, fr, &[a, b])?;
            }
            let x = eval(cx, fr, a, depth + 1)?;
            let y = eval(cx, fr, b, depth + 1)?;
            // `5 K` is a temperature literal: what it stands for comes from its syntax.
            let literal = match op {
                Op::Mul => unit_literal(cx, fr, a, b).map_or(Temp::Plain, |(_, scale, name)| literal_temp(name, &scale)),
                _ => Temp::Plain,
            };
            pair(cx, &x, &y, |x, y| {
                let q = apply(*op, x.q, y.q)?;
                let t = match op {
                    Op::Add | Op::Sub => sum_temp(*op, x.t, y.t),
                    Op::Mul => literal,
                    _ => Temp::Plain,
                };
                Ok(Val { q, t })
            })
        }
        Expr::Call(name, args) => call(cx, fr, name, args, depth),
        Expr::Matrix { rows, cols, items } => matrix(cx, fr, *rows, *cols, items, depth),
        Expr::Index(name, at) => index(cx, fr, name, at, depth),
        // A range is evaluated only as the whole right side of a variable's definition.
        Expr::Range { .. } => Err(range_only_defines_a_variable()),
    }
}

fn range_only_defines_a_variable() -> CalcError {
    CalcError::Syntax("a range defines a variable, like i := 0..5".to_string())
}

/// How far a number may be from a whole number and still count as it (an index, a range's
/// element count).
const WHOLE: f64 = 1e-9;

/// A literal `[…]` of `rows` × `cols` items, row by row. Each item is one value.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn matrix(cx: &Cx<'_>, fr: &Frame<'_>, rows: usize, cols: usize, items: &[Expr], depth: usize) -> Result<Data, CalcError> {
    array::check_size(rows, cols, items.len())?;
    let values = items
        .iter()
        .map(|e| match eval(cx, fr, e, depth + 1)? {
            Data::One(v) => Ok(v),
            Data::Grid(_) => Err(CalcError::Syntax("a matrix element can't be a vector".to_string())),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Grid::new(rows, cols, values).map(Data::Grid)
}

/// The element `name[at]`: one index into a vector, or a row and a column. Only the element is
/// copied, so reading one costs one step however large the grid. The name and shape are checked
/// before the indexes are evaluated.
// Not inlined, and the lookup and messages kept in helpers that aren't, so the recursive frames
// (`eval` → `index` → `index_value` → `eval`) stay small: their depth is bounded by `MAX_EVAL_DEPTH`.
#[inline(never)]
fn index(cx: &Cx<'_>, fr: &Frame<'_>, name: &str, at: &[Expr], depth: usize) -> Result<Data, CalcError> {
    let g = indexed_grid(cx, fr, name, at.len())?;
    let mut idx = [0.0; 2];
    for (slot, e) in idx.iter_mut().zip(at) {
        *slot = index_value(cx, fr, e, depth)?;
    }
    element(g, name, cx.env.origin, idx.get(..at.len()).unwrap_or_default())
}

/// The grid `name` holds, when it can be read with `count` indexes.
#[inline(never)]
fn indexed_grid<'a>(cx: &'a Cx<'_>, fr: &'a Frame<'_>, name: &str, count: usize) -> Result<&'a Grid, CalcError> {
    let target = match fr.param(name) {
        Some(d) => d,
        None => match cx.env.entries.get(name) {
            Some(Entry::Var(d)) => d,
            Some(Entry::Failed) => return Err(CalcError::Upstream),
            Some(Entry::Func { .. }) => return Err(not_a_value(name)),
            None => {
                // A constant or unit is a single value; anything else is undefined.
                resolve(cx, fr, name)?;
                return Err(not_a_vector(name));
            }
        },
    };
    let Data::Grid(g) = target else {
        return Err(not_a_vector(name));
    };
    match count {
        1 if g.rows == 1 || g.cols == 1 => Ok(g),
        1 => Err(CalcError::IndexRange(format!("{name} is a matrix — give a row and a column, like {name}[1, 0]"))),
        2 => Ok(g),
        _ => Err(CalcError::Syntax("an index is one number, or a row and a column, like D[2] or M[1, 0]".to_string())),
    }
}

/// The element of `g` (named `name`) at the whole-number indexes `at`, counted from `origin`: one
/// index into a vector, or a row and a column.
#[inline(never)]
fn element(g: &Grid, name: &str, origin: u8, at: &[f64]) -> Result<Data, CalcError> {
    let o = origin;
    let last = |n: usize| n.saturating_sub(1).saturating_add(usize::from(o));
    let k = match *at {
        [i] => slot(i, o, g.items.len())
            .ok_or_else(|| CalcError::IndexRange(format!("index {} is outside {name} ({o} to {})", index_text(i), last(g.items.len()))))?,
        [r, c] => match (slot(r, o, g.rows), slot(c, o, g.cols)) {
            (Some(r), Some(c)) => r.saturating_mul(g.cols).saturating_add(c),
            _ => {
                return Err(CalcError::IndexRange(format!(
                    "index [{}, {}] is outside {name} (rows {o} to {}, columns {o} to {})",
                    index_text(r),
                    index_text(c),
                    last(g.rows),
                    last(g.cols)
                )));
            }
        },
        _ => return Err(CalcError::Syntax("an index is one number, or a row and a column, like D[2] or M[1, 0]".to_string())),
    };
    g.items.get(k).map(|v| Data::One(*v)).ok_or_else(|| CalcError::IndexRange(format!("{name} has no element {k}")))
}

fn not_a_vector(name: &str) -> CalcError {
    CalcError::IndexRange(format!("{name} isn't a vector"))
}

/// An index: a plain number within [`WHOLE`] of a whole number, which it is rounded to.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn index_value(cx: &Cx<'_>, fr: &Frame<'_>, e: &Expr, depth: usize) -> Result<f64, CalcError> {
    match eval(cx, fr, e, depth + 1)? {
        Data::One(v) if v.q.dim == Dim::ONE => whole(v.q.value),
        _ => None,
    }
    .ok_or_else(not_whole)
}

#[cold]
fn not_whole() -> CalcError {
    CalcError::Syntax("an index is a whole number, like D[2]".to_string())
}

/// `v` rounded to the whole number it is within [`WHOLE`] of.
fn whole(v: f64) -> Option<f64> {
    let r = v.round();
    (v.is_finite() && (v - r).abs() <= WHOLE).then_some(r)
}

/// Where the whole number `i`, counted from `origin`, falls among `len` places; `None` outside.
fn slot(i: f64, origin: u8, len: usize) -> Option<usize> {
    let k = i - f64::from(origin);
    // `len` is at most `MAX_ELEMENTS`, so the comparison is exact and the cast can't truncate.
    (k >= 0.0 && k < len as f64).then_some(k as usize)
}

/// A whole number as the author would write it: `3`, `-1`, and `1e300` for one too large to spell.
fn index_text(i: f64) -> String {
    if i.abs() < 1e15 { format!("{}", i as i64) } else { format!("{i:e}") }
}

/// The range `first..last` or `first, second..last`, both ends included: a column of the values
/// from `first` stepping by `second − first` (by 1 towards `last` when there is no second value)
/// for as long as they don't pass `last`.
fn range(cx: &Cx<'_>, first: &Expr, second: Option<&Expr>, last: &Expr) -> Result<Data, CalcError> {
    tick(cx)?;
    let end = |e: &Expr| match eval(cx, &Frame::GLOBAL, e, 1)? {
        Data::One(v) => Ok(v),
        Data::Grid(_) => Err(CalcError::Syntax("a range's values are single values, like 0..5".to_string())),
    };
    let a = end(first)?;
    let b = second.map(end).transpose()?;
    let z = end(last)?;
    let span = z.q.sub(a.q).map_err(unit)?;
    let step = match b {
        Some(b) => b.q.sub(a.q).map_err(unit)?,
        None if a.q.dim == Dim::ONE && z.q.dim == Dim::ONE => Quantity::dimensionless(if span.value < 0.0 { -1.0 } else { 1.0 }),
        // Counting by 1 in SI units (1 m from 0 in) would be a wrong number that looks right.
        None => return Err(CalcError::Syntax("a range with units needs a step, like 0 in, 0.5 in..2 in".to_string())),
    };
    if step.value == 0.0 {
        return Err(CalcError::Syntax("a range's step is zero — its second value must differ from the first, like 1, 1.5..3".to_string()));
    }
    let ratio = span.value / step.value;
    if ratio.is_nan() {
        return Err(CalcError::NonFinite);
    }
    if ratio + WHOLE < 0.0 {
        return Err(CalcError::Syntax("a range's step goes away from its last value — like 1, 1.5..3 or 3, 2.5..1".to_string()));
    }
    // An infinite or huge count is too large; the cast below is then of a number at most 10,000.
    let count = (ratio + WHOLE).floor() + 1.0;
    if !count.is_finite() || count > MAX_ELEMENTS as f64 {
        return Err(CalcError::TooLarge);
    }
    let n = count as usize;
    charge_elements(cx, n)?;
    let t = between(a.t, z.t);
    let values =
        (0..n).map(|k| a.q.add(Quantity::new(k as f64 * step.value, step.dim)).map(|q| Val { q, t }).map_err(unit)).collect::<Result<Vec<_>, _>>()?;
    Grid::new(n, 1, values).map(Data::Grid)
}

/// What values between two temperatures stand for: absolute when either end is, a bare `K`
/// temperature when both ends are, and plain otherwise.
fn between(a: Temp, b: Temp) -> Temp {
    match (a, b) {
        (Temp::Plain, _) | (_, Temp::Plain) => Temp::Plain,
        (Temp::Absolute, _) | (_, Temp::Absolute) => Temp::Absolute,
        _ => Temp::Either,
    }
}

/// The indexed definition `name[i] := body`: `body` evaluated once for each element of the range
/// variable `i`, with `i` bound to that element, setting element `i` of `name`. Every element from
/// the origin up to the largest index must be set; a body that is a vector, or an `i` that isn't
/// a vector of whole numbers, is an error.
fn indexed(env: &Env, name: &str, i: &str, body: &Expr, pass: &Pass) -> Result<Data, CalcError> {
    let range = match env.entries.get(i) {
        Some(Entry::Var(d)) => d,
        Some(Entry::Failed) => return Err(CalcError::Upstream),
        Some(Entry::Func { .. }) => return Err(not_a_value(i)),
        None => return Err(CalcError::Undefined(i.to_string())),
    };
    let not_a_range = || CalcError::Syntax(format!("{i} must be a range of whole numbers, like {i} := 0..5"));
    let g = match range {
        Data::Grid(g) if g.rows == 1 || g.cols == 1 => g,
        _ => return Err(not_a_range()),
    };
    let o = env.origin;
    // Where each element of `i` puts its value, checked before anything is built.
    let mut places = Vec::with_capacity(g.items.len());
    let mut size = 0.0_f64;
    for v in &g.items {
        let w = (v.q.dim == Dim::ONE).then(|| whole(v.q.value)).flatten().ok_or_else(not_a_range)?;
        let k = w - f64::from(o);
        if k < 0.0 {
            return Err(CalcError::IndexRange(format!("index {} is outside {name} — its elements start at {o}", index_text(w))));
        }
        size = size.max(k + 1.0);
        places.push(k);
    }
    if size > MAX_ELEMENTS as f64 {
        return Err(CalcError::TooLarge);
    }
    // At most `MAX_ELEMENTS`, so the casts are exact.
    let size = size as usize;
    let cx = Cx { env, steps: Cell::new(0), pass };
    charge_elements(&cx, size)?;
    let places: Vec<usize> = places.into_iter().map(|k| k as usize).collect();
    let mut set = vec![false; size];
    for &k in &places {
        if let Some(s) = set.get_mut(k) {
            *s = true;
        }
    }
    let unset =
        |k: usize| CalcError::Syntax(format!("{name}[{}] isn't set by {i} — the range must cover every element", k.saturating_add(usize::from(o))));
    if let Some(k) = set.iter().position(|s| !s) {
        return Err(unset(k));
    }
    let names = [i.to_string()];
    let mut values: Vec<Option<Val>> = vec![None; size];
    for (&k, v) in places.iter().zip(&g.items) {
        let bound = [Data::One(*v)];
        let frame = Frame { names: &names, values: &bound, calls: 0 };
        let value = match eval(&cx, &frame, body, 0).map_err(|e| array::element_error(size, 1, k, o, e))? {
            Data::One(v) => v,
            Data::Grid(_) => return Err(CalcError::Syntax(format!("{name}[{i}] is one element — its right side can't be a vector"))),
        };
        if let Some(slot) = values.get_mut(k) {
            *slot = Some(value);
        }
    }
    let values = values.into_iter().enumerate().map(|(k, v)| v.ok_or_else(|| unset(k))).collect::<Result<Vec<_>, _>>()?;
    Grid::new(size, 1, values).map(Data::Grid)
}

/// `f` over every element of `x`, each counted as a step of the pass's work.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn each(cx: &Cx<'_>, x: &Data, f: impl FnMut(Val) -> Result<Val, CalcError>) -> Result<Data, CalcError> {
    if let Some((rows, cols)) = x.shape() {
        charge_elements(cx, rows.saturating_mul(cols))?;
    }
    array::map(x, cx.env.origin, f)
}

/// `f` over `x` and `y` element by element (a single value broadcast over a grid), each element
/// counted as a step of the pass's work.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn pair(cx: &Cx<'_>, x: &Data, y: &Data, f: impl FnMut(Val, Val) -> Result<Val, CalcError>) -> Result<Data, CalcError> {
    if let Some((rows, cols)) = array::result_shape(x, y)? {
        charge_elements(cx, rows.saturating_mul(cols))?;
    }
    array::zip(x, y, cx.env.origin, f)
}

/// Count `n` elements' work against the pass.
fn charge_elements(cx: &Cx<'_>, n: usize) -> Result<(), CalcError> {
    cx.pass.charge(u64::try_from(n).unwrap_or(u64::MAX))
}

/// What a literal in the unit `name` stands for as a temperature: absolute on an offset scale,
/// either in a plain temperature unit (`K`, `°R`), a difference in `Δ°C`/`Δ°F`.
fn literal_temp(name: &str, scale: &Scale) -> Temp {
    if scale.dim != Dim::TEMPERATURE {
        Temp::Plain
    } else if scale.offset != 0.0 {
        Temp::Absolute
    } else if name.trim_start().starts_with(['Δ', '∆']) {
        Temp::Plain
    } else {
        Temp::Either
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

// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
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

/// `number unit` where the unit name is not shadowed by a parameter, variable or constant: the
/// number, the unit's scale and its name.
fn unit_literal<'e>(cx: &Cx<'_>, fr: &Frame<'_>, num: &Expr, unit_name: &'e Expr) -> Option<(f64, Scale, &'e str)> {
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
    if fr.param(n).is_some() || cx.env.entries.contains_key(n.as_str()) || cx.env.constant(n).is_some() {
        return None;
    }
    Some((v, lookup(n)?, n))
}

/// `5 °F`: a number times an offset unit that no variable or constant shadows is an absolute
/// temperature. Anything else involving an offset unit is refused in [`resolve`].
fn offset_literal(cx: &Cx<'_>, fr: &Frame<'_>, num: &Expr, unit_name: &Expr) -> Option<Result<Quantity, CalcError>> {
    let (v, scale, _) = unit_literal(cx, fr, num, unit_name)?;
    (scale.offset != 0.0).then(|| from_scale(v, &scale))
}

/// Whether `e` is `number unit` for a temperature unit with no offset (`5 K`, `5 °R`, `5 Δ°C`):
/// a temperature interval.
fn is_non_offset_temperature_literal(cx: &Cx<'_>, fr: &Frame<'_>, e: &Expr) -> bool {
    matches!(e, Expr::Binary(Op::Mul, a, b)
        if unit_literal(cx, fr, a, b).is_some_and(|(_, s, _)| s.offset == 0.0 && s.dim == Dim::TEMPERATURE))
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
fn resolve(cx: &Cx<'_>, fr: &Frame<'_>, name: &str) -> Result<Data, CalcError> {
    if let Some(v) = fr.param(name) {
        return copy(cx, v);
    }
    match cx.env.entries.get(name) {
        Some(Entry::Var(v)) => return copy(cx, v),
        Some(Entry::Failed) => return Err(CalcError::Upstream),
        Some(Entry::Func { .. }) => return Err(not_a_value(name)),
        None => {}
    }
    if let Some(q) = cx.env.constant(name) {
        return Ok(Data::One(Val::plain(q)));
    }
    // A unit wins over a built-in of the same spelling: `5 min` is minutes, `min(1, 2)` a call.
    if let Some(scale) = lookup(name) {
        if scale.offset != 0.0 {
            return Err(CalcError::Unit(UnitError::AffineTemperature));
        }
        return finite_q(Quantity::new(scale.factor, scale.dim)).map(|q| Data::One(Val::plain(q)));
    }
    if funcs::is_builtin(name) {
        return Err(not_a_value(name));
    }
    Err(CalcError::Undefined(name.to_string()))
}

/// A copy of a name's value. Copying a grid is work: each element counts as a step, so a hostile
/// calculation can't copy large grids for free.
fn copy(cx: &Cx<'_>, v: &Data) -> Result<Data, CalcError> {
    if let Some((rows, cols)) = v.shape() {
        charge_elements(cx, rows.saturating_mul(cols))?;
    }
    Ok(v.clone())
}

// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn call(cx: &Cx<'_>, fr: &Frame<'_>, name: &str, args: &[Expr], depth: usize) -> Result<Data, CalcError> {
    // Resolve what is being called before evaluating the arguments, so a missing name is reported
    // as missing rather than after unrelated errors in its arguments.
    let user = if fr.param(name).is_some() { None } else { cx.env.entries.get(name) };
    match user {
        Some(Entry::Failed) => return Err(CalcError::Upstream),
        Some(Entry::Var(..)) => return Err(not_callable(name)),
        Some(Entry::Func { params, body }) => return call_user(cx, fr, name, params, body, args, depth),
        None => {}
    }
    if fr.param(name).is_some() {
        return Err(not_callable(name));
    }
    if !funcs::is_builtin(name) {
        return if is_constant(name) || lookup(name).is_some() { Err(not_callable(name)) } else { Err(CalcError::Undefined(name.to_string())) };
    }
    let exprs = args;
    let args = eval_args(cx, fr, exprs, depth)?;
    let env = cx.env;
    let acx = array_funcs::Ctx { origin: env.origin, pass: cx.pass, system: env.system, precision: env.precision };
    if let Some(result) = array_funcs::call_array(name, &args, exprs, &acx) {
        return result;
    }
    let values: Vec<Val> = args.iter().filter_map(|a| if let Data::One(v) = a { Some(*v) } else { None }).collect();
    if values.len() < args.len() {
        return call_each(cx, name, &args);
    }
    let quantities: Vec<Quantity> = values.iter().map(|v| v.q).collect();
    let q = funcs::call(name, &quantities).unwrap_or_else(|| Err(CalcError::Undefined(name.to_string())))?;
    // The smaller or larger of absolute temperatures is one of them; any other result is plain.
    let t = if matches!(name, "min" | "max") { picked(&values) } else { Temp::Plain };
    Ok(Data::One(Val { q, t }))
}

/// What a value picked from `values` (their smallest, largest, or one between them) stands for
/// as a temperature: absolute when they are all temperatures and one is absolute, a bare `K`
/// temperature when they all are, and plain otherwise (and for no values).
pub(crate) fn picked(values: &[Val]) -> Temp {
    if values.is_empty() || values.iter().any(|v| v.t == Temp::Plain) {
        Temp::Plain
    } else if values.iter().any(|v| v.t == Temp::Absolute) {
        Temp::Absolute
    } else {
        Temp::Either
    }
}

/// A scalar built-in called with a grid among its arguments: applied element by element over
/// every argument, a single value broadcast. The result of each is not an absolute temperature.
/// (`min`, `max` and the array functions take whole grids, in [`array_funcs::call_array`].)
fn call_each(cx: &Cx<'_>, name: &str, args: &[Data]) -> Result<Data, CalcError> {
    let builtin = |qs: &[Quantity]| funcs::call(name, qs).unwrap_or_else(|| Err(CalcError::Undefined(name.to_string()))).map(Val::plain);
    // The number of arguments is checked once, before any element: not "element 0: sqrt expects 1".
    let firsts: Vec<Quantity> = args.iter().filter_map(first).map(|v| v.q).collect();
    if let Err(e @ CalcError::Arity(..)) = builtin(&firsts) {
        return Err(e);
    }
    match args {
        [x] => each(cx, x, |x| builtin(&[x.q])),
        [x, y] => pair(cx, x, y, |x, y| builtin(&[x.q, y.q])),
        // Every scalar built-in but `min` and `max` (which never get here) takes one or two
        // arguments, so the check above reported any other count.
        _ => Err(CalcError::Arity(name.to_string(), 2)),
    }
}

/// A single value, or a grid's first element.
fn first(d: &Data) -> Option<Val> {
    match d {
        Data::One(v) => Some(*v),
        Data::Grid(g) => g.items.first().copied(),
    }
}

fn not_callable(name: &str) -> CalcError {
    CalcError::Syntax(format!("{name} isn't a function — it can't be called with ( )"))
}

fn eval_args(cx: &Cx<'_>, fr: &Frame<'_>, args: &[Expr], depth: usize) -> Result<Vec<Data>, CalcError> {
    args.iter().map(|a| eval(cx, fr, a, depth + 1)).collect()
}

/// Call a user function in a scope holding only its own parameters (and the global names).
fn call_user(cx: &Cx<'_>, fr: &Frame<'_>, name: &str, params: &[String], body: &Expr, args: &[Expr], depth: usize) -> Result<Data, CalcError> {
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
