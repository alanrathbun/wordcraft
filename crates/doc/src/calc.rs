//! Data for inline calculations: the payload an equation carries when it is a calculation
//! region, the document's calculation settings, and the `WCCALC` field code of a quoted value.
//!
//! Plain data only. Evaluating a region is `wordcraft-calc`'s job; this crate depends on neither
//! it nor the units crate, so [`UnitSystem`] is a local copy that the evaluator maps across.

use serde::{Deserialize, Serialize};

/// A calculation region: what an [`crate::para::InlineObject::Equation`] carries when it is a
/// calculation rather than typeset math. The equation's `linear` and `math` are the displayed
/// result (rewritten by the recalculation pass); `source` is what the user typed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalcRegion {
    /// Stable within the document; names the region in diagnostics and results.
    pub id: u64,
    /// The calculation as written (`σ_max := P / A`).
    pub source: String,
    /// Unit the result is shown in (`ksi`); `None` = whatever the value comes out in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Significant decimals for this region; `None` = the document's [`CalcSettings::precision`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
    /// Shown with an automatic equation number, like a numbered plain equation (`#`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub numbered: bool,
    #[serde(default)]
    pub state: CalcState,
}

impl CalcRegion {
    /// Bytes this region owns beyond its own size: its source, unit and error message, all of
    /// which come from files. What the paste and clone cost guards count.
    pub fn heap_bytes(&self) -> usize {
        let error = if let CalcState::Error(message) = &self.state { message.len() } else { 0 };
        std::mem::size_of::<CalcRegion>()
            .saturating_add(self.source.len())
            .saturating_add(self.unit.as_ref().map_or(0, String::len))
            .saturating_add(error)
    }
}

/// Where a [`CalcRegion`] stands with respect to its displayed result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum CalcState {
    /// The displayed result matches `source`.
    #[default]
    Fresh,
    /// An input changed (manual calculation mode): the displayed result is out of date.
    Stale,
    /// Evaluating `source` failed; the message says why.
    Error(String),
    /// The equation's text was edited by something other than the calculator (another editor,
    /// a round trip through a format that doesn't carry the payload): the result can't be trusted.
    EditedOutside,
}

/// The unit system calculations show results in. Local to this crate; `wordcraft-calc` maps it
/// to the units crate's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitSystem {
    #[default]
    UsCustomary,
    Si,
}

/// What `g` means in a document's calculations. Mathcad's convention is the default: `g` is
/// standard gravity and the gram is `gm`. The SI convention makes `g` the gram (gravity is then
/// `g_0`). It belongs to the document, so a report calculates the same on every computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GMeans {
    #[default]
    Gravity,
    Gram,
}

/// Document-wide calculation settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalcSettings {
    pub system: UnitSystem,
    /// Decimals shown when a region sets none. Consumers clamp it to what they can display.
    pub precision: u8,
    /// Recalculate only on request (regions go [`CalcState::Stale`] instead).
    pub manual: bool,
    /// What `g` stands for (see [`GMeans`]).
    pub g: GMeans,
}

impl Default for CalcSettings {
    fn default() -> Self {
        CalcSettings { system: UnitSystem::UsCustomary, precision: 3, manual: false, g: GMeans::Gravity }
    }
}

/// The tag prefix of the content control a calculation is saved in (`.docx`): the region's id
/// follows (`wordcraft-calc:7`). WordCraft's own bookkeeping, never shown as a form field.
pub const CALC_TAG_PREFIX: &str = "wordcraft-calc:";

/// Most regions a [`CalcStore`] read from a file may hold; a larger one is ignored.
pub const MAX_STORED_REGIONS: usize = 100_000;

/// Largest serialized [`CalcStore`] read from a file, bytes; a larger one is ignored.
pub const MAX_STORE_BYTES: usize = 16 * 1024 * 1024;

/// What a saved file (`.docx`) keeps of its calculations beside the ordinary equations Word
/// shows: the document's settings and each region's source, keyed by the id in its content
/// control's tag ([`CALC_TAG_PREFIX`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalcStore {
    pub settings: CalcSettings,
    pub regions: Vec<StoredRegion>,
}

