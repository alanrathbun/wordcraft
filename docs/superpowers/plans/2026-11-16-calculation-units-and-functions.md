# Calculation Units and Functions Catch-up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** WordCraft calculations accept Mathcad's units and the built-in functions the user's Mathcad worksheets use, with Mathcad's numbers, and results use the user's spellings (`hr`, `sec`, `watt`, `joule`).

**Architecture:** Units live in `craft-units` (repo `/home/alan/arath/storytold/craft-libs`, crate `crates/units`), which WordCraft pins by git rev; during this work WordCraft points at the local checkout with a `[patch]`. Scalar built-ins go in `crates/calc/src/funcs.rs`; whole-array built-ins (curve fitting, matrices) in `crates/calc/src/array_funcs.rs`.

**Tech Stack:** Rust 2024 workspace; `libm` (already in `Cargo.lock`, 0.2.16) for `erf`, `j0`, `j1`.

**Spec:** `docs/superpowers/specs/2026-11-16-calculation-units-and-functions-design.md`

## Global Constraints

- `cargo` is not on PATH in a fresh shell: `export PATH="$HOME/.cargo/bin:$PATH"` first.
- Commits: no `Co-Authored-By` or other agent attribution lines. Push nothing.
- WordCraft branch: `feature/calc-units-and-functions` (already checked out in `/home/alan/arath/storytold/wordcraft`). craft-libs branch: `feature/calc-units-and-functions`, created from `feature/craft-units` in Task 1.
- US spelling in prose and messages ("meter", "liter").
- Every factor is Mathcad's. Results WordCraft writes itself use `sec`, `watt`, `joule`, `hr`, `Btu`, `liter` (unprefixed only; prefixed units keep the symbol: `ms`, `kW`, `kJ`, `mL`); `mol` stays `mol`; units the user typed show as typed.
- Accepted spellings: `hr` (the `h` unit is removed), `sec`/`s`, `Btu`/`BTU`, `liter`/`L`, `watt`/`W`, `joule`/`J`, `mol`/`mole`, `Å`/`Angstrom`.
- `lb` and `R` are NOT units.
- Every new unit and function name is reserved (refused as a variable name by the existing `check_name`).
- Lowercase `ceil`, `floor`, `round` are unchanged.
- WordCraft's no-extrapolation rule stands: a spline `interp` outside `vx` is an error, as for `spline`.
- Tests transcribe Mathcad values below; never read the user's Mathcad files from a test.

## Review Focus

1. A document that used `h` for hours, or that defines a newly reserved name (`bar`, `day`, `ton`, `cal`): it must say what is wrong (hint "for hours, use hr"; the reserved-name error) — tests in Task 2.
2. New exact unit names must not change existing prefixed spellings: `ms`, `min`, `mi`, `cd`, `mm`, `kPa` keep their meaning — test in Task 1.
3. `Ceil`/`Floor`/`Round` with a negative `x`, a zero or negative step, or mismatched units — tests in Task 3.
4. `interp` handed a vector that is not from `cspline`/`lspline`/`pspline`/`regress`, or one built over different-length data — test in Task 4.
5. SI results whose composed unit contains seconds (`m/sec²`) and SI natural energy and power (`joule`, `watt`) — tests in Task 1 and Task 2.

---

### Task 1: craft-units — new units and result spellings

**Files:**
- Modify: `craft-libs/crates/units/src/catalog.rs` (CATALOG, constants)
- Modify: `craft-libs/crates/units/src/format.rs` (`base_names`, the `natural_unit` table)
- Modify: `craft-libs/crates/units/src/parse.rs` (`is_ident_char` / tokenizer: `%`)
- Test: the existing `#[cfg(test)]` modules of those files

**Interfaces:**
- Produces: `lookup(name) -> Option<Scale>` and `parse_unit(text)` resolve every name below; `natural_unit(dim, sys).text` uses the new spellings.

- [ ] **Step 1: Branch.** In `/home/alan/arath/storytold/craft-libs`: `git switch -c feature/calc-units-and-functions` (from `feature/craft-units`).

