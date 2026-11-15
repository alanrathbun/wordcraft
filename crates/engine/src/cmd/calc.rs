//! Inline calculations: insert a calculation region, change its source, quote a variable's value,
//! list and check names, recalculate, set the document's calculation options, and resolve a
//! region edited outside the calculator.
//!
//! Results never wait for a request: after every command that changes the document,
//! [`after_command`] recalculates (or, in manual mode, marks the results stale) inside the same
//! undo step, so undo and redo bring back results that match their sources.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use wordcraft_calc::{
    CalcError, RecalcReport, calc_unchanged, check_new_name, check_quote_unit, defined_name, defined_names, freeze_outside_body, is_calc_object,
    mark_stale, parse, recalc,
};
use wordcraft_doc::calc::{CalcRegion, CalcState, UnitSystem, calc_field_instr, parse_calc_field};
use wordcraft_doc::math::Math;
use wordcraft_doc::{Block, Document, InlineObject, Paragraph, Pos, StoryRef};

use super::delete_selection;
use crate::{CmdError, CmdResult, CommandSpec, ImeEditOutcome, Selection, Session, p};

/// Longest calculation source accepted (the parser's own limit).
const MAX_SOURCE_BYTES: usize = 16 * 1024;
/// Longest name `calc.checkName` and `insert.calcValue` accept.
const MAX_NAME_BYTES: usize = 1024;
/// Highest precision a quoted value or the document can ask for.
const MAX_PRECISION: u64 = 15;

/// Commands after which the automatic pass doesn't run: undo and redo put back a document whose
/// results already match (re-marking them stale would change what undo restores), and the
/// calculation commands that recalculate or re-mark themselves.
const NO_AUTO_PASS: [&str; 5] = ["edit.undo", "edit.redo", "edit.undoTo", "calc.recalculate", "calc.settings"];

/// Builds the calculation command table.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("insert.calculation", "Calculation", "Insert › Symbols", insert_calculation)
            .key("Mod+Alt+=")
            .params(r#"{"source": string (e.g. "P := 1500 lbf", "σ := P/A_b = ? ksi"), "display"?: bool}"#),
        CommandSpec::new("calc.setSource", "Set Calculation Source", "Calculation", set_source).params(r#"{"id": n, "source": string}"#),
        CommandSpec::new("calc.edit", "Edit Calculation", "Calculation", edit)
            .key("F2")
            .pure()
            .when(|s| calc_at_caret(s).is_none().then_some("no calculation at the caret")),
        CommandSpec::new("insert.calcValue", "Calculated Value", "Insert › Symbols", insert_value)
            .params(r#"{"name": string (a variable, see calc.names), "unit"?: string, "precision"?: n (0–15)}"#),
        CommandSpec::new("calc.names", "Calculation Names", "Calculation", names).pure(),
        CommandSpec::new("calc.checkName", "Check Calculation Name", "Calculation", check).pure().params(r#"{"name": string, "id"?: n}"#),
        CommandSpec::new("calc.recalculate", "Recalculate", "References › Calculation", recalculate),
        CommandSpec::new("calc.settings", "Calculation Options", "References › Calculation", settings)
            .params(r#"{"system"?: "us|si", "precision"?: n (0–15), "manual"?: bool}"#),
        CommandSpec::new("calc.resolveEdited", "Resolve Edited Calculation", "Calculation", resolve_edited)
            .params(r#"{"id": n, "keep": "source|word"}"#),
    ]
}

// ---------------------------------------------------------------------------------------------
// The automatic pass
// ---------------------------------------------------------------------------------------------

/// Whether the body holds a calculation region or a quoted value (`WCCALC` field): when it
/// doesn't, editing pays nothing for calculations. One pass over the paragraphs' objects, tables
/// walked with an explicit stack (a hostile file can nest them deeply).
pub(crate) fn has_calc(doc: &Document) -> bool {
    let mut todo: Vec<&Block> = Vec::new();
    for top in &doc.body {
        todo.push(top);
        while let Some(b) = todo.pop() {
            match b {
                Block::Para(p) => {
                    if p.objects.iter().any(is_calc_object) {
                        return true;
                    }
                }
                Block::Table(t) => todo.extend(t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).map(|b| &**b)),
            }
        }
    }
    false
}

/// Whether a story other than the body (a header, footer, note, comment or text box) holds a
/// calculation region or a quoted value: they are never calculated there, so the pass freezes
/// them ([`wordcraft_calc::freeze_outside_body`]).
pub(crate) fn parts_have_calc(doc: &Document) -> bool {
    doc.parts.values().any(|part| {
        let mut todo: Vec<&Block> = part.blocks.iter().map(|b| &**b).collect();
        while let Some(b) = todo.pop() {
            match b {
                Block::Para(p) => {
                    if p.objects.iter().any(is_calc_object) {
                        return true;
                    }
                }
                Block::Table(t) => todo.extend(t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).map(|b| &**b)),
            }
        }
        false
    })
}

/// After a successful mutating command, inside its undo step: recalculate, or in manual mode mark
/// the results stale. Skipped when the command changed nothing (it stays a no-op), when it
/// changed nothing a calculation reads or shows ([`calc_unchanged`]: typing ordinary text) and
/// when the document has no calculations.
///
/// The pass reads file content, so it runs inside its own `catch_unwind` (the command's has
/// already returned): a panic puts the document back as the command left it, greys every result
/// (if even that works, so no result looks current) and says so in the status line. The edit
/// itself stands.
pub(crate) fn after_command(s: &mut Session, id: &str, before: &Document) {
    if NO_AUTO_PASS.contains(&id) || s.doc.same_content(before) || calc_unchanged(before, &s.doc) {
        return;
    }
    let (body, outside) = (has_calc(&s.doc), parts_have_calc(&s.doc));
    if !body && !outside {
        return;
    }
    let manual = s.doc.calc_settings.as_ref().is_some_and(|c| c.manual);
    let saved = s.doc.clone();
    let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(test)]
        tests::maybe_inject_panic();
        if outside {
            freeze_outside_body(&mut s.doc);
        }
        if body && manual {
            mark_stale(&mut s.doc);
        } else if body {
            recalc_now(s);
        }
    }));
    let Err(panic) = pass else { return };
    let message = panic.downcast_ref::<&str>().map(ToString::to_string).or_else(|| panic.downcast_ref::<String>().cloned()).unwrap_or_default();
    s.doc = saved.clone();
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mark_stale(&mut s.doc))).is_err() {
        s.doc = saved;
    }
    s.status = format!("Calculations couldn't be updated: {message}");
}

