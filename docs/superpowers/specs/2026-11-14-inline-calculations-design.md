# Inline engineering calculations in WordCraft — Stage 1 design

Date: 2026-11-14
Branch: `feature/inline-calculations` (fork `alanrathbun/wordcraft`)
Related fork: `alanrathbun/craft-libs` (new `craft-units` crate)

## Why

The user's team writes engineering calculation reports in Microsoft Word and does the maths in
Mathcad. The two don't connect:

- Mathcad pages pasted into Word become pictures: the equations can't be edited, searched,
  restyled or cross-referenced.
- Any change in Mathcad means re-pasting every affected page.
- The alternative, appending a Mathcad PDF to the Word PDF, loses live references between the
  report text and the calculations.

The goal is one document where the calculations are part of the text: real equations that are
searchable and formatted like the rest of the report, where changing an input updates every
result and every reference to it.

## The workflow it serves

1. The engineer writes the report in WordCraft. Calculations are live and recalculate as they edit.
2. For review or release they save it as a Word `.docx`. Reviewers open it in Microsoft Word: the
   calculations are ordinary Word equations showing the last results. Word does not recalculate
   them, and doesn't need to.
3. Review comments come back on that `.docx`. The engineer opens it in WordCraft, where the
   calculations become live again, makes the changes, and saves a new revision for the next review
   or for final release.

Saving to `.docx` is the hand-off point. The document is expected to be ready for review or
release when it happens.

## Goals (Stage 1)

- **Live calculations in the document flow**, in Mathcad style:
  - definitions with units: `P := 1500 lbf`, `A := 2.5 in^2`
  - evaluations: `σ := P/A` then `σ =` shows `σ = 600 psi`
  - user-defined functions: `f(x, y) := x^2 + y`
- **Real unit arithmetic and checking.** Dimensions are tracked through every operation:
  `lbf/in^2` is a pressure; `1 in + 1 lbf` is an error with a clear message. Mixed systems
  (`in` and `mm`) just work.
- **Chosen result units.** A result is shown in the document's default unit system unless the
  author names a unit (`σ = ? ksi` shows `σ = 0.6 ksi`). Display precision is a document setting
  with a per-result override.
- **Calculation placement:** a calculation can be its own display equation (its own paragraph,
  optionally numbered like any WordCraft equation) or sit inline inside a sentence.
- **Linked values in text.** An inline "calculated value" can show a variable's current value
  anywhere later in the document ("the stress, 600 psi, is below the allowable"). It updates when
  the calculation does.
- **Automatic recalculation** as the author edits, with a manual mode (recalculate on command) for
  very large documents.
- **Word round trip.** `.docx` files carry each calculation twice: as a normal Word equation
  (OMML) showing its current result, which is what Word displays, and as hidden calculation
  source, which WordCraft reads to make it live again.
- **Edits made in Word are detected, never silently overwritten** (see "Edited in Word").

## Non-goals (Stage 1; most are later stages)

- Symbolic algebra (simplify, differentiate, integrate as formulas). Not planned at any stage.
- if/then, programs, matrices, ranges and data tables (Stage 2).
- Solve blocks (Stage 3).
- Plots (Stage 4).
- Excel connections (Stage 5).
- Importing Mathcad `.mcdx` worksheets (optional Stage 6).
- Complex numbers.
- Calculations in headers, footers, footnotes, comments and text boxes. Stage 1 covers the main
  document body, including table cells.
- Stripping the hidden calculation source for a final "frozen" release copy. This may come later.

## Anti-goals (would make it a failure even if it "works")

- **A wrong number that looks right.** That includes a unit mistake that isn't caught, a result
  shown in the wrong unit, or a stale result shown as current.
- **A published `.docx` that looks broken in Word.** Missing equations, raw markup, or layout that
  differs from WordCraft's would all count.
- **Losing a reviewer's work.** That means overwriting an equation someone edited in Word without
  telling the author.
