//! The recalculation pass over a whole document: collection order, unique names, dependency
//! order, cycles, quoted values and the displayed result.

use craft_units::Formatted;
use std::sync::Arc;
use std::time::Instant;
use wordcraft_calc::{CalcError, check_new_name, defined_names, mark_stale, recalc, render};
use wordcraft_doc::calc::{CalcRegion, CalcSettings, CalcState, calc_field_instr};
use wordcraft_doc::math::{MNode, MRun, MSty, Math};
use wordcraft_doc::{Block, Cell, CharProps, Document, InlineObject, Paragraph, Rgb, Row, Table, para_block};

const ERROR_RED: Rgb = Rgb(0xC0, 0x00, 0x00);

fn calc_eq(id: u64, source: &str) -> InlineObject {
    InlineObject::Equation {
        linear: String::new(),
        display: false,
        math: Math::default(),
        calc: Some(Box::new(CalcRegion { id, source: source.into(), unit: None, precision: None, state: CalcState::Fresh })),
    }
}

fn field(instr: &str) -> InlineObject {
    InlineObject::Field { instr: instr.into(), result: "stale".into(), locked: true, dirty: false, code: false, form: String::new() }
}

fn para(objs: Vec<InlineObject>) -> Paragraph {
    para_with(objs, &CharProps::default())
}

fn para_with(objs: Vec<InlineObject>, props: &CharProps) -> Paragraph {
    let mut p = Paragraph::with_text("Text ", CharProps::default());
    for o in objs {
        let off = p.text.len();
        p.insert_object(off, o, props).unwrap();
    }
    p
}

fn doc_of(blocks: Vec<Arc<Block>>) -> Document {
    let mut d = Document::new();
    d.body = blocks;
    d
}

/// One region per paragraph, ids 1, 2, 3…
fn doc_with(sources: &[&str]) -> Document {
    doc_of(sources.iter().enumerate().map(|(i, s)| para_block(para(vec![calc_eq(i as u64 + 1, s)]))).collect())
}

