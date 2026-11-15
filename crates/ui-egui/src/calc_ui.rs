//! Inline calculations in the interface: the source box (Insert › Calculation, and a double-click
//! on a calculation), the unique-name prompt it shows before committing, Insert › Calculated
//! Value, Calculation Options, the tooltip on an errored or out-of-date region and the bar of a
//! region edited outside WordCraft.
//!
//! All the work is the engine's: the box ends in `insert.calculation` or `calc.setSource`, the
//! prompt asks `calc.checkName`, the picker runs `insert.calcValue`, the options `calc.settings`
//! and the bar `calc.resolveEdited`. Shown through [`crate::dialogs::Dialog::Calc`].

use egui::{Pos2, Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_doc::calc::{CalcRegion, CalcState, UnitSystem};
use wordcraft_doc::{Document, InlineObject, Pos, StoryRef};
use wordcraft_engine::Session;

use crate::WordApp;

/// The definition mark the box shows for a typed `:=` (U+2254 COLON EQUALS). The parser reads
/// either; what is stored is the ASCII `:=`.
pub(crate) const DEFINE_MARK: char = '\u{2254}';

/// How many `<name>_N` the rename suggestion tries before leaving the field to the user.
const MAX_SUGGESTIONS: u32 = 99;

/// The object character a calculation sits on in its paragraph's text.
const OBJ_LEN: usize = '\u{FFFC}'.len_utf8();

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "form", rename_all = "camelCase")]
pub enum CalcDialog {
    /// The source box: a new calculation, or a region's source (`editing`).
    Source(CalcBox),
    /// Insert › Calculated Value.
    Value(ValueForm),
    /// References › Calculation › Options.
    Options(OptionsForm),
}

impl CalcDialog {
    pub fn name(&self) -> &'static str {
        match self {
            CalcDialog::Source(b) if b.editing.is_some() => "editCalculation",
            CalcDialog::Source(_) => "calculation",
            CalcDialog::Value(_) => "calcValue",
            CalcDialog::Options(_) => "calcOptions",
        }
    }

    /// The window title (English; the caller translates it).
    pub fn title(&self) -> &'static str {
        match self {
            CalcDialog::Source(b) if b.editing.is_some() => "Edit Calculation",
            CalcDialog::Source(_) => "Calculation",
            CalcDialog::Value(_) => "Calculated Value",
            CalcDialog::Options(_) => "Calculation Options",
        }
    }

    /// Where the window goes: the source box opens at the caret; the others are centred.
    pub fn anchor(&self) -> Option<Pos2> {
        match self {
            CalcDialog::Source(b) => b.anchor,
            _ => None,
        }
    }
}

/// The dialog `ui.dialog` opens by `name`: `calculation` (a new one at the caret),
/// `editCalculation` (the calculation at the caret), `calcValue` or `calcOptions`.
pub fn open(name: &str, app: &mut WordApp) -> Option<CalcDialog> {
    Some(match name {
        "calculation" => CalcDialog::Source(CalcBox::new(None, "", caret_anchor(app))),
        "editCalculation" => {
            let (_, region) = calc_at_caret(app)?;
            CalcDialog::Source(CalcBox::new(Some(region.id), &region.source, caret_anchor(app)))
        }
        "calcValue" => CalcDialog::Value(ValueForm::read(app)),
        "calcOptions" => CalcDialog::Options(OptionsForm::read(&app.session.doc)),
        _ => return None,
    })
}

/// Draws the dialog; returns true to close it.
pub fn body(app: &mut WordApp, ui: &mut Ui, d: &mut CalcDialog) -> bool {
    match d {
        CalcDialog::Source(b) => source_ui(app, ui, b),
        CalcDialog::Value(f) => value_ui(app, ui, f),
        CalcDialog::Options(f) => options_ui(app, ui, f),
    }
}

/// Just below the caret, on screen.
fn caret_anchor(app: &mut WordApp) -> Option<Pos2> {
    crate::canvas::caret_screen(app).map(|(p, h)| p + vec2(0.0, h + 4.0))
}

// ---------------------------------------------------------------------------------------------
// Sources and names

