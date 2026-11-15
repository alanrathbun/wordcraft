use serde_json::{Value, json};
use wordcraft_doc::calc::{CalcRegion, CalcState};
use wordcraft_doc::math::Math;
use wordcraft_doc::{Block, Document, InlineObject, Pos};

use crate::{Session, catalog};

thread_local! {
    /// Makes the automatic pass panic, to test that the panic is contained.
    static INJECT_PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Called at the start of the automatic pass in test builds.
pub(super) fn maybe_inject_panic() {
    if INJECT_PANIC.with(std::cell::Cell::get) {
        std::panic::resume_unwind(Box::new("injected failure"));
    }
}

fn session() -> Session {
    let mut s = Session::new(Document::new());
    s.run("document.setText", &json!({"text": ""})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Every equation and field in the body, tables included, in document order.
fn objects(doc: &Document) -> Vec<InlineObject> {
    fn walk(blocks: &[std::sync::Arc<Block>], out: &mut Vec<InlineObject>) {
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

/// (region, linear) of every calculation region in the body.
fn regions(s: &Session) -> Vec<(CalcRegion, String)> {
    objects(&s.doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Equation { linear, calc: Some(c), .. } => Some((*c, linear)),
            _ => None,
        })
        .collect()
}

fn region(s: &Session, id: u64) -> (CalcRegion, String) {
    regions(s).into_iter().find(|(r, _)| r.id == id).unwrap_or_else(|| panic!("no region {id}"))
}

fn field_results(s: &Session) -> Vec<String> {
    objects(&s.doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect()
}

/// Inserts a calculation on a line of its own; returns its id.
fn calc(s: &mut Session, source: &str) -> u64 {
    let r = run(s, "insert.calculation", json!({"source": source, "display": true}));
    run(s, "text.newParagraph", json!({}));
    r["id"].as_u64().unwrap_or_else(|| panic!("no id in {r}"))
}

/// `P`, `A_b` and `σ := P/A_b =`.
fn sample(s: &mut Session) -> (u64, u64, u64) {
    (calc(s, "P := 1500 lbf"), calc(s, "A_b := 2.5 in^2"), calc(s, "σ := P/A_b ="))
}

#[test]
fn insert_and_recalculate() {
    let mut s = session();
    let (p, a, sigma) = sample(&mut s);
    assert!(p < a && a < sigma, "fresh ids {p} {a} {sigma}");
    let (r, linear) = region(&s, sigma);
    assert_eq!(r.state, CalcState::Fresh);
    assert!(linear.ends_with("=600 \"psi\""), "{linear}");
}

#[test]
fn fresh_ids_never_reuse_one() {
    let mut s = session();
    let (p, a, _) = sample(&mut s);
    let ids: Vec<u64> = regions(&s).iter().map(|(r, _)| r.id).collect();
    let max = *ids.iter().max().unwrap();
    let next = calc(&mut s, "x := 1");
    assert_eq!(next, max + 1);
    assert_ne!(next, p);
    assert_ne!(next, a);
}

#[test]
fn editing_input_updates_result() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\""), "{}", region(&s, sigma).1);
    assert_eq!(region(&s, p).0.source, "P := 3000 lbf");
}

#[test]
fn set_source_on_an_unknown_id_is_refused() {
    let mut s = session();
    sample(&mut s);
    let before = s.doc.clone();
    assert!(s.run("calc.setSource", &json!({"id": 999, "source": "q := 1"})).is_err());
    assert!(s.doc.same_content(&before));
}

#[test]
fn duplicate_insert_is_refused() {
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    let before = s.doc.clone();
    let e = s.run("insert.calculation", &json!({"source": "P := 2 lbf"})).unwrap_err();
    assert!(e.to_string().contains("already defined"), "{e}");
    assert!(s.doc.same_content(&before), "the document is unchanged");
    // A source that doesn't parse is still inserted: it shows its error at the region.
    let bad = calc(&mut s, "q := (");
    assert!(matches!(region(&s, bad).0.state, CalcState::Error(_)));
    // Renaming a region to a name another owns is refused too.
    let e = s.run("calc.setSource", &json!({"id": bad, "source": "A_b := 1 in^2"})).unwrap_err();
    assert!(e.to_string().contains("already defined"), "{e}");
    // Its own name is fine.
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 1500 lbf"}));

    let r = run(&mut s, "calc.checkName", json!({"name": "P"}));
    assert_eq!(r["ok"], json!(false), "{r}");
    assert_eq!(r["reason"], json!("duplicate"), "{r}");
    assert_eq!(r["existing"], json!(p), "{r}");
    assert!(r["message"].as_str().unwrap().contains("already defined"), "{r}");
    let r = run(&mut s, "calc.checkName", json!({"name": "m"}));
    assert_eq!(r["ok"], json!(false), "{r}");
    assert_eq!(r["reason"], json!("reserved"), "{r}");
    assert!(r["message"].as_str().unwrap().contains("unit, function or constant"), "{r}");
    let r = run(&mut s, "calc.checkName", json!({"name": "a_"}));
    assert_eq!(r["reason"], json!("reserved"), "{r}");
    let r = run(&mut s, "calc.checkName", json!({"name": "P", "id": p}));
    assert_eq!(r, json!({"ok": true}));
    let r = run(&mut s, "calc.checkName", json!({"name": "brand_new"}));
    assert_eq!(r, json!({"ok": true}));
}

#[test]
fn duplicate_message_names_page() {
    let mut s = session();
    sample(&mut s);
    // A pasted copy of P's definition (the paste bypasses the name check).
    copy_object(&mut s, 0);
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.paste", json!({}));
    let (r, _) = regions(&s).pop().unwrap();
    assert_eq!(r.source, "P := 1500 lbf");
    match r.state {
        CalcState::Error(m) => assert!(m.contains("already defined on page 1"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn undo_restores_results() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\""));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(region(&s, p).0.source, "P := 1500 lbf");
    assert!(region(&s, sigma).1.ends_with("=600 \"psi\""), "{}", region(&s, sigma).1);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(region(&s, p).0.source, "P := 3000 lbf");
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\""), "{}", region(&s, sigma).1);
}

#[test]
fn manual_mode_marks_stale() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    // Switching to manual changes no result: they are still current.
    assert_eq!(region(&s, sigma).0.state, CalcState::Fresh);
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    let (r, linear) = region(&s, sigma);
    assert_eq!(r.state, CalcState::Stale);
    assert!(linear.contains('≈'), "{linear}");
    let report = run(&mut s, "calc.recalculate", json!({}));
    assert_eq!(report["regions"], json!(3), "{report}");
    assert_eq!(report["errors"], json!(0), "{report}");
    let (r, linear) = region(&s, sigma);
    assert_eq!(r.state, CalcState::Fresh);
    assert!(linear.ends_with("=1200 \"psi\""), "{linear}");
    // Undo puts the stale results back exactly; it doesn't recalculate.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Stale);
    // Back to automatic recalculates.
    run(&mut s, "calc.settings", json!({"manual": false}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Fresh);
}

#[test]
fn a_command_that_changes_nothing_stays_a_no_op_in_manual_mode() {
    let mut s = session();
    sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    run(&mut s, "caret.docStart", json!({}));
    let undo = s.undo_depth();
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(s.undo_depth(), undo, "Backspace at the start adds no undo step");
    assert!(regions(&s).iter().all(|(r, _)| r.state == CalcState::Fresh));
}

#[test]
fn settings_change_the_display() {
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    let r = run(&mut s, "calc.settings", json!({"system": "si", "precision": 2}));
    assert_eq!(r["system"], json!("si"), "{r}");
    assert_eq!(r["precision"], json!(2), "{r}");
    assert_eq!(r["manual"], json!(false), "{r}");
    let linear = region(&s, sigma).1;
    assert!(!linear.contains("psi"), "{linear}");
    assert!(s.run("calc.settings", &json!({"precision": 16})).is_err());
    assert!(s.run("calc.settings", &json!({"system": "metric"})).is_err());
}

#[test]
fn resolve_edited_keep_word() {
    let mut s = session();
    let (_, a, sigma) = sample(&mut s);
    let names = run(&mut s, "calc.names", json!({}));
    assert!(names.to_string().contains("\"A_b\""), "{names}");
    run(&mut s, "calc.resolveEdited", json!({"id": a, "keep": "word"}));
    assert!(regions(&s).iter().all(|(r, _)| r.id != a), "A_b is a plain equation now");
    let plain = objects(&s.doc).into_iter().filter(|o| matches!(o, InlineObject::Equation { calc: None, .. })).count();
    assert_eq!(plain, 1);
    let names = run(&mut s, "calc.names", json!({}));
    assert!(!names.to_string().contains("\"A_b\""), "{names}");
    // σ depends on a name that is gone now.
    assert!(matches!(region(&s, sigma).0.state, CalcState::Error(_)));
}

#[test]
fn resolve_edited_keep_source() {
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    let fresh = region(&s, p).1;
    // Something other than the calculator rewrote P's equation.
    let path = s.doc.para_paths(wordcraft_doc::StoryRef::Body)[0].clone();
    let para = s.doc.para_mut(wordcraft_doc::StoryRef::Body, &path).unwrap();
    if let Some(InlineObject::Equation { linear, calc: Some(c), .. }) = para.objects.get_mut(0) {
        *linear = "P=1".into();
        c.state = CalcState::EditedOutside;
    }
    run(&mut s, "calc.resolveEdited", json!({"id": p, "keep": "source"}));
    let (r, linear) = region(&s, p);
    assert_eq!(r.state, CalcState::Fresh);
    assert_eq!(linear, fresh);
    assert!(s.run("calc.resolveEdited", &json!({"id": p, "keep": "both"})).is_err());
}

#[test]
fn calc_value_quotes_a_variable() {
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    run(&mut s, "insert.calcValue", json!({"name": "σ", "unit": "ksi", "precision": 2}));
    assert_eq!(field_results(&s), ["0.6 ksi"]);
    let field = objects(&s.doc).into_iter().find(|o| matches!(o, InlineObject::Field { .. })).unwrap();
    assert!(matches!(field, InlineObject::Field { locked: true, ref instr, .. } if instr == "WCCALC σ \\u ksi \\p 2"), "{field:?}");
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert_eq!(field_results(&s), ["1.2 ksi"]);
    // Not a variable, a precision over 15: refused, nothing inserted.
    let e = s.run("insert.calcValue", &json!({"name": "nope"})).unwrap_err();
    assert!(e.to_string().contains("nope isn't a defined variable"), "{e}");
    assert!(s.run("insert.calcValue", &json!({"name": "P", "precision": 16})).is_err());
    assert_eq!(field_results(&s).len(), 1);
}

#[test]
fn calc_value_outside_the_body_is_refused() {
    let mut s = session();
    sample(&mut s);
    run(&mut s, "insert.editHeader", json!({}));
    let e = s.run("insert.calcValue", &json!({"name": "P"})).unwrap_err();
    assert!(e.to_string().contains("body"), "{e}");
}

#[test]
fn equation_commands_refuse_a_calculation() {
    let mut s = session();
    run(&mut s, "insert.equation", json!({"linear": "x^2", "edit": false}));
    run(&mut s, "text.newParagraph", json!({}));
    let p = calc(&mut s, "P := 1500 lbf");
    let before = region(&s, p);
    // Caret just after P's equation.
    let path = s.doc.para_paths(wordcraft_doc::StoryRef::Body)[1].clone();
    s.sel = crate::Selection::caret(Pos { story: wordcraft_doc::StoryRef::Body, path, off: wordcraft_doc::para::OBJ.len_utf8() });
    for (id, v) in [("equation.set", json!({"linear": "y"})), ("equation.edit", json!({}))] {
        let e = s.run(id, &v).unwrap_err();
        assert!(e.to_string().contains("calculation source"), "{id}: {e}");
    }
    assert_eq!(region(&s, p), before);
    // Convert All converts the plain equation and leaves the calculation alone.
    let r = run(&mut s, "equation.convert", json!({"to": "linear", "all": true}));
    assert_eq!(r["converted"], json!(1), "{r}");
    assert_eq!(region(&s, p), before);
}

#[test]
fn plain_documents_pay_nothing() {
    let mut s = session();
    run(&mut s, "text.insert", json!({"text": "hello"}));
    assert!(!super::has_calc(&s.doc));
    calc(&mut s, "x := 1");
    assert!(super::has_calc(&s.doc));
}

#[test]
fn hostile_parameters_are_refused() {
    let mut s = session();
    let huge = "x := ".to_string() + &"1+".repeat(20_000) + "1";
    assert!(s.run("insert.calculation", &json!({"source": huge})).is_err());
    assert!(s.run("insert.calculation", &json!({})).is_err());
    assert!(s.run("calc.setSource", &json!({"id": "x", "source": "y := 1"})).is_err());
    assert!(s.run("calc.checkName", &json!({})).is_err());
    assert!(s.run("calc.resolveEdited", &json!({"id": 1})).is_err());
}

#[test]
fn catalog_has_calc_commands() {
    let reg = crate::cmd::registry();
    let e = catalog::entries();
    for id in ["insert.calculation", "insert.calcValue", "calc.recalculate", "calc.settings"] {
        assert!(e.iter().any(|x| x.3 == id), "{id} catalogued");
        assert!(reg.get(id).is_some(), "{id} registered");
    }
    for id in ["calc.setSource", "calc.names", "calc.checkName", "calc.resolveEdited"] {
        assert!(reg.get(id).is_some(), "{id} registered");
    }
}

/// The region `id`'s equation in the body, to change behind the calculator's back.
fn edit_outside(s: &mut Session, id: u64, shown: &str) {
    for path in s.doc.para_paths(wordcraft_doc::StoryRef::Body) {
        let para = s.doc.para_mut(wordcraft_doc::StoryRef::Body, &path).unwrap();
        for o in para.objects.iter_mut() {
            if let InlineObject::Equation { linear, calc: Some(c), .. } = o
                && c.id == id
            {
                *linear = shown.into();
                c.state = CalcState::EditedOutside;
            }
        }
    }
}

#[test]
fn calculation_outside_the_body_is_refused() {
    let mut s = session();
    sample(&mut s);
    run(&mut s, "insert.editHeader", json!({}));
    let before = s.doc.clone();
    let e = s.run("insert.calculation", &json!({"source": "h := 1 in"})).unwrap_err();
    assert!(e.to_string().contains("body"), "{e}");
    assert!(s.doc.same_content(&before));
}

#[test]
fn a_panic_in_the_pass_is_contained() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    INJECT_PANIC.with(|c| c.set(true));
    let r = s.run("calc.setSource", &json!({"id": p, "source": "P := 3000 lbf"}));
    INJECT_PANIC.with(|c| c.set(false));
    // The edit stands, no result looks current, and the status line says why.
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(region(&s, p).0.source, "P := 3000 lbf");
    let (r, linear) = region(&s, sigma);
    assert_eq!(r.state, CalcState::Stale);
    assert!(linear.contains('≈'), "{linear}");
    assert!(s.status.contains("injected failure"), "{}", s.status);
    // The next edit recalculates.
    run(&mut s, "text.insert", json!({"text": "x"}));
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\""), "{}", region(&s, sigma).1);
}