/// Every equation and field in the body, tables included, in document order.
fn objects(doc: &Document) -> Vec<InlineObject> {
    fn walk(blocks: &[Arc<Block>], out: &mut Vec<InlineObject>) {
        for b in blocks {
            match &**b {
                Block::Para(p) => out.extend(p.objects.iter().cloned()),
                Block::Table(t) => {
                    for row in &t.rows {
                        for cell in &row.cells {
                            walk(&cell.blocks, out);
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.body, &mut out);
    out
}

/// (region, linear, math) of every calculation region in the body.
fn regions(doc: &Document) -> Vec<(CalcRegion, String, Math)> {
    objects(doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Equation { linear, math, calc: Some(c), .. } => Some((*c, linear, math)),
            _ => None,
        })
        .collect()
}

fn region(doc: &Document, id: u64) -> (CalcRegion, String, Math) {
    regions(doc).into_iter().find(|(r, ..)| r.id == id).unwrap()
}

fn field_results(doc: &Document) -> Vec<String> {
    objects(doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect()
}

/// Every run in a math tree, at any depth.
fn runs(nodes: &[MNode]) -> Vec<MRun> {
    let mut out = Vec::new();
    let mut todo: Vec<&MNode> = nodes.iter().collect();
    while let Some(n) = todo.pop() {
        match n {
            MNode::Run(r) => out.push(r.clone()),
            MNode::Frac { num, den, .. } => todo.extend(num.iter().chain(den.iter())),
            MNode::Script { base, sub, sup, .. } => todo.extend(base.iter().chain(sub.iter()).chain(sup.iter())),
            MNode::Rad { deg, e, .. } => todo.extend(deg.iter().chain(e.iter())),
            MNode::Delim { elems, .. } => todo.extend(elems.iter().flatten()),
            MNode::Func { name, e } => todo.extend(name.iter().chain(e.iter())),
            _ => {}
        }
    }
    out
}

/// Change the region `id` (and its displayed text) in a document built by [`doc_with`].
fn edit_region(d: &mut Document, id: u64, f: impl Fn(&mut CalcRegion, &mut String)) {
    for b in &mut d.body {
        if let Block::Para(p) = Arc::make_mut(b) {
            for o in &mut p.objects {
                if let InlineObject::Equation { linear, calc: Some(c), .. } = o
                    && c.id == id
                {
                    f(c, linear);
                }
            }
        }
    }
}

fn no_page(_: u64) -> Option<u32> {
    None
}

fn err(e: CalcError) -> CalcState {
    CalcState::Error(e.to_string())
}

#[test]
fn recalculates_in_order() {
    let mut d = doc_with(&["P := 1500 lbf", "A_b := 2.5 in^2", "σ := P/A_b ="]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.regions, 3);
    assert_eq!(report.errors, 0);
    let (r, linear, _) = region(&d, 3);
    assert_eq!(r.state, CalcState::Fresh);
    assert!(linear.ends_with("=600 \"psi\""), "{linear}");
    assert_eq!(linear, "σ≔P/A_b=600 \"psi\"");
    let (_, p_linear, p_math) = region(&d, 1);
    assert_eq!(p_linear, "P≔1500 \"lbf\"");
    // The unit is an upright run of its own, without the quotes of the linear form.
    let p_runs = runs(&p_math.nodes);
    assert!(p_runs.iter().any(|r| r.text.trim() == "lbf" && r.sty == Some(MSty::Plain)), "{p_runs:?}");
    assert!(p_runs.iter().all(|r| !r.text.contains('"')), "{p_runs:?}");
}

#[test]
fn use_before_define_works() {
    let mut d = doc_with(&["σ := P/A_b =", "P := 1500 lbf", "A_b := 2.5 in^2"]);
    recalc(&mut d, &no_page);
    let (r, linear, _) = region(&d, 1);
    assert_eq!(r.state, CalcState::Fresh);
    assert!(linear.ends_with("=600 \"psi\""), "{linear}");
}

#[test]
fn duplicate_definition_is_error() {
    let mut d = doc_with(&["x:=1", "x =", "x:=2"]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.errors, 1);
    assert_eq!(region(&d, 1).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 3).0.state, CalcState::Error("x is already defined elsewhere in the document — choose a unique name".into()));
    let (r, linear, _) = region(&d, 2);
    assert_eq!(r.state, CalcState::Fresh);
    assert_eq!(linear, "x=1");
}

#[test]
fn duplicate_message_uses_page_lookup() {
    let mut d = doc_with(&["x:=1", "x:=2"]);
    recalc(&mut d, &|id| (id == 1).then_some(4));
    assert_eq!(region(&d, 2).0.state, CalcState::Error("x is already defined on page 4 — choose a unique name".into()));
}

#[test]
fn circular_definitions_error() {
    let mut d = doc_with(&["a := b + 1", "b := a/2", "c := a ="]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.errors, 3);
    assert_eq!(region(&d, 1).0.state, CalcState::Error("a, b depend on each other".into()));
    assert_eq!(region(&d, 2).0.state, CalcState::Error("a, b depend on each other".into()));
    assert_eq!(region(&d, 3).0.state, err(CalcError::Upstream));
}

#[test]
fn self_reference_and_mutual_functions_are_circular() {
    let mut d = doc_with(&["x := x + 1", "f_a(t) := f_b(t)", "f_b(t) := f_a(t)", "y := 2 =", "z := f_a(1) + y ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, err(CalcError::Circular("x".into())));
    assert_eq!(region(&d, 2).0.state, err(CalcError::Circular("f_a, f_b".into())));
    assert_eq!(region(&d, 3).0.state, err(CalcError::Circular("f_a, f_b".into())));
    assert_eq!(region(&d, 4).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 5).0.state, err(CalcError::Upstream));
}

#[test]
fn a_calculation_between_two_cycles_is_upstream_not_circular() {
    let mut d = doc_with(&["a := b", "b := a", "m_id := a + 1", "c := d + m_id", "d := c"]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).0.state, err(CalcError::Upstream));
    assert_eq!(region(&d, 4).0.state, err(CalcError::Circular("c, d".into())));
}

#[test]
fn pasted_unit_name_definition_is_error() {
    let mut d = doc_with(&["A := 2 in^2", "2 A ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, err(CalcError::ReservedName("A".into())));
    let (r, linear, _) = region(&d, 2);
    assert_eq!(r.state, CalcState::Fresh);
    assert!(linear.ends_with("=2 \"A\""), "{linear}");
}

#[test]
fn quoted_value_updates() {
    let mut d = doc_of(vec![
        para_block(para(vec![field("WCCALC σ")])),
        para_block(para(vec![calc_eq(1, "P := 1500 lbf"), calc_eq(2, "A_b := 2.5 in^2")])),
        para_block(para(vec![calc_eq(3, "σ := P/A_b")])),
        para_block(para(vec![field("WCCALC σ"), field(&calc_field_instr("σ", Some("ksi"), Some(2)))])),
    ]);
    recalc(&mut d, &no_page);
    assert_eq!(field_results(&d), ["600 psi", "600 psi", "0.6 ksi"]);
}

#[test]
fn quoted_value_without_a_valid_value_is_a_question_mark() {
    let mut d = doc_of(vec![
        para_block(para(vec![calc_eq(1, "x := 1/0"), calc_eq(2, "f(t) := t"), calc_eq(3, "len := 2 in")])),
        para_block(para(vec![
            field("WCCALC x"),
            field("WCCALC nope"),
            field("WCCALC m"),
            field("WCCALC f"),
            field("WCCALC len \\u lbf"),
            field("WCCALC len \\u frob"),
            field("WCCALC len"),
            field("PAGE"),
        ])),
    ]);
    recalc(&mut d, &no_page);
    assert_eq!(field_results(&d), ["?", "?", "?", "?", "?", "?", "2 in", "stale"]);
}

#[test]
fn quoted_value_clears_dirty_flag() {
    let mut d = doc_of(vec![para_block(para(vec![
        calc_eq(1, "x := 3"),
        InlineObject::Field { instr: "WCCALC x".into(), result: "3".into(), locked: true, dirty: true, code: false, form: String::new() },
    ]))]);
    recalc(&mut d, &no_page);
    assert!(matches!(&objects(&d)[1], InlineObject::Field { result, dirty: false, locked: true, .. } if result == "3"));
}

#[test]
fn check_new_name_reports() {
    let d = doc_with(&["P := 1500 lbf", "Q_x := 2"]);
    let p_id = 1;
    assert_eq!(check_new_name(&d, "P", None), Err(CalcError::Duplicate { name: "P".into(), page: None }));
    assert_eq!(check_new_name(&d, "P", Some(p_id)), Ok(()));
    assert_eq!(check_new_name(&d, "m", None), Err(CalcError::ReservedName("m".into())));
    assert_eq!(check_new_name(&d, "Q", None), Ok(()));
}

#[test]
fn defined_names_lists_owners_sorted() {
    let d = doc_with(&["b := 1", "a := 2", "b := 3", "m := 1", "a + b =", "f(x) := x"]);
    assert_eq!(defined_names(&d), [("a".to_string(), 2), ("b".to_string(), 1), ("f".to_string(), 6)]);
}

#[test]
fn table_cells_in_row_order() {
    let cell = |id: u64, src: &str| Cell { blocks: vec![para_block(para(vec![calc_eq(id, src)]))], ..Cell::empty() };
    let table = Table {
        rows: vec![
            Row { cells: vec![cell(1, "a := 10"), cell(2, "b := 1")], ..Default::default() },
            Row { cells: vec![cell(3, "b := 2"), cell(4, "c := a + b =")], ..Default::default() },
        ],
        grid: vec![100.0, 100.0],
        ..Default::default()
    };
    let mut d = doc_of(vec![Arc::new(Block::Table(table)), para_block(para(vec![calc_eq(5, "b =")]))]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.regions, 5);
    assert_eq!(region(&d, 2).0.state, CalcState::Fresh);
    assert!(matches!(region(&d, 3).0.state, CalcState::Error(m) if m.contains("already defined")));
    assert_eq!(region(&d, 4).1, "c≔a+b=11");
    assert_eq!(region(&d, 5).1, "b=1");
}

#[test]
fn duplicate_ids_are_reassigned() {
    let mut d = doc_of(vec![para_block(para(vec![calc_eq(7, "x := 1"), calc_eq(3, "y := 2")])), para_block(para(vec![calc_eq(7, "z := 3")]))]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.reassigned_ids, 1);
    let ids: Vec<u64> = regions(&d).iter().map(|(r, ..)| r.id).collect();
    assert_eq!(ids, [7, 3, 8]);
    assert_eq!(region(&d, 8).0.source, "z := 3");
    // A second pass finds nothing to fix.
    assert_eq!(recalc(&mut d, &no_page).reassigned_ids, 0);
}

#[test]
fn duplicate_ids_at_the_top_of_the_range_get_unused_ids() {
    let mut d = doc_with(&["x := 1", "y := 2", "z := 3"]);
    for id in 1..=3 {
        edit_region(&mut d, id, |c, _| c.id = u64::MAX);
    }
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.reassigned_ids, 2);
    let mut ids: Vec<u64> = regions(&d).iter().map(|(r, ..)| r.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "{ids:?}");
}

#[test]
fn tracked_deleted_region_is_ignored() {
    let mut d = Document::new();
    d.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "me", "2026-11-14T00:00:00Z"));
    let deleted = CharProps { del: Some(0), ..Default::default() };
    let mut gone = calc_eq(1, "x := 1");
    if let InlineObject::Equation { linear, .. } = &mut gone {
        *linear = "kept as is".into();
    }
    d.body = vec![para_block(para_with(vec![gone, field("WCCALC x")], &deleted)), para_block(para(vec![calc_eq(2, "x := 2"), calc_eq(3, "x =")]))];
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.regions, 2);
    assert_eq!(report.errors, 0);
    let (r, linear, _) = region(&d, 1);
    assert_eq!((r.state, linear.as_str()), (CalcState::Fresh, "kept as is"));
    assert_eq!(region(&d, 2).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 3).1, "x=2");
    assert_eq!(field_results(&d), ["stale"], "a deleted quoted value is left alone");
}

#[test]
fn plain_equations_untouched() {
    let plain = InlineObject::Equation {
        linear: "a=b/c".into(),
        display: true,
        math: Math { nodes: wordcraft_doc::math::parse_linear("a=b/c"), omml: "<m:oMath/>".into(), ..Default::default() },
        calc: None,
    };
    let mut d = doc_of(vec![para_block(para(vec![plain.clone(), calc_eq(1, "x := 1 =")])), para_block(para(vec![plain.clone()]))]);
    let before = format!("{:?}", d.body[1]);
    recalc(&mut d, &no_page);
    assert_eq!(objects(&d)[0], plain);
    assert_eq!(format!("{:?}", objects(&d)[0]), format!("{plain:?}"));
    assert_eq!(format!("{:?}", d.body[1]), before);
}

#[test]
fn unchanged_paragraphs_are_not_copied() {
    let mut d = doc_with(&["x := 1 =", "y := 2"]);
    recalc(&mut d, &no_page);
    let snapshot = d.body.clone();
    recalc(&mut d, &no_page);
    for (a, b) in snapshot.iter().zip(&d.body) {
        assert!(Arc::ptr_eq(a, b), "a recalculation that changes nothing copies nothing");
    }
}

#[test]
fn edited_outside_region_keeps_its_display_but_still_defines() {
    let mut d = doc_with(&["x := 5", "x ="]);
    edit_region(&mut d, 1, |c, linear| {
        *linear = "x≔6".into();
        c.state = CalcState::EditedOutside;
    });
    recalc(&mut d, &no_page);
    let (r, linear, _) = region(&d, 1);
    assert_eq!((r.state, linear.as_str()), (CalcState::EditedOutside, "x≔6"));
    assert_eq!(region(&d, 2).1, "x=5");
}

#[test]
fn errors_show_a_red_marker_and_no_number() {
    let mut d = doc_with(&["y := q + 1 =", "z := 1 +"]);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.errors, 2);
    let (r, linear, math) = region(&d, 1);
    assert_eq!(r.state, err(CalcError::Undefined("q".into())));
    assert_eq!(linear, "y≔q+1 ⚠");
    assert!(runs(&math.nodes).iter().any(|r| r.text.contains('⚠') && r.color == Some(ERROR_RED)));
    // A parse error shows the source as typed.
    let (r, linear, math) = region(&d, 2);
    assert!(matches!(r.state, CalcState::Error(_)));
    assert_eq!(linear, "z := 1 + ⚠");
    assert!(runs(&math.nodes).iter().any(|r| r.text.contains('⚠') && r.color == Some(ERROR_RED)));
}