/// The source as the box shows it: `:=` as `≔` when the interface font can draw `≔` (`covered`),
/// else `≔` as `:=` (a missing glyph would show as a box).
pub fn display_source(source: &str, covered: bool) -> String {
    if covered { source.replace(":=", &DEFINE_MARK.to_string()) } else { stored_source(source) }
}

/// The source as it is stored: `≔` back to `:=`.
pub fn stored_source(shown: &str) -> String {
    shown.replace(DEFINE_MARK, ":=")
}

/// The name a source defines (`σ` for `σ := P/A_b =`); `None` when it defines nothing or doesn't
/// parse (its error then shows at the region).
pub fn defined_name_of(source: &str) -> Option<String> {
    let stmt = wordcraft_calc::parse(source).ok()?;
    wordcraft_calc::defined_name(&stmt).map(str::to_string)
}

/// The source with its defined name `old` (the head before `:=` or `≔`) renamed `new`; a
/// function's parameters stay (`f(x) := …` → `g(x) := …`). `None` when the head doesn't start
/// with the whole name.
pub fn rename_source(source: &str, old: &str, new: &str) -> Option<String> {
    let head_end = [source.find(":="), source.find(DEFINE_MARK)].into_iter().flatten().min()?;
    let head = source.get(..head_end)?;
    let lead = head.len() - head.trim_start().len();
    let rest = head.get(lead..)?.strip_prefix(old)?;
    // The whole name: `Px` doesn't start with the name `P`.
    if rest.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let after = source.get(lead + old.len()..)?;
    Some(format!("{}{new}{after}", source.get(..lead)?))
}

/// The first of `<name>_1`, `<name>_2`, … up to `<name>_99` that `calc.checkName` accepts (for the
/// region `editing`, if one is being edited); `None` if none is.
pub fn suggest_name(session: &mut Session, name: &str, editing: Option<u64>) -> Option<String> {
    (1..=MAX_SUGGESTIONS).map(|n| format!("{name}_{n}")).find(|candidate| check_name(session, candidate, editing).is_ok_and(|v| is_ok(&v)))
}

fn check_name(session: &mut Session, name: &str, editing: Option<u64>) -> Result<Value, String> {
    let mut q = json!({"name": name});
    if let Some(id) = editing {
        q["id"] = json!(id);
    }
    session.run("calc.checkName", &q).map_err(|e| e.to_string())
}

fn is_ok(v: &Value) -> bool {
    v.get("ok").and_then(Value::as_bool).unwrap_or(false)
}

/// The face the interface borrows to draw `≔` in the source box (its own fonts have none): the
/// math font equations are drawn with, which has it, else any document font that does. Looked
/// up once per [`wordcraft_fonts::FontDb::generation`].
pub(crate) fn define_mark_face() -> Option<std::sync::Arc<wordcraft_fonts::FontFace>> {
    type Memo = Option<(u64, Option<std::sync::Arc<wordcraft_fonts::FontFace>>)>;
    static MEMO: std::sync::Mutex<Memo> = std::sync::Mutex::new(None);
    let db = wordcraft_fonts::FontDb::global();
    let generation = db.generation();
    let mut memo = MEMO.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((g, face)) = memo.as_ref()
        && *g == generation
    {
        return face.clone();
    }
    // Very large faces (system CJK collections) aren't borrowed by the interface.
    let usable = |f: &std::sync::Arc<wordcraft_fonts::FontFace>| f.covers(DEFINE_MARK) && f.data().len() <= 16 << 20;
    let (math, _) = wordcraft_fonts::math::resolve_math("Cambria Math");
    let face = Some(db.face(&math.family, &math.style)).filter(usable).or_else(|| db.fallback_for(DEFINE_MARK, u32::MAX).filter(usable));
    *memo = Some((generation, face.clone()));
    face
}

// ---------------------------------------------------------------------------------------------
// Finding regions

/// The calculation region of the equation at `at`, if it is one.
pub fn region_at<'a>(doc: &'a Document, at: &Pos) -> Option<&'a CalcRegion> {
    match doc.para_at(at)?.object_at(at.off)? {
        InlineObject::Equation { calc: Some(c), .. } => Some(c),
        _ => None,
    }
}