#[test]
fn set_source_drops_a_result_typed_outside() {
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    edit_outside(&mut s, sigma, "σ=999 \"psi\"");
    run(&mut s, "calc.setSource", json!({"id": sigma, "source": "σ := P/A_b = ? ksi"}));
    let (r, linear) = region(&s, sigma);
    assert_eq!(r.state, CalcState::Stale);
    assert!(!linear.contains("999"), "{linear}");
}

#[test]
fn check_name_wants_one_plain_name() {
    let mut s = session();
    sample(&mut s);
    for name in ["f(x)", "a b", "", "  ", "x := 2"] {
        let r = run(&mut s, "calc.checkName", json!({"name": name}));
        assert_eq!(r["ok"], json!(false), "{name}: {r}");
        assert_eq!(r["reason"], json!("reserved"), "{name}: {r}");
    }
    assert_eq!(run(&mut s, "calc.checkName", json!({"name": " q "})), json!({"ok": true}));
}

#[test]
fn a_pasted_copy_is_found_by_its_recalculated_id_in_manual_mode() {
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    // A pasted copy of σ's equation, renamed, keeping σ's id (manual mode doesn't renumber it).
    let end = s.doc.end_of(wordcraft_doc::StoryRef::Body);
    let copy = InlineObject::Equation {
        linear: String::new(),
        display: true,
        math: Math::default(),
        calc: Some(Box::new(CalcRegion {
            id: sigma,
            source: "τ := 2*P/A_b =".into(),
            unit: None,
            precision: None,
            numbered: false,
            state: CalcState::Fresh,
        })),
    };
    s.doc.insert_object(&end, copy, &Default::default()).unwrap();
    run(&mut s, "text.insert", json!({"text": "x"}));
    let names = run(&mut s, "calc.names", json!({}));
    let tau = names["names"].as_array().unwrap().iter().find(|n| n["name"] == "τ").unwrap()["id"].as_u64().unwrap();
    assert_ne!(tau, sigma);
    run(&mut s, "calc.setSource", json!({"id": tau, "source": "τ := 3*P/A_b ="}));
    let sources: Vec<String> = regions(&s).into_iter().map(|(r, _)| r.source).collect();
    assert!(sources.contains(&"σ := P/A_b =".to_string()), "{sources:?}");
    assert!(sources.contains(&"τ := 3*P/A_b =".to_string()), "{sources:?}");
    // The copy's own name is its own; σ's isn't.
    assert_eq!(run(&mut s, "calc.checkName", json!({"name": "τ", "id": tau})), json!({"ok": true}));
    assert!(s.run("calc.setSource", &json!({"id": tau, "source": "σ := 1"})).is_err());
    // A new region's id is no region's id now.
    let next = calc(&mut s, "z := 1");
    assert!(next > tau, "{next} {tau}");
}

