//! WordCraft calculation language.
//!
//! Inline calculations are written the way an engineer writes them on paper: `P := 1500 lbf`,
//! `σ := P/A_b = ? ksi`, `f(x, y) := x^2 + y`. This crate turns that text into a syntax tree
//! ([`parse`]), reports what a statement defines and uses ([`defined_name`], [`free_names`]), and
//! evaluates it against the names defined so far ([`run_stmt`], [`Env`]). [`recalc`] runs every
//! calculation in a document in dependency order and rewrites what each one shows ([`render`]).
//!
//! The parser knows nothing about units or variables: `1500 lbf` is the product of the number 1500
//! and the name `lbf`, and the evaluator decides what each name means. Source text is hostile
//! input, so the parser bounds source size, token count and expression depth and never panics.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod ast;
pub mod docx;
mod eval;
mod funcs;
mod parse;
mod recalc;
pub mod render;

pub use ast::{Expr, Op, Show, Stmt, defined_name, free_names};
pub use eval::{Env, check_name, eval_expr, run_stmt};
pub use parse::parse;
pub use recalc::{RecalcReport, check_new_name, defined_names, mark_stale, recalc};

/// An error from parsing or evaluating a calculation. The message is shown to the author next to
/// the calculation, so it is written for them rather than for a programmer.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CalcError {
    /// The text isn't a valid calculation.
    #[error("{0}")]
    Syntax(String),
    /// A name that is used but never defined.
    #[error("{0} isn't defined")]
    Undefined(String),
    /// A unit or dimension error.
    #[error("{0}")]
    Unit(#[from] craft_units::UnitError),
    /// The result is NaN or infinite.
    #[error("result isn't a finite number")]
    NonFinite,
    /// The calculation is larger or more deeply nested than the limits allow.
    #[error("too complex")]
    TooComplex,
    /// A function was called with the wrong number of arguments (function name, expected count).
    #[error("{0} expects {1} argument(s)")]
    Arity(String, usize),
    /// A calculation this one depends on has an error.
    #[error("depends on a calculation with an error")]
    Upstream,
    /// The calculation was edited outside WordCraft (in Word): what it shows is no longer what its
    /// source says, so nothing may use either.
    #[error("edited in Word — restore it from the calculation or keep Word's equation")]
    EditedOutside,
    /// A calculation this one depends on (directly or through others) was edited in Word.
    #[error("depends on a calculation edited in Word — resolve it")]
    UpstreamEdited,
    /// Two calculations define the same name.
    #[error("{name} is already defined {} — choose a unique name", .page.map_or("elsewhere in the document".to_string(), |p| format!("on page {p}")))]
    Duplicate {
        /// The name defined twice.
        name: String,
        /// The page of the other definition, when known.
        page: Option<u32>,
    },
    /// A name that is a unit, function or constant.
    #[error("{0} is a unit, function or constant name — choose another name")]
    ReservedName(String),
    /// Calculations that depend on each other in a cycle. Holds the names joined with `, `.
    #[error("{0} depend on each other")]
    Circular(String),
}