/// Where the body region `id` sits (found the way the engine finds it, table cells at any depth).
pub fn region_pos(doc: &Document, id: u64) -> Option<Pos> {
    wordcraft_calc::body_regions(doc).into_iter().find(|(_, _, c)| c.id == id).map(|(path, off, _)| Pos {
        story: StoryRef::Body,
        path: wordcraft_doc::Path(path),
        off,
    })
}

/// The calculation just after or just before the caret (a click on one puts the caret before
/// it), with where it sits.
fn calc_at_caret(app: &WordApp) -> Option<(Pos, CalcRegion)> {
    let focus = &app.session.sel.focus;
    let before = focus.off.checked_sub(OBJ_LEN).map(|off| Pos { off, ..focus.clone() });
    std::iter::once(focus.clone()).chain(before).find_map(|at| region_at(&app.session.doc, &at).map(|c| (at.clone(), c.clone())))
}

/// What the right-click menu adds when the caret is at a calculation: editing its source, its
/// equation number (numbering a calculation works like numbering a plain equation) and
/// recalculating. Empty anywhere else.
pub fn menu_items(app: &WordApp) -> Vec<(&'static str, &'static str, Value)> {
    let Some((_, c)) = calc_at_caret(app) else { return Vec::new() };
    let number = if c.numbered { "Remove Equation Number" } else { "Add Equation Number" };
    vec![
        ("Edit Calculation", "calc.edit", json!({})),
        (number, "equation.number", json!({"value": !c.numbered})),
        ("Recalculate", "calc.recalculate", json!({})),
    ]
}

/// Whether the selection is exactly one calculation (its object character, either way round).
pub fn selected_calc(app: &WordApp) -> bool {
    let sel = &app.session.sel;
    let (a, b) = if sel.anchor.off <= sel.focus.off { (&sel.anchor, &sel.focus) } else { (&sel.focus, &sel.anchor) };
    a.story == b.story && a.path == b.path && b.off == a.off + OBJ_LEN && region_at(&app.session.doc, a).is_some()
}

// ---------------------------------------------------------------------------------------------
// The source box

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalcBox {
    /// What the box shows (`:=` as `≔` where the font has it; see [`display_source`] and
    /// [`stored_source`]).
    pub source: String,
    /// The region whose source this is; `None` for a new calculation.
    pub editing: Option<u64>,
    /// The unique-name prompt, while it shows.
    pub prompt: Option<NamePrompt>,
    /// Why the engine refused the last commit.
    pub message: String,
    /// Put the keyboard in the box on the next frame.
    #[serde(skip)]
    pub focus: bool,
    #[serde(skip)]
    pub anchor: Option<Pos2>,
}

/// The unique-name prompt: the clash message and a new name to rename the definition to.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamePrompt {
    /// The name the source defines.
    pub name: String,
    /// `duplicate` or `reserved`.
    pub reason: String,
    pub message: String,
    /// The region that already defines the name (duplicates).
    pub existing: Option<u64>,
    /// The field, pre-filled with the first free `<name>_N`.
    pub new_name: String,
    /// Why Rename couldn't rewrite the source.
    pub error: String,
    #[serde(skip)]
    pub focus: bool,
}

/// What a commit led to.
#[derive(Clone, Debug, PartialEq)]
pub enum Commit {
    /// Inserted or changed: the box closes.
    Done,
    /// The name clashes: the prompt shows.
    Prompt,
    /// Refused (the message says why), or nothing to commit: the box stays.
    Stay,
}

impl CalcBox {
    pub fn new(editing: Option<u64>, source: &str, anchor: Option<Pos2>) -> CalcBox {
        CalcBox { source: source.to_string(), editing, prompt: None, message: String::new(), focus: true, anchor }
    }

    /// Enter: check the defined name, then insert the calculation or change the region's source.
    /// A clashing name shows the prompt instead; nothing is inserted under it.
    pub fn commit(&mut self, app: &mut WordApp) -> Commit {
        let source = stored_source(&self.source);
        if source.trim().is_empty() {
            return Commit::Stay;
        }
        if let Some(name) = defined_name_of(&source) {
            match check_name(&mut app.session, &name, self.editing) {
                Ok(v) if !is_ok(&v) => {
                    self.prompt = Some(NamePrompt::new(app, &name, &v, self.editing));
                    return Commit::Prompt;
                }
                Ok(_) => {}
                Err(e) => {
                    self.message = e;
                    return Commit::Stay;
                }
            }
        }
        let r = match self.editing {
            Some(id) => app.run("calc.setSource", json!({"id": id, "source": source})),
            None => app.run("insert.calculation", json!({"source": source})),
        };
        match r {
            Ok(_) => Commit::Done,
            Err(e) => {
                self.message = e;
                Commit::Stay
            }
        }
    }