#[test]
fn region_unit_and_precision_apply() {
    let mut d = doc_with(&["P := 1500 lbf", "A_b := 2.5 in^2", "σ := P/A_b =", "σ = ? ksi", "P/3 ="]);
    edit_region(&mut d, 3, |c, _| c.unit = Some("ksi".into()));
    edit_region(&mut d, 5, |c, _| c.precision = Some(1));
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).1, "σ≔P/A_b=0.6 \"ksi\"");
    assert_eq!(region(&d, 4).1, "σ=0.6 \"ksi\"");
    assert_eq!(region(&d, 5).1, "P/3=500 \"lbf\"");
}

#[test]
fn document_precision_is_clamped() {
    let mut d = doc_with(&["half := 0.5 =", "third := 2/3 ="]);
    d.calc_settings = Some(CalcSettings { precision: 40, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).1, "half≔0.5=0.5");
    assert_eq!(region(&d, 2).1, "third≔2/3=0.666666666666667");
}

#[test]
fn region_unit_of_the_wrong_dimension_is_an_error() {
    let mut d = doc_with(&["x := 2 in ="]);
    if let Block::Para(p) = Arc::make_mut(&mut d.body[0])
        && let Some(InlineObject::Equation { calc: Some(c), .. }) = p.objects.first_mut()
    {
        c.unit = Some("lbf".into());
    }
    recalc(&mut d, &no_page);
    let (r, linear, _) = region(&d, 1);
    assert!(matches!(r.state, CalcState::Error(_)));
    assert!(!linear.contains('='), "{linear}");
}