#[test]
fn a_tracked_deletion_is_no_region() {
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "select.paragraph", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert!(s.run("calc.setSource", &json!({"id": p, "source": "P := 1 lbf"})).is_err(), "P is deleted");
}

#[test]
fn wrong_typed_optional_parameters_are_refused() {
    let mut s = session();
    sample(&mut s);
    assert!(s.run("insert.calculation", &json!({"source": "q := 1", "display": "yes"})).is_err());
    assert!(s.run("insert.calcValue", &json!({"name": "P", "unit": 5})).is_err());
    assert!(s.run("insert.calcValue", &json!({"name": "P", "precision": "2"})).is_err());
    assert!(s.run("calc.checkName", &json!({"name": "P", "id": "x"})).is_err());
}

#[test]
fn only_a_wccalc_field_counts() {
    let mut s = session();
    let field = |instr: &str| InlineObject::Field {
        instr: instr.into(),
        result: String::new(),
        locked: true,
        dirty: false,
        code: false,
        form: String::new(),
    };
    let at = s.doc.end_of(wordcraft_doc::StoryRef::Body);
    s.doc.insert_object(&at, field("WCCALCX y"), &Default::default()).unwrap();
    assert!(!super::has_calc(&s.doc));
    let at = s.doc.end_of(wordcraft_doc::StoryRef::Body);
    s.doc.insert_object(&at, field(" wccalc y"), &Default::default()).unwrap();
    assert!(super::has_calc(&s.doc));
}