- **Crashing** on any document, however hostile. The repo's no-panic rules apply.

## Constraints

- **The repo rules in `AGENTS.md` apply in full:** Rust only, no `unsafe`, no panics in non-test
  code, every crash fix with a test, clean room, and dependency licences checked.
- **No embedded browser, Python or JavaScript.** EngineeringPaper.xyz is behavioural inspiration
  only. Its code is not copied.
- **New dependencies are storytold sibling crates.** They are MIT OR Apache-2.0 and pinned by git
  revision, the way `Cargo.toml` already pins `craft-libs`. They point at the user's forks while
  this is fork-only work.
- **`craft-units` must stand on its own** so it could be offered to `storytold/craft-libs` later:
  no WordCraft or GridCraft types in its API, with its own tests and docs.

## Approaches considered

**A. Compile calculations into a hidden GridCraft workbook.** Variables become defined names and
equations become Excel formulas, with units tracked on the side. This reuses the most code, but
units can only be checked loosely (not inside functions or arrays) and errors surface Excel-style
(`#VALUE!`). Rejected: unit checking is the core of an engineering calc.

**B. Hybrid: WordCraft calculation engine with native units, reusing GridCraft where it is
cheaper. Chosen.** A small engine in WordCraft parses Mathcad-style notation and carries units
through every step. Units live in a separate shared crate, `craft-units`, in `craft-libs`. Function
maths, and later Excel files and charts, come from GridCraft crates where reusing them saves work.

**C. Add units to GridCraft's own engine and call it from WordCraft.** This would give one engine
for both apps, but it changes GridCraft's core value type, which all ~12,000 lines of its functions
use. It is the largest job and the hardest to keep merged with upstream. GridCraft targets Excel
parity, and `.xlsx` can't store units, so it is also unlikely to be accepted upstream. Rejected.

The user's standing instruction: **wherever calling into GridCraft is more efficient than writing
new code, do that.**

## Design decisions

### Pieces

| Piece | Where | What |
|---|---|---|
| `craft-units` | new crate in the `craft-libs` fork | Dimensions, unit catalog, parsing unit expressions, conversion, formatting a quantity in a chosen unit |
| `wordcraft-calc` | new crate in the WordCraft workspace | Calculation language (parse), evaluator over quantities, dependency tracking and recalculation, function adapters |
| Document model | `crates/doc` | A calculation region type, the inline calculated-value object, and calc settings on the document |
| Layout/render | `crates/layout`, `crates/render` | Draws a calculation as an equation through the existing math layout (`crates/layout/src/math*`) |
| UI and commands | `crates/engine`, `crates/ui-egui` | Insert › Calculation, editing, recalculate, settings, error display |
| DOCX | `crates/docx` | Write and read the equation plus the hidden calculation source |

### `craft-units`

- **Dimensions are a vector of exponents over the SI base quantities:** length, mass, time,
  current, temperature, amount and luminosity. Angle is tracked separately as an optional
  dimension so that a user can ask for degrees, but it multiplies like a dimensionless quantity
  (the Mathcad behaviour).
- **Quantities are stored internally in SI base units as f64.** Conversion happens only on input
  and display.
- **Catalog:** SI units with prefixes, plus the US customary and engineering units a mechanical
  calc report uses. That covers in, ft, yd, mi, mil, lbm, slug, lbf, kip, psi, ksi, psf, BTU,
  hp, °R, ft·lbf, in·lbf, gal, and the common derived SI units (N, Pa, MPa, GPa, J, W, Hz).
  GridCraft's Excel `CONVERT` table (`gridcraft/crates/functions/src/engineering/units.rs`) can
  seed the conversion factors; it is MIT/Apache in the same organisation.
- **Temperature:** K and °R are absolute. °C and °F are allowed only as a whole value's input or
  display unit (affine conversion). Inside arithmetic they act as temperature differences, with
  `Δ°C`/`Δ°F` spelled explicitly. An expression that would make an affine conversion ambiguous is
  an error, not a guess.