    /// Rename: rewrite the defined name in the source to the prompt's new name, then commit
    /// (which checks the new name in turn).
    pub fn rename(&mut self, app: &mut WordApp) -> Commit {
        let Some(p) = self.prompt.as_mut() else { return Commit::Stay };
        let new_name = p.new_name.trim().to_string();
        match rename_source(&stored_source(&self.source), &p.name, &new_name) {
            Some(renamed) if !new_name.is_empty() => {
                self.source = renamed;
                self.prompt = None;
                self.commit(app)
            }
            _ => {
                p.error = crate::i18n::fmt(tl!("{name} can't be renamed in this calculation"), &[("name", &p.name)]);
                Commit::Stay
            }
        }
    }

    /// Go to existing: the caret moves to the region that defines the name, and the box closes.
    pub fn go_to_existing(&self, app: &mut WordApp) -> bool {
        let Some(at) = self.prompt.as_ref().and_then(|p| p.existing).and_then(|id| region_pos(&app.session.doc, id)) else { return false };
        app.run("caret.set", json!({"pos": at})).is_ok()
    }
}

impl NamePrompt {
    fn new(app: &mut WordApp, name: &str, check: &Value, editing: Option<u64>) -> NamePrompt {
        let reason = check.get("reason").and_then(Value::as_str).unwrap_or("reserved").to_string();
        let existing = check.get("existing").and_then(Value::as_u64);
        let mut message = check.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
        // The engine checks without a layout; the page the existing definition is on helps.
        if reason == "duplicate"
            && let Some(page) = existing.and_then(|id| region_pos(&app.session.doc, id)).and_then(|at| page_number(app, &at))
        {
            message = crate::i18n::fmt(
                tl!("{name} is already defined on page {page} — choose a unique name"),
                &[("name", name), ("page", &page.to_string())],
            );
        }
        let new_name = suggest_name(&mut app.session, name, editing).unwrap_or_else(|| name.to_string());
        NamePrompt { name: name.to_string(), reason, message, existing, new_name, error: String::new(), focus: true }
    }
}

/// The page number (as shown) the position is on.
fn page_number(app: &mut WordApp, at: &Pos) -> Option<u32> {
    let layout = app.session.layout();
    layout.page_of(at).and_then(|i| layout.pages.get(i)).map(|p| p.number)
}

/// The box's text with `:=` shown as `≔` (or back, see [`display_source`]), and where a cursor
/// at char index `cursor` goes in it: `:=` becoming `≔` before the cursor moves it back one.
pub fn show_define_mark(text: &str, cursor: usize, covered: bool) -> (String, usize) {
    let shown = display_source(text, covered);
    let before: String = text.chars().take(cursor).collect();
    // A cursor between `:` and `=` lands after the mark.
    (shown, display_source(&before, covered).chars().count())
}

/// Applies [`show_define_mark`] to the box after an edit, moving its cursor to match.
fn fix_define_mark(ui: &Ui, out: &mut egui::text_edit::TextEditOutput, text: &mut String, covered: bool) {
    if display_source(text, covered) == *text {
        return;
    }
    let cursor: usize = out.cursor_range.map_or(text.chars().count(), |r| r.primary.index.into());
    let (shown, at) = show_define_mark(text, cursor, covered);
    *text = shown;
    out.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
    out.state.clone().store(ui.ctx(), out.response.id);
}

fn error_line(ui: &mut Ui, message: &str) {
    if !message.is_empty() {
        ui.colored_label(ui.visuals().error_fg_color, message);
    }
}

