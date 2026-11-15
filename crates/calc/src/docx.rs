//! Calculations through `.docx`, for review in Word and back.
//!
//! Word knows nothing of calculations, so a saved file shows each one as what it displays: an
//! ordinary equation (`σ≔P/A_b=0.6 ksi`, or its `≈`/`⚠` form when stale or in error) inside a
//! rich text content control tagged `wordcraft-calc:<id>` and titled "Calculation". Quoted values
//! are locked `WCCALC` fields showing their result. The sources go in a hidden store
//! ([`CalcStore`], a custom XML part the `.docx` writer adds), each with a fingerprint of its
//! equation as saved.
//!
//! Reading the file back, every tagged equation whose fingerprint still matches becomes a live
//! calculation again; one whose text was changed in Word keeps Word's version and is flagged
//! [`CalcState::EditedOutside`] for the author to resolve, never silently recalculated. Then the
//! document is recalculated.
//!
//! The fingerprint is FNV-1a 64 of the equation's linear form as WordCraft writes it from its math
//! tree (`to_linear` of the tree), on both sides: the stored display text itself doesn't survive
//! OMML (units lose their quotes, a script base gains parentheses), while the tree's linear form
//! does, byte for byte.

use std::collections::{BTreeMap, HashMap, HashSet};

use wordcraft_doc::calc::{CALC_TAG_PREFIX, CalcRegion, CalcSettings, CalcState, CalcStore, MAX_STORED_REGIONS, StoredRegion};
use wordcraft_doc::control::{ControlKind, walk_controls};
use wordcraft_doc::math::{Math, parse_linear, to_linear};
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{ContentControl, Document, InlineObject, Path, StoryRef};

use crate::recalc::{assign_ids, collect, para_mut};

/// The title Word shows on a calculation's content control.
pub const CONTROL_TITLE: &str = "Calculation";

/// Mixed into the fingerprint saved for a region already edited outside WordCraft, so that it
/// can't match the equation read back: saving a flagged region again must not make Word's text
/// look like WordCraft's.
const EDITED_SALT: &[u8] = b"\0wordcraft-calc:edited\0";