impl CalcStore {
    /// Nothing worth saving: no regions and the default settings.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty() && self.settings == CalcSettings::default()
    }
}

/// One region of a [`CalcStore`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredRegion {
    pub id: u64,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub numbered: bool,
    /// Hash of the equation as saved: when the equation read back hashes differently, it was
    /// edited outside WordCraft.
    pub fingerprint: u64,
}

/// The field type of a quoted value: a locked [`crate::para::InlineObject::Field`] whose code is
/// built by [`calc_field_instr`] and whose result is the value as it was quoted.
pub const CALC_FIELD: &str = "WCCALC";

/// Longest field code [`parse_calc_field`] accepts. Field codes come from files; a calculation
/// name and unit are short.
pub const MAX_INSTR_BYTES: usize = 1024;

/// Highest precision switch [`parse_calc_field`] accepts.
const MAX_PRECISION: u8 = 15;

/// The field code quoting the value of `var`: `WCCALC σ \u ksi \p 2`, each switch left out when
/// `None`. A unit with spaces is double-quoted (`\u "kN m"`). `var` is an identifier (no
/// whitespace); a `"` in a unit, which no unit has, is dropped, and an empty unit counts as none.
pub fn calc_field_instr(var: &str, unit: Option<&str>, precision: Option<u8>) -> String {
    let mut instr = format!("{CALC_FIELD} {var}");
    if let Some(unit) = unit {
        let unit: String = unit.chars().filter(|&c| c != '"').collect();
        if !unit.trim().is_empty() {
            if unit.chars().any(char::is_whitespace) {
                instr.push_str(&format!(" \\u \"{unit}\""));
            } else {
                instr.push_str(&format!(" \\u {unit}"));
            }
        }
    }
    if let Some(precision) = precision {
        instr.push_str(&format!(" \\p {precision}"));
    }
    instr
}

/// Splits a field code into words; a word starting with `"` runs to the closing `"` and may hold
/// spaces. `None` for an unterminated quote or a closing quote followed by more word.
fn words(instr: &str) -> Option<Vec<(String, bool)>> {
    let mut out = Vec::new();
    let mut chars = instr.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else { return Some(out) };
        let mut word = String::new();
        if first == '"' {
            chars.next();
            loop {
                match chars.next()? {
                    '"' => break,
                    c => word.push(c),
                }
            }
            if chars.peek().is_some_and(|c| !c.is_whitespace()) {
                return None;
            }
            out.push((word, true));
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                word.push(c);
            }
            out.push((word, false));
        }
    }
}

/// Reads a [`calc_field_instr`] code back: the variable name, unit and precision. `None` if it
/// isn't a `WCCALC` code, is over [`MAX_INSTR_BYTES`], has no name, repeats or doesn't know a
/// switch, or gives a precision over 15.
pub fn parse_calc_field(instr: &str) -> Option<(String, Option<String>, Option<u8>)> {
    if instr.len() > MAX_INSTR_BYTES {
        return None;
    }
    let mut words = words(instr)?.into_iter();
    let (kind, quoted) = words.next()?;
    if quoted || !kind.eq_ignore_ascii_case(CALC_FIELD) {
        return None;
    }
    let (name, quoted) = words.next()?;
    if quoted || name.is_empty() || name.starts_with('\\') {
        return None;
    }
    let (mut unit, mut precision) = (None, None);
    while let Some((switch, quoted)) = words.next() {
        if quoted {
            return None;
        }
        let (value, _) = words.next()?;
        match switch.as_str() {
            "\\u" if unit.is_none() && !value.trim().is_empty() && !value.starts_with('\\') => unit = Some(value),
            "\\p" if precision.is_none() => {
                // `u8::from_str` takes a leading `+`; a field code doesn't.
                if !value.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                precision = Some(value.parse::<u8>().ok().filter(|&p| p <= MAX_PRECISION)?);
            }
            _ => return None,
        }
    }
    Some((name, unit, precision))
}

#[cfg(test)]
mod tests;