fn source_ui(app: &mut WordApp, ui: &mut Ui, b: &mut CalcBox) -> bool {
    if b.prompt.is_some() {
        return prompt_ui(app, ui, b);
    }
    let label = ui.label(tl!("Calculation"));
    let font = egui::TextStyle::Body.resolve(ui.style());
    let covered = ui.fonts_mut(|f| f.has_glyph(&font, DEFINE_MARK));
    // What the font can't draw never reaches the box (a source read from a file may have `≔`).
    if !covered && b.source.contains(DEFINE_MARK) {
        b.source = stored_source(&b.source);
    }
    let mut out = egui::TextEdit::singleline(&mut b.source)
        .font(egui::TextStyle::Body)
        .id(egui::Id::unique("calc_source_box"))
        .desired_width(380.0)
        .hint_text(tl!("For example: P := 1500 lbf"))
        .show(ui);
    out.response.response = out.response.response.clone().labelled_by(label.id);
    if std::mem::take(&mut b.focus) {
        out.response.request_focus();
    }
    if out.response.changed() {
        b.message.clear();
    }
    fix_define_mark(ui, &mut out, &mut b.source, covered);
    error_line(ui, &b.message);
    ui.label(egui::RichText::new(tl!("Define with :=, show a result with = at the end (σ := P/A =).")).small().weak());
    let ok = if b.editing.is_some() { tl!("Update") } else { tl!("Insert") };
    let (ok, cancel) = crate::dialogs::buttons(ui, ok);
    if cancel {
        return true;
    }
    if ok {
        match b.commit(app) {
            Commit::Done => return true,
            Commit::Prompt => {}
            Commit::Stay => b.focus = true,
        }
    }
    false
}