thread_local! {
    /// What [`after_open`] has to tell about the document it last failed on, for `file.open` to
    /// show in the status line (opening has no session to show it in).
    static OPEN_NOTICE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Take the message [`after_open`] left, if any (see `OPEN_NOTICE`).
pub(crate) fn take_open_notice() -> Option<String> {
    OPEN_NOTICE.with(|n| n.borrow_mut().take())
}

/// After a document is opened (any format): calculations saved in a `.docx` come back live from
/// the store read with it (`wordcraft_calc::docx::import`), and every calculation is
/// recalculated so the results are current; regions edited outside WordCraft keep what they
/// show. A document with no calculations is left as it was read.
///
/// The pass reads file content, so it runs inside `catch_unwind` like [`after_command`]: a panic
/// puts the document back as read, greys every result so none looks current, and leaves a
/// message for the status line ([`take_open_notice`]).
pub(crate) fn after_open(doc: &mut Document) {
    let store = doc.calc_store.take();
    if store.is_none() && !has_calc(doc) && !wordcraft_calc::docx::has_calc_controls(doc) && !parts_have_calc(doc) {
        return;
    }
    let had_store = store.is_some();
    let saved = doc.clone();
    let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(test)]
        tests::maybe_inject_panic();
        freeze_outside_body(doc);
        wordcraft_calc::docx::import(doc, store)
    }));
    let panic = match pass {
        Ok(report) => {
            if let Some(notice) = edited_notice(report.edited) {
                OPEN_NOTICE.with(|n| *n.borrow_mut() = Some(notice));
            }
            return;
        }
        Err(panic) => panic,
    };
    let message = panic.downcast_ref::<&str>().map(ToString::to_string).or_else(|| panic.downcast_ref::<String>().cloned()).unwrap_or_default();
    *doc = saved.clone();
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mark_stale(doc))).is_err() {
        *doc = saved;
    }
    let notice = if had_store {
        format!("Calculations couldn't be restored ({message}): the saved sources were lost, so they opened as plain equations.")
    } else {
        format!("Calculations couldn't be updated ({message}): their results are shown out of date.")
    };
    OPEN_NOTICE.with(|n| *n.borrow_mut() = Some(notice));
}

