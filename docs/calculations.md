# Calculations

WordCraft can calculate inside a document the way Mathcad does: you write an equation with
units, and the result is worked out and shown beside it. Change an input and every result that
depends on it, anywhere in the document, updates. Insert one from Insert › Calculation
(Ctrl+Alt+=, ⌘⌥= on a Mac). To change one, double-click it, press F2 with the caret beside it, or
select it and press Enter.

Calculations work in the main text and its tables. A calculation pasted or moved anywhere else (a
text box, header, footer, footnote or comment) stops being one: it becomes a plain equation
showing its calculation with no result, marked ⚠ "not calculated here". A quoted value there shows
`?`.

## Notation

| You write | It means |
| --- | --- |
| `P := 1500 lbf` | defines `P` (`≔` is accepted as well) |
| `P/A_b =` | works out the expression and shows the result after the `=` |
| `σ := P/A_b =` | both: defines `σ` and shows it |
| `P/A_b = ? ksi` or `P/A_b = ksi` | shows the result in the unit you pick |
| `f(x, y) := x^2 + y` | defines a function of your own |
| `2 P` | implicit multiplication |
| `x²` | the same as `x^2` |

Names are global. A name is defined once, anywhere in the document, and can be used above or
below its definition. Defining a name twice is an error ("already defined on page N — choose a
unique name"), and so is a circular definition (`a := b`, `b := a`).

A name can't be a unit, a built-in function or a constant. That rules out `m`, `A`, `L`, `h`, `g`,
`s`, `N`, `W`, `J`, `kN`, `min`, `sin`, `π`, `e`, `g_0`, `c_0`, `R_u` and the rest. Use a subscript
instead: `A_b`, `L_1`. A subscript can't be empty (`a__b` and `x_` are refused).

## Constants

| Name | Value |
|---|---|
| `π` or `pi` | 3.141592653589793… |
| `e` | 2.718281828459045… |
| `g_0` | standard gravity, 9.80665 m/s² (`g` alone is the gram) |
| `c_0` | speed of light, 299 792 458 m/s |
| `R_u` | molar gas constant, 8.314462618 J/(mol·K) |

The constants carry their units, so `W_t := 10 kg·g_0 = ? N` shows 98.067 N and `1 lbm·g_0 = ? lbf`
shows 1 lbf.

## Symbols

In the calculation box, **Insert Symbol** lists Greek letters, operators (`≔ · × ± √ ² ³`), the
temperature units and the constants; picking one puts it at the cursor. You can also type a
backslash name followed by a space, as in Word: `\sigma` → σ, `\theta` → θ, `\Delta` → Δ, `\mu` → μ,
`\omega` → ω, `\sqrt` → √, `\cdot` → ·, `\times` → ×, `\pm` → ±, `\deg` → °. Any Greek letter's
name works, with a capital for the capital letter.

## Units

SI units with prefixes and US customary units both work: `in`, `ft`, `lbf`, `kip`, `psi`, `ksi`,
`lbm`, `slug`, `BTU`, `hp`, `gal` and so on. Results are checked for dimensions, so adding a length
to a force is an error rather than a number.

- `°C` and `°F` can only be the unit of a whole value. Temperatures on an offset scale can't be
  added together; use `Δ°F` for a difference.
- A result is shown in `°C` or `°F` only when it is an absolute temperature: a `°C`/`°F` value, a
  `K`/`°R` value, or one of those plus or minus a difference (`T_1 + 5 Δ°C`). A difference
  (`T_2 - T_1`, `20 Δ°C`) or anything else made from temperatures (`2 T`, `(T_1 + T_2)/2`) shown
  in `°C` is an error: "°C shows an absolute temperature; use Δ°C for a difference". Show it in
  `Δ°C`, `Δ°F` or `K` instead. For an average temperature write `T_1 + (T_2 - T_1)/2`.
- `J/mol K` is refused as ambiguous. Write `J/(mol·K)`.
- Thousands separators written as spaces (`1 500`) are refused, because a space is multiplication.
  Write `1500`.

## How results are shown

- Three decimal places, trailing zeros trimmed.
- Exponential form (`6×10^6`) when the value is 1 000 000 or more, or too small to show at least
  two significant digits at the chosen precision: at three places that is below 0.01, so 0.0005
  shows as `5×10^-4`, not `0.001`. (The threshold is `10^(1 − places)`, never above 1 and never
  below 0.0001.)
- The default unit system is US customary. Choose SI in References › Calculation Options.
- `= ? unit` overrides the unit for one result.
- To number a calculation, right-click it and choose Add Equation Number (Remove Equation Number
  takes it off). The number stays as the result changes and is saved to Word.
- A calculation on an empty line is a display equation, on its own line. Right-click it to choose
  Align Left, Align Center or Align Right.
- To write notes after a calculation on the same line, right-click it and choose Change to Inline,
  then type after it (for example a Tab and "bearing stress, see ref. 3"). The line follows the
  paragraph's alignment and tab stops. A calculation inserted into a line that already has text is
  inline from the start. An inline calculation can't be numbered; Change to Display puts it back on
  its own line.

## Quoted values

Insert › Calculated Value puts a variable's value into a sentence ("each bolt carries 45.271 ksi").
It shows only the body of the result (no name, no `=`) and follows the variable, with an exponent
written in superscript (`2.9×10⁷ psi`). It shows `?` while the value isn't available (an error, or
a name that's gone). A unit the variable can't be shown in is refused when the value is inserted.

## Recalculation

Results update automatically. In manual mode (References › Calculation Options) they are marked
`≈` when an input changes, until you choose References › Recalculate. Typing ordinary text marks
nothing out of date.

A document whose calculations together need too much work (deeply recursive functions, say) is
calculated up to a fixed limit; the calculations past it show the error "too complex to
calculate" rather than a result.

## Word

Saving as `.docx` writes each calculation as an ordinary Word equation, so Word shows the results,
plus hidden calculation source. Opening that file in WordCraft makes the calculations live again.
Quoted values are written the same way.

If an equation is edited in Word, WordCraft notices when you open the file: the status bar says how
many were edited, and each one keeps Word's version with an amber ⚠ after it. Until you resolve it,
it defines nothing: every calculation that uses its name shows the error "depends on a calculation
edited in Word — resolve it" instead of a result, and quoted values of them show `?`. Hover over it
(or put the caret beside it) for two choices: "Restore from calculation" brings back what the
calculation says, and "Keep Word's equation" keeps what was typed in Word and stops it being a
calculation. The ⚠ is WordCraft's: it isn't saved into the Word equation. Calculations inside
tracked deletions are kept when you save and reopen.

## Example

A bolt-stress check, as exercised by the engine's `flange_bolt_sample` test:

```
n_b := 8
d_b := 0.75 in
F_i := 20 kip
σ_allow := 45 ksi
A_t(d) := π/4·d^2
σ_b := F_i/A_t(d_b) = ? ksi        → 45.271 ksi
F_total := n_b·F_i = ? kip         → 160 kip
Each bolt carries [σ_b in ksi] of stress.
```

Changing `F_i` to `18 kip` updates both results and the sentence (40.744 ksi, 144 kip).

## Building

The calculation engine uses the `craft-units` crate from craft-libs. It is pinned in the workspace
`Cargo.toml` to a commit on `alanrathbun/craft-libs` (branch `feature/craft-units`), so a plain
`cargo build` fetches it; no separate checkout is needed. Move the pin to `storytold/craft-libs`
if the crate is accepted there.