fn prompt_ui(app: &mut WordApp, ui: &mut Ui, b: &mut CalcBox) -> bool {
    let Some(p) = b.prompt.as_mut() else { return false };
    ui.label(egui::RichText::new(stored_source(&b.source)).monospace());
    ui.add_space(4.0);
    error_line(ui, &p.message);
    ui.add_space(4.0);
    let label = ui.label(tl!("New name"));
    let field = ui.add(egui::TextEdit::singleline(&mut p.new_name).desired_width(240.0)).labelled_by(label.id);
    if std::mem::take(&mut p.focus) {
        field.request_focus();
    }
    if field.changed() {
        p.error.clear();
    }
    error_line(ui, &p.error);
    let duplicate = p.reason == "duplicate" && p.existing.is_some();
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let escape = crate::dialogs::cancel_key_pressed(ui);
    let (mut rename, mut go, mut cancel) = (enter, false, escape);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(crate::rtl::far_side(ui, egui::Align::Center), |ui| {
            cancel |= ui.button(tl!("Cancel")).clicked();
            if duplicate {
                go |= ui.button(tl!("Go to existing")).clicked();
            }
            rename |= ui.add(crate::widgets::primary_button(ui, tl!("Rename"))).clicked();
        });
    });
    if cancel {
        // Back to the box, with the source as typed.
        b.prompt = None;
        b.focus = true;
        return false;
    }
    if go {
        return b.go_to_existing(app);
    }
    if rename {
        match b.rename(app) {
            Commit::Done => return true,
            Commit::Prompt => {}
            Commit::Stay => {
                if b.prompt.is_none() {
                    b.focus = true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// Insert › Calculated Value

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueForm {
    /// Every variable a value can quote (`calc.names`).
    pub names: Vec<String>,
    pub name: String,
    /// Unit to show the value in; empty: the variable's own.
    pub unit: String,
    /// Decimal places of its own (else the document's).
    pub precision_on: bool,
    pub precision: u8,
    pub message: String,
}

impl ValueForm {
    pub fn read(app: &mut WordApp) -> ValueForm {
        let names: Vec<String> = app
            .session
            .run("calc.names", &json!({}))
            .ok()
            .and_then(|v| v.get("names").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|n| n.get("name").and_then(Value::as_str).map(str::to_string))
            .collect();
        let precision = app.session.doc.calc_settings.clone().unwrap_or_default().precision.min(15);
        ValueForm {
            name: names.first().cloned().unwrap_or_default(),
            names,
            unit: String::new(),
            precision_on: false,
            precision,
            message: String::new(),
        }
    }

    /// The `insert.calcValue` parameters.
    pub fn params(&self) -> Value {
        let mut p = json!({"name": self.name});
        if !self.unit.trim().is_empty() {
            p["unit"] = json!(self.unit.trim());
        }
        if self.precision_on {
            p["precision"] = json!(self.precision.min(15));
        }
        p
    }
}

fn value_ui(app: &mut WordApp, ui: &mut Ui, f: &mut ValueForm) -> bool {
    if f.names.is_empty() {
        ui.label(tl!("No calculation defines a variable yet. Insert one with Insert › Calculation."));
    } else {
        egui::Grid::new("calc_value_grid").num_columns(2).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
            let label = ui.label(tl!("Variable"));
            craft_ui::ComboBox::from_id_salt("calc_value_name")
                .selected_text(f.name.clone())
                .show_ui(ui, |ui| {
                    for n in &f.names {
                        ui.selectable_value(&mut f.name, n.clone(), n);
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();
            let label = ui.label(tl!("Unit (optional)"));
            ui.add(egui::TextEdit::singleline(&mut f.unit).desired_width(120.0)).labelled_by(label.id);
            ui.end_row();
            let own = ui.checkbox(&mut f.precision_on, tl!("Decimal places"));
            ui.add_enabled(f.precision_on, egui::DragValue::new(&mut f.precision).range(0..=15)).labelled_by(own.id);
            ui.end_row();
        });
    }
    error_line(ui, &f.message);
    let (ok, cancel) = crate::dialogs::buttons(ui, tl!("Insert"));
    if cancel {
        return true;
    }
    if ok && !f.names.is_empty() {
        match app.run("insert.calcValue", f.params()) {
            Ok(_) => return true,
            Err(e) => f.message = e,
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// Calculation Options

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionsForm {
    /// `us` or `si`.
    pub system: String,
    /// Decimal places, 0–15.
    pub precision: u8,
    /// Recalculate only on request.
    pub manual: bool,
    pub message: String,
}

impl OptionsForm {
    /// The document's settings as they are (read from the document: `calc.settings` would
    /// recalculate).
    pub fn read(doc: &Document) -> OptionsForm {
        let s = doc.calc_settings.clone().unwrap_or_default();
        let system = match s.system {
            UnitSystem::UsCustomary => "us",
            UnitSystem::Si => "si",
        };
        OptionsForm { system: system.into(), precision: s.precision.min(15), manual: s.manual, message: String::new() }
    }

    /// The `calc.settings` parameters.
    pub fn params(&self) -> Value {
        json!({"system": self.system, "precision": self.precision.min(15), "manual": self.manual})
    }
}

fn options_ui(app: &mut WordApp, ui: &mut Ui, f: &mut OptionsForm) -> bool {
    ui.label(egui::RichText::new(tl!("Show results in")).strong());
    ui.radio_value(&mut f.system, "us".to_string(), tl!("US customary"));
    ui.radio_value(&mut f.system, "si".to_string(), tl!("SI (metric)"));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let label = ui.label(tl!("Decimal places"));
        ui.add(egui::DragValue::new(&mut f.precision).range(0..=15)).labelled_by(label.id);
    });
    ui.add_space(6.0);
    ui.checkbox(&mut f.manual, tl!("Recalculate manually"))
        .on_hover_text(tl!("Results update only on References › Recalculate; until then they show out of date"));
    error_line(ui, &f.message);
    let (ok, cancel) = crate::dialogs::buttons(ui, tl!("OK"));
    if cancel {
        return true;
    }
    if ok {
        match app.run("calc.settings", f.params()) {
            Ok(_) => return true,
            Err(e) => f.message = e,
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// On the page: tooltips and the edited-outside bar

/// What hovering a region shows: an error's message, or that the result is out of date. An
/// edited-outside region has its bar instead ([`bar`]).
pub fn hover_text(state: &CalcState) -> Option<String> {
    match state {
        CalcState::Error(message) => Some(message.clone()),
        CalcState::Stale => Some(tl!("Out of date — recalculate (References › Recalculate)").to_string()),
        CalcState::Fresh | CalcState::EditedOutside => None,
    }
}

/// The edited-outside region the pointer is on, kept while the pointer is on it or on its bar.
#[derive(Clone)]
struct HoverBar {
    id: u64,
    at: Pos,
    /// The frame the pointer was last on the region.
    frame: u64,
    /// Where the bar was drawn last (screen).
    rect: Option<egui::Rect>,
}

fn bar_key() -> egui::Id {
    egui::Id::unique("calc_edited_bar")
}

/// The pointer is on the page at (`page`, `x`, `y`): over a calculation, an error or a stale
/// result shows as a tooltip, and an edited-outside region brings up its bar.
pub(crate) fn hover(app: &WordApp, ui: &Ui, layout: &wordcraft_layout::DocLayout, page: usize, x: f32, y: f32, story: StoryRef) {
    let Some((at, _)) = layout.equation_hit(page, x, y, story) else { return };
    let Some(region) = region_at(&app.session.doc, &at) else { return };
    let ctx = ui.ctx();
    if region.state == CalcState::EditedOutside {
        let rect = ctx.data(|d| d.get_temp::<HoverBar>(bar_key())).filter(|h| h.id == region.id).and_then(|h| h.rect);
        let frame = ctx.cumulative_frame_nr();
        ctx.data_mut(|d| d.insert_temp(bar_key(), HoverBar { id: region.id, at, frame, rect }));
        return;
    }
    if let Some(text) = hover_text(&region.state) {
        egui::Tooltip::always_open(ctx.clone(), ui.layer_id(), egui::Id::unique("calc_tip"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(text);
        });
    }
}

/// The edited-outside region whose bar shows from the caret: the one the caret is on.
pub fn bar_target(app: &WordApp) -> Option<(u64, Pos)> {
    if app.session.view.read_mode || !app.session.sel.is_collapsed() {
        return None;
    }
    calc_at_caret(app).filter(|(_, c)| c.state == CalcState::EditedOutside).map(|(at, c)| (c.id, at))
}

/// The bar under a region edited outside WordCraft (the one hovered, else the one at the caret):
/// Restore from calculation, or Keep Word's equation (`calc.resolveEdited`).
pub(crate) fn bar(app: &mut WordApp, ctx: &egui::Context) {
    // Not over a dialog (the source box opens at the caret, where the bar would be).
    if app.dialog.is_some() {
        return;
    }
    let pointer = ctx.pointer_hover_pos();
    let frame = ctx.cumulative_frame_nr();
    let hovered = ctx
        .data(|d| d.get_temp::<HoverBar>(bar_key()))
        .filter(|h| h.frame == frame || pointer.is_some_and(|p| h.rect.is_some_and(|r| r.expand(8.0).contains(p))));
    let target = hovered.as_ref().map(|h| (h.id, h.at.clone())).or_else(|| bar_target(app));
    let still_edited =
        |app: &WordApp, id: u64, at: &Pos| region_at(&app.session.doc, at).is_some_and(|c| c.id == id && c.state == CalcState::EditedOutside);
    let Some((id, at)) = target.filter(|(id, at)| still_edited(app, *id, at)) else {
        ctx.data_mut(|d| d.remove::<HoverBar>(bar_key()));
        return;
    };
    let hint = app.session.page_hint;
    let Some(eq) = app.session.layout().equation_box(&at, hint) else { return };
    let Some(anchor) = crate::canvas::page_to_screen(app, eq.page, eq.x, eq.bottom) else { return };
    let mut keep = None;
    let shown = egui::Area::new(bar_key().with("area")).order(egui::Order::Foreground).fixed_pos(anchor + vec2(0.0, 2.0)).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tl!("This calculation was edited outside WordCraft."));
                if ui.button(tl!("Restore from calculation")).clicked() {
                    keep = Some("source");
                }
                if ui.button(tl!("Keep Word's equation")).clicked() {
                    keep = Some("word");
                }
            });
        });
    });
    match hovered {
        Some(mut h) => {
            h.rect = Some(shown.response.rect);
            ctx.data_mut(|d| d.insert_temp(bar_key(), h));
        }
        None => ctx.data_mut(|d| d.remove::<HoverBar>(bar_key())),
    }
    if let Some(keep) = keep {
        resolve(app, id, keep);
        ctx.data_mut(|d| d.remove::<HoverBar>(bar_key()));
    }
}

/// Runs `calc.resolveEdited` for the region `id`: `source` redraws it from its calculation,
/// `word` keeps the equation as edited (no longer a calculation).
pub fn resolve(app: &mut WordApp, id: u64, keep: &str) {
    let _ = app.run("calc.resolveEdited", json!({"id": id, "keep": keep}));
    app.canvas.want_focus = true;
}