#[test]
fn si_settings_show_si_units() {
    let mut d = doc_with(&["F := 1 kN ="]);
    d.calc_settings = Some(CalcSettings { system: wordcraft_doc::calc::UnitSystem::Si, ..CalcSettings::default() });
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).1, "F≔1 \"kN\"=1000 \"N\"");
}

#[test]
fn mark_stale_greys_results_with_approx() {
    let mut d = doc_with(&["P := 1500 lbf", "A_b := 2.5 in^2", "σ := P/A_b =", "q =", "big := 6000000 ="]);
    recalc(&mut d, &no_page);
    mark_stale(&mut d);
    let (r, linear, math) = region(&d, 3);
    assert_eq!(r.state, CalcState::Stale);
    assert_eq!(linear, "σ≔P/A_b≈600 \"psi\"");
    assert!(!linear.contains('='));
    let grey: Vec<MRun> = runs(&math.nodes).into_iter().filter(|r| r.color.is_some_and(|c| c != ERROR_RED)).collect();
    assert!(grey.iter().any(|r| r.text.contains('≈')), "{grey:?}");
    assert!(grey.iter().any(|r| r.text.contains("600")), "{grey:?}");
    // A definition with no shown value goes stale but has no number to mark.
    assert_eq!(region(&d, 1).0.state, CalcState::Stale);
    // Errors stay errors.
    assert!(matches!(region(&d, 4).0.state, CalcState::Error(_)));
    // Exponential results survive the round trip through the display.
    let (_, big, _) = region(&d, 5);
    assert!(big.contains('≈') && big.contains("6×10^6") && !big.contains('='), "{big}");
    // Recalculating makes them current again.
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 3).0.state, CalcState::Fresh);
    assert_eq!(region(&d, 3).1, "σ≔P/A_b=600 \"psi\"");
}