// ---------------------------------------------------------------------------------------------
// Opening and saving
// ---------------------------------------------------------------------------------------------

/// A session on `name` reopened from what `s` saves under that name.
fn reopen(s: &Session, name: &str) -> Session {
    let bytes = crate::io::save_bytes(name, &s.doc).unwrap();
    Session::new(crate::io::open_bytes(name, &bytes).unwrap())
}

#[test]
fn docx_save_and_open_keeps_calculations_live() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "insert.calcValue", json!({"name": "σ", "unit": "psi"}));
    let before = s.doc.clone();
    let back = reopen(&s, "report.docx");
    assert_eq!(s.doc, before, "saving leaves the live document alone");
    assert_eq!(regions(&back).len(), 3);
    for (r, linear) in regions(&back) {
        assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    }
    assert_eq!(region(&back, p).0.source, "P := 1500 lbf");
    assert_eq!(region(&back, sigma).1, region(&s, sigma).1);
    assert_eq!(field_results(&back), field_results(&s));
    // Flat XML too.
    assert_eq!(regions(&reopen(&s, "report.xml")).len(), 3);
}

#[test]
fn opening_recalculates_stale_results() {
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    run(&mut s, "insert.calculation", json!({"source": "Q := 2 lbf"}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Stale);
    // Native JSON keeps the stale state; opening makes the results current.
    let back = reopen(&s, "report.json");
    for (r, linear) in regions(&back) {
        assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    }
    // And through .docx, which shows the stale `≈` in Word.
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let xml = {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
        xml
    };
    assert!(xml.contains('≈'), "Word sees what WordCraft showed");
    let back = Session::new(crate::io::open_bytes("report.docx", &bytes).unwrap());
    assert!(regions(&back).iter().all(|(r, _)| r.state == CalcState::Fresh));
}

#[test]
fn opening_a_plain_document_changes_nothing() {
    let mut s = session();
    run(&mut s, "text.insert", json!({"text": "Plain text"}));
    run(&mut s, "insert.equation", json!({"linear": "x^2", "edit": false}));
    for name in ["plain.docx", "plain.json"] {
        let bytes = crate::io::save_bytes(name, &s.doc).unwrap();
        let mut read = if name.ends_with("docx") { wordcraft_docx::read(&bytes).unwrap() } else { serde_json::from_slice(&bytes).unwrap() };
        read.ensure_nonempty();
        let opened = crate::io::open_bytes(name, &bytes).unwrap();
        assert!(opened.same_content(&read), "{name}");
    }
    let mut doc = s.doc.clone();
    super::after_open(&mut doc);
    assert_eq!(doc, s.doc);
}

#[test]
fn a_panicking_open_pass_leaves_nothing_looking_current() {
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    let bytes = crate::io::save_bytes("report.json", &s.doc).unwrap();
    INJECT_PANIC.with(|p| p.set(true));
    let opened = crate::io::open_bytes("report.json", &bytes);
    INJECT_PANIC.with(|p| p.set(false));
    let back = Session::new(opened.unwrap());
    assert_eq!(region(&back, sigma).0.state, CalcState::Stale);
    assert!(super::take_open_notice().is_some_and(|n| n.contains("out of date")));

    // Through file.open, from a .docx: the status line says what happened.
    let docx = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let mut t = session();
    INJECT_PANIC.with(|p| p.set(true));
    let opened = t.run("file.open", &json!({"path": "report.docx", "data": crate::cmd::insert::base64_encode(&docx)}));
    INJECT_PANIC.with(|p| p.set(false));
    opened.unwrap();
    assert!(t.status.contains("opened as plain equations"), "{}", t.status);
    assert!(regions(&t).is_empty());
}

#[test]
fn a_tracked_deleted_calculation_survives_docx_and_comes_back_on_reject() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "select.paragraph", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Error("P isn't defined".into()));
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    // The control is outside the deletion, which marks the equation inside it.
    let sdt = &xml[xml.find("<w:sdt>").unwrap()..xml.find("</w:sdt>").unwrap()];
    assert!(sdt.contains("wordcraft-calc:1") && sdt.contains("<w:del ") && sdt.contains("P≔1500"), "{sdt}");
    let mut back = reopen(&s, "report.docx");
    assert_eq!(region(&back, p).0.source, "P := 1500 lbf", "the deleted calculation keeps its source");
    assert!(matches!(region(&back, sigma).0.state, CalcState::Error(_)), "still deleted, so still undefined");
    run(&mut back, "review.rejectAll", json!({}));
    assert_eq!(region(&back, p).0.state, CalcState::Fresh);
    let (sig, linear) = region(&back, sigma);
    assert_eq!(sig.state, CalcState::Fresh, "{linear}");
    assert_eq!(
        linear,
        region(
            &{
                let mut fresh = session();
                sample(&mut fresh);
                fresh
            },
            sigma
        )
        .1
    );
}