- [ ] **Step 2: Write failing tests** (`catalog.rs` tests; factors with relative tolerance 1e-12):

```rust
#[test]
fn mathcad_units() {
    let f = |n: &str| lookup(n).unwrap_or_else(|| panic!("{n}")).factor;
    assert_eq!(f("hr"), 3600.0);
    assert!(lookup("h").is_none());
    assert!(lookup("lb").is_none());
    assert!(lookup("R").is_none());
    assert_eq!(f("sec"), 1.0);
    assert_eq!(f("Btu"), f("BTU"));
    assert_eq!(f("liter"), 1e-3);
    assert_eq!(f("watt"), 1.0);
    assert_eq!(f("joule"), 1.0);
    assert_eq!(f("mole"), 1.0);
    assert_eq!(f("Å"), 1e-10);
    assert_eq!(f("Angstrom"), 1e-10);
    // close(): relative 1e-12
    close(f("atm"), 101325.0);
    close(f("bar"), 1e5);
    close(f("mbar"), 100.0);
    close(f("torr"), 101325.0 / 760.0);
    close(f("gpm"), 0.003785411784 / 60.0);
    close(f("cfm"), 0.3048_f64.powi(3) / 60.0);
    close(f("mph"), 1609.344 / 3600.0);
    close(f("poise"), 0.1);
    close(f("cP"), 1e-3);
    close(f("%"), 0.01);
    close(f("kgf"), 9.80665);
    close(f("ton"), 2000.0 * 0.45359237);
    close(f("tonne"), 1000.0);
    close(f("cal"), 4.1868);
    close(f("kcal"), 4186.8);
    close(f("day"), 86400.0);
    close(f("yr"), 365.2422 * 86400.0);
    close(f("plf"), 4.4482216152605 / 0.3048);
    close(f("pcf"), 4.4482216152605 / 0.3048_f64.powi(3));
    close(f("ksf"), 1000.0 * 4.4482216152605 / 0.3048_f64.powi(2));
}

#[test]
fn new_names_keep_old_prefixed_spellings() {
    for (n, v) in [("ms", 1e-3), ("min", 60.0), ("mi", 1609.344), ("mm", 1e-3), ("kPa", 1e3), ("cd", 1.0)] {
        close(lookup(n).unwrap().factor, v);
    }
}
```

Dimensions too: `poise`/`cP` are Pa·s, `%` is dimensionless, `kgf` force, `ton`/`tonne` mass, `plf` force/length, `pcf` force/volume, `ksf` pressure.

In `format.rs` tests, two values Mathcad saved (`Stwy 1st to Mezz.xmcd`, `Shielded Transfer Cart.xmcd`):

```rust
#[test]
fn pcf_is_force_per_cubic_foot() {
    // 150 pcf · 0.25 ft = 37.5 psf; 490 pcf = 76.97285728466062 kN/m³
}

#[test]
fn results_use_the_users_spellings() {
    assert_eq!(natural_unit(Dim::TIME, UnitSystem::UsCustomary).text, "sec");
    assert_eq!(natural_unit(Dim::TIME, UnitSystem::Si).text, "sec");
    // SI energy → "joule", SI power → "watt"; US energy stays "lbf·in", US power "hp";
    // SI acceleration composes as "m/sec²" (whatever form `compose` gives "m/s²" today, with sec).
}
```

`parse.rs` tests: `parse_unit("%")` is 0.01 dimensionless; `parse_unit("Btu/hr")`, `parse_unit("gal/min")` and `parse_unit("lbf/ft^3")` parse.

- [ ] **Step 3: Run** `cargo test -p craft-units` — expect the new tests to fail.

- [ ] **Step 4: Implement.**
  - `h` → `hr`. Separate, non-prefixable entries for `sec`, `liter`, `watt`, `joule` (the symbols `s`, `L`, `W`, `J` stay prefixable). `Btu` joins the `BTU` entry; `mole` joins `mol`; `Angstrom` joins `Å`.
  - Prefixable new units: `bar`, `cal`. All others are not.
  - Add `%` as a one-character name in the unit tokenizer: it never joins letters around it.
  - In `format.rs`: `base_names` time `"s"` → `"sec"` in both systems. In the natural table: time `("sec", "sec")`, SI energy `"joule"`, SI power `"watt"`.