/// What opening tells about calculations edited in Word, when there are any.
fn edited_notice(edited: usize) -> Option<String> {
    match edited {
        0 => None,
        1 => Some("1 calculation was edited in Word: it is marked ⚠, and calculations that use it show an error until you resolve it.".to_string()),
        n => Some(format!(
            "{n} calculations were edited in Word: they are marked ⚠, and calculations that use them show an error until you resolve them."
        )),
    }
}

/// Recalculate the document. A duplicate-name message names the page of the first definition,
/// so in Print Layout, when two regions define the same name, the document's pages are looked up
/// (once per revision: see [`RegionPages`]). In the other views, which have no pages, and when no
/// name repeats, no layout is made and the message says "elsewhere in the document".
pub(crate) fn recalc_now(s: &mut Session) -> RecalcReport {
    if s.view.read_mode || s.view.mode != wordcraft_layout::ViewMode::Print || !repeats_a_name(&s.doc) {
        return recalc(&mut s.doc, &|_| None);
    }
    let pages = region_pages(s);
    recalc(&mut s.doc, &|id| pages.get(&id).copied())
}

/// The page of each region (see [`region_pages`]), with what it was found for: the document,
/// its revision and its body's blocks (compared by pointer, so an equal revision reached by a
/// different edit after an undo never passes for the same document).
pub(crate) struct RegionPages {
    document_id: u64,
    rev: u64,
    body: Vec<std::sync::Arc<Block>>,
    pages: std::sync::Arc<HashMap<u64, u32>>,
}

/// Whether two (live) body regions define the same name.
fn repeats_a_name(doc: &Document) -> bool {
    let mut seen = HashSet::new();
    body_regions(doc).iter().any(|(_, c)| parse(&c.source).ok().is_some_and(|stmt| defined_name(&stmt).is_some_and(|n| !seen.insert(n.to_string()))))
}

/// The page number (as shown) of each body region, by id; the first region with an id wins.
/// Reused from [`Session::calc_pages`] while the document is the one it was found for.
fn region_pages(s: &mut Session) -> std::sync::Arc<HashMap<u64, u32>> {
    let (document_id, rev) = (s.document_id(), s.rev());
    if let Some(c) = &s.calc_pages
        && c.document_id == document_id
        && c.rev == rev
        && c.body.len() == s.doc.body.len()
        && c.body.iter().zip(&s.doc.body).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
    {
        return c.pages.clone();
    }
    let layout = s.print_layout_now();
    let mut pages = HashMap::new();
    for (at, c) in body_regions(&s.doc) {
        if pages.contains_key(&c.id) {
            continue;
        }
        if let Some(number) = layout.page_of(&at).and_then(|i| layout.pages.get(i)).map(|p| p.number) {
            pages.insert(c.id, number);
        }
    }
    let pages = std::sync::Arc::new(pages);
    s.calc_pages = Some(RegionPages { document_id, rev, body: s.doc.body.clone(), pages: pages.clone() });
    pages
}

// ---------------------------------------------------------------------------------------------
// Finding regions
// ---------------------------------------------------------------------------------------------

/// Every live calculation region in the body (table cells included, tracked deletions left out),
/// in document order, with where its equation sits, and with the id recalculation gives it: the
/// same traversal and ids as the recalculation pass itself (`wordcraft_calc::body_regions`).
fn body_regions(doc: &Document) -> Vec<(Pos, CalcRegion)> {
    wordcraft_calc::body_regions(doc)
        .into_iter()
        .map(|(path, off, c)| (Pos { story: StoryRef::Body, path: wordcraft_doc::Path(path), off }, c))
        .collect()
}

/// Where the region `id` (as recalculation numbers it) sits.
fn find_region(doc: &Document, id: u64) -> Result<Pos, CmdError> {
    body_regions(doc).into_iter().find(|(_, c)| c.id == id).map(|(at, _)| at).ok_or_else(|| CmdError::Failed(format!("there's no calculation {id}")))
}

/// The region at `at`, to change.
fn region_mut<'a>(s: &'a mut Session, at: &Pos) -> Result<(&'a mut Paragraph, usize), CmdError> {
    let para = s.doc.para_mut(at.story, &at.path)?;
    if !matches!(para.object_at(at.off), Some(InlineObject::Equation { calc: Some(_), .. })) {
        return Err(CmdError::Failed("no calculation there".into()));
    }
    Ok((para, at.off))
}