/// `bytes` (a `.docx`) with `word/document.xml` rewritten by `f`: an edit made in Word.
fn edit_in_word(bytes: &[u8], f: impl Fn(String) -> String) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut part = z.by_index(i).unwrap();
        let name = part.name().to_string();
        let mut data = Vec::new();
        part.read_to_end(&mut data).unwrap();
        if name == "word/document.xml" {
            data = f(String::from_utf8(data).unwrap()).into_bytes();
        }
        w.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(&data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

#[test]
fn a_calculation_edited_in_word_stops_its_dependents_and_says_so() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "insert.calcValue", json!({"name": "σ"}));
    assert_eq!(field_results(&s), ["600 psi"]);
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let edited = edit_in_word(&bytes, |xml| {
        assert!(xml.contains(">P≔1500<"), "the saved equation shows P≔1500");
        xml.replace(">P≔1500<", ">P≔2000<")
    });
    let mut back = Session::new(Document::new());
    run(&mut back, "file.open", json!({"path": "report.docx", "data": crate::cmd::insert::base64_encode(&edited)}));
    // P keeps Word's equation, marked; nothing shows a number from either version of P.
    let (r, linear) = region(&back, p);
    assert_eq!(r.state, CalcState::EditedOutside);
    assert!(linear.contains("2000") && linear.ends_with(" ⚠"), "{linear}");
    let (r, linear) = region(&back, sigma);
    assert_eq!(r.state, CalcState::Error("depends on a calculation edited in Word — resolve it".into()));
    assert!(!linear.contains("psi") && !linear.contains("600") && !linear.contains("800"), "{linear}");
    assert_eq!(field_results(&back), ["?"]);
    assert!(back.status.contains("1 calculation was edited in Word"), "{}", back.status);
    // Resolving it from the calculation brings everything back.
    run(&mut back, "calc.resolveEdited", json!({"id": p, "keep": "source"}));
    assert_eq!(region(&back, sigma).0.state, CalcState::Fresh);
    assert_eq!(field_results(&back), ["600 psi"]);
}