- [ ] **Step 5: Run** `cargo test -p craft-units` and `cargo clippy -p craft-units --all-targets -- -D warnings` — expect clean.

- [ ] **Step 6: Commit** (in craft-libs): `feat(units): Mathcad units and result spellings`.

---

### Task 2: WordCraft — use the new units

**Files:**
- Modify: `wordcraft/Cargo.toml` (`[patch]`)
- Modify: `crates/calc/src/parse.rs` (`lex`: `%`)
- Modify: `crates/calc/src/lib.rs` (`CalcError::Undefined` message)
- Modify: any test or sample whose expected text changes (`s`→`sec`, `J`→`joule`, `W`→`watt`, `h`→`hr`)
- Test: `crates/calc/src/eval/tests.rs`, `crates/calc/src/parse/tests.rs`

**Interfaces:**
- Consumes: Task 1's `lookup`, `parse_unit`, `natural_unit`.
- Produces: `%` lexes as a name token `"%"`.

- [ ] **Step 1: Point at the local craft-units.** Add to `wordcraft/Cargo.toml`:

```toml
[patch."https://github.com/alanrathbun/craft-libs"]
craft-units = { path = "../craft-libs/crates/units" }
```

Check that `cargo build -p wordcraft-calc` uses it, from `Compiling craft-units … (/home/alan/arath/storytold/craft-libs/crates/units)`.

