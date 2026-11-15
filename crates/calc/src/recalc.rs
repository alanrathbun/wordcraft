//! The recalculation pass: every calculation region in the document's body, evaluated in
//! dependency order, with unique names, and its display and quoted values rewritten.
//!
//! Stage 1 covers the main body and table cells (to any depth); headers, footers, notes, comments
//! and text boxes are skipped, as is tracked-deleted content. The pass reads the whole document
//! first and changes only the objects whose display, state or id actually changes, so plain
//! equations and untouched paragraphs stay exactly as they were (and shared, for undo).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use craft_units::{Formatted, Precision, Quantity, UnitSystem, format, parse_unit};
use wordcraft_doc::calc::{self, CalcSettings, CalcState, parse_calc_field};
use wordcraft_doc::math::Arg;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{Block, Blocks, Document, InlineObject, Paragraph};

use crate::eval::{MAX_PASS_STEPS, Pass, run_stmt_in};
use crate::render::{self, Rendered, rendered, shown_result, shows_value};
use crate::{CalcError, Env, Expr, Stmt, check_name, defined_name, eval_expr, free_names, parse};

/// What a [`recalc`] pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecalcReport {
    /// Calculation regions in the body (tracked deletions left out).
    pub regions: usize,
    /// Regions left in [`CalcState::Error`].
    pub errors: usize,
    /// Regions given a new id because an earlier region already had theirs (a copy and paste).
    pub reassigned_ids: usize,
    /// Regions edited outside WordCraft ([`CalcState::EditedOutside`]) waiting to be resolved.
    pub edited: usize,
}

/// Most names a circular-definition message lists.
const MAX_CYCLE_NAMES: usize = 5;

