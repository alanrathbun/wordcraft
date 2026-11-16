//! Grids of values (vectors and matrices) and the element-wise operations on them.
//!
//! A grid is rows × cols values stored row by row; a column vector has one column. Operations work
//! element by element, broadcasting a single value over a grid, so every rule that holds for single
//! values (units, temperatures, finiteness) holds for each element. One element in error makes the
//! whole result an error naming that element. A grid holds at most [`MAX_ELEMENTS`], checked before
//! its elements are built.

use crate::eval::{Val, text_use};
use crate::{CalcError, Value};

/// How many elements a grid may hold.
pub(crate) const MAX_ELEMENTS: usize = 10_000;

/// How many characters a text may hold.
pub(crate) const MAX_TEXT_CHARS: usize = 1000;

/// `rows` × `cols` values, row by row: numbers (the default) or texts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Grid<T = Val> {
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    pub(crate) items: Vec<T>,
}

impl<T> Grid<T> {
    /// A grid of `items`, row by row: [`CalcError::TooLarge`] beyond [`MAX_ELEMENTS`], and an
    /// error for no elements or a count that isn't `rows · cols`.
    pub(crate) fn new(rows: usize, cols: usize, items: Vec<T>) -> Result<Grid<T>, CalcError> {
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
pub(crate) fn element_at<T: Clone>(g: &Grid<T>, origin: u8, at: (u32, Option<u32>)) -> Option<T> {
    let place = |i: u32, len: usize| usize::try_from(i).ok()?.checked_sub(usize::from(origin)).filter(|&k| k < len);
    let k = match at {
        (i, None) if g.rows == 1 || g.cols == 1 => place(i, g.items.len())?,
        (_, None) => return None,
        (r, Some(c)) => place(r, g.rows)?.checked_mul(g.cols)?.checked_add(place(c, g.cols)?)?,
    };
    g.items.get(k).cloned()
}

/// An evaluated value: one value, or a grid of them; a number or a text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Data {
    One(Val),
    Grid(Grid),
    Text(String),
    TextGrid(Grid<String>),
}

impl Data {
    /// The public form, without what each value stands for as a temperature.
    pub(crate) fn value(&self) -> Value {
        match self {
            Data::One(v) => Value::One(v.q),
            Data::Grid(g) => Value::Grid { rows: g.rows, cols: g.cols, items: g.items.iter().map(|v| v.q).collect() },
            Data::Text(s) => Value::Text(s.clone()),
            Data::TextGrid(g) => Value::TextGrid { rows: g.rows, cols: g.cols, items: g.items.clone() },
        }
    }

    /// A grid's rows and columns; `None` for a single value or text.
    pub(crate) fn shape(&self) -> Option<(usize, usize)> {
        match self {
            Data::One(_) | Data::Text(_) => None,
            Data::Grid(g) => Some((g.rows, g.cols)),
            Data::TextGrid(g) => Some((g.rows, g.cols)),
        }
    }

    /// The work of building or copying this value, in steps: one per element of a grid, plus one
    /// per 32 bytes of text, so copying text costs in proportion to the memory it takes (a text is
    /// up to [`MAX_TEXT_CHARS`] characters, several times a number's size).
    pub(crate) fn copy_work(&self) -> u64 {
        let elements = |rows: usize, cols: usize| u64::try_from(rows.saturating_mul(cols)).unwrap_or(u64::MAX);
        match self {
            Data::One(_) => 0,
            Data::Grid(g) => elements(g.rows, g.cols),
            Data::Text(s) => text_steps(s),
            Data::TextGrid(g) => g.items.iter().fold(elements(g.rows, g.cols), |n, s| n.saturating_add(text_steps(s))),
        }
    }

    /// The text itself, or a text grid's first element; `None` for numbers.
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Data::Text(s) => Some(s),
            Data::TextGrid(g) => g.items.first().map(String::as_str),
            Data::One(_) | Data::Grid(_) => None,
        }
    }
}

/// The steps of work a text costs to copy: one per 32 bytes (UTF-8).
pub(crate) fn text_steps(text: &str) -> u64 {
    u64::try_from(text.len() / 32).unwrap_or(u64::MAX)
}

/// The error for a `rows` × `cols` grid that would hold both texts and numbers: it names the later
/// of the element `text_at` (a text) and the element `number_at` (a number), then the earlier.
/// Elements are named as [`element_name`] does, counted from `origin`.
pub(crate) fn mixed(rows: usize, cols: usize, text_at: usize, number_at: usize, origin: u8) -> CalcError {
    let (t, n) = (element_name(rows, cols, text_at, origin), element_name(rows, cols, number_at, origin));
    CalcError::Syntax(if text_at > number_at {
        format!("element {t} is text but element {n} is a number")
    } else {
        format!("element {n} is a number but element {t} is text")
    })
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
        // The evaluator refuses text before it gets here, with the place it was used.
        _ => Err(not_numbers(&[a, b])),
    }
}

/// `f` over every element of `a` (or `a` itself when it is a single value, whose error is returned
/// as it is); an element's error names the element, counted from `origin`.
pub(crate) fn map(a: &Data, origin: u8, mut f: impl FnMut(Val) -> Result<Val, CalcError>) -> Result<Data, CalcError> {
    match a {
        Data::One(x) => f(*x).map(Data::One),
        Data::Grid(g) => build(g, origin, g.items.iter().map(|x| f(*x))),
        Data::Text(_) | Data::TextGrid(_) => Err(not_numbers(&[a])),
    }
}

/// The error for text among `operands` of an element-wise operation.
fn not_numbers(operands: &[&Data]) -> CalcError {
    match operands.iter().find_map(|d| d.text()) {
        Some(text) => text_use(text, "in arithmetic"),
        None => CalcError::Syntax("an array operation needs numbers".to_string()),
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