#[test]
fn keeping_words_equation_drops_the_mark() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    edit_outside(&mut s, p, "P≔2000 \"lbf\"");
    run(&mut s, "calc.recalculate", json!({}));
    assert!(region(&s, p).1.ends_with(" ⚠"));
    run(&mut s, "calc.resolveEdited", json!({"id": p, "keep": "word"}));
    let plain: Vec<String> = objects(&s.doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Equation { linear, calc: None, math, .. } => {
                assert!(!wordcraft_calc::render::has_edited_mark(&math));
                Some(linear)
            }
            _ => None,
        })
        .collect();
    assert_eq!(plain, ["P≔2000 \"lbf\""]);
    assert_eq!(region(&s, sigma).0.state, CalcState::Error("P isn't defined".into()));
}

/// Every equation and field in the stories other than the body.
fn objects_outside_body(doc: &Document) -> Vec<InlineObject> {
    let mut out = Vec::new();
    for k in doc.parts.keys() {
        let story = wordcraft_doc::StoryRef::Part(*k);
        for path in doc.para_paths(story) {
            if let Some(p) = doc.para(story, &path) {
                out.extend(p.objects.iter().cloned());
            }
        }
    }
    out
}

/// Copies the one object of body paragraph `block` (a calculation or a quoted value).
fn copy_object(s: &mut Session, block: usize) {
    run(s, "select.range", json!({"anchor": {"block": block, "off": 0}, "focus": {"block": block, "off": 3}}));
    run(s, "edit.copy", json!({}));
}

/// Nothing outside the body shows a number: no calculation, and every quoted value `?`.
fn assert_nothing_live_outside_body(s: &Session, what: &str) {
    let outside = objects_outside_body(&s.doc);
    let mut seen = 0;
    for o in &outside {
        match o {
            InlineObject::Equation { calc, linear, .. } => {
                assert!(calc.is_none(), "{what}: a calculation outside the body: {linear}");
                if linear.contains("not calculated here") {
                    assert!(!linear.contains("psi") && !linear.contains('='), "{what}: {linear}");
                    seen += 1;
                }
            }
            InlineObject::Field { result, instr, .. } if instr.starts_with("WCCALC") => {
                assert_eq!(result, "?", "{what}");
                seen += 1;
            }
            _ => {}
        }
    }
    assert!(seen >= 2, "{what}: the pasted calculation and value are there: {outside:?}");
}

#[test]
fn calculations_pasted_outside_the_body_never_look_live() {
    for target in ["textBox", "footnote"] {
        let mut s = session();
        let p = calc(&mut s, "P := 1500 lbf");
        let sigma = calc(&mut s, "σ := P/(2 in^2) =");
        run(&mut s, "insert.calcValue", json!({"name": "σ"}));
        assert_eq!(field_results(&s), ["750 psi"]);
        // The σ region, then the quoted value, into a text box or a footnote.
        copy_object(&mut s, 1);
        run(&mut s, "caret.docEnd", json!({}));
        match target {
            "textBox" => {
                run(&mut s, "insert.textBox", json!({}));
            }
            _ => {
                run(&mut s, "references.footnote", json!({}));
            }
        }
        run(&mut s, "edit.paste", json!({}));
        assert!(s.sel.focus.story != wordcraft_doc::StoryRef::Body, "{target}: pasting outside the body");
        let story = s.sel.focus.story;
        // Back to the body to copy the quoted value, then into the same story again.
        let field_block = s.doc.body.len() - 1;
        let at_story = s.sel.focus.clone();
        copy_object(&mut s, field_block);
        run(&mut s, "caret.set", json!({"pos": at_story}));
        assert_eq!(s.sel.focus.story, story);
        run(&mut s, "edit.paste", json!({}));
        assert_nothing_live_outside_body(&s, target);
        // The input changes: the body follows, the copies outside still show no number.
        run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
        assert!(region(&s, sigma).1.ends_with("=1500 \"psi\""), "{}", region(&s, sigma).1);
        assert_eq!(field_results(&s), ["1500 psi"]);
        assert_nothing_live_outside_body(&s, target);
    }
}

#[test]
fn calculations_outside_the_body_are_frozen_on_open() {
    let mut s = session();
    sample(&mut s);
    run(&mut s, "insert.textBox", json!({}));
    let part = match s.sel.focus.story {
        wordcraft_doc::StoryRef::Part(k) => k,
        other => panic!("not in a text box: {other:?}"),
    };
    // A hostile native file: a live-looking calculation and quoted value in a text box.
    let path = s.doc.para_paths(wordcraft_doc::StoryRef::Part(part))[0].clone();
    let para = s.doc.para_mut(wordcraft_doc::StoryRef::Part(part), &path).unwrap();
    let eq = InlineObject::Equation {
        linear: "y≔2=2".into(),
        display: false,
        math: Math::default(),
        calc: Some(Box::new(CalcRegion { id: 99, source: "y := 2 =".into(), unit: None, precision: None, numbered: false, state: CalcState::Fresh })),
    };
    let field =
        InlineObject::Field { instr: "WCCALC σ".into(), result: "600 psi".into(), locked: true, dirty: false, code: false, form: String::new() };
    para.insert_object(0, eq, &Default::default()).unwrap();
    para.insert_object(0, field, &Default::default()).unwrap();
    let back = reopen(&s, "report.json");
    assert_nothing_live_outside_body(&back, "opened");
}

