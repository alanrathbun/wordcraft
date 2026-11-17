# Calculation units and functions catch-up — design

Approved by the user on 2026-11-16. Step 1 of the road to the Mathcad importer (units and functions,
solver, Excel, importer, programs, calculus, plots).

## Why

The user is moving from Mathcad and Word to WordCraft. A survey of the 1,024 Mathcad 14/15 worksheets
in their `MathCad` folder found that, with WordCraft's current features, most files fail on units or
functions WordCraft simply lacks: `hr` (235 files), `lb` (203), `sec` (194), `atm` (132), `gpm` (63),
`Ceil(x, y)` (20), `interp` with `cspline` (19) and so on. This step closes those gaps so that typing
feels like Mathcad and, later, imported worksheets calculate.

## Goals

- Every unit and built-in function the survey found in more than a couple of files works, with
  Mathcad's spelling and Mathcad's numbers.
- Results use the spellings the user reads every day.

## Non-goals

- Complex numbers (`i`, `Re`, `Im`, `arg`): a round of their own, after the importer.
- The importer itself, and its translations (`lb` → `lbm`, `R` → `°R`, `sec(x)` → `secant(x)`).
- Multivariate `regress`, `loess`, `polyfitc`, `fft`, statistics beyond `slope` and `intercept`.

## Anti-goals

- A function or unit that gives a different number from Mathcad for the same input.
- Breaking an existing document silently: a name that becomes reserved must say so.

## Approaches

There is one sensible way: add the units to `craft-units` and the functions to `wordcraft-calc`,
following the existing catalog and built-in tables. The alternative of keeping Mathcad spellings
only in a later importer was rejected by the user, who wants to type them.

## Units (in `craft-units`, alanrathbun/craft-libs)

**Spellings the user chose for results.** Wherever WordCraft picks the unit text itself (natural
units, composed base units), it writes `hr`, `sec`, `Btu`, `liter`, `watt` and `joule`; `mol`
stays `mol`. Prefixed units keep the symbol: `kW`, `kJ`, `mL`, `ms` (user's call accepted: a spelled
prefix like `kwatt` reads badly). Units the user typed, as in `= ? BTU/h`, show as typed. This
applies to US and SI documents alike.

**Accepted spellings.** Typing either form works:

| Preferred (shown) | Also accepted |
| --- | --- |
| `hr` | — (the `h` unit is removed, freeing `h` as a name) |
| `sec` | `s` |
| `Btu` | `BTU` |
| `liter` | `L` |
| `watt` | `W` |
| `joule` | `J` |
| `mol` | `mole` |
| `Å` | `Angstrom` |

A document that used `h` for hours gets the ordinary undefined-name error, with the hint "for hours,
use hr".

**New units.** Each factor is Mathcad's. Where a surveyed worksheet saved a result using the unit,
the builder confirms the factor and the dimension against it (for `pcf` in particular: mass or force
per cubic foot is whatever Mathcad's saved results show).

| Unit | Meaning |
| --- | --- |
| `atm` | 101325 Pa |
| `bar` | 10⁵ Pa |
| `torr` | 101325/760 Pa |
| `gpm` | gal/min |
| `cfm` | ft³/min |
| `mph` | mi/hr |
| `poise` | 0.1 Pa·sec |
| `cP` | 0.01 poise |
| `%` | 0.01 (a postfix unit: `5 %`) |
| `kgf` | 9.80665 N |
| `ton` | 2000 lbm |
| `tonne` | 1000 kg |
| `cal` | 4.1868 joule, prefixable (`kcal`) |
| `day` | 86400 sec |
| `yr` | Mathcad's year |
| `Å` | 10⁻¹⁰ m |
| `plf` | lbf/ft |
| `pcf` | per cubic foot, as Mathcad defines it |
| `ksf` | kip/ft² |

Every new name is a unit name, so it can't be a variable name. A document that already defines one
(`day := 7`) gets the existing clash handling for unit names.

`lb` and `R` are not units in WordCraft: `lb` reads as force to many engineers, and `R` is a common
name. The importer will translate Mathcad's `lb` and `R`.

## Functions (in `wordcraft-calc`)

| Function | Gives |
| --- | --- |
| `Ceil(x, y)`, `Floor(x, y)`, `Round(x, y)` | `x` rounded up, down or to the nearest multiple of `y`; `x` and `y` share a dimension, `y > 0`; the result is in `x`'s unit. `Ceil(2.06 in, 0.125 in)` is `2.125 in` |
| `cspline(vx, vy)`, `lspline(vx, vy)`, `pspline(vx, vy)` | the spline coefficient vector for cubic, linear and parabolic ends |
| `regress(vx, vy, k)` | the coefficient vector of a degree-`k` least-squares polynomial |
| `interp(vs, vx, vy, x)` | the value at `x` of a spline or regression from the above |
| `slope(vx, vy)`, `intercept(vx, vy)` | the least-squares line, with units `y/x` and `y` |
| `submatrix(A, ir, jr, ic, jc)` | rows `ir..jr`, columns `ic..jc`, counted from the document's index base |
| `augment(A, B, …)` | side by side; rows must match |
| `stack(A, B, …)` | top to bottom; columns must match |
| `erf(x)` | the error function of a plain number |
| `csc(θ)`, `cot(θ)`, `secant(θ)` | the reciprocal trig functions |
| `atanh(x)` | the inverse hyperbolic tangent |
| `J0(x)`, `J1(x)` | Bessel functions of the first kind |

- The coefficient vectors of `cspline`, `lspline`, `pspline` and `regress` use Mathcad's layout,
  element for element, so a worksheet that indexes into them gets the same numbers. The builder
  confirms the layout from saved results in the surveyed worksheets.
- `interp` works over a vector `x` per element, like `linterp`, including the lenient rule inside
  cases.
- Lowercase `ceil`, `floor` and `round` are unchanged.
- Every new name is reserved like the other built-ins.

## Testing

- Unit tests for each unit and function, with values transcribed from Mathcad's saved results in the
  surveyed worksheets where one exists (the worksheets themselves are not committed).
- Existing tests and samples whose results now read `sec`, `watt`, `joule`, `liter`, `Btu` or `hr`
  are updated to the new text.

## Delivery

- `craft-units` changes go on a new branch of alanrathbun/craft-libs. During development WordCraft
  points at the local checkout; before a pull request, the craft-libs branch is pushed (with the
  user's OK) and WordCraft's pinned `rev` is updated to it.
- WordCraft changes go on `feature/calc-units-and-functions`, branched from
  `feature/inline-calculations`.
- The user guide (`docs/calculations.md`) gets a units table and a section on the new functions. The
  equation editor's symbol list gains `Å` and `%`.

## Left to the builder

- How the coefficient vector carries its kind (cubic, linear, parabolic, polynomial), as long as the
  numbers match Mathcad's layout.
- Error wording, following the existing messages.
