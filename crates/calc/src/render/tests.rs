use super::*;
use craft_units::{Precision, Quantity, UnitSystem, format};
use proptest::prelude::*;

use crate::array::MAX_ELEMENTS;

fn f(number: &str, unit: &str) -> Formatted {
    Formatted { number: number.into(), unit: unit.into() }
}

fn grid(rows: usize, cols: usize, items: Vec<Formatted>, unit: Option<&str>, total_rows: usize) -> Shown {
    Shown::Grid { rows, cols, items, unit: unit.map(str::to_string), total_rows }
}

fn shown_linear(shown: &Shown) -> String {
    rendered(&parse("x ="), "x =", Some(shown), &CalcState::Fresh).linear
}

#[test]
fn a_single_value_reads_back() {
    for one in [f("600", "psi"), f("1", ""), f("-1.5×10^-5", "m"), f("6×10^6", "J/(mol·K)")] {
        let shown = Shown::One(one);
        assert_eq!(shown_result(&shown_linear(&shown)), Some(shown));
    }
}

#[test]
fn a_cut_matrix_has_a_dots_row_under_every_column() {
    let items = (0..100).map(|k| f(&k.to_string(), "")).collect();
    let shown = grid(50, 2, items, Some("in"), 70);
    let linear = shown_linear(&shown);
    assert!(linear.starts_with("x=[■(0&1@2&3@"), "{linear}");
    assert!(linear.ends_with("@98&99@⋮&⋮)] \"in\" \"(70 rows)\""), "{linear}");
    assert_eq!(shown_result(&linear), Some(shown));
}

#[test]
fn the_cells_are_built_whole_however_many() {
    // Far more cells than the linear parser reads in one go: each is drawn all the same.
    let items: Vec<Formatted> = (0..10_000).map(|k| f(&format!("-1.235×10^-{}", k % 20 + 1), if k % 2 == 0 { "kN·m" } else { "°F" })).collect();
    let shown = grid(1, 10_000, items, None, 1);
    let math = rendered(&parse("x ="), "x =", Some(&shown), &CalcState::Fresh).math;
    let Some(MNode::Delim { elems, .. }) = math.iter().rfind(|n| matches!(n, MNode::Delim { .. })) else { panic!("{math:?}") };
    let Some(MNode::Matrix { rows, .. }) = elems.first().and_then(|e| e.first()) else { panic!("{elems:?}") };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.first().map(Vec::len), Some(10_000));
    assert_eq!(shown_result(&shown_linear(&shown)), Some(shown));
}

#[test]
fn the_result_matrix_is_the_one_the_linear_text_describes() {
    // Built cell by cell, the tree is the one Word's linear format gives for the same text.
    for (items, text) in
        [(vec![f("0.84", ""), f("-1.5×10^-5", "")], "[■(0.84@-1.5×〖10〗^(-5))]"), (vec![f("2", ""), f("6×10^6", "")], "[■(2@6×〖10〗^6)]")]
    {
        let shown = grid(2, 1, items, None, 2);
        let math = rendered(&parse("x ="), "x =", Some(&shown), &CalcState::Fresh).math;
        assert_eq!(math.last(), restore_pieces(parse_linear(text), &[]).first(), "{text}");
    }
}

#[test]
fn what_rendered_never_writes_reads_as_nothing() {
    for bad in [
        "x=[■()]",
        "x=[■(1@2&3)]",
        "x=[■(1@⋮)]",
        "x=[■(1@⋮)] \"in\"",
        "x=[■(1@2)] \"(5 rows)\"",
        "x=[■(⋮@1)] \"(5 rows)\"",
        "x=[■(1@⋮&2)] \"(5 rows)\"",
        "x=[■(1@⋮)] \"(1 rows)\"",
        "x=[■(1@⋮)] \"(99999999999999999999 rows)\"",
        "x=[■(1 \"in\"@2)] \"in\"",
        "x=[■(a@2)]",
        "x=[■(1@2)",
        "x=[■(1@2)] \"in",
        "x=[■(1 \"in@2)]",
        "x=[■(1@2)] \"in\" \"lbf\"",
    ] {
        assert_eq!(shown_result(bad), None, "{bad}");
    }
    // More rows than are ever shown, or more cells than a grid holds.
    assert_eq!(shown_result(&format!("x=[■({}1)]", "1@".repeat(50))), None);
    assert_eq!(shown_result(&format!("x=[■({}1)]", "1&".repeat(MAX_ELEMENTS))), None);
    assert_eq!(shown_result(&format!("x=[■({}1)]", "1@".repeat(1_000_000))), None);
}

fn number() -> impl Strategy<Value = String> {
    let value = prop_oneof![any::<f64>(), -1e3..1e3_f64, -1e-3..1e-3_f64, Just(0.0)];
    (value.prop_filter("finite", |v| v.is_finite()), 0u8..=15)
        .prop_map(|(v, p)| format(Quantity::dimensionless(v), None, UnitSystem::Si, Precision::new(p)).map_or_else(|_| "0".to_string(), |f| f.number))
}

const UNITS: [&str; 10] = ["", "in", "lbf", "kN·m", "J/(mol·K)", "ft^2", "°F", "Δ°C", "kN m", "1/s"];

/// A grid as [`rendered`] is given one: at most 50 rows shown of `total_rows`, and a shared unit
/// (never empty) only when the cells carry none.
fn shown_grid() -> impl Strategy<Value = Shown> {
    (1usize..=MAX_SHOWN_ROWS, 1usize..=4, any::<bool>(), any::<bool>(), any::<u16>()).prop_flat_map(|(rows, cols, shared, cut, extra)| {
        let total_rows = if cut { rows + 1 + usize::from(extra) % (MAX_ELEMENTS / cols - rows) } else { rows };
        let cells = prop::collection::vec((number(), 0..UNITS.len()), rows * cols);
        let unit = if shared { (1..UNITS.len()).prop_map(Some).boxed() } else { Just(None).boxed() };
        (cells, unit).prop_map(move |(cells, unit)| {
            let unit = unit.and_then(|k| UNITS.get(k)).map(|u| u.to_string());
            let items = cells
                .into_iter()
                .map(|(number, k)| Formatted {
                    number,
                    unit: if unit.is_some() { String::new() } else { UNITS.get(k).copied().unwrap_or("").to_string() },
                })
                .collect();
            Shown::Grid { rows, cols, items, unit, total_rows }
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn every_grid_reads_back(shown in shown_grid()) {
        let linear = shown_linear(&shown);
        prop_assert_eq!(shown_result(&linear), Some(shown.clone()));
        // And with an equation number after it.
        prop_assert_eq!(shown_result(&format!("{linear}#")), Some(shown));
    }

    #[test]
    fn hostile_results_never_panic(s in "[\\[■(@&)\\]\" 0-9.⋮×〖〗^\\-=a-z#]{0,300}") {
        let _ = shown_result(&s);
    }
}