- **Unit expressions** parse to a unit plus a scale: `lbf/in^2`, `N·m`, `kg*m/s^2`, `ft^3/min`.
- **Formatting** picks the display unit as follows: the explicit unit if one was given; otherwise
  the document's unit system (SI or US customary) chooses a "natural" unit for the dimension (for
  pressure, `psi` in US customary and `Pa`/`MPa` in SI). Builder's call on the exact
  natural-unit table.
- Pure Rust, no I/O, no unsafe, fuzz-friendly parser, bounded recursion.

### Calculation language (in `wordcraft-calc`)

- **Mathcad-style notation, typed as linear text,** shown built up through the existing math
  layout:
  - `name := expr` defines a variable
  - `f(a, b) := expr` defines a function
  - `expr =` evaluates and shows the result
  - `name := expr =` defines and shows the result at once
  - a unit placeholder after `=` sets the display unit
- **Numbers and units:**
  - a number followed by a unit is a quantity (`5 in`, `5in`, `5·in`)
  - variables and units share one namespace, and a user variable shadows a unit from its
    definition onwards (Mathcad behaviour); the editor should mark the shadowing
  - subscripts in names are allowed and display as subscripts (`σ_allow`, typed `σ_allow`)
  - Greek letters can be typed with the existing math autocorrect (`\sigma`)
- **Operators:** `+ - * / ^`, unary minus, parentheses, `·` and `×` for multiplication, `√` and
  `sqrt()`, and absolute value `|x|`.
- **Built-in functions (Stage 1):** sqrt, abs, exp, ln, log (base 10), log(x, b), sin, cos, tan,
  asin, acos, atan, atan2, sinh, cosh, tanh, min, max, round, floor, ceil, mod, and π and e as
  constants. Each function has a unit rule. For example, trig functions take dimensionless or
  angle input; `min`/`max` need matching dimensions; `sqrt` halves exponents and errors on odd
  ones.
  - Numerical implementations come from `gridcraft-functions` where they exist and match. An
    adapter strips the quantity to an SI magnitude, applies the unit rule, calls the function,
    and re-attaches the result unit. Where calling GridCraft costs more than `f64` std maths (for
    example plain `sin`), the builder uses std.
- **Evaluation order is document order** (top to bottom, then left to right within a line; table
  cells row by row). A name must be defined before it is used, and a later definition replaces an
  earlier one from that point on. This is Mathcad's model and what the user's team already
  expects.