/// Recalculate every calculation in the body of `doc`.
///
/// 1. Collects the regions in document order (paragraphs top to bottom, objects left to right,
///    table cells row by row), giving a fresh id to every region whose id an earlier one has.
/// 2. Parses each region's source; a parse error defines nothing.
/// 3. The first region in document order defining a name owns it; a later one is a duplicate
///    error (`page_of` maps the owner's id to its page for the message), and a unit, function or
///    constant name is a reserved-name error. Neither defines anything.
/// 4. Evaluates in dependency order. Regions on a cycle are circular errors and every region that
///    depends on a failed name is an upstream error.
/// 5. Rewrites each region's displayed result. A region edited outside the calculator keeps what
///    it shows, with the edited mark after it ([`render::mark_edited`]); it defines nothing, so
///    every region that uses its name is an error ("depends on a calculation edited in Word")
///    and every quoted value of it shows `?`.
/// 6. Rewrites the result of every `WCCALC` field quoting a value; one with no valid value
///    shows `?`.
///
/// Never panics; any document, however malformed, gives a report.
pub fn recalc(doc: &mut Document, page_of: &dyn Fn(u64) -> Option<u32>) -> RecalcReport {
    let settings = doc.calc_settings.clone().unwrap_or_default();
    let found = collect(&doc.body);
    let (ids, reassigned_ids) = assign_ids(&found);
    let regions = &found.regions;
    let mut index = Index::build(regions, &ids, page_of);
    // A region's own display unit applies when its source names none.
    for (stmt, r) in index.parsed.iter_mut().zip(regions) {
        if let (Ok(stmt), Some(unit)) = (stmt, r.unit.as_deref().filter(|u| !u.trim().is_empty())) {
            match stmt {
                Stmt::Eval { show, .. } | Stmt::Define { show: Some(show), .. } if show.unit.is_none() => show.unit = Some(unit.to_string()),
                _ => {}
            }
        }
    }
    let (env, outcomes, failed) = evaluate(&index);

    let sys = unit_system(&settings);
    let mut errors = 0;
    let mut edited = 0;
    let mut edits: Vec<Edit> = Vec::new();
    for (i, r) in regions.iter().enumerate() {
        let id = ids.get(i).copied().unwrap_or(r.id);
        if r.state == CalcState::EditedOutside {
            edited += 1;
            // Word's equation stays as it is, with the mark after it so it never looks current.
            let display = object(&doc.body, &r.at).and_then(|o| match o {
                InlineObject::Equation { linear, math, .. } => render::mark_edited(linear, math),
                _ => None,
            });
            if id != r.id || display.is_some() {
                edits.push(Edit { at: r.at.clone(), change: Change::Region { id, state: r.state.clone(), display } });
            }
            continue;
        }
        let stmt = index.parsed.get(i);
        let outcome = outcomes.get(i).cloned().unwrap_or(Err(CalcError::Upstream));
        let shown = match outcome {
            Ok(Some(q)) => {
                let unit = stmt.and_then(|s| s.as_ref().ok()).and_then(show_unit);
                format_value(q, unit.as_deref(), sys, precision(r.precision, &settings)).map(Some)
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        };
        let (state, shown) = match shown {
            Ok(shown) => (CalcState::Fresh, shown),
            Err(e) => (CalcState::Error(e.to_string()), None),
        };
        if matches!(state, CalcState::Error(_)) {
            errors += 1;
        }
        let parse_failed = Err(CalcError::Upstream);
        let display = rendered(stmt.unwrap_or(&parse_failed), &r.source, shown.as_ref(), &state);
        edits.push(Edit { at: r.at.clone(), change: Change::Region { id, state, display: Some(display) } });
    }
    for f in &found.fields {
        let result = field_result(f, &index, &env, &failed, &settings);
        edits.push(Edit { at: f.at.clone(), change: Change::Field { result } });
    }
    apply(&mut doc.body, edits);
    RecalcReport { regions: regions.len(), errors, reassigned_ids, edited }
}

/// Every variable owned by a definition, with its region's id, sorted by name: what a quoted
/// value can show. Owning means the first region in document order that defines the name, with
/// a name that isn't a unit, function or constant. User functions are left out (they have no
/// single value); ids are the ones [`recalc`] gives, a repeated id already replaced.
pub fn defined_names(doc: &Document) -> Vec<(String, u64)> {
    let (ids, index) = names_of(doc);
    let mut names: Vec<(String, u64)> = index
        .owners
        .iter()
        .filter(|&(_, &i)| matches!(index.parsed.get(i), Some(Ok(Stmt::Define { params, .. })) if params.is_empty()))
        .map(|(name, &i)| (name.clone(), ids.get(i).copied().unwrap_or(0)))
        .collect();
    names.sort();
    names
}

/// The regions' ids as [`recalc`] gives them (a repeated id replaced) and who owns each name.
fn names_of(doc: &Document) -> (Vec<u64>, Index) {
    let found = collect(&doc.body);
    let (ids, _) = assign_ids(&found);
    let index = Index::build(&found.regions, &ids, &|_| None);
    (ids, index)
}

/// Whether `name` may be given to a new calculation, or to the region `editing` (whose own
/// definition of the name doesn't count): [`CalcError::ReservedName`] for a unit, function or
/// constant name, [`CalcError::Duplicate`] when another region already owns it (a variable or a
/// function).
pub fn check_new_name(doc: &Document, name: &str, editing: Option<u64>) -> Result<(), CalcError> {
    check_name(name)?;
    // Ids as recalculation gives them, so a pasted copy's repeated id can't stand for its owner.
    let (ids, index) = names_of(doc);
    match index.owners.get(name).and_then(|&i| ids.get(i)) {
        Some(&owner) if Some(owner) != editing => Err(CalcError::Duplicate { name: name.to_string(), page: None }),
        _ => Ok(()),
    }
}

/// Mark every current ([`CalcState::Fresh`]) region in the body [`CalcState::Stale`], for manual
/// recalculation: its result is redrawn grey after `≈`, so it never looks current. A region whose
/// id an earlier one has (a pasted copy) gets the id [`recalc`] would give it. Quoted values
/// (`WCCALC` fields) get a leading `≈ ` too; [`recalc`] puts back the exact value.
pub fn mark_stale(doc: &mut Document) {
    let found = collect(&doc.body);
    // A pasted copy gets its own id now, as recalculation would give it: the id stored on every
    // region is unique after any command, so nothing that names a region by it reaches another.
    let (ids, _) = assign_ids(&found);
    let mut edits = Vec::new();
    for (i, r) in found.regions.iter().enumerate() {
        let id = ids.get(i).copied().unwrap_or(r.id);
        if r.state != CalcState::Fresh {
            if id != r.id {
                edits.push(Edit { at: r.at.clone(), change: Change::Region { id, state: r.state.clone(), display: None } });
            }
            continue;
        }
        let stmt = parse(&r.source);
        let old = object(&doc.body, &r.at).and_then(|o| match o {
            InlineObject::Equation { linear, .. } => shown_result(linear),
            _ => None,
        });
        let shown = stmt.as_ref().ok().filter(|s| shows_value(s)).and(old);
        let display = rendered(&stmt, &r.source, shown.as_ref(), &CalcState::Stale);
        edits.push(Edit { at: r.at.clone(), change: Change::Region { id, state: CalcState::Stale, display: Some(display) } });
    }
    // A quoted value is out of date too: it gets the same `≈`, once.
    for f in &found.fields {
        if let Some(InlineObject::Field { result, .. }) = object(&doc.body, &f.at)
            && result != "?"
            && !result.starts_with('≈')
        {
            edits.push(Edit { at: f.at.clone(), change: Change::Field { result: format!("≈ {result}") } });
        }
    }
    apply(&mut doc.body, edits);
}

// ---------------------------------------------------------------------------------------------
// Collecting
// ---------------------------------------------------------------------------------------------

/// Where an object sits: the path to its paragraph (body block index, then row, cell and block
/// index for each table level) and its index in the paragraph's objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct At {
    pub(crate) path: Vec<u32>,
    pub(crate) obj: usize,
}