#[test]
fn mark_stale_marks_quoted_values_once() {
    let mut d = doc_of(vec![
        para_block(para(vec![calc_eq(1, "P := 1500 lbf"), calc_eq(2, "A_b := 2.5 in^2"), calc_eq(3, "σ := P/A_b")])),
        para_block(para(vec![field("WCCALC σ"), field("WCCALC nope")])),
    ]);
    recalc(&mut d, &no_page);
    assert_eq!(field_results(&d), ["600 psi", "?"]);
    mark_stale(&mut d);
    assert_eq!(field_results(&d), ["≈ 600 psi", "?"]);
    mark_stale(&mut d);
    assert_eq!(field_results(&d), ["≈ 600 psi", "?"]);
    recalc(&mut d, &no_page);
    assert_eq!(field_results(&d), ["600 psi", "?"]);
}

#[test]
fn mark_stale_without_a_readable_result_shows_a_grey_question_mark() {
    let mut d = doc_with(&["x := 2 ="]);
    if let Block::Para(p) = Arc::make_mut(&mut d.body[0])
        && let Some(InlineObject::Equation { linear, .. }) = p.objects.first_mut()
    {
        *linear = "garbage".into();
    }
    mark_stale(&mut d);
    let (r, linear, _) = region(&d, 1);
    assert_eq!(r.state, CalcState::Stale);
    assert_eq!(linear, "x≔2≈?");
}

