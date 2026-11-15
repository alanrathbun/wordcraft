# Calculations

WordCraft can calculate inside a document the way Mathcad does: you write an equation with
units, and the result is worked out and shown beside it. Change an input and every result that
depends on it, anywhere in the document, updates. Insert one from Insert › Calculation.

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
`s`, `N`, `W`, `J`, `kN`, `min`, `sin`, `π`, `e` and the rest. Use a subscript instead: `A_b`,
`L_1`. A subscript can't be empty (`a__b` and `x_` are refused).

## Units

SI units with prefixes and US customary units both work: `in`, `ft`, `lbf`, `kip`, `psi`, `ksi`,
`lbm`, `slug`, `BTU`, `hp`, `gal` and so on. Results are checked for dimensions, so adding a length
to a force is an error rather than a number.

- `°C` and `°F` can only be the unit of a whole value. Temperatures on an offset scale can't be
  added together; use `Δ°F` for a difference.
- `J/mol K` is refused as ambiguous. Write `J/(mol·K)`.
- Thousands separators written as spaces (`1 500`) are refused, because a space is multiplication.
  Write `1500`.

## How results are shown

- Three decimal places, trailing zeros trimmed.
- Exponential form (`6×10^6`) when the value is 1 000 000 or more, below 0.0001, or would
  otherwise round to 0.
- The default unit system is US customary. Choose SI in References › Calculation Options.
- `= ? unit` overrides the unit for one result.

## Quoted values

Insert › Calculated Value puts a variable's value into a sentence ("each bolt carries 45.271 ksi").
It shows only the body of the result (no name, no `=`) and follows the variable. It shows `?` while
the value isn't available (an error, or a name that's gone).

## Recalculation

Results update automatically. In manual mode (References › Calculation Options) they are marked
`≈` when they're out of date, until you choose References › Recalculate.

## Word

Saving as `.docx` writes each calculation as an ordinary Word equation, so Word shows the results,
plus hidden calculation source. Opening that file in WordCraft makes the calculations live again.
Quoted values are written the same way.

If an equation is edited in Word, WordCraft notices when you open the file and flags it with two
choices: "Restore from calculation" brings back what the calculation says, and "Keep Word's
equation" keeps what was typed in Word and stops it being a calculation. Calculations inside
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

The calculation engine uses the `craft-units` crate from the craft-libs repository. For now it is a
path dependency (`../../../craft-libs/crates/units` from `crates/calc`), so craft-libs must be
checked out beside the wordcraft directory, on the `feature/craft-units` branch. It will move to a
pinned git revision once that branch is published.