#[test]
fn editing_a_pasted_copy_in_manual_mode_leaves_the_original() {
    let mut s = session();
    run(&mut s, "calc.settings", json!({"manual": true}));
    let a = calc(&mut s, "a := 1 =");
    copy_object(&mut s, 0);
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.paste", json!({}));
    // Every stored id is unique once the paste is done, in manual mode too.
    let ids: Vec<u64> = regions(&s).into_iter().map(|(r, _)| r.id).collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1], "the copy has its own id");
    assert_eq!(ids[0], a);
    // The interface edits the copy by the id stored on it.
    run(&mut s, "calc.setSource", json!({"id": ids[1], "source": "b := 2 ="}));
    let sources: Vec<String> = regions(&s).into_iter().map(|(r, _)| r.source).collect();
    assert_eq!(sources, ["a := 1 =", "b := 2 ="]);
}

#[test]
fn typing_text_in_manual_mode_leaves_results_current() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    run(&mut s, "text.insert", json!({"text": "The stress follows."}));
    run(&mut s, "text.newParagraph", json!({}));
    assert!(regions(&s).iter().all(|(r, _)| r.state == CalcState::Fresh), "typing text changes no input");
    // Editing a source still marks what follows from it out of date.
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Stale);
}

#[test]
fn a_quoted_value_unit_is_checked() {
    let mut s = session();
    sample(&mut s);
    calc(&mut s, "T_1 := 70 °F");
    calc(&mut s, "T_2 := 100 °F");
    calc(&mut s, "dT := T_2 - T_1");
    for (name, unit, why) in [
        ("σ", "furlongs", "furlongs"),
        ("σ", "in", "σ can't be shown in in"),
        ("dT", "°C", "°C shows an absolute temperature; use Δ°C for a difference"),
    ] {
        let e = s.run("insert.calcValue", &json!({"name": name, "unit": unit})).unwrap_err();
        assert!(e.to_string().contains(why), "{name} in {unit}: {e}");
    }
    assert!(field_results(&s).is_empty(), "nothing inserted");
    run(&mut s, "insert.calcValue", json!({"name": "σ", "unit": "ksi"}));
    run(&mut s, "insert.calcValue", json!({"name": "T_1", "unit": "°C"}));
    run(&mut s, "insert.calcValue", json!({"name": "dT", "unit": "Δ°C"}));
    assert_eq!(field_results(&s), ["0.6 ksi", "21.111 °C", "16.667 Δ°C"]);
}

/// The caret just after the region `id`'s equation.
fn caret_after(s: &mut Session, id: u64) {
    for path in s.doc.para_paths(wordcraft_doc::StoryRef::Body) {
        let para = s.doc.para(wordcraft_doc::StoryRef::Body, &path).unwrap();
        if let Some((off, _)) = para.objects_with_offsets().find(|(_, o)| matches!(o, InlineObject::Equation { calc: Some(c), .. } if c.id == id)) {
            s.sel = crate::Selection::caret(Pos { story: wordcraft_doc::StoryRef::Body, path, off: off + wordcraft_doc::para::OBJ.len_utf8() });
            return;
        }
    }
    panic!("no region {id}");
}

/// Whether the region `id`'s equation is numbered the way a plain one is: one row ending in `#`.
fn numbered_eq(doc: &Document, id: u64) -> bool {
    let mut found = None;
    for o in objects(doc) {
        if let InlineObject::Equation { math, display, calc: Some(c), .. } = o
            && c.id == id
        {
            found = Some(
                display
                    && matches!(math.nodes.as_slice(), [wordcraft_doc::math::MNode::EqArr { rows }] if rows.len() == 1 && wordcraft_doc::math::has_number_mark(&rows[0])),
            );
        }
    }
    found.unwrap_or_else(|| panic!("no region {id}"))
}

#[test]
fn a_calculation_can_be_numbered() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    caret_after(&mut s, sigma);
    run(&mut s, "equation.number", json!({}));
    assert!(region(&s, sigma).0.numbered);
    assert!(numbered_eq(&s.doc, sigma), "drawn with an automatic number");
    let (_, linear) = region(&s, sigma);
    assert!(linear.ends_with("=600 \"psi\"#"), "{linear}");
    // The number stays through a recalculation, and in manual mode through going stale.
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert!(numbered_eq(&s.doc, sigma));
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\"#"), "{}", region(&s, sigma).1);
    run(&mut s, "calc.settings", json!({"manual": true}));
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 1500 lbf"}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Stale);
    assert!(numbered_eq(&s.doc, sigma));
    assert!(region(&s, sigma).1.contains("≈1200"), "the stale result is kept: {}", region(&s, sigma).1);
    run(&mut s, "calc.recalculate", json!({}));
    // Through .docx: Word gets a numbered equation, and it comes back live and numbered.
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let mut xml = String::new();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains("<m:eqArr>") && xml.contains("#"), "Word sees a numbered equation");
    let back = Session::new(crate::io::open_bytes("report.docx", &bytes).unwrap());
    let (r, linear) = region(&back, sigma);
    assert_eq!(r.state, CalcState::Fresh, "{linear}");
    assert!(r.numbered && numbered_eq(&back.doc, sigma), "{linear}");
    assert_eq!(linear, region(&s, sigma).1);
    assert!(regions(&back).iter().all(|(r, _)| r.state == CalcState::Fresh));
    // And the number can be taken away again.
    caret_after(&mut s, sigma);
    run(&mut s, "equation.number", json!({"value": false}));
    assert!(!region(&s, sigma).0.numbered && !numbered_eq(&s.doc, sigma));
    assert!(!region(&s, sigma).1.ends_with('#'));
}