#[test]
fn display_renders_from_the_syntax_tree() {
    let f = |number: &str, unit: &str| Formatted { number: number.into(), unit: unit.into() };
    assert_eq!(render::display("σ := P/A_b", None, &CalcState::Fresh), "σ≔P/A_b");
    assert_eq!(render::display("σ := P/A_b =", Some(&f("600", "psi")), &CalcState::Fresh), "σ≔P/A_b=600 \"psi\"");
    assert_eq!(render::display("x =", Some(&f("1", "")), &CalcState::Fresh), "x=1");
    assert_eq!(render::display("σ := P/A_b =", Some(&f("600", "psi")), &CalcState::Error("bad".into())), "σ≔P/A_b ⚠");
    assert_eq!(render::display("σ := P/A_b =", Some(&f("600", "psi")), &CalcState::Stale), "σ≔P/A_b≈600 \"psi\"");
    assert_eq!(render::display("x := 1 +", None, &CalcState::Error("bad".into())), "x := 1 + ⚠");
    // √x² parses as (√x)²: the display says so.
    assert_eq!(render::display("y := √x²", None, &CalcState::Fresh), "y≔(√x)^2");
    assert_eq!(render::display("y := √(x+1)", None, &CalcState::Fresh), "y≔√(x+1)");
    assert_eq!(render::display("A_b := 2.5 in^2", None, &CalcState::Fresh), "A_b≔2.5 \"in\"^2");
    assert_eq!(render::display("M := 5 kN*m", None, &CalcState::Fresh), "M≔5 \"kN\"·\"m\"");
    assert_eq!(render::display("f(x, y) := x^2 + y", None, &CalcState::Fresh), "f(x,y)≔x^2+y");
    assert_eq!(render::display("f_a(t) := t^2", None, &CalcState::Fresh), "f_a(t)≔t^2");
    assert_eq!(render::display("y := f_a(2) =", Some(&f("4", "")), &CalcState::Fresh), "y≔f_a(2)=4");
    assert_eq!(render::display("z := a*b/c", None, &CalcState::Fresh), "z≔(a·b)/c");
    assert_eq!(render::display("z := a/(b*c)", None, &CalcState::Fresh), "z≔a/(b·c)");
    assert_eq!(render::display("z := a - (b - c)", None, &CalcState::Fresh), "z≔a-(b-c)");
    assert_eq!(render::display("z := (a + b)*c", None, &CalcState::Fresh), "z≔(a+b)·c");
    assert_eq!(render::display("z := -(a + b)", None, &CalcState::Fresh), "z≔-(a+b)");
    assert_eq!(render::display("z := (-a)^2", None, &CalcState::Fresh), "z≔(-a)^2");
    assert_eq!(render::display("z := -a^2", None, &CalcState::Fresh), "z≔-a^2");
    assert_eq!(render::display("z := (a^b)^c", None, &CalcState::Fresh), "z≔(a^b)^c");
    assert_eq!(render::display("z := a^b^c", None, &CalcState::Fresh), "z≔a^(b^c)");
    assert_eq!(render::display("z := x^(-1)", None, &CalcState::Fresh), "z≔x^(-1)");
    assert_eq!(render::display("z := x^0.5", None, &CalcState::Fresh), "z≔x^(0.5)");
    assert_eq!(render::display("z := |a - b|", None, &CalcState::Fresh), "z≔|a-b|");
    assert_eq!(render::display("z := sin(θ) + max(a, b)", None, &CalcState::Fresh), "z≔sin(θ)+max(a,b)");
    assert_eq!(render::display("z := 1e300", None, &CalcState::Fresh), "z≔1×10^300");
    assert_eq!(render::display("z := 1.5e-7 m", None, &CalcState::Fresh), "z≔1.5×10^(-7) \"m\"");
    assert_eq!(render::display("z := 1.5×10^-5 m =", Some(&f("1.5×10^-5", "m")), &CalcState::Fresh).rsplit('=').next(), Some("1.5×10^(-5) \"m\""));
}