/// FNV-1a, 64 bits.
fn fnv1a64(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in parts.iter().flat_map(|p| p.iter()) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The linear form a fingerprint is taken of: the math tree's, or (no tree) the linear text's.
fn canonical(linear: &str, math: &Math) -> String {
    if math.nodes.is_empty() { to_linear(&parse_linear(linear)) } else { to_linear(&math.nodes) }
}

/// The fingerprint of an equation as shown: FNV-1a 64 of its canonical linear form.
pub fn fingerprint(linear: &str, math: &Math) -> u64 {
    fnv1a64(&[canonical(linear, math).as_bytes()])
}

/// A copy of `doc` to give a `.docx` writer, and the store to save beside it.
///
/// Every calculation region the recalculation pass sees (body and table cells, tracked deletions
/// left out) becomes a plain equation, showing what it shows now, inside a content control
/// tagged with its id; a repeated id (a copy not yet recalculated) gets the id recalculation
/// would give it. The quoted values stay as they are (locked fields). `doc` is not changed.
pub fn export(doc: &Document) -> (Document, CalcStore) {
    let mut copy = doc.clone();
    copy.calc_store = None;
    let found = collect(&copy.body);
    let (ids, _) = assign_ids(&found);
    // Each paragraph's regions together, last object first, so earlier offsets stay valid.
    let mut by_para: BTreeMap<Vec<u32>, Vec<(usize, usize)>> = BTreeMap::new();
    for (i, r) in found.regions.iter().enumerate() {
        by_para.entry(r.at.path.clone()).or_default().push((r.at.obj, i));
    }
    let mut stored: Vec<Option<StoredRegion>> = vec![None; found.regions.len()];
    for (path, mut objs) in by_para {
        let Some(p) = para_mut(&mut copy.body, &path) else { continue };
        let anchors: Vec<usize> = p.text.char_indices().filter(|&(_, c)| c == OBJ).map(|(i, _)| i).collect();
        objs.sort_unstable_by(|a, b| b.cmp(a));
        for (k, i) in objs {
            let (Some(&off), Some(&id)) = (anchors.get(k), ids.get(i)) else { continue };
            let Some(InlineObject::Equation { linear, math, calc, .. }) = p.objects.get_mut(k) else { continue };
            let Some(region) = calc.take() else { continue };
            let text = canonical(linear, math);
            let fingerprint =
                if region.state == CalcState::EditedOutside { fnv1a64(&[EDITED_SALT, text.as_bytes()]) } else { fnv1a64(&[text.as_bytes()]) };
            let CalcRegion { source, unit, precision, .. } = *region;
            if let Some(slot) = stored.get_mut(i) {
                *slot = Some(StoredRegion { id, source, unit, precision, fingerprint });
            }
            let control = ContentControl {
                tag: format!("{CALC_TAG_PREFIX}{id}"),
                title: CONTROL_TITLE.to_string(),
                kind: ControlKind::RichText,
                ..Default::default()
            };
            let props = p.props_of_char(off).clone();
            // Neither can fail: both offsets are character boundaries of this paragraph.
            if p.insert_object(off + OBJ.len_utf8(), InlineObject::ControlEnd, &props).is_ok() {
                let _ = p.insert_object(off, InlineObject::ControlStart { control: Box::new(control) }, &props);
            }
        }
    }
    let settings = doc.calc_settings.clone().unwrap_or_default();
    (copy, CalcStore { settings, regions: stored.into_iter().flatten().collect() })
}

/// The region id a content control tag names, if it is a calculation's.
fn tagged_id(tag: &str) -> Option<Option<u64>> {
    tag.strip_prefix(CALC_TAG_PREFIX).map(|id| id.parse().ok())
}

/// Whether the body holds a calculation's content control (as read from a `.docx`).
pub fn has_calc_controls(doc: &Document) -> bool {
    let mut found = false;
    walk_controls(&doc.body, &mut |c| found |= c.tag.starts_with(CALC_TAG_PREFIX));
    found
}

/// Turn a document read from a `.docx` back into a live one, with the store read beside it.
///
/// For each content control in the body tagged as a calculation (in document order), the first
/// plain equation inside it gets its stored region back: [`CalcState::Fresh`] when its
/// fingerprint matches, [`CalcState::EditedOutside`] (Word's text and structure kept) when it
/// doesn't. Every tagged control's markers are removed, matched or not. Left as plain math: a
/// control naming no stored region, a second control naming the same one (a copy made in Word),
/// and every control when there is no store. A stored region whose control is gone is dropped.
///
/// The store is untrusted: one over [`MAX_STORED_REGIONS`] counts as none, and an id stored twice
/// counts as not stored. The store's settings become the document's. Finally the document is
/// recalculated (regions edited outside keep their display). A document with no calculations
/// and no calculation controls is left exactly as it is.
pub fn import(doc: &mut Document, store: Option<CalcStore>) {
    doc.calc_store = None;
    let mut by_id: HashMap<u64, StoredRegion> = HashMap::new();
    if let Some(store) = store.filter(|s| s.regions.len() <= MAX_STORED_REGIONS) {
        let mut repeated: HashSet<u64> = HashSet::new();
        for r in store.regions {
            let id = r.id;
            if by_id.insert(id, r).is_some() {
                repeated.insert(id);
            }
        }
        for id in repeated {
            by_id.remove(&id);
        }
        doc.calc_settings = (store.settings != CalcSettings::default()).then_some(store.settings);
    }

    let mut ranges: Vec<_> = doc.control_ranges(StoryRef::Body).into_iter().filter_map(|r| tagged_id(&r.control.tag).map(|id| (id, r))).collect();
    ranges.sort_by(|(_, a), (_, b)| (&a.start.path, a.start.off).cmp(&(&b.start.path, b.start.off)));

    let mut markers: BTreeMap<Path, Vec<usize>> = BTreeMap::new();
    for (id, r) in ranges {
        markers.entry(r.start.path.clone()).or_default().push(r.start.off);
        markers.entry(r.end.path.clone()).or_default().push(r.end.off);
        let Some(id) = id else { continue };
        let Ok(p) = doc.para_mut(StoryRef::Body, &r.start.path) else { continue };
        let end = if r.end.path == r.start.path { r.end.off } else { p.text.len() };
        let anchors: Vec<usize> = p.text.char_indices().filter(|&(_, c)| c == OBJ).map(|(i, _)| i).collect();
        let first = anchors
            .iter()
            .enumerate()
            .find(|&(k, &off)| off > r.start.off && off < end && matches!(p.objects.get(k), Some(InlineObject::Equation { calc: None, .. })));
        let Some((k, _)) = first else { continue };
        // Claimed by the first control naming it.
        let Some(stored) = by_id.remove(&id) else { continue };
        if let Some(InlineObject::Equation { linear, math, calc, .. }) = p.objects.get_mut(k) {
            let state = if fingerprint(linear, math) == stored.fingerprint { CalcState::Fresh } else { CalcState::EditedOutside };
            let StoredRegion { id, source, unit, precision, .. } = stored;
            *calc = Some(Box::new(CalcRegion { id, source, unit, precision, state }));
            p.touch();
        }
    }
    for (path, mut offs) in markers {
        let Ok(p) = doc.para_mut(StoryRef::Body, &path) else { continue };
        offs.sort_unstable();
        offs.dedup();
        for off in offs.into_iter().rev() {
            if matches!(p.object_at(off), Some(InlineObject::ControlStart { .. } | InlineObject::ControlEnd)) {
                let _ = p.delete(off, off + OBJ.len_utf8());
            }
        }
    }
    crate::recalc(doc, &|_| None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_matches_the_reference_values() {
        assert_eq!(fnv1a64(&[b""]), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(&[b"a"]), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(&[b"foo", b"bar"]), fnv1a64(&[b"foobar"]));
    }

    #[test]
    fn tags_name_ids() {
        assert_eq!(tagged_id("wordcraft-calc:42"), Some(Some(42)));
        assert_eq!(tagged_id("wordcraft-calc:x"), Some(None));
        assert_eq!(tagged_id("other"), None);
    }

    #[test]
    fn a_plain_document_is_left_alone() {
        let mut doc = Document::from_text("Plain\nText");
        let before = doc.clone();
        import(&mut doc, None);
        assert_eq!(doc, before);
        let (copy, store) = export(&doc);
        assert_eq!(copy, doc);
        assert!(store.is_empty());
    }
}