/// A pasted copy of P's definition at the end of the body.
fn paste_copy_of_p(s: &mut Session) -> CalcRegion {
    copy_object(s, 0);
    run(s, "caret.docEnd", json!({}));
    run(s, "edit.paste", json!({}));
    regions(s).pop().unwrap().0
}

#[test]
fn duplicate_pages_are_looked_up_only_in_print_layout_and_once_per_revision() {
    let mut s = session();
    sample(&mut s);
    s.view.mode = wordcraft_layout::ViewMode::Draft;
    let copy = paste_copy_of_p(&mut s);
    // Draft view has no pages: no layout, and the message says so plainly.
    assert_eq!(copy.state, CalcState::Error("P is already defined elsewhere in the document — choose a unique name".into()));
    assert!(s.calc_pages.is_none(), "no page lookup outside Print Layout");
    s.view.mode = wordcraft_layout::ViewMode::Print;
    run(&mut s, "calc.recalculate", json!({}));
    assert!(matches!(regions(&s).pop().unwrap().0.state, CalcState::Error(m) if m.contains("on page 1")));
    // The same document at the same revision reuses the pages found.
    let first = super::region_pages(&mut s);
    let again = super::region_pages(&mut s);
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    // Any edit looks again.
    run(&mut s, "text.insert", json!({"text": "x"}));
    let after = super::region_pages(&mut s);
    assert!(!std::sync::Arc::ptr_eq(&first, &after));
}

#[test]
fn calculation_commands_have_keys() {
    let reg = crate::cmd::registry();
    assert_eq!(reg.by_shortcut("Mod+Alt+=").map(|c| c.id), Some("insert.calculation"));
    assert_eq!(reg.by_shortcut("F2").map(|c| c.id), Some("calc.edit"));
    // calc.edit names the calculation at the caret (on it, just after it, or selected) and is
    // greyed out anywhere else.
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    run(&mut s, "caret.docEnd", json!({}));
    assert!(s.why_disabled("calc.edit").is_some());
    caret_after(&mut s, p);
    assert_eq!(run(&mut s, "calc.edit", json!({})), json!({"id": p, "source": "P := 1500 lbf"}));
    copy_object(&mut s, 0);
    assert_eq!(run(&mut s, "calc.edit", json!({}))["id"], json!(p));
}

#[test]
fn a_numbered_calculation_is_cross_referenced_like_a_plain_equation() {
    // Equation cross-references go to Equation captions, for a plain equation and a calculation
    // alike; the `#` number marks neither as a target on its own.
    let targets = |s: &mut Session| crate::cmd::citations::ref_targets(s, "equation").into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    let mut plain = session();
    run(&mut plain, "insert.equation", json!({"linear": "x=1", "edit": false}));
    run(&mut plain, "equation.number", json!({}));
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    caret_after(&mut s, sigma);
    run(&mut s, "equation.number", json!({}));
    assert_eq!(targets(&mut s), targets(&mut plain));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "references.caption", json!({"label": "Equation", "text": "Bolt stress"}));
    let listed = targets(&mut s);
    assert!(listed.iter().any(|t| t.contains("Bolt stress")), "{listed:?}");
    assert!(region(&s, sigma).0.numbered, "the caption leaves the calculation as it was");
}

/// The display flag and justification of the region `id`'s equation.
fn layout_of(doc: &Document, id: u64) -> (bool, wordcraft_doc::math::MathJc) {
    let mut found = None;
    for o in objects(doc) {
        if let InlineObject::Equation { display, math, calc: Some(c), .. } = o
            && c.id == id
        {
            found = Some((display, math.jc));
        }
    }
    found.unwrap_or_else(|| panic!("no region {id}"))
}

#[test]
fn a_calculation_can_be_aligned_and_made_inline() {
    use wordcraft_doc::math::MathJc;
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    caret_after(&mut s, sigma);
    run(&mut s, "equation.justify", json!({"jc": "left"}));
    assert_eq!(layout_of(&s.doc, sigma), (true, MathJc::Left));
    // Alignment survives a recalculation and the .docx round trip.
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert_eq!(layout_of(&s.doc, sigma), (true, MathJc::Left));
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\""), "{}", region(&s, sigma).1);
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let back = Session::new(crate::io::open_bytes("report.docx", &bytes).unwrap());
    assert_eq!(layout_of(&back.doc, sigma), (true, MathJc::Left));
    assert_eq!(region(&back, sigma).0.state, CalcState::Fresh);
    caret_after(&mut s, sigma);
    run(&mut s, "equation.justify", json!({"jc": "right"}));
    assert_eq!(layout_of(&s.doc, sigma).1, MathJc::Right);
    // Inline, so text can follow it on the same line; a number needs display, so it goes.
    run(&mut s, "equation.number", json!({"value": true}));
    caret_after(&mut s, sigma);
    run(&mut s, "equation.display", json!({"value": false}));
    assert!(!layout_of(&s.doc, sigma).0, "inline now");
    assert!(!region(&s, sigma).0.numbered, "an inline calculation isn't numbered");
    caret_after(&mut s, sigma);
    run(&mut s, "text.insert", json!({"text": "\tbearing stress, see ref. 3"}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Fresh, "text after it leaves it live");
    caret_after(&mut s, sigma);
    run(&mut s, "equation.display", json!({"value": true}));
    assert!(layout_of(&s.doc, sigma).0, "display again");
    // Undo restores the inline state.
    run(&mut s, "edit.undo", json!({}));
    assert!(!layout_of(&s.doc, sigma).0);
}
