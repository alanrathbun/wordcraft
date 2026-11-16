//! The syntax tree of a calculation, and what a statement defines and uses.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// One calculation statement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Stmt {
    /// `name := body`, `name(params) := body` or `name[i] := body`, optionally followed by `=` to
    /// show the value.
    Define {
        /// The defined name.
        name: String,
        /// The range variable of an indexed definition `name[i] := body`, which defines every
        /// element of `name` at once; `None` otherwise.
        index: Option<String>,
        /// Parameter names; empty for a plain variable.
        params: Vec<String>,
        /// The defining expression.
        body: Expr,
        /// Present when the statement ends with `=` (define and show).
        show: Option<Show>,
    },
    /// `expr =` or `expr = ? unit`: evaluate and show.
    Eval {
        /// The expression to evaluate.
        expr: Expr,
        /// How to show it.
        show: Show,
    },
}

/// How a result is shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Show {
    /// The unit text after `=` (with any leading `?` removed), verbatim and unparsed; `None` to
    /// let the evaluator choose a unit.
    pub unit: Option<String>,
}

/// An expression.
// Not `Deserialize`: the parser's depth limit is what keeps recursion over an `Expr` safe.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Expr {
    /// A number.
    Num(f64),
    /// A text literal, without its quotes: `"OK"`.
    Text(String),
    /// A variable, unit or constant; the evaluator decides which.
    Name(String),
    /// A number with a unit. The parser never produces this (`5 kN` parses as `5 * kN`); it is for
    /// the evaluator and for building expressions directly.
    Quantity(f64, String),
    /// A prefix operator (only [`Op::Neg`]).
    Unary(Op, Box<Expr>),
    /// An infix operator.
    Binary(Op, Box<Expr>, Box<Expr>),
    /// A comparison or a chain of them: `a < b` is one link, `a < b ≤ c` two (`rest` is never
    /// empty). A chain holds when every link does.
    Compare {
        /// The leftmost operand.
        first: Box<Expr>,
        /// Each further operand with the comparison that joins it to the one before.
        rest: Vec<(Cmp, Expr)>,
    },
    /// `a and b` or `a or b`.
    Logic(Logic, Box<Expr>, Box<Expr>),
    /// `not a`.
    Not(Box<Expr>),
    /// A function call; `√x` is `Call("sqrt", [x])`.
    Call(String, Vec<Expr>),
    /// Absolute value, `|x|`.
    Abs(Box<Expr>),
    /// A vector or matrix literal, `[1, 2; 3, 4]`: `rows × cols` items, row-major. A list with no
    /// `;` (`[1, 2, 3]`) is a column.
    Matrix {
        /// Number of rows.
        rows: usize,
        /// Number of columns.
        cols: usize,
        /// The elements, row by row.
        items: Vec<Expr>,
    },
    /// An element of a named vector or matrix: `D[2]` (one index) or `M[i, j]` (two).
    Index(String, Vec<Expr>),
    /// A range, `first..last` or `first, second..last` (the second value sets the step). The
    /// parser only produces one as the whole right side of a definition.
    Range {
        /// The first value.
        first: Box<Expr>,
        /// The second value, when it is given.
        second: Option<Box<Expr>>,
        /// The last value.
        last: Box<Expr>,
    },
}

/// A comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cmp {
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `≤` or `<=`
    Le,
    /// `≥` or `>=`
    Ge,
    /// `==`
    Eq,
    /// `≠` or `!=`
    Ne,
}

impl Cmp {
    /// The symbol it is shown as and named by in messages.
    pub fn symbol(self) -> &'static str {
        match self {
            Cmp::Lt => "<",
            Cmp::Gt => ">",
            Cmp::Le => "≤",
            Cmp::Ge => "≥",
            Cmp::Eq => "==",
            Cmp::Ne => "≠",
        }
    }
}

/// A logical connective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Logic {
    /// `and` or `∧`
    And,
    /// `or` or `∨`
    Or,
}

/// An operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`, `·`, `×` or adjacency
    Mul,
    /// `/`
    Div,
    /// `^`
    Pow,
    /// Prefix `-`
    Neg,
}