/// A new region's id: one more than any id in the document (every story, deleted ones too) and
/// than any id a pasted copy is known by, so it is no region's id now.
fn fresh_id(doc: &Document) -> Result<u64, CmdError> {
    let stories = std::iter::once(StoryRef::Body).chain(doc.parts.keys().map(|k| StoryRef::Part(*k)));
    let mut max: Option<u64> = body_regions(doc).iter().map(|(_, c)| c.id).max();
    for story in stories {
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            for o in &para.objects {
                if let InlineObject::Equation { calc: Some(c), .. } = o {
                    max = Some(max.map_or(c.id, |m| m.max(c.id)));
                }
            }
        }
    }
    match max {
        None => Ok(1),
        Some(m) => m.checked_add(1).ok_or_else(|| CmdError::Failed("no calculation id is left".into())),
    }
}

// ---------------------------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------------------------

fn req_u64(v: &Value, k: &str) -> Result<u64, CmdError> {
    v.get(k).and_then(Value::as_u64).ok_or_else(|| CmdError::Params(format!("missing or invalid `{k}` (a whole number)")))
}

/// An optional whole-number parameter: absent or null is `None`, anything else must be a number.
fn opt_u64(v: &Value, k: &str) -> Result<Option<u64>, CmdError> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => x.as_u64().map(Some).ok_or_else(|| CmdError::Params(format!("`{k}` must be a whole number"))),
    }
}

/// An optional string parameter: absent or null is `None`, anything else must be a string.
fn opt_str<'a>(v: &'a Value, k: &str) -> Result<Option<&'a str>, CmdError> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => x.as_str().map(Some).ok_or_else(|| CmdError::Params(format!("`{k}` must be a string"))),
    }
}

/// An optional true/false parameter: absent or null is `None`, anything else must be a bool.
fn opt_bool(v: &Value, k: &str) -> Result<Option<bool>, CmdError> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => x.as_bool().map(Some).ok_or_else(|| CmdError::Params(format!("`{k}` is true or false"))),
    }
}

/// Stage 1 recalculates the main text and its tables only: calculations and quoted values go
/// nowhere else (headers, footers, notes, comments and text boxes are other stories).
fn require_body(s: &Session, id: &str) -> Result<(), CmdError> {
    if s.sel.focus.story != StoryRef::Body || s.sel.anchor.story != StoryRef::Body {
        return Err(CmdError::Disabled(format!("{id}: calculations go in the document body (its text or a table in it)")));
    }
    Ok(())
}

fn precision_param(v: &Value) -> Result<Option<u8>, CmdError> {
    match opt_u64(v, "precision")? {
        None => Ok(None),
        Some(n) if n <= MAX_PRECISION => Ok(u8::try_from(n).ok()),
        Some(_) => Err(CmdError::Params(format!("`precision` is at most {MAX_PRECISION}"))),
    }
}

fn source_param(v: &Value) -> Result<&str, CmdError> {
    let source = p::req_str(v, "source")?;
    if source.len() > MAX_SOURCE_BYTES {
        return Err(CmdError::Params(format!("the calculation is longer than {MAX_SOURCE_BYTES} bytes")));
    }
    Ok(source)
}

fn name_param(v: &Value) -> Result<&str, CmdError> {
    let name = p::req_str(v, "name")?;
    if name.len() > MAX_NAME_BYTES {
        return Err(CmdError::Params(format!("the name is longer than {MAX_NAME_BYTES} bytes")));
    }
    Ok(name)
}