- **Errors appear at the region,** in the error style with a plain-language message ("can't add
  lbf to in", "σ_allow isn't defined above this point"). Regions that depend on a failed region
  show that they depend on an error rather than a value. Errors never block saving.

### Recalculation

- A calculation's identity is a stable ID stored on the region, not its text or position.
- On any edit, the engine re-parses changed regions, rebuilds the define/use graph in document
  order, and re-evaluates only regions downstream of the change. Layout updates their displayed
  results.
- **Target:** a document with 2,000 calculation regions recalculates and repaints in under 100 ms
  after a single edit on the user's machine (32 cores, ~45 GB RAM). The builder adds a benchmark.
- **Modes:** automatic (default) or manual. A document setting chooses the mode, and a
  Recalculate command works in both. In manual mode, stale results are visibly marked stale. A
  stale result must never look current.

### Document model

- **A calculation region is a math object that also carries:**
  - the region ID
  - the calculation source (the linear text the user typed)
  - the last evaluated result, as value plus display unit and precision, or an error
  - the display-unit override and precision override

  The rendered `Math` tree is derived from these on each recalculation. The builder chooses
  whether this is a new `InlineObject` variant or an extension of the existing equation object,
  but existing plain equations must remain plain.
- **An inline calculated-value object** names a variable and a display unit and precision, and
  shows the value as of that point in the document.
- **Document calc settings:** unit system (US customary by default, since the user's examples
  are in lbf/in/psi), default precision, and recalculation mode. They are saved with the
  document.
- Equation numbering and cross-references use WordCraft's existing equation-number and
  bookmark/REF machinery. Nothing new is needed for "see Eq. (4)".

### `.docx` round trip

- **What Word sees:**
  - each calculation region is written as an ordinary `m:oMath` (display or inline) showing the
    definition and current result exactly as WordCraft lays it out, wrapped in a content control
    (`w:sdt`) tagged with the region ID
  - inline calculated values are written as plain formatted text in a tagged content control
  - Word shows, searches, prints and comments on all of it normally
- **What WordCraft reads back:** a custom XML part (`customxml/wordcraftcalc.xml`, alongside the
  existing `customxml/wordcraftfit.xml`) holds the document calc settings and, for each region
  ID: the source text, the overrides, and a fingerprint of the OMML that was written.
- WordCraft's own native format stores the same data directly in the model.

### Edited in Word

- **On opening a `.docx`,** for each tagged region, WordCraft compares the current OMML
  fingerprint to the stored one.
  - **Unchanged:** the region becomes live again.
  - **Changed (someone edited the equation in Word):** the region is flagged, keeping Word's
    version visible. The author chooses per region to restore it from the calculation source,
    or keep Word's equation as a plain (non-calculating) equation.
  - **Deleted in Word:** its source is dropped, and anything that depended on it shows an error
    so the author notices.
  - **Untagged equations** (typed in Word) stay plain equations.
- Review comments and tracked changes made in Word survive, because they live outside the
  regions or attach to them as normal ranges.

### UI (Stage 1 minimum)

- **Insert › Calculation** (with a keyboard shortcut) opens an empty region. The user types
  linear text, and it renders built up on leaving the region. Clicking back in shows the source
  text for editing. Typing `:` then `=` produces `:=`.
- **Insert › Calculated Value** picks a defined variable from a list valid at that point.
- **Calculation settings:** unit system, precision, recalculation mode.
- **A Recalculate command.**
- Errors and stale states are visible on screen and in print preview, but **not** written into
  the `.docx` as error styling unless the region really is in error at save.
- Every new command goes through the existing command catalog (`crates/engine/src/catalog.rs`),
  so it is also reachable by keyboard and through WordCraft's MCP/agent control, as the repo
  expects.

## Success criteria

- The user can rebuild a typical Mathcad sheet as part of a report. For example, a flange or bolt
  stress calculation using only Stage 1 features: inputs with units, intermediate results, a
  user function, results in chosen units, and a sentence that quotes a result.
- **Changing one input updates every dependent result and quoted value.**
- **The saved `.docx` opens in Microsoft Word** with every calculation showing as a proper
  editable equation and nothing broken. The user checks this by hand in Word.
- **Reopening that `.docx` in WordCraft makes the calculations live again.** An equation edited
  in Word is flagged, not overwritten.
- **Tests:**
  - unit tests in `craft-units` (conversion factors, dimension algebra, parsing, affine
    temperature)
  - evaluator tests in `wordcraft-calc`
  - `.docx` round-trip tests in `crates/docx/tests`
  - a fuzz or property test for both parsers
  - the recalculation benchmark

## Left to the builder

- Exact crate boundaries inside WordCraft, and the internal API between `wordcraft-calc` and the
  document model.
- The new `InlineObject` variant vs. extending the equation object (within the constraint above).
- The natural-unit table per unit system, and the default precision.
- Which `gridcraft-functions` routines to call vs. std maths.
- The fingerprint algorithm.
- The keyboard shortcut, and the menu placement within the existing ribbon catalog.
- How stale and error states are drawn, within the "never looks current" rule.

## Later stages (separate designs)

2. if/then and programs, matrices, ranges, data tables
3. Solve blocks (numerical)
4. Plots via `gridcraft-chart`
5. Excel connections via `gridcraft-xlsx` / `craft-xlsx` and `gridcraft-calc`
6. Optional: Mathcad `.mcdx` import