pub(crate) struct FoundRegion {
    pub(crate) at: At,
    pub(crate) id: u64,
    source: String,
    unit: Option<String>,
    precision: Option<u8>,
    state: CalcState,
}

struct FoundField {
    at: At,
    name: String,
    unit: Option<String>,
    precision: Option<u8>,
}

#[derive(Default)]
pub(crate) struct Found {
    pub(crate) regions: Vec<FoundRegion>,
    fields: Vec<FoundField>,
    /// Ids of tracked-deleted regions: never reused for a reassigned id.
    deleted_ids: Vec<u64>,
    /// Where each tracked-deleted region is, with its id (in document order).
    pub(crate) deleted: Vec<(At, u64)>,
}

/// The calculation regions and quoted values of the body, in document order.
pub(crate) fn collect(body: &Blocks) -> Found {
    let mut found = Found::default();
    walk_paras(body, &mut |path, p| visit(path, p, &mut found));
    found
}

/// Every paragraph of `blocks` in document order (table cells row by row, to any depth), with its
/// path: block index, then row, cell and block index for each table level.
fn walk_paras<'a>(blocks: &'a Blocks, f: &mut dyn FnMut(&[u32], &'a Paragraph)) {
    for (i, b) in blocks.iter().enumerate() {
        let top = u32::try_from(i).unwrap_or(u32::MAX);
        match &**b {
            Block::Para(p) => f(&[top], p),
            Block::Table(_) => {
                // An explicit stack, last first, so cells come off it row by row.
                let mut todo: Vec<(&Block, Vec<u32>)> = vec![(&**b, vec![top])];
                while let Some((b, path)) = todo.pop() {
                    match b {
                        Block::Para(p) => f(&path, p),
                        Block::Table(t) => {
                            for (r, row) in t.rows.iter().enumerate().rev() {
                                for (c, cell) in row.cells.iter().enumerate().rev() {
                                    for (j, cb) in cell.blocks.iter().enumerate().rev() {
                                        let mut inner = path.clone();
                                        inner.extend([r, c, j].map(|k| u32::try_from(k).unwrap_or(u32::MAX)));
                                        todo.push((&**cb, inner));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Whether `o` is a calculation region or a quoted value (`WCCALC` field; the field type is the
/// whole first word of its code).
pub fn is_calc_object(o: &InlineObject) -> bool {
    match o {
        InlineObject::Equation { calc, .. } => calc.is_some(),
        InlineObject::Field { instr, .. } => {
            let code = instr.trim_start();
            code.get(..calc::CALC_FIELD.len()).is_some_and(|w| w.eq_ignore_ascii_case(calc::CALC_FIELD))
                && code.get(calc::CALC_FIELD.len()..).is_some_and(|rest| rest.chars().next().is_none_or(char::is_whitespace))
        }
        _ => false,
    }
}

/// One calculation object as far as the recalculation pass is concerned: what it reads, and what
/// it writes (so a display or quoted value changed by anything else is drawn again).
#[derive(PartialEq)]
enum Item<'a> {
    Region { story: Option<u32>, deleted: bool, region: &'a calc::CalcRegion, linear: &'a str },
    Field { story: Option<u32>, deleted: bool, instr: &'a str, result: &'a str, dirty: bool },
}

/// Every calculation object of `doc` in document order: the body's, then each other story's.
fn items(doc: &Document) -> Vec<Item<'_>> {
    let mut out = Vec::new();
    let stories = std::iter::once((None, &doc.body)).chain(doc.parts.iter().map(|(&k, part)| (Some(k), &part.blocks)));
    for (story, blocks) in stories {
        walk_paras(blocks, &mut |_, p| {
            if !p.objects.iter().any(is_calc_object) {
                return;
            }
            let deleted = p.deleted_ranges();
            let mut anchors = p.text.char_indices().filter(|&(_, c)| c == OBJ).map(|(i, _)| i);
            for o in &p.objects {
                let deleted = anchors.next().is_some_and(|off| deleted.iter().any(|r| r.contains(&off)));
                match o {
                    InlineObject::Equation { linear, calc: Some(region), .. } => out.push(Item::Region { story, deleted, region, linear }),
                    InlineObject::Field { instr, result, dirty, .. } if is_calc_object(o) => {
                        out.push(Item::Field { story, deleted, instr, result, dirty: *dirty });
                    }
                    _ => {}
                }
            }
        });
    }
    out
}

/// Whether the pass has nothing to do after a command turned `before` into `after`: the same
/// calculation settings and the same calculation objects in the same order (each region's id,
/// source, unit, precision, state and display; each quoted value's code and result; whether
/// each is tracked-deleted and which story it is in), and, in automatic mode, no result out of
/// date. Typing ordinary text leaves all of that alone, so it costs no recalculation (and in
/// manual mode marks nothing stale).
pub fn calc_unchanged(before: &Document, after: &Document) -> bool {
    if before.calc_settings != after.calc_settings {
        return false;
    }
    let now = items(after);
    let manual = after.calc_settings.as_ref().is_some_and(|c| c.manual);
    // In automatic mode a stale result is left from a pass that failed: the next edit retries.
    let stale = !manual && now.iter().any(|i| matches!(i, Item::Region { region, .. } if region.state == CalcState::Stale));
    !stale && items(before) == now
}

/// What a calculation outside the main text says instead of a result.
pub const NOT_CALCULATED_HERE: &str = "not calculated here";

/// Calculations live in the main text and its tables only. One that lands anywhere else — pasted
/// into a text box, header, footer, note or comment, or read from a file that has it there — is
/// never recalculated, so it must not look current:
///
/// - a calculation region becomes a plain equation showing its calculation with no result, then
///   a red `⚠` and "not calculated here";
/// - a quoted value (`WCCALC` field) shows `?`.
///
/// Returns how many objects it changed. Paragraphs with nothing to change are not copied.
pub fn freeze_outside_body(doc: &mut Document) -> usize {
    let mut todo: Vec<(u32, Vec<u32>)> = Vec::new();
    for (&id, part) in &doc.parts {
        walk_paras(&part.blocks, &mut |path, p| {
            if p.objects.iter().any(|o| needs_freezing(o)) {
                todo.push((id, path.to_vec()));
            }
        });
    }
    let mut changed = 0;
    for (id, path) in todo {
        let Some(part) = doc.parts.get_mut(&id) else { continue };
        let Some(p) = para_mut(&mut part.blocks, &path) else { continue };
        for o in &mut p.objects {
            if needs_freezing(o) {
                freeze(o);
                changed += 1;
            }
        }
        p.touch();
    }
    changed
}

/// Whether [`freeze_outside_body`] has anything to do to `o`.
fn needs_freezing(o: &InlineObject) -> bool {
    match o {
        InlineObject::Equation { calc, .. } => calc.is_some(),
        InlineObject::Field { result, dirty, .. } => is_calc_object(o) && (result != "?" || *dirty),
        _ => false,
    }
}

fn freeze(o: &mut InlineObject) {
    match o {
        InlineObject::Equation { linear, math, calc, .. } => {
            let Some(region) = calc.take() else { return };
            let state = CalcState::Error(NOT_CALCULATED_HERE.to_string());
            let mut display = rendered(&parse(&region.source), &region.source, None, &state);
            render::push_note(&mut display, NOT_CALCULATED_HERE);
            *linear = display.linear;
            set_nodes(math, display.math);
        }
        InlineObject::Field { result, dirty, .. } => {
            *result = "?".to_string();
            *dirty = false;
        }
        _ => {}
    }
}

fn visit(path: &[u32], p: &Paragraph, found: &mut Found) {
    if !p.objects.iter().any(is_calc_object) {
        return;
    }
    // Object k sits at the k-th object character; one past them is shown nowhere.
    let anchors: Vec<usize> = p.text.char_indices().filter(|&(_, c)| c == OBJ).map(|(i, _)| i).collect();
    let deleted = p.deleted_ranges();
    for (k, o) in p.objects.iter().enumerate() {
        let Some(&off) = anchors.get(k) else { break };
        let is_deleted = deleted.iter().any(|r| r.contains(&off));
        let at = || At { path: path.to_vec(), obj: k };
        match o {
            InlineObject::Equation { calc: Some(c), .. } => {
                if is_deleted {
                    found.deleted_ids.push(c.id);
                    found.deleted.push((at(), c.id));
                } else {
                    found.regions.push(FoundRegion {
                        at: at(),
                        id: c.id,
                        source: c.source.clone(),
                        unit: c.unit.clone(),
                        precision: c.precision,
                        state: c.state.clone(),
                    });
                }
            }
            InlineObject::Field { instr, .. } if !is_deleted => {
                if let Some((name, unit, precision)) = parse_calc_field(instr) {
                    found.fields.push(FoundField { at: at(), name, unit, precision });
                }
            }
            _ => {}
        }
    }
}

/// Each region's id, the first of any repeated id kept and the rest given the next unused one;
/// and how many were reassigned.
pub(crate) fn assign_ids(found: &Found) -> (Vec<u64>, usize) {
    let mut used: HashSet<u64> = found.regions.iter().map(|r| r.id).chain(found.deleted_ids.iter().copied()).collect();
    let mut next = used.iter().max().and_then(|m| m.checked_add(1));
    let mut probe: u64 = 0;
    let mut seen: HashSet<u64> = HashSet::new();
    let mut reassigned = 0;
    let ids = found
        .regions
        .iter()
        .map(|r| {
            if seen.insert(r.id) {
                return r.id;
            }
            reassigned += 1;
            let id = match next {
                Some(n) => {
                    next = n.checked_add(1);
                    n
                }
                // The top of the range is taken: the lowest free id. There are far fewer regions
                // than ids, so one is always found.
                None => {
                    while used.contains(&probe) {
                        probe = probe.saturating_add(1);
                    }
                    probe
                }
            };
            used.insert(id);
            seen.insert(id);
            id
        })
        .collect();
    (ids, reassigned)
}

// ---------------------------------------------------------------------------------------------
// Names and evaluation
// ---------------------------------------------------------------------------------------------

/// Parsed regions and who owns each name.
struct Index {
    parsed: Vec<Result<Stmt, CalcError>>,
    /// An error found before evaluation: parse, reserved name or duplicate.
    early: Vec<Option<CalcError>>,
    /// Name → index of the region that owns it.
    owners: HashMap<String, usize>,
    /// Regions edited outside the calculator: they keep owning their name but define nothing.
    edited: Vec<bool>,
}

impl Index {
    fn build(regions: &[FoundRegion], ids: &[u64], page_of: &dyn Fn(u64) -> Option<u32>) -> Index {
        let parsed: Vec<Result<Stmt, CalcError>> = regions.iter().map(|r| parse(&r.source)).collect();
        let mut owners: HashMap<String, usize> = HashMap::new();
        let early = parsed
            .iter()
            .enumerate()
            .map(|(i, stmt)| {
                let stmt = match stmt {
                    Ok(stmt) => stmt,
                    Err(e) => return Some(e.clone()),
                };
                let name = defined_name(stmt)?;
                if let Err(e) = check_name(name) {
                    return Some(e);
                }
                if let Some(&owner) = owners.get(name) {
                    let page = ids.get(owner).and_then(|&id| page_of(id));
                    return Some(CalcError::Duplicate { name: name.to_string(), page });
                }
                owners.insert(name.to_string(), i);
                None
            })
            .collect();
        let edited = regions.iter().map(|r| r.state == CalcState::EditedOutside).collect();
        Index { parsed, early, owners, edited }
    }
}

type Outcome = Result<Option<Quantity>, CalcError>;

/// A region in the dependency graph. Indexes are positions in the region list; the graph's own
/// lists are read with `get`, so a wrong index is a skipped edge rather than a panic.
#[derive(Default)]
struct Node {
    outcome: Option<Outcome>,
    failed: bool,
    /// Failed because it, or a region it depends on (at any distance), was edited outside.
    edited: bool,
    /// Regions owning a name this one uses (itself, for `x := x + 1`).
    deps: Vec<usize>,
    /// Regions using the name this one owns.
    users: Vec<usize>,
    /// Dependencies not yet evaluated.
    pending: usize,
}

/// Evaluate every region in dependency order: the environment of defined names, each region's
/// outcome, and which regions failed.
fn evaluate(index: &Index) -> (Env, Vec<Outcome>, Vec<bool>) {
    let mut nodes: Vec<Node> = index.early.iter().map(|e| Node { outcome: e.clone().map(Err), ..Node::default() }).collect();
    // Edges run from the region owning a name to each region using it. Only owned names count:
    // anything else (a unit, constant, built-in, or nothing) is left for the evaluator.
    for (i, stmt) in index.parsed.iter().enumerate() {
        let Ok(stmt) = stmt else { continue };
        // An edited region uses nothing: it fails on its own, whatever its source says.
        if nodes.get(i).is_none_or(|n| n.outcome.is_some()) || index.edited.get(i).copied().unwrap_or(false) {
            continue;
        }
        for name in free_names(stmt) {
            let Some(&owner) = index.owners.get(&name) else { continue };
            if let Some(o) = nodes.get_mut(owner) {
                o.users.push(i);
            }
            if let Some(node) = nodes.get_mut(i) {
                node.deps.push(owner);
                node.pending += 1;
            }
        }
    }
    let mut env = Env::new();
    // One budget for the whole pass: a hostile document errors instead of taking seconds.
    let pass = Pass::new(MAX_PASS_STEPS);
    // Record that region `i` failed: using its name is then an upstream error.
    let fail = |env: &mut Env, node: &mut Node, i: usize| {
        node.failed = true;
        if let Some(Ok(stmt)) = index.parsed.get(i)
            && let Some(name) = defined_name(stmt)
            && index.owners.get(name) == Some(&i)
        {
            env.mark_failed(name);
        }
    };
    // Kahn's algorithm, in document order among the ready regions.
    let mut ready: VecDeque<usize> = nodes.iter().enumerate().filter(|(_, n)| n.outcome.is_none() && n.pending == 0).map(|(i, _)| i).collect();
    while let Some(i) = ready.pop_front() {
        let failed_dep =
            |edited: bool| nodes.get(i).is_some_and(|n| n.deps.iter().any(|&d| nodes.get(d).is_some_and(|dn| dn.failed && (dn.edited || !edited))));
        let (upstream, upstream_edited) = (failed_dep(false), failed_dep(true));
        let own_edit = index.edited.get(i).copied().unwrap_or(false);
        let outcome = match index.parsed.get(i) {
            _ if own_edit => Err(CalcError::EditedOutside),
            _ if upstream_edited => Err(CalcError::UpstreamEdited),
            _ if upstream => Err(CalcError::Upstream),
            Some(Ok(stmt)) => run_stmt_in(&mut env, stmt, &pass),
            Some(Err(e)) => Err(e.clone()),
            None => Err(CalcError::Upstream),
        };
        let Some(node) = nodes.get_mut(i) else { continue };
        if outcome.is_err() {
            node.edited = own_edit || upstream_edited;
            fail(&mut env, node, i);
        }
        node.outcome = Some(outcome);
        let users = std::mem::take(&mut node.users);
        for &u in &users {
            if let Some(un) = nodes.get_mut(u) {
                un.pending = un.pending.saturating_sub(1);
                if un.pending == 0 {
                    ready.push_back(u);
                }
            }
        }
        if let Some(node) = nodes.get_mut(i) {
            node.users = users;
        }
    }
    // What is left is on a cycle or depends on one.
    let left: Vec<bool> = nodes.iter().map(|n| n.outcome.is_none()).collect();
    if left.contains(&true) {
        let users: Vec<&[usize]> = nodes.iter().map(|n| n.users.as_slice()).collect();
        let sccs = strongly_connected(&users, &left);
        for mut members in sccs {
            let self_loop = || members.first().is_some_and(|&v| nodes.get(v).is_some_and(|n| n.deps.contains(&v)));
            if members.len() < 2 && !self_loop() {
                continue;
            }
            members.sort_unstable();
            let mut names: Vec<&str> = members
                .iter()
                .filter_map(|&v| index.parsed.get(v).and_then(|s| s.as_ref().ok()).and_then(defined_name))
                .take(MAX_CYCLE_NAMES)
                .collect();
            if members.len() > MAX_CYCLE_NAMES {
                names.push("…");
            }
            let message = names.join(", ");
            for &v in &members {
                if let Some(node) = nodes.get_mut(v) {
                    node.outcome = Some(Err(CalcError::Circular(message.clone())));
                    fail(&mut env, node, v);
                }
            }
        }
        // The rest depend, through other leftovers, on a cycle.
        for (i, node) in nodes.iter_mut().enumerate() {
            if node.outcome.is_none() {
                node.outcome = Some(Err(CalcError::Upstream));
                fail(&mut env, node, i);
            }
        }
    }
    let failed = nodes.iter().map(|n| n.failed).collect();
    let outcomes = nodes.into_iter().map(|n| n.outcome.unwrap_or(Err(CalcError::Upstream))).collect();
    (env, outcomes, failed)
}

/// Tarjan's bookkeeping for one node.
#[derive(Clone, Copy, Default)]
struct Visit {
    index: Option<usize>,
    low: usize,
    on_stack: bool,
}

/// The strongly connected components of the graph `succ` restricted to the nodes in `within`
/// (Tarjan's algorithm with an explicit stack instead of recursion).
fn strongly_connected(succ: &[&[usize]], within: &[bool]) -> Vec<Vec<usize>> {
    let inside = |v: usize| within.get(v).copied().unwrap_or(false);
    let mut visits = vec![Visit::default(); succ.len()];
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut counter = 0usize;
    // Number `v` and put it on the stack.
    let mut open = |visits: &mut Vec<Visit>, stack: &mut Vec<usize>, v: usize| {
        if let Some(x) = visits.get_mut(v) {
            *x = Visit { index: Some(counter), low: counter, on_stack: true };
        }
        counter += 1;
        stack.push(v);
    };
    for root in 0..succ.len() {
        if !inside(root) || visits.get(root).is_none_or(|x| x.index.is_some()) {
            continue;
        }
        open(&mut visits, &mut stack, root);
        // Frames of (node, next successor to look at).
        let mut calls: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&mut (v, ref mut next)) = calls.last_mut() {
            let edges = succ.get(v).copied().unwrap_or_default();
            if let Some(&w) = edges.get(*next) {
                *next += 1;
                if !inside(w) {
                    continue;
                }
                let seen = visits.get(w).copied().unwrap_or_default();
                match seen.index {
                    None => {
                        open(&mut visits, &mut stack, w);
                        calls.push((w, 0));
                    }
                    Some(wi) if seen.on_stack => {
                        if let Some(x) = visits.get_mut(v) {
                            x.low = x.low.min(wi);
                        }
                    }
                    Some(_) => {}
                }
                continue;
            }
            calls.pop();
            let done = visits.get(v).copied().unwrap_or_default();
            if let Some(&(parent, _)) = calls.last()
                && let Some(x) = visits.get_mut(parent)
            {
                x.low = x.low.min(done.low);
            }
            if done.index == Some(done.low) {
                let mut scc = Vec::new();
                while let Some(w) = stack.pop() {
                    if let Some(x) = visits.get_mut(w) {
                        x.on_stack = false;
                    }
                    scc.push(w);
                    if w == v {
                        break;
                    }
                }
                out.push(scc);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------------

fn unit_system(settings: &CalcSettings) -> UnitSystem {
    match settings.system {
        calc::UnitSystem::UsCustomary => UnitSystem::UsCustomary,
        calc::UnitSystem::Si => UnitSystem::Si,
    }
}

fn precision(own: Option<u8>, settings: &CalcSettings) -> Precision {
    Precision::new(own.unwrap_or(settings.precision))
}

/// The unit a statement asks its value to be shown in.
fn show_unit(stmt: &Stmt) -> Option<String> {
    match stmt {
        Stmt::Eval { show, .. } | Stmt::Define { show: Some(show), .. } => show.unit.clone(),
        Stmt::Define { show: None, .. } => None,
    }
}

/// `q` as number and unit text, in `unit` when given, else in the natural unit of `sys`.
fn format_value(q: Quantity, unit: Option<&str>, sys: UnitSystem, p: Precision) -> Result<Formatted, CalcError> {
    let scale = unit.map(parse_unit).transpose()?;
    Ok(format(q, scale.as_ref(), sys, p)?)
}

/// The text a quoted value shows: `"600 psi"`, or `"?"` when its name has no valid value or its
/// unit doesn't fit.
fn field_result(f: &FoundField, index: &Index, env: &Env, failed: &[bool], settings: &CalcSettings) -> String {
    let valid = index.owners.get(&f.name).is_some_and(|&i| {
        !failed.get(i).copied().unwrap_or(true) && matches!(index.parsed.get(i), Some(Ok(Stmt::Define { params, .. })) if params.is_empty())
    });
    if !valid {
        return "?".to_string();
    }
    let Ok(q) = eval_expr(env, &Expr::Name(f.name.clone())) else { return "?".to_string() };
    match format_value(q, f.unit.as_deref(), unit_system(settings), precision(f.precision, settings)) {
        Ok(shown) if shown.unit.is_empty() => render::text_number(&shown.number),
        Ok(shown) => format!("{} {}", render::text_number(&shown.number), shown.unit),
        Err(_) => "?".to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// Writing back
// ---------------------------------------------------------------------------------------------

enum Change {
    Region { id: u64, state: CalcState, display: Option<Rendered> },
    Field { result: String },
}

struct Edit {
    at: At,
    change: Change,
}

/// The paragraph at `path`, if there is one.
fn para<'a>(body: &'a Blocks, path: &[u32]) -> Option<&'a Paragraph> {
    let (&first, mut rest) = path.split_first()?;
    let mut block: &Block = body.get(first as usize)?;
    loop {
        match block {
            Block::Para(p) => return rest.is_empty().then_some(p),
            Block::Table(t) => {
                let [r, c, j, tail @ ..] = rest else { return None };
                block = t.rows.get(*r as usize)?.cells.get(*c as usize)?.blocks.get(*j as usize)?;
                rest = tail;
            }
        }
    }
}

/// The paragraph at `path`, to change: only the blocks on the way to it are copied when shared.
pub(crate) fn para_mut<'a>(body: &'a mut Blocks, path: &[u32]) -> Option<&'a mut Paragraph> {
    let (&first, mut rest) = path.split_first()?;
    let mut block: &mut Block = Arc::make_mut(body.get_mut(first as usize)?);
    loop {
        match block {
            Block::Para(p) => return rest.is_empty().then_some(p),
            Block::Table(t) => {
                let [r, c, j, tail @ ..] = rest else { return None };
                block = Arc::make_mut(t.rows.get_mut(*r as usize)?.cells.get_mut(*c as usize)?.blocks.get_mut(*j as usize)?);
                rest = tail;
            }
        }
    }
}

fn object<'a>(body: &'a Blocks, at: &At) -> Option<&'a InlineObject> {
    para(body, &at.path)?.objects.get(at.obj)
}

/// Whether applying `change` to `o` would change it.
fn differs(o: &InlineObject, change: &Change) -> bool {
    match (o, change) {
        (InlineObject::Equation { linear, math, calc: Some(c), .. }, Change::Region { id, state, display }) => {
            c.id != *id || c.state != *state || display.as_ref().is_some_and(|d| *linear != d.linear || math.nodes != d.math || !math.omml.is_empty())
        }
        (InlineObject::Field { result: old, dirty, .. }, Change::Field { result }) => old != result || *dirty,
        _ => false,
    }
}

fn change_object(o: &mut InlineObject, change: Change) {
    match (o, change) {
        (InlineObject::Equation { linear, math, calc: Some(c), .. }, Change::Region { id, state, display }) => {
            c.id = id;
            c.state = state;
            if let Some(d) = display {
                *linear = d.linear;
                set_nodes(math, d.math);
            }
        }
        (InlineObject::Field { result: old, dirty, .. }, Change::Field { result }) => {
            *old = result;
            *dirty = false;
        }
        _ => {}
    }
}

/// New structure for an equation: the source markup it was read with no longer matches it.
fn set_nodes(math: &mut wordcraft_doc::math::Math, nodes: Arg) {
    math.nodes = nodes;
    math.omml.clear();
}

/// Apply the edits that change something, copying only the paragraphs they touch.
fn apply(body: &mut Blocks, edits: Vec<Edit>) {
    let mut edits: Vec<Edit> = edits.into_iter().filter(|e| object(body, &e.at).is_some_and(|o| differs(o, &e.change))).collect();
    // Bring each paragraph's edits (its regions' and its fields') together, so it is copied and
    // touched once. The sort is stable: within a paragraph, edits keep their order.
    edits.sort_by(|a, b| a.at.path.cmp(&b.at.path));
    let mut rest = edits.drain(..).peekable();
    while let Some(first) = rest.next() {
        let path = first.at.path.clone();
        let mut group = vec![first];
        while let Some(e) = rest.next_if(|e| e.at.path == path) {
            group.push(e);
        }
        let Some(p) = para_mut(body, &path) else { continue };
        for e in group {
            if let Some(o) = p.objects.get_mut(e.at.obj) {
                change_object(o, e.change);
            }
        }
        p.touch();
    }
}