/// The backstop for the CLI, MCP and agents: a source whose name is a unit, function or constant,
/// or is already defined by another region (`editing` being the region whose source changes), is
/// refused. A source that doesn't parse defines nothing and passes; it shows its error at the
/// region.
fn check_source_name(doc: &Document, source: &str, editing: Option<u64>) -> Result<(), CmdError> {
    let Ok(stmt) = parse(source) else { return Ok(()) };
    match defined_name(&stmt) {
        Some(name) => check_new_name(doc, name, editing).map_err(|e| CmdError::Failed(e.to_string())),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

/// Inserts a calculation region at the selection with a fresh id. Its result is drawn by the
/// automatic pass that follows.
fn insert_calculation(s: &mut Session, v: &Value) -> CmdResult {
    let source = source_param(v)?.to_string();
    let display = opt_bool(v, "display")?;
    require_body(s, "insert.calculation")?;
    check_source_name(&s.doc, &source, None)?;
    let id = fresh_id(&s.doc)?;
    s.math = None;
    let props = s.typing_props();
    let at = delete_selection(s)?;
    // On an empty line it's a display equation, in text an inline one.
    let empty_line = s.doc.para_at(&at).is_none_or(|p| p.text.trim().is_empty());
    let display = display.unwrap_or(empty_line);
    let region = CalcRegion { id, source, unit: None, precision: None, numbered: false, state: CalcState::Fresh };
    let obj = InlineObject::Equation { linear: String::new(), display, math: Math::default(), calc: Some(Box::new(region)) };
    let end = s.doc.insert_object(&at, obj, &props)?;
    s.sel = Selection::caret(end);
    s.set_ime_edit_outcome(ImeEditOutcome { classified: true, content_changed: true, state_changed: true, ..ImeEditOutcome::default() });
    Ok(json!({"id": id}))
}

/// The calculation the caret is on or just after (a selection of exactly one calculation ends
/// just after it), with its id.
fn calc_at_caret(s: &Session) -> Option<(u64, String)> {
    let f = &s.sel.focus;
    if f.story != StoryRef::Body {
        return None;
    }
    let para = s.doc.para_at(f)?;
    let at = |off: usize| match para.object_at(off) {
        Some(InlineObject::Equation { calc: Some(c), .. }) => Some((c.id, c.source.clone())),
        _ => None,
    };
    at(f.off).or_else(|| f.off.checked_sub(wordcraft_doc::para::OBJ.len_utf8()).and_then(at))
}

/// The calculation at the caret, for its source box (the interface opens the box on this
/// command: F2, or Enter on a selected calculation).
fn edit(s: &mut Session, _: &Value) -> CmdResult {
    let (id, source) = calc_at_caret(s).ok_or_else(|| CmdError::Disabled("no calculation at the caret".into()))?;
    Ok(json!({"id": id, "source": source}))
}

/// Replaces a region's source. The region is current again (an error or an edit outside the
/// calculator was about the old source); the automatic pass redraws it.
fn set_source(s: &mut Session, v: &Value) -> CmdResult {
    let id = req_u64(v, "id")?;
    let source = source_param(v)?.to_string();
    let at = find_region(&s.doc, id)?;
    check_source_name(&s.doc, &source, Some(id))?;
    let (para, off) = region_mut(s, &at)?;
    if let Some(InlineObject::Equation { linear, math, calc: Some(c), .. }) = para.object_at_mut(off) {
        // What was typed outside the calculator goes with it: no copy of it can show (stale) in
        // manual mode.
        if c.state == CalcState::EditedOutside {
            linear.clear();
            math.nodes.clear();
            math.omml.clear();
        }
        c.source = source;
        c.state = CalcState::Fresh;
    }
    para.touch();
    Ok(json!({"id": id}))
}

/// Inserts a locked `WCCALC` field quoting a variable's value. Stage 1 keeps quoted values up to
/// date in the main text and its tables only, so it is refused anywhere else.
fn insert_value(s: &mut Session, v: &Value) -> CmdResult {
    let name = name_param(v)?.to_string();
    let unit = opt_str(v, "unit")?.map(str::trim).filter(|u| !u.is_empty()).map(str::to_string);
    let precision = precision_param(v)?;
    require_body(s, "insert.calcValue")?;
    if !defined_names(&s.doc).iter().any(|(n, _)| *n == name) {
        return Err(CmdError::Failed(format!("{name} isn't a defined variable")));
    }
    // A unit the value can't be shown in would only ever show `?`.
    if let Some(unit) = unit.as_deref() {
        check_quote_unit(&s.doc, &name, unit).map_err(|e| CmdError::Failed(format!("{name} can't be shown in {unit}: {e}")))?;
    }
    // The field code must read back as written, or the value would never be updated.
    let instr = calc_field_instr(&name, unit.as_deref(), precision);
    if unit.as_deref().is_some_and(|u| u.contains('"')) || parse_calc_field(&instr) != Some((name.clone(), unit.clone(), precision)) {
        return Err(CmdError::Params("that unit can't be quoted".into()));
    }
    s.math = None;
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let field = InlineObject::Field { instr, result: "?".into(), locked: true, dirty: false, code: false, form: String::new() };
    let end = s.doc.insert_object(&at, field, &props)?;
    s.sel = Selection::caret(end);
    s.set_ime_edit_outcome(ImeEditOutcome { classified: true, content_changed: true, state_changed: true, ..ImeEditOutcome::default() });
    Ok(json!({"name": name}))
}

/// Every variable a quoted value can show, with its region's id.
fn names(s: &mut Session, _: &Value) -> CmdResult {
    let names: Vec<Value> = defined_names(&s.doc).into_iter().map(|(name, id)| json!({"name": name, "id": id})).collect();
    Ok(json!({"names": names}))
}

/// Whether `name` may be given to a new region (or to region `id`).
fn check(s: &mut Session, v: &Value) -> CmdResult {
    let name = name_param(v)?;
    let editing = opt_u64(v, "id")?;
    // The name as the parser reads it: anything that isn't a single name is refused.
    let name = name.trim();
    let parsed = parse(&format!("{name} := 0")).ok().and_then(|stmt| defined_name(&stmt).map(str::to_string));
    // Exactly one plain name: `f(x)` defines `f` and is no name.
    let Some(parsed) = parsed.filter(|n| !name.is_empty() && n == name) else {
        let message = wordcraft_calc::check_name(name).err().map_or_else(|| format!("{name} isn't a valid name"), |e| e.to_string());
        return Ok(json!({"ok": false, "reason": "reserved", "message": message}));
    };
    Ok(match check_new_name(&s.doc, &parsed, editing) {
        Ok(()) => json!({"ok": true}),
        Err(e @ CalcError::Duplicate { .. }) => {
            let mut r = json!({"ok": false, "reason": "duplicate", "message": e.to_string()});
            if let Some((_, id)) = defined_names(&s.doc).into_iter().find(|(n, _)| *n == parsed) {
                r["existing"] = json!(id);
            }
            r
        }
        // A unit, function or constant name, or an empty subscript part.
        Err(e) => json!({"ok": false, "reason": "reserved", "message": e.to_string()}),
    })
}

fn report_json(r: RecalcReport) -> Value {
    json!({"regions": r.regions, "errors": r.errors, "reassignedIds": r.reassigned_ids})
}

/// Recalculates every calculation now (in manual mode, the only way results become current).
fn recalculate(s: &mut Session, _: &Value) -> CmdResult {
    Ok(report_json(recalc_now(s)))
}

fn system_name(sys: UnitSystem) -> &'static str {
    match sys {
        UnitSystem::UsCustomary => "us",
        UnitSystem::Si => "si",
    }
}

/// Sets the document's unit system, precision and manual mode. Back in automatic mode the
/// document recalculates; in manual mode a change to how results are shown marks them stale,
/// and switching to manual alone leaves them current.
fn settings(s: &mut Session, v: &Value) -> CmdResult {
    let old = s.doc.calc_settings.clone().unwrap_or_default();
    let mut new = old.clone();
    match v.get("system") {
        None | Some(Value::Null) => {}
        Some(x) => {
            new.system = match x.as_str() {
                Some("us") => UnitSystem::UsCustomary,
                Some("si") => UnitSystem::Si,
                _ => return Err(CmdError::Params("`system` is \"us\" or \"si\"".into())),
            }
        }
    }
    if let Some(precision) = precision_param(v)? {
        new.precision = precision;
    }
    match v.get("manual") {
        None | Some(Value::Null) => {}
        Some(x) => new.manual = x.as_bool().ok_or_else(|| CmdError::Params("`manual` is true or false".into()))?,
    }
    if new != old {
        s.doc.calc_settings = Some(new.clone());
    }
    if has_calc(&s.doc) {
        if !new.manual {
            recalc_now(s);
        } else if new.system != old.system || new.precision != old.precision {
            mark_stale(&mut s.doc);
        }
    }
    Ok(json!({"system": system_name(new.system), "precision": new.precision, "manual": new.manual}))
}

/// A region edited outside the calculator: `"source"` draws it again from its source (the edited
/// result is dropped, so no stale copy of it can show); `"word"` keeps the equation as edited and
/// makes it a plain equation, no longer a calculation.
fn resolve_edited(s: &mut Session, v: &Value) -> CmdResult {
    let id = req_u64(v, "id")?;
    let keep = p::req_str(v, "keep")?;
    if !matches!(keep, "source" | "word") {
        return Err(CmdError::Params("`keep` is \"source\" or \"word\"".into()));
    }
    let at = find_region(&s.doc, id)?;
    let (para, off) = region_mut(s, &at)?;
    if let Some(InlineObject::Equation { linear, math, calc, .. }) = para.object_at_mut(off) {
        if keep == "word" {
            *calc = None;
            wordcraft_calc::render::strip_edited_mark(linear, math);
        } else if let Some(c) = calc {
            c.state = CalcState::Fresh;
            linear.clear();
            math.nodes.clear();
            math.omml.clear();
        }
    }
    para.touch();
    Ok(json!({"id": id, "keep": keep}))
}

#[cfg(test)]
mod flange_bolt_sample;
#[cfg(test)]
mod tests;
