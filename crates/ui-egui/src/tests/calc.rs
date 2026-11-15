//! Inline calculations in the interface: the source box (Insert › Calculation and a double-click
//! on a calculation), the unique-name prompt, Insert › Calculated Value, Calculation Options and
//! the bar of a calculation edited outside WordCraft.

use egui::{Event, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use wordcraft_doc::calc::{CalcRegion, CalcState, UnitSystem};
use wordcraft_doc::{InlineObject, Pos, StoryRef};

use super::*;
use crate::calc_ui::{self, CalcDialog};

/// Every calculation region in the body, in document order, with where it sits.
fn regions(a: &WordApp) -> Vec<(Pos, CalcRegion)> {
    let doc = &a.session.doc;
    let mut out = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(para) = doc.para(StoryRef::Body, &path) else { continue };
        for (off, o) in para.objects_with_offsets() {
            if let InlineObject::Equation { calc: Some(c), .. } = o {
                out.push((Pos { story: StoryRef::Body, path: path.clone(), off }, (**c).clone()));
            }
        }
    }
    out
}

fn sources(a: &WordApp) -> Vec<String> {
    regions(a).into_iter().map(|(_, c)| c.source).collect()
}

/// The open source box, if the dialog is one.
fn calc_box(a: &WordApp) -> Option<calc_ui::CalcBox> {
    match a.dialog.as_ref() {
        Some(dialogs::Dialog::Calc(d)) => match &**d {
            CalcDialog::Source(b) => Some(b.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn ready(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    let mut h = harness(a);
    h.run_steps(6);
    h
}

/// Insert › Calculation opens the box (from the ribbon button's command without a source).
fn open_box(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.state_mut().run("insert.calculation", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(dialogs::Dialog::name), Some("calculation"));
}

fn type_text(h: &mut egui_kittest::Harness<'static, WordApp>, text: &str) {
    h.event(Event::Text(text.into()));
    h.run_steps(2);
}

fn enter(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

/// Typing a calculation and Enter inserts it with that source; the text goes into the box, not
/// the page, and `:=` shows as `≔` while it is typed.
#[test]
fn calc_box_commits_source() {
    let mut h = ready(app());
    open_box(&mut h);
    // `:` then `=`: the box borrows the math font for `≔` and shows it.
    type_text(&mut h, "P :");
    type_text(&mut h, "= 1500 lbf");
    h.run_steps(3);
    let font = egui::TextStyle::Body.resolve(&h.ctx.global_style());
    assert!(h.ctx.fonts_mut(|f| f.has_glyph(&font, '\u{2254}')), "the interface can draw the definition mark while the box is open");
    let b = calc_box(h.state()).expect("the box is open");
    assert_eq!(b.source, "P \u{2254} 1500 lbf", "`:=` shows as the definition mark");
    enter(&mut h);
    assert!(h.state().dialog.is_none(), "Enter commits and closes the box");
    assert_eq!(sources(h.state()), ["P := 1500 lbf"]);
    assert_eq!(regions(h.state())[0].1.state, CalcState::Fresh);
    let texts: Vec<String> = h
        .state()
        .session
        .doc
        .para_paths(StoryRef::Body)
        .iter()
        .filter_map(|p| h.state().session.doc.para(StoryRef::Body, p))
        .map(|p| p.text.clone())
        .collect();
    assert_eq!(texts, ["\u{FFFC}"], "nothing typed reached the page: one paragraph holding the calculation");
    // Esc cancels: nothing more is inserted.
    open_box(&mut h);
    type_text(&mut h, "Q := 2 ft");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_eq!(sources(h.state()), ["P := 1500 lbf"]);
}

/// Double-clicking a calculation opens the box with its source; Enter runs `calc.setSource`.
#[test]
fn double_click_opens_source() {
    let mut a = app();
    a.run("text.insert", json!({"text": "Load "})).unwrap();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf", "display": false})).unwrap();
    a.run("text.insert", json!({"text": " here"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = timed(a);
    let (at, region) = regions(h.state()).remove(0);
    let p = on_screen(&mut h, &at);
    double_click(&mut h, p);
    let b = calc_box(h.state()).expect("the double-click opens the source box");
    assert_eq!(b.editing, Some(region.id));
    assert_eq!(calc_ui::stored_source(&b.source), "P := 1500 lbf");
    assert!(h.state().session.math.is_none(), "not the equation editor");
    // Replace the source and commit.
    select_all_in_box(&mut h);
    type_text(&mut h, "P := 2000 lbf");
    enter(&mut h);
    assert!(h.state().dialog.is_none());
    assert_eq!(sources(h.state()), ["P := 2000 lbf"]);
    assert_eq!(regions(h.state())[0].1.id, region.id, "the same region");
}

/// A harness whose frames advance the clock (egui needs it to see a double-click or a hover).
fn timed(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(6);
    h
}

/// The middle of the calculation at `at`, on screen.
fn on_screen(h: &mut egui_kittest::Harness<'static, WordApp>, at: &Pos) -> Pos2 {
    let eq = h.state_mut().session.layout().equation_box(at, 0).expect("the calculation is laid out");
    crate::canvas::page_to_screen(h.state_mut(), eq.page, eq.x + eq.math.width / 2.0, eq.base - 2.0).expect("on screen")
}

fn set_state(a: &mut WordApp, index: usize, state: CalcState) {
    let (at, _) = regions(a).remove(index);
    if let Ok(p) = a.session.doc.para_mut(at.story, &at.path)
        && let Some(InlineObject::Equation { calc: Some(c), .. }) = p.object_at_mut(at.off)
    {
        c.state = state;
    }
}

fn double_click(h: &mut egui_kittest::Harness<'static, WordApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
}

fn select_all_in_box(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.event(Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
    h.run_steps(1);
}

/// A name already defined: the prompt says so and offers `P_1`; Rename inserts the calculation
/// under that name and leaves the first one alone. Nothing is inserted under the clashing name.
#[test]
fn duplicate_name_prompts_for_rename() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf"})).unwrap();
    a.run("text.newParagraph", json!({})).unwrap();
    let mut h = ready(a);
    open_box(&mut h);
    type_text(&mut h, "P := 2 lbf");
    enter(&mut h);
    let b = calc_box(h.state()).expect("the box stays open");
    let prompt = b.prompt.expect("the unique-name prompt shows");
    assert_eq!(prompt.new_name, "P_1");
    assert!(prompt.message.starts_with("P is already defined on page 1"), "{}", prompt.message);
    assert_eq!(sources(h.state()), ["P := 1500 lbf"], "nothing inserted under the clashing name");
    assert!(h.query_by_label("New name").is_some(), "the New name field is drawn");
    assert!(h.query_by_label("Go to existing").is_some(), "a duplicate can be gone to");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Rename").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none(), "Rename commits");
    assert_eq!(sources(h.state()), ["P := 1500 lbf", "P_1 := 2 lbf"]);
}

/// A unit's name: the prompt gives the reserved-name message (and no Go to existing).
#[test]
fn unit_name_prompts_for_rename() {
    let mut h = ready(app());
    open_box(&mut h);
    type_text(&mut h, "A := 2 in^2");
    enter(&mut h);
    let prompt = calc_box(h.state()).and_then(|b| b.prompt).expect("the prompt shows");
    assert_eq!(prompt.message, "A is a unit, function or constant name — choose another name");
    assert_eq!(prompt.new_name, "A_1");
    assert!(sources(h.state()).is_empty(), "nothing inserted");
    assert!(h.query_by_label("Go to existing").is_none(), "only a duplicate has one to go to");
    // Cancel goes back to the box with the source as typed.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").click();
    h.run_steps(3);
    let b = calc_box(h.state()).expect("back in the box");
    assert!(b.prompt.is_none());
    assert_eq!(calc_ui::stored_source(&b.source), "A := 2 in^2");
}

/// Calculation Options: unit system, precision and manual recalculation, applied by OK as one
/// `calc.settings`.
#[test]
fn options_dialog_sets_settings() {
    let mut a = app();
    a.run("calc.settings", json!({})).unwrap();
    assert_eq!(a.dialog.as_ref().map(dialogs::Dialog::name), Some("calcOptions"));
    assert!(a.session.doc.calc_settings.is_none(), "opening the dialog changes nothing");
    let mut h = ready(a);
    h.get_by_label("SI (metric)").click();
    h.run_steps(2);
    h.get_by_label("Recalculate manually").click();
    h.run_steps(2);
    if let Some(dialogs::Dialog::Calc(d)) = h.state_mut().dialog.as_mut()
        && let CalcDialog::Options(f) = &mut **d
    {
        f.precision = 5;
    }
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    let s = h.state().session.doc.calc_settings.clone().expect("settings set");
    assert_eq!((s.system, s.precision, s.manual), (UnitSystem::Si, 5, true));
}

/// Insert › Calculated Value lists the defined names and inserts the chosen one's value.
#[test]
fn calc_value_picker_inserts_a_value() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf"})).unwrap();
    a.run("text.newParagraph", json!({})).unwrap();
    a.run("insert.calcValue", json!({})).unwrap();
    let Some(dialogs::Dialog::Calc(d)) = a.dialog.clone() else { panic!("no dialog: {:?}", a.dialog.as_ref().map(dialogs::Dialog::name)) };
    let CalcDialog::Value(mut f) = *d else { panic!("not the picker") };
    assert_eq!(f.names, ["P"]);
    assert_eq!(f.name, "P");
    f.unit = "kip".into();
    f.precision_on = true;
    f.precision = 2;
    assert_eq!(f.params(), json!({"name": "P", "unit": "kip", "precision": 2}));
    a.dialog = None;
    a.run("insert.calcValue", f.params()).unwrap();
    let fields: Vec<String> = a
        .session
        .doc
        .body
        .iter()
        .filter_map(|b| match &**b {
            wordcraft_doc::Block::Para(p) => Some(p),
            _ => None,
        })
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Field { instr, result, .. } => Some(format!("{instr} → {result}")),
            _ => None,
        })
        .collect();
    assert_eq!(fields.len(), 1, "{fields:?}");
    assert!(fields[0].contains("1.5"), "{fields:?}");
}

/// The rename suggestion: `<name>_1`, `<name>_2`, … the first free one.
#[test]
fn calc_rename_suggestion_takes_the_first_free_name() {
    let mut a = app();
    for s in ["P := 1 lbf", "P_1 := 2 lbf", "P_2 := 3 lbf"] {
        a.run("insert.calculation", json!({"source": s})).unwrap();
        a.run("text.newParagraph", json!({})).unwrap();
    }
    assert_eq!(calc_ui::suggest_name(&mut a.session, "P", None).as_deref(), Some("P_3"));
    assert_eq!(calc_ui::suggest_name(&mut a.session, "A_b", None).as_deref(), Some("A_b_1"));
    assert_eq!(calc_ui::rename_source("P := 2 lbf", "P", "P_3").as_deref(), Some("P_3 := 2 lbf"));
    assert_eq!(calc_ui::rename_source("  f(x) \u{2254} x^2", "f", "g").as_deref(), Some("  g(x) \u{2254} x^2"));
    assert_eq!(calc_ui::rename_source("Px := 1", "P", "Q"), None, "only the whole name");
    assert_eq!(calc_ui::defined_name_of("σ := P/A_b =").as_deref(), Some("σ"));
    assert_eq!(calc_ui::defined_name_of("P/2 ="), None, "a bare evaluation defines nothing");
}

/// `:=` shows as `≔` when the font has it, with the cursor kept in place; never without.
#[test]
fn calc_define_mark_shows_when_the_font_has_it() {
    let mark = "P \u{2254} 1";
    assert_eq!(calc_ui::show_define_mark("P := 1", 4, true), (mark.to_string(), 3), "after `:=`, after the mark");
    assert_eq!(calc_ui::show_define_mark("P := 1", 6, true), (mark.to_string(), 5));
    assert_eq!(calc_ui::show_define_mark("P := 1", 3, true), (mark.to_string(), 3), "between `:` and `=`: after the mark");
    assert_eq!(calc_ui::show_define_mark("P := 1", 1, true), (mark.to_string(), 1));
    assert_eq!(calc_ui::show_define_mark(mark, 3, false), ("P := 1".to_string(), 4), "no glyph: back to ASCII");
    assert_eq!(calc_ui::stored_source(mark), "P := 1", "stored as ASCII");
}

/// Hovering a region: an error shows its message, a stale result says how to update it.
#[test]
fn calc_hover_text_follows_the_state() {
    assert_eq!(calc_ui::hover_text(&CalcState::Error("units don't match".into())).as_deref(), Some("units don't match"));
    assert_eq!(calc_ui::hover_text(&CalcState::Stale).as_deref(), Some("Out of date — recalculate (References › Recalculate)"));
    assert_eq!(calc_ui::hover_text(&CalcState::Fresh), None);
    assert_eq!(calc_ui::hover_text(&CalcState::EditedOutside), None, "that one has its bar");
}

/// A region edited outside WordCraft: with the caret on it, its bar offers Restore from
/// calculation and Keep Word's equation, which run `calc.resolveEdited`.
#[test]
fn calc_edited_outside_bar_resolves_the_region() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf ="})).unwrap();
    a.run("text.newParagraph", json!({})).unwrap();
    a.run("insert.calculation", json!({"source": "Q := 2 lbf ="})).unwrap();
    let set_edited = |a: &mut WordApp| {
        for path in a.session.doc.para_paths(StoryRef::Body) {
            if let Ok(p) = a.session.doc.para_mut(StoryRef::Body, &path) {
                for o in &mut p.objects {
                    if let InlineObject::Equation { calc: Some(c), .. } = o {
                        c.state = CalcState::EditedOutside;
                    }
                }
            }
        }
    };
    set_edited(&mut a);
    let (first, _) = regions(&a).remove(0);
    a.session.sel = wordcraft_engine::Selection::caret(first);
    assert!(calc_ui::bar_target(&a).is_some(), "the caret is on an edited region");
    let mut h = ready(a);
    h.get_by_label("Restore from calculation").click();
    h.run_steps(3);
    let r = regions(h.state());
    assert_eq!(r[0].1.state, CalcState::Fresh, "restored from its source");
    assert_eq!(r[1].1.state, CalcState::EditedOutside, "the other is left alone");
    let second = r[1].0.clone();
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(second);
    h.run_steps(3);
    h.get_by_label("Keep Word's equation").click();
    h.run_steps(3);
    let r = regions(h.state());
    assert_eq!(r.len(), 1, "kept as a plain equation, no longer a calculation");
}

/// The bar doesn't show over a dialog (the source box opens where it would be).
#[test]
fn calc_edited_outside_bar_hides_behind_a_dialog() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf ="})).unwrap();
    set_state(&mut a, 0, CalcState::EditedOutside);
    let (at, _) = regions(&a).remove(0);
    a.session.sel = wordcraft_engine::Selection::caret(at);
    let mut h = ready(a);
    assert!(h.query_by_label("Restore from calculation").is_some(), "the bar shows");
    h.state_mut().run("insert.calculation", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Restore from calculation").is_none(), "not while the box is open");
}

/// Hovering an errored calculation shows its message; hovering one edited outside WordCraft
/// brings up its bar (the caret elsewhere).
#[test]
fn calc_hover_shows_the_error_and_the_edited_bar() {
    let mut a = app();
    a.run("text.insert", json!({"text": "Bad "})).unwrap();
    a.run("insert.calculation", json!({"source": "x := 1 lbf + 2 ft =", "display": false})).unwrap();
    a.run("text.insert", json!({"text": " and "})).unwrap();
    a.run("insert.calculation", json!({"source": "Q := 2 lbf =", "display": false})).unwrap();
    a.run("text.insert", json!({"text": " end"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    set_state(&mut a, 1, CalcState::EditedOutside);
    a.run("caret.docStart", json!({})).unwrap();
    let r = regions(&a);
    let CalcState::Error(message) = r[0].1.state.clone() else { panic!("not an error: {:?}", r[0].1.state) };
    let mut h = timed(a);
    let first = on_screen(&mut h, &r[0].0);
    h.event(Event::PointerMoved(first));
    h.run_steps(4);
    assert!(h.query_by_label(&message).is_some(), "the error's message shows: {message}");
    assert!(h.query_by_label("Restore from calculation").is_none());
    let second = on_screen(&mut h, &r[1].0);
    h.event(Event::PointerMoved(second));
    h.run_steps(4);
    assert!(h.query_by_label(&message).is_none(), "the tooltip goes with the pointer");
    assert!(h.query_by_label("Restore from calculation").is_some(), "hovering brings up the bar");
    assert!(h.query_by_label("Keep Word's equation").is_some());
    // Out of both: the bar goes.
    h.event(Event::PointerMoved(Pos2::new(700.0, 850.0)));
    h.run_steps(4);
    assert!(h.query_by_label("Restore from calculation").is_none(), "leaving takes the bar away");
}

/// A refused resolve reaches the status bar.
#[test]
fn calc_resolve_refusal_shows_in_the_status_bar() {
    let mut a = app();
    calc_ui::resolve(&mut a, 9999, "source");
    let status = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
    assert!(status.contains("no calculation 9999"), "{status:?}");
}

/// Go to existing finds a definition inside a table cell.
#[test]
fn calc_go_to_existing_finds_a_definition_in_a_table() {
    let mut a = app();
    a.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf"})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    let (owner, region) = regions(&a).remove(0);
    assert!(owner.path.cell().is_some(), "the definition is in a cell");
    assert_eq!(calc_ui::region_pos(&a.session.doc, region.id), Some(owner.clone()));
    let mut h = ready(a);
    open_box(&mut h);
    type_text(&mut h, "P := 2 lbf");
    enter(&mut h);
    assert_eq!(calc_box(h.state()).and_then(|b| b.prompt).and_then(|p| p.existing), Some(region.id));
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Go to existing").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none(), "the box closes");
    assert_eq!(h.state().session.sel.focus, owner, "the caret is at the owning region");
    assert_eq!(sources(h.state()), ["P := 1500 lbf"]);
}

/// Keyboard routes to the source box: Mod+Alt+= opens a new one, F2 the calculation at the caret,
/// and Enter a selected calculation; Enter beside one still starts a new paragraph.
#[test]
fn calc_keyboard_opens_the_source_box() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf", "display": false})).unwrap();
    let mut h = ready(a);
    h.state_mut().canvas.want_focus = true;
    h.run_steps(3);
    assert!(h.state().canvas.focused, "the page has the keyboard");
    let id = regions(h.state())[0].1.id;
    // F2 with the caret just after the calculation.
    h.key_press(egui::Key::F2);
    h.run_steps(3);
    assert_eq!(calc_box(h.state()).and_then(|b| b.editing), Some(id), "F2 opens the box on the calculation");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    // Enter with the calculation selected.
    let (at, _) = regions(h.state()).remove(0);
    let end = Pos { off: at.off + '\u{FFFC}'.len_utf8(), ..at.clone() };
    h.state_mut().run("select.range", json!({"anchor": at, "focus": end})).unwrap();
    h.state_mut().canvas.want_focus = true;
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(calc_box(h.state()).and_then(|b| b.editing), Some(id), "Enter opens the box on a selected calculation");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    // Enter with the caret beside it: a new paragraph, no box.
    h.state_mut().run("caret.set", json!({"pos": end})).unwrap();
    h.state_mut().canvas.want_focus = true;
    h.run_steps(3);
    let paragraphs = h.state().session.doc.paragraph_count();
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().session.doc.paragraph_count(), paragraphs + 1);
    // Mod+Alt+= opens a new calculation.
    h.key_press_modifiers(egui::Modifiers { command: true, ctrl: !cfg!(target_os = "macos"), alt: true, ..Default::default() }, egui::Key::Equals);
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(dialogs::Dialog::name), Some("calculation"));
}

#[test]
fn right_click_menu_numbers_a_calculation() {
    let mut a = app();
    let _ = a.session.run("insert.calculation", &serde_json::json!({"source": "P := 1500 lbf =", "display": true}));
    let (at, _) = regions(&a).remove(0);
    a.session.sel = wordcraft_engine::Selection { anchor: at.clone(), focus: at.clone() };
    let pick = |a: &WordApp, label: &str| {
        calc_ui::menu_items(a).into_iter().find(|(l, _, _)| *l == label).unwrap_or_else(|| panic!("no {label} in the menu"))
    };
    let labels: Vec<&str> = calc_ui::menu_items(&a).iter().map(|(l, _, _)| *l).collect();
    assert_eq!(labels, ["Edit Calculation", "Change to Inline", "Align Left", "Align Center", "Align Right", "Add Equation Number", "Recalculate"]);
    let (_, id, params) = pick(&a, "Add Equation Number");
    a.session.run(id, &params).expect("numbering a calculation from the menu works");
    assert!(regions(&a)[0].1.numbered, "the calculation is numbered");
    a.session.sel = wordcraft_engine::Selection { anchor: at.clone(), focus: at.clone() };
    assert!(calc_ui::menu_items(&a).iter().any(|(l, _, _)| *l == "Remove Equation Number"));
    let (_, id, params) = pick(&a, "Align Left");
    a.session.run(id, &params).expect("aligning a calculation from the menu works");
    assert!(
        matches!(a.session.doc.para_at(&at).and_then(|p| p.object_at(at.off)), Some(InlineObject::Equation { math, .. }) if math.jc == wordcraft_doc::math::MathJc::Left)
    );
    a.session.sel = wordcraft_engine::Selection { anchor: at.clone(), focus: at.clone() };
    let (_, id, params) = pick(&a, "Change to Inline");
    a.session.run(id, &params).expect("making a calculation inline from the menu works");
    a.session.sel = wordcraft_engine::Selection { anchor: at.clone(), focus: at };
    let labels: Vec<&str> = calc_ui::menu_items(&a).iter().map(|(l, _, _)| *l).collect();
    assert_eq!(labels, ["Edit Calculation", "Change to Display", "Add Equation Number", "Recalculate"], "inline: no alignment choices");
    // Away from a calculation the menu adds nothing.
    assert!(calc_ui::menu_items(&app()).is_empty());
}

#[test]
fn right_clicking_a_calculation_offers_its_number() {
    let mut a = app();
    let _ = a.session.run("insert.calculation", &serde_json::json!({"source": "P := 1500 lbf =", "display": true}));
    let mut h = timed(a);
    let (at, _) = regions(h.state()).remove(0);
    let point = on_screen(&mut h, &at);
    h.event(Event::PointerMoved(point));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: point, button: PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
    h.get_by_label_contains("Add Equation Number").click();
    h.run_steps(3);
    assert!(regions(h.state())[0].1.numbered, "the menu numbered the calculation");
}
