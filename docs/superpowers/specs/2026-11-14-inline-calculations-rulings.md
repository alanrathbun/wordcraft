# Inline calculations (Stage 1) — decisions made during the build

These are the decisions taken while the plan was being built and reviewed, in the order they
were made. Each one says what it costs if it turns out to be wrong.

| # | Decision | Cost if wrong |
|---|---|---|
| R1 | Duplicate-name messages give a real page number from the session's layout (Print Layout only), else "elsewhere in the document". | Messages lack page numbers. |
| R2 | UI tests use the existing headless harness; behaviour that can't be driven headlessly is tested through its handler. | Weaker UI coverage. |
| R3 | Angle multiplies like a dimensionless quantity: every compatibility check ignores the angle exponent. | J vs N·m·rad mismatches. |
| — | `deg` = `°`; `g` = gram; exact unit names beat prefix splits (`min`, `mil`, `Pa`, `cd`); `µ`/`μ`/`u` = micro; `°C`/`°F` only as a lone unit. | Unusual unit spellings refused. |
| R4 | `J/mol K` and similar (more factors after `/` without parentheses) are refused as ambiguous; write `J/(mol·K)`. | Users must add parentheses. |
| R5 | Fallback unit text keeps `rad` (e.g. `rad/s`). | Cosmetic. |
| R6 | A non-zero value never displays as `0`: it switches to exponential form. | More exponential output. |
| — | Parser: `name(` with no space is a call; `= ? unit` text kept verbatim; token cap 10,000. | Odd spacing refused. |
| R7 | Two values in a row (`1 500`, `x 2`) are refused; write `1500` or `x*2`. | Users can't use space thousands separators. |
| R8 | Superscript digits are powers (`x²` = `x^2`); multi-digit superscript runs refused. | None expected. |
| R9 | Nesting ≤ 64, tree height ≤ 1024 (sums of ~1000 terms work). | Very long expressions refused. |
| R10 | En dash accepted as minus; `1/2 x` = `(1/2)·x` (Mathcad convention, shown as a fraction). | Readers expecting `1/(2x)` must look at the rendering. |
| R11 | The displayed equation is drawn from the parsed formula, never the typed text, so what you see is what was computed. | Display can differ slightly from what was typed. |
| R12 | Absolute temperatures are held in kelvin; `2*°F` = `2 °F`; offset literals inside arithmetic are errors. Plan test using `L` (litre) replaced. | Scaling an absolute temperature uses kelvin. |
| R13 | Adding two absolute temperatures (`100 °C + 5 °C`, `T + 5 °C`) is an error; an offset literal may only pair with a K/°R/Δ°C/Δ°F literal; an offset literal is never subtracted from an interval. | A few legitimate forms need rewriting. |
| — | Using a function name as a value, or calling a variable, is an error. Names that are units (incl. prefixed), built-in functions, `π`/`pi`/`e`, or have empty subscripts are refused. | Common short names (`L`, `h`, `g`, `s`, `m`, `A`…) need subscripts. |
| R14 | WordCraft's native format is its JSON save. | None. |
| R15 | Name lists include names whose calculation failed (they still block duplicates). A manual-mode `≈` may be saved in quoted text. | Picker shows a name that quotes as `?`. |
| R16 | The Calculated Value picker lists variables only; id lookups use the same numbering as recalculation. | None expected. |
| R17 | Derived results inside Restrict Editing regions still update. | A protected document's results change. |
| R18 | Edited-in-Word detection compares a canonical form of the equation. Every document is recalculated on open (manual mode doesn't persist through reopen). | Manual mode resets on reopen. |
| R19 | WordCraft's hidden stores get Word's standard itemProps parts. | — |
| R20 | Results trim trailing zeros at the chosen precision ("1.5 kip" at 2 places). | No fixed-width decimals. |
| R21 | The calculation box borrows the math font to show `≔`. | — |
| R22 | Final fix wave covered: edited-in-Word regions taint dependents; calculations pasted outside the body become plain with a "not calculated here" mark; unique ids in manual mode; one work budget per pass and no pass on unrelated typing; temperature differences can't be shown in °C/°F; quoted values use superscript exponents; numbered calculations; ≥2 significant digits on small values; page lookup cached and Print-Layout-only; Ctrl+Alt+= shortcut, F2/Enter to edit. Deferred: tracked changes on source edits, natural-unit table (°R, Pa), SI-base error text, display toggle. | Deferred items stay as listed. |
| Parked | A K value shown in °C (`20 K = ? °C` → −253.15 °C) is allowed, because K can be absolute or a difference. | A K difference shown in °C misreads. |
| Parked | Marking a Word-edited region drops Word's original equation XML; on save it's rebuilt from WordCraft's tree. | Word-only formatting in that equation is lost. |

Still open for the user: pinning `craft-units` to a pushed git revision (needs approval to push),
and a manual check in Microsoft Word that a saved `.docx` keeps its calculation data and shows
equations, ≔, ⚠ and equation numbers correctly.
