//! The syntax tree of a calculation, and what a statement defines and uses.

use serde::{Deserialize, Serialize};

/// One calculation statement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Stmt {
    /// `name := body` or `name(params) := body`, optionally followed by `=` to show the value.
    Define {
        /// The defined name.
        name: String,
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
    /// A variable, unit or constant; the evaluator decides which.
    Name(String),
    /// A number with a unit. The parser never produces this (`5 kN` parses as `5 * kN`); it is for
    /// the evaluator and for building expressions directly.
    Quantity(f64, String),
    /// A prefix operator (only [`Op::Neg`]).
    Unary(Op, Box<Expr>),
    /// An infix operator.
    Binary(Op, Box<Expr>, Box<Expr>),
    /// A function call; `√x` is `Call("sqrt", [x])`.
    Call(String, Vec<Expr>),
    /// Absolute value, `|x|`.
    Abs(Box<Expr>),
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

/// The names a statement uses (variables, units, constants and called functions), minus a
/// function's own parameters, without duplicates, in order of first use.
pub fn free_names(s: &Stmt) -> Vec<String> {
    let (body, params): (&Expr, &[String]) = match s {
        Stmt::Define { params, body, .. } => (body, params),
        Stmt::Eval { expr, .. } => (expr, &[]),
    };
    let mut out: Vec<String> = Vec::new();
    // An explicit stack keeps this safe on trees built by hand, deeper than the parser allows.
    let mut stack: Vec<&Expr> = vec![body];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Num(_) | Expr::Quantity(..) => {}
            Expr::Name(n) => note(&mut out, params, n),
            Expr::Unary(_, a) | Expr::Abs(a) => stack.push(a),
            Expr::Binary(_, a, b) => {
                stack.push(b);
                stack.push(a);
            }
            Expr::Call(n, args) => {
                note(&mut out, params, n);
                stack.extend(args.iter().rev());
            }
        }
    }
    out
}

fn note(out: &mut Vec<String>, params: &[String], name: &str) {
    if !params.iter().any(|p| p == name) && !out.iter().any(|o| o == name) {
        out.push(name.to_string());
    }
}