- [ ] **Step 2: Write failing tests** (with the file's existing helpers):
  - `percent_unit`: `5 % · 200` is 10; `x := 0.25` then `x = ? %` shows `25 %`.
  - `hour_is_hr`: `q := 3412.14 Btu/hr` then `q = ? watt` shows about `1000 watt`.
  - `typed_units_show_as_typed`: `q := 1 kW` then `q = ? BTU/hr` shows the unit `BTU/hr` (not `Btu/hr`), and `t := 90 s` then `t = ? s` shows `s`.
  - `h_hints_hr`: an error on `5 h` names `h` and contains `for hours, use hr`.
  - `newly_reserved_names`: `bar := 3`, `day := 7`, `ton := 1` and `cal := 2` are each the reserved-name error.
  - `si_results_spell_units`: in an SI document, `1 kJ/sec` with no unit shows `1000 watt`, and `9.8 m/sec^2` shows in `m/sec²`.

- [ ] **Step 3: Run** `cargo test -p wordcraft-calc` — expect the new tests to fail.

- [ ] **Step 4: Implement.**
  - `lex`: `'%'` becomes a one-character name token `"%"` (so `5 %` is an implied product with the unit).
  - Render: `%` already shows upright as a unit, through the `lookup(name)` check in `render.rs`. Confirm with a render test that `5 %` displays as `5 %`.
  - `CalcError::Undefined` display: when the name is exactly `h`, append the hint `— for hours, use hr`.

- [ ] **Step 5: Update changed expectations.** Run the whole workspace: `cargo test --workspace --no-fail-fast`. Fix every expectation whose only change is the new spelling: `s`→`sec`, `J`→`joule`, `W`→`watt`, `h`→`hr` in sources. The known environment failures stay as they are (7 tests: hyphenation, proofing, RTL template, glyph coverage). List each changed expectation in the report.

- [ ] **Step 6: Run** `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check`. Expect only the 7 known failures.

- [ ] **Step 7: Commit:** `feat(calc): Mathcad units, percent, hr`.

---

### Task 3: Scalar functions

**Files:**
- Modify: `crates/calc/src/funcs.rs` (`BUILTINS`, `dispatch`)
- Modify: `crates/calc/Cargo.toml` (and `[workspace.dependencies]` if that is the pattern): add `libm = "0.2"`
- Test: `crates/calc/src/eval/tests.rs`

**Interfaces:**
- Produces: built-ins `Ceil`, `Floor`, `Round` (2 arguments), `erf`, `csc`, `cot`, `secant`, `atanh`, `J0`, `J1`. Like the existing scalar built-ins, they apply element-wise over grids.

- [ ] **Step 1: Write failing tests:**
  - `ceil_to_multiple`:
    - `Ceil(2.06 in, 0.125 in)` is `2.125 in`.
    - `Ceil(7.2, 1)` is 8.
    - `Ceil(1.9 in, 25.4 mm)` is `50.8 mm` (Mathcad's `hicks_si_p_2.68.xmcd`).
    - `Floor(2.06 in, 0.125 in)` is `2 in`.
    - `Round(2.06 in, 0.125 in)` is `2 in`.
    - `Round(2.0625, 0.125)` is 2.125 (half away from zero).
  - `ceil_negative_and_bad_steps`:
    - `Ceil(-2.06, 0.125)` is −2.
    - `Floor(-2.06, 0.125)` is −2.125.
    - A step of 0 or −1 is an error that names the function and the step.
    - `Ceil(2 in, 1 lbf)` is the unit-mismatch error.
  - `erf_matches_mathcad`:
    - `0.5·√π·12000·(1 + erf(15000/12000))` is `20449.510423409225`, to relative 1e-12 (`Molar Mass.xmcd`).
    - `erf(2 in)` is the plain-number error.
  - `reciprocal_trig`:
    - `csc(30 deg)` is 2.
    - `cot(45 deg)` is 1.
    - `secant(60 deg)` is 2 (relative 1e-12).
    - `atanh(0.5)` is 0.5493061443340548.
  - `bessel`:
    - `J0(1)` is 0.7651976865579666.
    - `J1(1)` is 0.44005058574493355 (relative 1e-14).
  - `lowercase_unchanged`: `ceil(2.1)` is 3, and `round(2.345, 2)` is unchanged from today.
  - `new_function_names_reserved`: `erf := 1` and `Ceil := 1` are reserved-name errors.

- [ ] **Step 2: Run** `cargo test -p wordcraft-calc` — expect FAIL.

- [ ] **Step 3: Implement** in `funcs.rs`.
  - `Ceil`/`Floor`/`Round(x, y)`: `y·⌈x/y⌉` and so on. `x` and `y` must have the same dimension, through `same`. The result takes `x`'s dimension. The step must be greater than 0.
  - `csc`, `cot` and `secant` take angles like `sin`.
  - `erf`, `atanh`, `J0` and `J1` take plain numbers, and use `libm::erf`, `f64::atanh`, `libm::j0` and `libm::j1`.

- [ ] **Step 4: Run** `cargo test -p wordcraft-calc` and clippy — expect PASS.

- [ ] **Step 5: Commit:** `feat(calc): Mathcad rounding, erf, Bessel and reciprocal trig`.

---

### Task 4: Curve fitting — splines, regress, interp, slope, intercept

**Files:**
- Modify: `crates/calc/src/array_funcs.rs` (`call_array` dispatch, the per-element `x` rule, the text-refusal list, new functions)
- Modify: `crates/calc/src/funcs.rs` (`ARRAY_BUILTINS`)
- Test: `crates/calc/src/eval/tests.rs`

**Interfaces:**
- Consumes: the existing `second_derivatives`, `between`, `one_dim`, `fits` and `positions` helpers in `array_funcs.rs`.
- Produces:
  - `cspline(vx, vy)`, `lspline(vx, vy)` and `pspline(vx, vy)` return a column vector of `n + 3` elements, for `n = length(vx)`.
  - `regress(vx, vy, k)` returns `k + 4` elements.
  - `interp(vs, vx, vy, x)`.
  - `slope(vx, vy)` and `intercept(vx, vy)`.

**Coefficient vector layout** (positions counted from the document's index base):
- Element 0 is the kind: 1 = lspline, 2 = pspline, 3 = cspline, 4 = regress.
- Element 1 is 3.
- Element 2 is `k` for regress, 0 for a spline.
- From element 3 on:
  - for a spline, the second derivatives at each `vx`, in the unit of `vy/vx²`;
  - for regress, the coefficients `a_0 … a_k`, where `a_j` is in `vy/vx^j`.
- The header elements are plain numbers.
- Mathcad's regress puts `a_0` at element 3; that is confirmed from saved results. The header values are WordCraft's own; Mathcad's are not known.

**End conditions** (`M` is the second derivative):
- lspline: `M_0 = M_{n−1} = 0` (natural; the existing `second_derivatives`).
- pspline: `M_0 = M_1` and `M_{n−1} = M_{n−2}`.
- cspline: `M` runs in a straight line in `x` across the two end intervals at each end. The first row is `h_1·M_0 − (h_0+h_1)·M_1 + h_0·M_2 = 0`, and the last end mirrors it. This reproduced all 41 of Mathcad's saved pump-curve values to 6e-14.

- [ ] **Step 1: Write failing tests:**
  - `cspline_matches_mathcad_pump_curve`:
    - Data: `flowrate := [0, 1000, 2000, 3000, 3500, 4000] gpm` and `head := [230, 228.5, 221, 200.5, 183.5, 157] ft`.
    - With `H(Q) := interp(cspline(flowrate, head), flowrate, head, Q)`:
      - `H(100 gpm)` is `229.892 ft`
      - `H(200 gpm)` is `229.796 ft`
      - `H(500 gpm)` is `229.5 ft`
      - `H(1000 gpm)` is `228.5 ft`
    - All to relative 1e-12. Mathcad's `Mathcad Verification Simple Pump Model.xmcd` saved these.
    - `H` over the vector `Q := [0, 100, 200] gpm` gives `[230, 229.892, 229.796] ft`.
  - `lspline_is_natural`: `interp(lspline(xs, ys), xs, ys, x)` equals the existing `spline(xs, ys, x)` at three points inside the data.
  - `pspline_parabolic_ends`: for the same pump data, the elements of `pspline(flowrate, head)` at positions 3 and 4 are equal (`M_0 = M_1`).
  - `regress_matches_mathcad`:
    - Data: `T := [100, 200, 300, 400, 500]` and `k := [0.25, 0.31, 0.40, 0.51, 0.64]`.
    - With `c := regress(T, k, 2)`, the elements at index-base positions 3, 4 and 5 are `0.20799999999999994`, `0.0002942857142857147` and `1.1428571428571426E-06`, to relative 1e-9 (`ASTM C547 PolyRegress w Header.xmcd`).
    - `interp(c, T, k, 600)` is `0.208 + 2.942857e-4·600 + 1.142857e-6·600²`: a fit evaluates anywhere.
  - `slope_intercept`: for `x := [1, 2, 3] in` and `y := [2, 4, 6] lbf`, `slope(x, y)` is `2 lbf/in` and `intercept(x, y)` is `0 lbf`.
  - `interp_refuses_outside_spline_data`: `interp(cspline(…), flowrate, head, 4100 gpm)` is the existing outside-the-data error (as `spline` gives).
  - `interp_rejects_foreign_vectors`:
    - `interp([1, 2, 3], xs, ys, x)` errors.
    - So does `interp(cspline(xs, ys), xs2, ys2, x)` with `xs2` of another length.
    - Both messages contain `cspline, lspline, pspline or regress`.
  - `interp_per_element_in_cases`: inside a case branch, an `x` element outside the data fails only the elements that pick that branch, as `linterp` does today.
  - `regress_degree_checks`: `k` must be a whole number from 1 to `n − 1`, or an error.

- [ ] **Step 2: Run** `cargo test -p wordcraft-calc` — expect FAIL.

- [ ] **Step 3: Implement.**
  - `interp`'s `x` is its fourth argument (index 3). The per-element rule in `call_array` must use index 3 for `interp`, and index 2 for `lookup`, `linterp` and `spline`.
  - Charge the work like `interpolate`.
  - Solve regress by least squares on a Vandermonde system scaled for stability. Plain normal equations lose digits at `x` around 500.

- [ ] **Step 4: Run** `cargo test -p wordcraft-calc` and clippy — expect PASS.

- [ ] **Step 5: Commit:** `feat(calc): Mathcad splines, regress, interp, slope and intercept`.

---

### Task 5: Matrix functions — submatrix, augment, stack

**Files:**
- Modify: `crates/calc/src/array_funcs.rs` and `crates/calc/src/funcs.rs` (`ARRAY_BUILTINS`)
- Test: `crates/calc/src/eval/tests.rs`

**Interfaces:**
- Produces: `submatrix(A, ir, jr, ic, jc)`, `augment(A, B, …)` and `stack(A, B, …)`. They work on numbers, and on text the way `transpose` does. Mixing a text grid with a number grid is the existing `array::mixed` error.

- [ ] **Step 1: Write failing tests:**
  - `submatrix_counts_from_index_base`:
    - Take `M := [1, 2, 3; 4, 5, 6; 7, 8, 9]`.
    - With index base 0, `submatrix(M, 1, 2, 0, 1)` is `[4, 5; 7, 8]`.
    - With index base 1, the same call gives `[1, 2; 4, 5]`.
    - A row or column out of range is an error that names it.
    - `ir > jr` is an error.
  - `augment_and_stack`:
    - `augment([1; 2], [3; 4])` is `[1, 3; 2, 4]`.
    - `stack([1, 2], [3, 4])` is `[1, 2; 3, 4]`.
    - Mismatched rows or columns are errors that give both sizes.
    - Units are kept per element: `augment([1; 2] in, [3; 4] lbf)` works.
  - `augment_text`: `augment(["a"; "b"], ["c"; "d"])` works, and `augment(["a"; "b"], [1; 2])` is the mixed error.

- [ ] **Step 2–4:**
  - Run the tests and see them fail.
  - Implement, charging the work by element.
  - Run them again and see them pass, with clippy clean.

- [ ] **Step 5: Commit:** `feat(calc): submatrix, augment and stack`.

---

### Task 6: Guide and editor

**Files:**
- Modify: `docs/calculations.md` (the Units section, and a new "Mathcad functions" section after "Functions")
- Modify: `crates/ui-egui/src/calc_ui.rs` (`SYMBOL_GROUPS` "Units" row)
- Test: `crates/ui-egui/tests/calc.rs`

- [ ] **Step 1: Write a failing test.** `units_row_has_angstrom_and_percent`: the "Units" group contains `Å` and `%`.

- [ ] **Step 2: Implement.** Add `"Å"` and `"%"` to the Units row.

- [ ] **Step 3: Update the guide.**
  - A units table with the new units and their meanings, taken from the spec.
  - The accepted spellings, and the spellings results use.
  - `h` is gone: write `hr`.
  - `lb` and `R` are not units: write `lbm` and `°R`.
  - A "Mathcad functions" section with the function table from the spec, noting:
    - the coefficient-vector layout;
    - that `interp` refuses `x` outside spline data but evaluates a regression anywhere;
    - that `Ceil`/`Floor`/`Round` with a step differ from lowercase `ceil`/`floor`/`round`.
  - Use US spelling.

- [ ] **Step 4: Run** `cargo test -p wordcraft-ui-egui --test calc` — expect PASS.

- [ ] **Step 5: Commit:** `docs(calc): Mathcad units and functions`.

---

## After the tasks (controller, not an implementer)

- Before any pull request, ask the user before pushing craft-libs `feature/calc-units-and-functions`. After the push:
  - replace the `[patch]` with a `rev` pin to the pushed commit in `wordcraft/Cargo.toml`;
  - re-run the workspace tests.
- Rulings carried from planning:
  - `yr` is 365.2422 days. No saved Mathcad result was found to confirm it; check it in Mathcad 15.
  - The cspline end rule is "linear in `x`". The pump data had equal end intervals, so it can't tell this from "linear in index".
  - `interp` over a regression evaluates outside `vx` (as the user's worksheets do with fitted formulas), while splines refuse.