/// The name a statement defines, if it is a definition.
pub fn defined_name(s: &Stmt) -> Option<&str> {
    match s {
        Stmt::Define { name, .. } => Some(name),
        Stmt::Eval { .. } => None,
    }
}

/// The names a statement uses (variables, units, constants, called functions and indexed
/// vectors), minus a function's own parameters, without duplicates, in order of first use. The
/// range variable of an indexed definition (`i` in `t[i] := …`) is used, so it comes first.
pub fn free_names(s: &Stmt) -> Vec<String> {
    let (body, params, index): (&Expr, &[String], Option<&String>) = match s {
        Stmt::Define { params, body, index, .. } => (body, params, index.as_ref()),
        Stmt::Eval { expr, .. } => (expr, &[], None),
    };
    let mut out: Vec<String> = Vec::new();
    // Names already in `out` (or parameters): one lookup each, so a long body costs linear time.
    let mut seen: HashSet<&str> = params.iter().map(String::as_str).collect();
    if let Some(i) = index
        && seen.insert(i)
    {
        out.push(i.clone());
    }
    // An explicit stack keeps this safe on trees built by hand, deeper than the parser allows.
    let mut stack: Vec<&Expr> = vec![body];
    while let Some(e) = stack.pop() {
        let name = match e {
            Expr::Num(_) | Expr::Text(_) | Expr::Quantity(..) => continue,
            Expr::Name(n) => n,
            Expr::Unary(_, a) | Expr::Abs(a) | Expr::Not(a) => {
                stack.push(a);
                continue;
            }
            Expr::Binary(_, a, b) | Expr::Logic(_, a, b) => {
                stack.push(b);
                stack.push(a);
                continue;
            }
            Expr::Compare { first, rest } => {
                stack.extend(rest.iter().rev().map(|(_, e)| e));
                stack.push(first);
                continue;
            }
            Expr::Call(n, args) | Expr::Index(n, args) => {
                stack.extend(args.iter().rev());
                n
            }
            Expr::Matrix { items, .. } => {
                stack.extend(items.iter().rev());
                continue;
            }
            Expr::Range { first, second, last } => {
                stack.push(last);
                if let Some(second) = second {
                    stack.push(second);
                }
                stack.push(first);
                continue;
            }
        };
        if seen.insert(name) {
            out.push(name.clone());
        }
    }
    out
}

/// The keywords a statement uses as operators, without duplicates, in order of first use: `and`,
/// `or` and `not` for now. A document that also defines one of these words as a name (written
/// before the word became a keyword) has changed meaning, which [`crate::recalc`] reports.
pub(crate) fn keywords_used(s: &Stmt) -> Vec<&'static str> {
    let body = match s {
        Stmt::Define { body, .. } => body,
        Stmt::Eval { expr, .. } => expr,
    };
    let mut out: Vec<&'static str> = Vec::new();
    // An explicit stack keeps this safe on trees built by hand, deeper than the parser allows.
    let mut stack: Vec<&Expr> = vec![body];
    while let Some(e) = stack.pop() {
        let word = match e {
            Expr::Num(_) | Expr::Text(_) | Expr::Name(_) | Expr::Quantity(..) => None,
            Expr::Unary(_, a) | Expr::Abs(a) => {
                stack.push(a);
                None
            }
            Expr::Not(a) => {
                stack.push(a);
                Some("not")
            }
            Expr::Binary(_, a, b) => {
                stack.push(b);
                stack.push(a);
                None
            }
            Expr::Logic(logic, a, b) => {
                stack.push(b);
                stack.push(a);
                Some(if *logic == Logic::And { "and" } else { "or" })
            }
            Expr::Compare { first, rest } => {
                stack.extend(rest.iter().rev().map(|(_, e)| e));
                stack.push(first);
                None
            }
            Expr::Call(_, args) | Expr::Index(_, args) => {
                stack.extend(args.iter().rev());
                None
            }
            Expr::Matrix { items, .. } => {
                stack.extend(items.iter().rev());
                None
            }
            Expr::Range { first, second, last } => {
                stack.push(last);
                if let Some(second) = second {
                    stack.push(second);
                }
                stack.push(first);
                None
            }
        };
        if let Some(word) = word
            && !out.contains(&word)
        {
            out.push(word);
        }
    }
    out
}