#[test]
fn a_subscripted_function_keeps_its_arguments_out_of_the_subscript() {
    let mut d = doc_with(&["f_a(t) := t^2"]);
    recalc(&mut d, &no_page);
    let (_, _, math) = region(&d, 1);
    match math.nodes.first() {
        Some(MNode::Script { sub, .. }) => assert_eq!(runs(sub).iter().map(|r| r.text.as_str()).collect::<String>(), "a"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(math.nodes.get(1), Some(MNode::Delim { .. })), "{:?}", math.nodes);
}

#[test]
fn display_of_a_deep_expression_does_not_overflow() {
    let sum = vec!["1"; 1000].join("+");
    let src = format!("z := {sum}");
    let shown =
        std::thread::Builder::new().stack_size(256 * 1024).spawn(move || render::display(&src, None, &CalcState::Fresh)).unwrap().join().unwrap();
    assert!(shown.starts_with("z≔1+1+"));
}

#[test]
fn hostile_documents_return_a_report() {
    let long = format!("x := {}", "1+".repeat(60_000));
    let mut sources: Vec<String> = vec![long, "y := 10^400 =".into(), "√-1 =".into(), String::new(), "=".into(), ":=".into()];
    // A 3,000-long cycle.
    for i in 0..3000 {
        sources.push(format!("c_{i} := c_{} + 1", (i + 1) % 3000));
    }
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    let mut d = doc_with(&refs);
    let report = recalc(&mut d, &no_page);
    assert_eq!(report.regions, refs.len());
    assert_eq!(report.errors, refs.len());
    let (r, ..) = region(&d, 7);
    match r.state {
        CalcState::Error(m) => assert!(m.len() < 200 && m.ends_with("depend on each other"), "{m}"),
        other => panic!("{other:?}"),
    }
}

fn chained_doc(n: usize) -> Document {
    let mut sources = vec!["x_0 := 1 in".to_string()];
    for i in 1..n {
        sources.push(format!("x_{i} := x_{} + 1 in =", i - 1));
    }
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    doc_with(&refs)
}

#[test]
fn chained_regions_recalculate() {
    let mut d = chained_doc(50);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 50).1, "x_49≔x_48+1 \"in\"=50 \"in\"");
}

#[test]
#[ignore = "timing: run with --release -- --ignored perf_2000_regions"]
fn perf_2000_regions() {
    let mut d = chained_doc(2000);
    recalc(&mut d, &no_page);
    // An undo snapshot shares the blocks, so a recalculation pays for what it copies.
    let _snapshot = d.clone();
    if let Block::Para(p) = Arc::make_mut(&mut d.body[0])
        && let Some(InlineObject::Equation { calc: Some(c), .. }) = p.objects.first_mut()
    {
        c.source = "x_0 := 2 in".into();
    }
    let start = Instant::now();
    let report = recalc(&mut d, &no_page);
    let took = start.elapsed();
    println!("perf_2000_regions: {took:?}");
    assert_eq!(report.regions, 2000);
    assert_eq!(report.errors, 0);
    assert_eq!(region(&d, 2000).1, "x_1999≔x_1998+1 \"in\"=2001 \"in\"");
    assert!(took.as_millis() < 100, "recalc took {took:?}");
}
