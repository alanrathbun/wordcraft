//! Grids of values (vectors and matrices) and the element-wise operations on them.
//!
//! A grid is rows × cols values stored row by row; a column vector has one column. Operations work
//! element by element, broadcasting a single value over a grid, so every rule that holds for single
//! values (units, temperatures, finiteness) holds for each element. One element in error makes the
//! whole result an error naming that element. A grid holds at most [`MAX_ELEMENTS`], checked before
//! its elements are built.

use crate::eval::Val;
use crate::{CalcError, Value};

/// How many elements a grid may hold.
pub(crate) const MAX_ELEMENTS: usize = 10_000;

/// `rows` × `cols` values, row by row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Grid {
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    pub(crate) items: Vec<Val>,
}

impl Grid {
    /// A grid of `items`, row by row: [`CalcError::TooLarge`] beyond [`MAX_ELEMENTS`], and an
    /// error for no elements or a count that isn't `rows · cols`.
    pub(crate) fn new(rows: usize, cols: usize, items: Vec<Val>) -> Result<Grid, CalcError> {
        check_size(rows, cols, items.len())?;
        Ok(Grid { rows, cols, items })
    }
}

/// Whether a grid of `rows` × `cols` may be built from `len` elements, so a caller can check
/// before converting them. The cap is checked first (an overflowing shape is too large), then that
/// there is at least one element, then the count.
pub(crate) fn check_size(rows: usize, cols: usize, len: usize) -> Result<(), CalcError> {
    let n = rows.checked_mul(cols).filter(|&n| n <= MAX_ELEMENTS).ok_or(CalcError::TooLarge)?;
    if n == 0 {
        return Err(CalcError::Syntax("an array needs at least one element".to_string()));
    }
    if n != len {
        return Err(CalcError::Syntax(format!("an array of {rows} rows × {cols} needs {n} elements, not {len}")));
    }
    Ok(())
}

/// The element of `g` a quoted value's index (`\i`) picks, counted from `origin`: one index into a
/// vector, or a row and a column; `None` outside the grid, and for one index into a matrix.
pub(crate) fn element_at(g: &Grid, origin: u8, at: (u32, Option<u32>)) -> Option<Val> {
    let place = |i: u32, len: usize| usize::try_from(i).ok()?.checked_sub(usize::from(origin)).filter(|&k| k < len);
    let k = match at {
        (i, None) if g.rows == 1 || g.cols == 1 => place(i, g.items.len())?,
        (_, None) => return None,
        (r, Some(c)) => place(r, g.rows)?.checked_mul(g.cols)?.checked_add(place(c, g.cols)?)?,
    };
    g.items.get(k).copied()
}

/// An evaluated value: one value, or a grid of them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Data {
    One(Val),
    Grid(Grid),
}

impl Data {
    /// The public form, without what each value stands for as a temperature.
    pub(crate) fn value(&self) -> Value {
        match self {
            Data::One(v) => Value::One(v.q),
            Data::Grid(g) => Value::Grid { rows: g.rows, cols: g.cols, items: g.items.iter().map(|v| v.q).collect() },
        }
    }

    /// A grid's rows and columns; `None` for a single value.
    pub(crate) fn shape(&self) -> Option<(usize, usize)> {
        match self {
            Data::One(_) => None,
            Data::Grid(g) => Some((g.rows, g.cols)),
        }
    }
}

/// The shape of combining `a` and `b` element by element: `None` for two single values, a grid's
/// shape when the other is a single value (broadcast) or has the same shape, else
/// [`CalcError::Shape`].
pub(crate) fn result_shape(a: &Data, b: &Data) -> Result<Option<(usize, usize)>, CalcError> {
    match (a.shape(), b.shape()) {
        (Some(l), Some(r)) if l != r => Err(CalcError::Shape { left: l, right: r }),
        (l, r) => Ok(l.or(r)),
    }
}

/// `f` over `a` and `b` element by element, a single value broadcast over a grid. Two single
/// values give `f`'s result (and its error) as it is; an element's error names the element,
/// counted from `origin`.
pub(crate) fn zip(a: &Data, b: &Data, origin: u8, mut f: impl FnMut(Val, Val) -> Result<Val, CalcError>) -> Result<Data, CalcError> {
    result_shape(a, b)?;
    match (a, b) {
        (Data::One(x), Data::One(y)) => f(*x, *y).map(Data::One),
        (Data::Grid(g), Data::One(y)) => build(g, origin, g.items.iter().map(|x| f(*x, *y))),
        (Data::One(x), Data::Grid(g)) => build(g, origin, g.items.iter().map(|y| f(*x, *y))),
        (Data::Grid(g), Data::Grid(h)) => build(g, origin, g.items.iter().zip(&h.items).map(|(x, y)| f(*x, *y))),
    }
}

/// `f` over every element of `a` (or `a` itself when it is a single value, whose error is returned
/// as it is); an element's error names the element, counted from `origin`.
pub(crate) fn map(a: &Data, origin: u8, mut f: impl FnMut(Val) -> Result<Val, CalcError>) -> Result<Data, CalcError> {
    match a {
        Data::One(x) => f(*x).map(Data::One),
        Data::Grid(g) => build(g, origin, g.items.iter().map(|x| f(*x))),
    }
}

/// A grid of `shape`'s shape from per-element results, stopping at the first error.
fn build(shape: &Grid, origin: u8, results: impl Iterator<Item = Result<Val, CalcError>>) -> Result<Data, CalcError> {
    let (rows, cols) = (shape.rows, shape.cols);
    let items = results.enumerate().map(|(k, r)| r.map_err(|e| element_error(rows, cols, k, origin, e))).collect::<Result<Vec<_>, _>>()?;
    Grid::new(rows, cols, items).map(Data::Grid)
}

/// `error` as the error of element `k` of a `rows` × `cols` grid. Running out of work isn't about
/// an element, so it is left as it is.
pub(crate) fn element_error(rows: usize, cols: usize, k: usize, origin: u8, error: CalcError) -> CalcError {
    match error {
        CalcError::TooMuchWork | CalcError::TooComplex => error,
        error => CalcError::Element { at: element_name(rows, cols, k, origin), error: Box::new(error) },
    }
}

/// How element `k` (row-major) of a `rows` × `cols` grid is named, counted from `origin`: a
/// vector's element by one index (`2`; a row vector counts along its columns), a matrix's by row
/// and column (`[1, 2]`).
pub(crate) fn element_name(rows: usize, cols: usize, k: usize, origin: u8) -> String {
    let o = usize::from(origin);
    if rows == 1 || cols == 1 {
        k.saturating_add(o).to_string()
    } else {
        let (r, c) = (k.checked_div(cols).unwrap_or(0), k.checked_rem(cols).unwrap_or(0));
        format!("[{}, {}]", r.saturating_add(o), c.saturating_add(o))
    }
}
