# Inline Engineering Calculations (Stage 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Live, unit-aware, Mathcad-style calculations inside WordCraft documents. They are saved
to `.docx` as ordinary Word equations and become live again when reopened in WordCraft.

**Architecture:**

- **`craft-units`** is a new crate in the `craft-libs` fork. It owns dimensions, quantities, the
  unit catalog, unit-expression parsing and display formatting.
- **`wordcraft-calc`** is a new crate in WordCraft. It owns the calculation language, the
  evaluator and a recalculation pass that evaluates in dependency order.
- **No new rendering code in `wordcraft-doc`.** A calculation is the existing
  `InlineObject::Equation` with an optional `calc` payload; the pass writes the rendered result
  into the equation's existing `linear`/`math` fields, so layout and rendering need no change.
- **Quoted values reuse fields.** A quoted value is a *locked* `InlineObject::Field` with
  instruction `WCCALC …`. Word shows its result and never updates a locked field.
- **The `.docx` round trip reuses two existing mechanisms:** content controls (`w:sdt`, tag
  `wordcraft-calc:<id>`) around each calculation, and a custom XML part
  `customxml/wordcraftcalc.xml`, modelled on `crates/docx/src/fit.rs`.

**Tech Stack:** Rust 2024 (toolchain 1.100 installed via rustup; workspace `rust-version = "1.95"`),
serde/serde_json, quick-xml (already used by `crates/docx`), proptest (already a workspace dev-dep).

**Spec:** `docs/superpowers/specs/2026-11-14-inline-calculations-design.md`

**Repos and branches:**

| Repo | Location | Branch | Base |
|---|---|---|---|
| WordCraft fork | `/home/alan/arath/storytold/wordcraft` | `feature/inline-calculations` | already exists |
| craft-libs fork | `/home/alan/arath/storytold/craft-libs` | `feature/craft-units` | create from `main` in Task 1 |

## Global Constraints

- **`AGENTS.md` rules in both repos apply:**
  - no `unsafe` (`#![forbid(unsafe_code)]`)
  - no panics in non-test code; every new crate root starts with
    `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`
  - bounded recursion and allocations; treat input as hostile
- **Rust only.** No embedded browser, Python or JavaScript. EngineeringPaper.xyz and Mathcad are
  behavioural references only; copy no code.
- **Dependencies:** only crates already in the workspaces, plus `craft-units`. The licence is
  `MIT OR Apache-2.0` (`license.workspace = true`).
- **`craft-units` has no WordCraft or GridCraft types in its public API.**
- **Commits** carry no agent attribution lines, because storytold's rules forbid them. Commit on
  the branches above only, and push nothing without the user's OK.
- **Before each commit in a repo,** run `cargo fmt --check` and
  `cargo clippy -p <crate> --all-targets -- -D warnings` for the touched crates. The full
  `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` run in the
  final task.
- **Defaults pinned by this plan:**
  - unit system US customary
  - display precision 3 decimal places with trailing zeros trimmed
  - exponential display when |x| ≥ 1e6 or 0 < |x| < 1e-4
  - recalculation mode automatic
- **Performance:** 2,000 calculation regions recalculate in < 100 ms after one edit (release
  build).
- **Stage 1 covers the main body and table cells only.** Headers, footers, notes, comments and
  text boxes are skipped.

## Review Focus

1. **Variable/unit name clashes:** `m := 5 kg` and `A := 2 in^2` are rejected as `ReservedName`.
   `2 m =` anywhere in the document still means 2 metres. A pasted clash is an error, never a
   silent re-meaning of the unit. Test: Task 5 `unit_names_cannot_be_variables`, Task 7
   `pasted_unit_name_definition_is_error`.
2. **Copy-paste of a calculation duplicates its region ID.** The recalculation pass must give
   duplicates fresh IDs, keeping the first. Test: Task 7 `duplicate_ids_are_reassigned`.
3. **Hostile numeric input never panics:**
   - `10^400`, `1/0 in` and `sqrt(-1)` produce errors
   - 200 nested parentheses and a 100 kB source produce errors
   - Test: Task 4 `hostile_sources_error_not_panic`, Task 5 `non_finite_results_are_errors`.
4. **Undo/redo keeps results consistent.** After undoing an input change, every dependent result
   shows the old value again. Test: Task 8 `undo_restores_results`.
5. **Tracked deletions:** a calculation inside a tracked-deleted range defines nothing, so its
   name is undefined everywhere, and re-typing that name elsewhere is not a duplicate. Test:
   Task 7 `tracked_deleted_region_is_ignored`.

---

### Task 1: `craft-units` — dimensions and quantity arithmetic

Repo: craft-libs. First `git switch -c feature/craft-units`.

**Files:**
- Create: `crates/units/Cargo.toml` (model on `crates/time/Cargo.toml`: workspace package fields,
  `[lints] workspace = true`, deps `serde`, `thiserror`; dev-dep `proptest = "1.4"`)
- Create: `crates/units/README.md` (one paragraph: purpose, no app types, f64 SI magnitudes)
- Create: `crates/units/src/lib.rs`, `crates/units/src/dim.rs`, `crates/units/src/quantity.rs`
- Modify: `Cargo.toml` (workspace root): add `craft-units = { path = "crates/units" }` under the
  internal-crates comment
- Test: `crates/units/tests/quantity.rs`

**Interfaces:**
- Produces:
  - `pub struct Dim { pub exp: [i8; 8] }` — indices `0` L, `1` M, `2` T, `3` I, `4` Θ, `5` N,
    `6` J, `7` angle
  - `Dim::ONE`, `Dim::LENGTH`, `Dim::MASS`, `Dim::TIME`, `Dim::CURRENT`, `Dim::TEMPERATURE`,
    `Dim::AMOUNT`, `Dim::LUMINOSITY`, `Dim::ANGLE`
  - `fn mul(self, Dim) -> Option<Dim>`, `fn div(self, Dim) -> Option<Dim>`,
    `fn powi(self, i32) -> Option<Dim>`, `fn root(self, u8) -> Option<Dim>` (`None` if an
    exponent isn't divisible), `fn is_dimensionless(self) -> bool` (ignores angle),
    `fn same_ignoring_angle(self, Dim) -> bool`. `Option` is `None` on i8 overflow.
  - `#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)] pub struct Quantity { pub value: f64, pub dim: Dim }`
    — value in SI base units
  - `Quantity::{new, dimensionless, add, sub, mul, div, neg, powf, powi, sqrt}`; each returns
    `Result<Quantity, UnitError>`
  - `#[derive(Debug, Clone, PartialEq, thiserror::Error)] pub enum UnitError { Mismatch { left: Dim, right: Dim }, Overflow, NonFinite, Fractional, NotDimensionless(Dim), UnknownUnit(String), Syntax(String), AffineTemperature }`
  - Rules: `add`/`sub` require `same_ignoring_angle`. A result that is not finite is `NonFinite`.
    `powf` requires a dimensionless base unless the exponent is an exact integer.

- [ ] **Step 1: Write failing tests** in `tests/quantity.rs`:

```rust
#[test] fn force_over_area_is_pressure() {
    let f = Quantity::new(10.0, Dim::MASS.mul(Dim::LENGTH).unwrap().div(Dim::TIME.powi(2).unwrap()).unwrap());
    let a = Quantity::new(2.0, Dim::LENGTH.powi(2).unwrap());
    let p = f.div(a).unwrap();
    assert_eq!(p.value, 5.0);
    assert_eq!(p.dim.exp, [-1, 1, -2, 0, 0, 0, 0, 0]);
}
#[test] fn adding_length_to_mass_is_mismatch() {
    assert!(matches!(Quantity::new(1.0, Dim::LENGTH).add(Quantity::new(1.0, Dim::MASS)), Err(UnitError::Mismatch { .. })));
}
#[test] fn sqrt_of_odd_dimension_is_fractional() {
    assert_eq!(Quantity::new(4.0, Dim::LENGTH).sqrt(), Err(UnitError::Fractional));
    assert_eq!(Quantity::new(4.0, Dim::LENGTH.powi(2).unwrap()).sqrt().unwrap(), Quantity::new(2.0, Dim::LENGTH));
}
#[test] fn overflowing_exponent_is_error() { assert!(Dim::LENGTH.powi(200).is_none()); }
#[test] fn non_finite_is_error() {
    assert_eq!(Quantity::dimensionless(1.0).div(Quantity::dimensionless(0.0)), Err(UnitError::NonFinite));
}
#[test] fn angle_adds_to_dimensionless() {
    assert!(Quantity::new(1.0, Dim::ANGLE).add(Quantity::dimensionless(1.0)).is_ok());
}
```

Add a proptest: for random dims `a`, `b` with exponents in −3..=3, `a.mul(b).unwrap().div(b) == Some(a)`.

- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test -p craft-units`. Expected:
  compile errors (types not defined).
- [ ] **Step 3: Implement `dim.rs`, `quantity.rs`, `lib.rs`** with the signatures above. Use
  checked i8 arithmetic.
- [ ] **Step 4: Run the tests and confirm they pass.** Run `cargo test -p craft-units`.
  Expected: all PASS.
- [ ] **Step 5: Commit**: `git add Cargo.toml crates/units && git commit -m "units: dimensions and quantity arithmetic"`

### Task 2: `craft-units` — unit catalog and unit-expression parser

**Files:**
- Create: `crates/units/src/catalog.rs`, `crates/units/src/parse.rs`
- Test: `crates/units/tests/parse.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces:
  - `pub struct UnitDef { pub names: &'static [&'static str], pub dim: Dim, pub factor: f64, pub offset: f64, pub prefixable: bool }`
    — SI value = (x + offset) × factor
  - `pub fn lookup(name: &str) -> Option<Scale>`, which handles SI prefixes on prefixable units
    (`k`, `M`, `G`, `m`, `µ`/`u`, `n`, `c`)
  - `#[derive(Clone, Debug, PartialEq)] pub struct Scale { pub dim: Dim, pub factor: f64, pub offset: f64, pub text: String }`
  - `pub fn parse_unit(expr: &str) -> Result<Scale, UnitError>`
    - grammar: products with `*`, `·` or a space; `/`; integer powers `^n` and `²`/`³`;
      parentheses
    - input above 256 bytes is `Syntax`; nesting is limited to 16
    - an offset unit used inside a compound expression is `AffineTemperature`
  - `Scale::to_si(&self, x: f64) -> f64` and `Scale::from_si(&self, si: f64) -> f64`
- **Catalog content:**
  - SI base and derived units: m, g (prefixable, factor 1e-3 kg), s, A, K, mol, cd, rad, deg (°),
    N, Pa, J, W, Hz, L, min, h
  - US customary: in, ft, yd, mi, mil, lbm, slug, lbf, kip, psi, ksi, psf, BTU, hp, ft·lbf
    (as `ft*lbf`), gal
  - temperature: °R, °C and °F (offset units), Δ°C and Δ°F (delta units, no offset)
  - use exact defined factors (in = 0.0254 m, lbm = 0.45359237 kg, lbf = 4.4482216152605 N,
    psi = lbf/in²); GridCraft's `crates/functions/src/engineering/units.rs` may be consulted for
    factors

- [ ] **Step 1: Write failing tests**:

```rust
#[test] fn psi_is_lbf_per_square_inch() {
    let a = parse_unit("psi").unwrap(); let b = parse_unit("lbf/in^2").unwrap();
    assert_eq!(a.dim, b.dim); assert!((a.factor / b.factor - 1.0).abs() < 1e-12);
    assert!((a.factor - 6894.757293168361).abs() < 1e-6);
}
#[test] fn prefixes() { assert!((parse_unit("kN").unwrap().factor - 1000.0).abs() < 1e-9); assert!((parse_unit("mm").unwrap().factor - 1e-3).abs() < 1e-15); }
#[test] fn superscripts_and_middle_dot() { assert_eq!(parse_unit("N·m²").unwrap().dim, parse_unit("N*m^2").unwrap().dim); }
#[test] fn unknown_unit() { assert!(matches!(parse_unit("furlongz"), Err(UnitError::UnknownUnit(_)))); }
#[test] fn offset_unit_in_compound_is_rejected() { assert_eq!(parse_unit("°F/s"), Err(UnitError::AffineTemperature)); }
#[test] fn degf_round_trip() { let f = parse_unit("°F").unwrap(); assert!((f.to_si(32.0) - 273.15).abs() < 1e-9); assert!((f.from_si(373.15) - 212.0).abs() < 1e-9); }
#[test] fn delta_degf_has_no_offset() { assert!((parse_unit("Δ°F").unwrap().to_si(9.0) - 5.0).abs() < 1e-9); }
#[test] fn hostile_unit_text() { assert!(parse_unit(&"(".repeat(100)).is_err()); assert!(parse_unit(&"m*".repeat(500)).is_err()); }
```

- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test -p craft-units --test parse`.
  Expected: compile errors.
- [ ] **Step 3: Implement** `catalog.rs` as a `static` slice of `UnitDef`, and `parse.rs` as a
  recursive descent with a depth counter.
- [ ] **Step 4: Run the tests and confirm they pass.** Run `cargo test -p craft-units`.
- [ ] **Step 5: Commit**: `git commit -am "units: catalog and unit expression parser"` (add the new
  files first).

### Task 3: `craft-units` — display formatting

**Files:**
- Create: `crates/units/src/format.rs`
- Test: `crates/units/tests/format.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)] pub enum UnitSystem { #[default] UsCustomary, Si }`
  - `#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)] pub struct Precision { pub decimals: u8 }`
    — default 3, clamped to 0..=15
  - `pub fn natural_unit(dim: Dim, sys: UnitSystem) -> Scale`
  - `pub fn format(q: Quantity, unit: Option<&Scale>, sys: UnitSystem, p: Precision) -> Result<Formatted, UnitError>`
  - `pub struct Formatted { pub number: String, pub unit: String }`
    - `number` uses `.` as decimal separator, trailing zeros trimmed, exponential form
      `6.0E+6` → written as `6×10^6` (`number` holds `6×10^6`) when |x| ≥ 1e6 or 0 < |x| < 1e-4
    - a supplied `unit` whose dim differs is `Mismatch`
- **Natural units:**

  | Dimension | US customary | SI |
  |---|---|---|
  | length | in | m |
  | area | in² | m² |
  | volume | in³ | m³ |
  | mass | lbm | kg |
  | force | lbf | N |
  | pressure/stress | psi | Pa |
  | moment/energy (L²MT⁻²) | lbf·in | J |
  | power | hp | W |
  | time | s | s |
  | temperature | °R | K |
  | dimensionless | (empty) | (empty) |

  Any other dimension is written in base units: US `in`, `lbm`, `s`, `°R`; SI `m`, `kg`, `s`,
  `K`; positive powers first, then `/` and the negative powers, for example `lbm/(in·s)`.

- [ ] **Step 1: Write failing tests**:

```rust
#[test] fn stress_in_psi_us() { let q = Quantity::new(600.0 * 6894.757293168361, parse_unit("psi").unwrap().dim);
    let f = format(q, None, UnitSystem::UsCustomary, Precision::default()).unwrap(); assert_eq!((f.number.as_str(), f.unit.as_str()), ("600", "psi")); }
#[test] fn explicit_unit() { let q = Quantity::new(600.0 * 6894.757293168361, parse_unit("psi").unwrap().dim);
    let f = format(q, Some(&parse_unit("ksi").unwrap()), UnitSystem::UsCustomary, Precision::default()).unwrap(); assert_eq!(f.number, "0.6"); }
#[test] fn precision_and_trim() { let q = Quantity::dimensionless(2.0/3.0); assert_eq!(format(q, None, UnitSystem::Si, Precision { decimals: 3 }).unwrap().number, "0.667"); }
#[test] fn exponential() { assert_eq!(format(Quantity::dimensionless(6.0e6), None, UnitSystem::Si, Precision::default()).unwrap().number, "6×10^6"); }
#[test] fn wrong_display_unit() { let q = Quantity::new(1.0, Dim::LENGTH); assert!(format(q, Some(&parse_unit("psi").unwrap()), UnitSystem::Si, Precision::default()).is_err()); }
#[test] fn fallback_composition() { let d = Dim::MASS.div(Dim::LENGTH.mul(Dim::TIME).unwrap()).unwrap(); assert_eq!(natural_unit(d, UnitSystem::UsCustomary).text, "lbm/(in·s)"); }
#[test] fn temperature_display_in_degf() { let q = Quantity::new(373.15, Dim::TEMPERATURE); assert_eq!(format(q, Some(&parse_unit("°F").unwrap()), UnitSystem::UsCustomary, Precision::default()).unwrap().number, "212"); }
```

- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement `format.rs`.**
- [ ] **Step 4: Run the tests and confirm they pass:** `cargo test -p craft-units`, then
  `cargo clippy -p craft-units --all-targets -- -D warnings` and `cargo fmt --check`.
- [ ] **Step 5: Commit**: `git commit -m "units: display formatting and natural units"`

### Task 4: `wordcraft-calc` — calculation language parser

Repo: WordCraft. All later tasks are in this repo.

**Files:**
- Create: `crates/calc/Cargo.toml`
  - name `wordcraft-calc`, workspace package fields, `[lints] workspace = true`
  - deps `craft-units = { path = "../../../craft-libs/crates/units" }` (switched to a git pin in
    Task 11), `serde`, `thiserror`, `wordcraft-doc`
- Create: `crates/calc/src/lib.rs`, `crates/calc/src/ast.rs`, `crates/calc/src/parse.rs`
- Modify: `Cargo.toml` (workspace): add `wordcraft-calc = { path = "crates/calc" }` under
  `[workspace.dependencies]`
- Test: `crates/calc/src/parse/tests.rs` (`#[cfg(test)] mod tests;` in `parse.rs`, matching the
  repo's style)

**Interfaces:**
- Produces:
  - `pub enum Stmt { Define { name: String, params: Vec<String>, body: Expr, show: Option<Show> }, Eval { expr: Expr, show: Show } }`
  - `pub struct Show { pub unit: Option<String> }`
    - `x =` gives `Show { unit: None }`
    - `x = ? ksi`, `x = ksi` and a trailing unit after `=` give `Some("ksi")`
  - `pub enum Expr { Num(f64), Name(String), Quantity(f64, String) /* number+unit literal */, Unary(Op, Box<Expr>), Binary(Op, Box<Expr>, Box<Expr>), Call(String, Vec<Expr>), Abs(Box<Expr>) }`
  - `pub enum Op { Add, Sub, Mul, Div, Pow, Neg }`
  - `pub fn parse(src: &str) -> Result<Stmt, CalcError>`
  - `#[derive(Debug, Clone, PartialEq, thiserror::Error)] pub enum CalcError { #[error("{0}")] Syntax(String), #[error("{0} isn't defined")] Undefined(String), #[error("{0}")] Unit(#[from] craft_units::UnitError), #[error("result isn't a finite number")] NonFinite, #[error("too complex")] TooComplex, #[error("{0} expects {1} argument(s)")] Arity(String, usize), #[error("depends on a calculation with an error")] Upstream, #[error("{name} is already defined {} — choose a unique name", page.map_or("elsewhere in the document".to_string(), |p| format!("on page {p}")))] Duplicate { name: String, page: Option<u32> }, #[error("{0} is a unit, function or constant name — choose another name")] ReservedName(String), #[error("{0} depend on each other")] Circular(String) }`
    — `Circular` holds the names joined with `, `
- **Lexing:**
  - `:=` and `≔` define; `=` evaluates; `·`, `×` and `*` multiply; `√` is `sqrt`; `|…|` is
    absolute value
  - names are a letter (including Greek) followed by letters, digits or `_`, with an optional
    `_subscript`
  - a number immediately followed (with or without a space) by a name is emitted as
    `Binary(Mul, Num, Name)`. The parser keeps no unit/variable knowledge, and the evaluator
    resolves the name. Variables can never be unit names, so this is unambiguous.
- **Also produces, in `ast.rs`:**
  - `pub fn defined_name(s: &Stmt) -> Option<&str>`
  - `pub fn free_names(s: &Stmt) -> Vec<String>` — the names an expression uses, minus a
    function's own parameters, deduplicated, in first-use order; Task 7 builds the dependency
    graph from it
  - implicit multiplication between adjacent factors (`2 P`, `P A`) is allowed
- **Limits:** source above 16 kB is `TooComplex`; expression depth is limited to 64, beyond
  which it is `TooComplex`.

- [ ] **Step 1: Write failing tests:**
  - `define_with_units` — `"P := 1500 lbf"` gives `Define{name:"P", params:[], body: Binary(Mul, Num(1500.0), Name("lbf")), show: None}`
  - `define_and_show` — `"σ := P/A_b ="` gives `show: Some(Show{unit: None})`
  - `eval_with_unit` — `"σ = ? ksi"` and `"σ = ksi"` both give `Eval{.., show: Show{unit: Some("ksi")}}`
  - `function_def` — `"f(x, y) := x^2 + y"` gives params `["x","y"]`
  - `subscripted_greek` — `"σ_allow := 20 ksi"` gives name `"σ_allow"`
  - `precedence` — `"a + b*c^2"` gives the expected tree, `^` right-associative
  - `unary_minus_binds_looser_than_pow` — `"-x^2"` gives `Neg(Pow(x,2))`
  - `free_names_excludes_params` — `free_names(parse("f(x) := x*k + g(x)"))` gives
    `["k", "g"]`; `defined_name` of it is `Some("f")`
  - `hostile_sources_error_not_panic`:
    - `"(".repeat(200)` gives `TooComplex`
    - `"x:=" + "1+".repeat(60_000)` gives `TooComplex`
    - `":= ="` gives `Syntax`
    - `"\u{0}"` gives `Syntax`
- [ ] **Step 2: Run them and confirm they fail:** `cargo test -p wordcraft-calc`.
- [ ] **Step 3: Implement `ast.rs` and `parse.rs`** as a Pratt parser with an explicit depth
  counter.
- [ ] **Step 4: Run the tests and confirm they pass.**
- [ ] **Step 5: Commit**: `git add Cargo.toml Cargo.lock crates/calc && git commit -m "calc: calculation language parser"`

### Task 5: `wordcraft-calc` — evaluator and functions

**Files:**
- Create: `crates/calc/src/eval.rs`, `crates/calc/src/funcs.rs`
- Test: `crates/calc/src/eval/tests.rs`

**Interfaces:**
- Consumes: `Stmt`, `Expr`, `CalcError` (Task 4); `Quantity`, `parse_unit`, `lookup` (Tasks 1–2).
- Produces:
  - `pub struct Env` with `Env::new()`, `fn define_var(&mut self, name: &str, q: Quantity)`,
    `fn define_fn(&mut self, name: &str, params: Vec<String>, body: Expr)`,
    `fn mark_failed(&mut self, name: &str)` (later uses give `Upstream`), and
    `fn names(&self) -> Vec<String>`
  - `pub fn eval_expr(env: &Env, e: &Expr) -> Result<Quantity, CalcError>`
  - `pub fn run_stmt(env: &mut Env, s: &Stmt) -> Result<Option<Quantity>, CalcError>` returns
    `Some(value)` when the statement shows a result
- **Also produces:** `pub fn check_name(name: &str) -> Result<(), CalcError>`, which gives
  `ReservedName` when the name is a unit (including prefixed forms such as `mm` or `kN`), a
  built-in function, or `π`/`pi`/`e`. `run_stmt` calls it for every `Define` name and function
  parameter. The env does not decide duplicates; Task 7 does, since only it sees the whole
  document.
- **Name resolution order:** function parameter in scope, then user variable, then the constants
  `π`/`pi` and `e`, then a unit (`lookup`), otherwise `Undefined`. Units resolve to
  `Quantity { value: factor, dim }`. An offset unit (`°F`) used anywhere except a lone
  `number unit` literal is `AffineTemperature`.
- **User functions:** evaluated in a child env holding the params. Recursion depth is limited to
  32 (`TooComplex`).
- **Built-ins (`funcs.rs`)** use std f64:
  - `sqrt`, `abs`
  - `exp`, `ln`, `log(x)` base 10, `log(x, b)` — argument dimensionless
  - `sin`, `cos`, `tan` — argument dimensionless or angle
  - `asin`, `acos`, `atan`, `atan2(y, x)` — `atan2` needs y and x of the same dim; all return
    `Dim::ANGLE`
  - `sinh`, `cosh`, `tanh`
  - `min`, `max` — variadic, same dim
  - `round(x)`, `round(x, n)`, `floor`, `ceil` — dimensionless
  - `mod(a, b)` — same dim
  - wrong arity is `Arity`; any non-finite result is `NonFinite`

- [ ] **Step 1: Write failing tests:**
  - `stress_example` — run `"P := 1500 lbf"`, `"A_b := 2.5 in^2"`, `"σ := P/A_b ="`; the shown
    quantity formatted in US customary is `("600","psi")`
  - `mixed_systems` — `"L := 1 in + 25.4 mm ="` gives 0.0508 m within 1e-12
  - `unit_mismatch_message` — `"x := 1 in + 1 lbf ="` gives `Err(CalcError::Unit(UnitError::Mismatch{..}))`
  - `undefined_message` — `"y := z + 1 ="` with an empty env gives `Err(CalcError::Undefined("z"))`,
    and its `to_string()` is `"z isn't defined"`
  - `user_function` — `"f(x, y) := x^2 + y"`, then `"f(3 m, 1 m^2) ="` gives 10 m²
  - `unit_names_cannot_be_variables` — `"m := 5 kg"`, `"A := 2 in^2"`, `"kN := 1"`, `"sin := 1"`
    and `"f(m) := m"` each give `Err(ReservedName(..))`; `"A_b := 2 in^2"` is OK, and `"2 m ="`
    still gives 2 m
  - `trig_takes_angles` — `"sin(30 deg) ="` gives 0.5 within 1e-12; `"sin(1 m) ="` gives
    `NotDimensionless`
  - `non_finite_results_are_errors` — `"10^400 ="`, `"1/0 ="` and `"sqrt(-1) ="` all give
    `NonFinite`
  - `recursion_bound` — `"f(x) := f(x)"`, then `"f(1) ="` gives `TooComplex`
  - `upstream` — `mark_failed("a")`, then `"b := a + 1 ="` gives `Upstream`
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement `eval.rs` and `funcs.rs`.**
- [ ] **Step 4: Run the tests and confirm they pass:** `cargo test -p wordcraft-calc`.
- [ ] **Step 5: Commit**: `git commit -m "calc: evaluator with unit-checked functions"`

### Task 6: Document model — calculation payload, settings, quoted values

**Files:**
- Create: `crates/doc/src/calc.rs` (types only; `wordcraft-doc` does not depend on `wordcraft-calc`)
- Modify: `crates/doc/src/para/inline_object.rs:164-171`: add a field to `InlineObject::Equation`:
  `#[serde(default, skip_serializing_if = "Option::is_none")] calc: Option<Box<crate::calc::CalcRegion>>`
- Modify: `crates/doc/src/lib.rs`
  - add `pub mod calc;`
  - add a field next to `fit_state` (line ~151):
    `pub calc_settings: Option<calc::CalcSettings>`
- Modify: `crates/doc/src/document.rs` (constructor ~81 and destructure ~160) and every other
  exhaustive pattern the compiler reports. Existing `Equation { .. }` patterns with `..` need no
  change.
- Test: `crates/doc/src/calc/tests.rs`

**Interfaces:**
- Produces (all `Clone, Debug, PartialEq, Serialize, Deserialize`):
  - `pub struct CalcRegion { pub id: u64, pub source: String, pub unit: Option<String>, pub precision: Option<u8>, pub state: CalcState }`
  - `pub enum CalcState { #[default] Fresh, Stale, Error(String), EditedOutside }`
  - `pub enum UnitSystem { #[default] UsCustomary, Si }` — local to `wordcraft-doc`, which does
    not depend on `craft-units`; `wordcraft-calc` maps it to `craft_units::UnitSystem`
  - `pub struct CalcSettings { pub system: UnitSystem, pub precision: u8, pub manual: bool }` —
    `Default` is `{ UsCustomary, 3, false }`
  - `pub const CALC_FIELD: &str = "WCCALC";`
  - `pub fn calc_field_instr(var: &str, unit: Option<&str>, precision: Option<u8>) -> String`
    gives `WCCALC σ \u ksi \p 2` (switches omitted when `None`)
  - `pub fn parse_calc_field(instr: &str) -> Option<(String, Option<String>, Option<u8>)>`
- A quoted value is `InlineObject::Field { instr: calc_field_instr(..), result, locked: true, .. }`.

- [ ] **Step 1: Write failing tests:**
  - `instr_round_trip` — `parse_calc_field(&calc_field_instr("σ_max", Some("ksi"), Some(2)))`
    gives `Some(("σ_max".into(), Some("ksi".into()), Some(2)))`; a non-WCCALC instr gives `None`;
    a 10 kB instr gives `None`
  - `plain_equation_serde_unchanged` — serializing an `Equation` with `calc: None` produces JSON
    with no `calc` key
  - `calc_equation_serde_round_trip` — through `serde_json`
  - `calc_survives_native_format` — in `crates/docbin` tests, beside its existing round-trip
    tests: a document with a calc equation and `calc_settings = Some(..)` saves and reopens equal
- [ ] **Step 2: Run them and confirm they fail:** `cargo test -p wordcraft-doc calc`.
- [ ] **Step 3: Implement**, fixing the compile errors across the workspace with
  `cargo check --workspace`.
- [ ] **Step 4: Run** `cargo test -p wordcraft-doc` and `cargo check --workspace`. Expected:
  PASS, no errors.
- [ ] **Step 5: Commit**: `git commit -m "doc: calculation payload on equations, calc settings, WCCALC fields"`

### Task 7: Recalculation pass

**Files:**
- Create: `crates/calc/src/recalc.rs`, `crates/calc/src/render.rs`
- Test: `crates/calc/tests/recalc.rs`, including the `#[ignore]` timing test below

**Interfaces:**
- Consumes: Tasks 4–6.
- Produces:
  - `pub struct RecalcReport { pub regions: usize, pub errors: usize, pub reassigned_ids: usize }`
  - `pub fn recalc(doc: &mut wordcraft_doc::Document, page_of: &dyn Fn(u64) -> Option<u32>) -> RecalcReport`
    — `page_of` maps a region id to its page number, for duplicate messages; the engine passes
    a layout lookup, and tests pass `&|_| None`. The pass:
    1. **Collects** calc equations from the body story in document order (paragraphs top to
       bottom, objects left to right, table cells row by row), skipping other stories and
       tracked-deleted content. It keeps the first of any duplicate region `id`s and gives the
       rest the next unused id (max + 1).
    2. **Parses** each region's `source`. A parse error sets `Error` and defines nothing.
    3. **Indexes names:** the first region in document order that defines a name owns it. Every
       later region defining the same name gets
       `Error(Duplicate { name, page: page_of(owner_id) })` and defines nothing.
    4. **Builds the dependency graph** from `free_names` (owners only) and evaluates in
       topological order (Kahn's algorithm). Regions on a cycle get `Error(Circular(..))` and
       their names are marked failed. Dependents of failed names get `Upstream`. Eval-only
       regions (`x =`) run after the definitions they use.
    5. **Writes each region's display** into `linear` and `math` (via
       `wordcraft_doc::math::parse_linear`) using `render::display(...)`.
    6. **Updates every WCCALC field's `result`** from the final env, wherever the field sits.
  - `pub fn defined_names(doc: &Document) -> Vec<(String, u64)>` — every valid (owning) name
    with its region id, sorted by name
  - `pub fn check_new_name(doc: &Document, name: &str, editing: Option<u64>) -> Result<(), wordcraft_calc::CalcError>`
    — `ReservedName` via `check_name`, or `Duplicate` when another region (not `editing`)
    already owns `name`. Task 8 uses it for the unique-name prompt.
  - `pub fn mark_stale(doc: &mut Document)` sets every `Fresh` region to `Stale` (manual mode)
  - `pub fn render::display(stmt_src: &str, shown: Option<&Formatted>, state: &CalcState) -> String`
    builds linear-format text:
    - definition: `σ≔P/A_b`
    - shown result: append `=600 "psi"` (unit as upright text)
    - error: append ` ⚠` (layout already supports colour on math runs; set the error run's colour
      to the document's error red `Rgb(0xC0,0x00,0x00)` in `math` after parsing)
    - stale: prefix the result with `≈`, plus a grey run; never the plain form
  - Nothing here panics; `recalc` on any document returns a report.

- [ ] **Step 1: Write failing tests** (build docs with `Document::default()` plus the existing
  test helpers used in `crates/doc/src/tests.rs`):
  - `recalculates_in_order` — three regions (`P`, `A_b`, `σ = `) produce `linear` ending
    `=600 "psi"`
  - `use_before_define_works` — `σ := P/A_b =` placed **above** `P` and `A_b` shows `600 psi`
  - `duplicate_definition_is_error` — `x:=1`, `x =`, `x:=2`: the first `x` is `Fresh`, the second
    is `Error` with message `"x is already defined elsewhere in the document — choose a unique name"`,
    and `x =` shows 1
  - `circular_definitions_error` — `a := b + 1`, `b := a/2` both give `Error("a, b depend on each other")`,
    and `c := a =` gives `Upstream`
  - `pasted_unit_name_definition_is_error` — a region with source `A := 2 in^2` (as if pasted)
    gives `Error(ReservedName)`, and `2 A =` elsewhere shows `2 A`
  - `quoted_value_updates` — a WCCALC field for `σ` gets result `"600 psi"`, both after the
    regions and in a paragraph **before** them; with `\u ksi \p 2` it gets `"0.6 ksi"`
  - `check_new_name_reports` — with `P` defined: `check_new_name(doc, "P", None)` gives
    `Duplicate`; `check_new_name(doc, "P", Some(p_id))` is OK; `"m"` gives `ReservedName`;
    `"Q"` is OK
  - `table_cells_in_row_order`
  - `duplicate_ids_are_reassigned` — two regions with id 7 keep the first as 7, the second
    becomes 8, and `reassigned_ids == 1`
  - `tracked_deleted_region_is_ignored`
  - `plain_equations_untouched` — a `calc: None` equation is byte-for-byte unchanged
  - `perf_2000_regions` (`#[ignore]`, run with `--release -- --ignored`) — 2,000 chained regions,
    then change the first input and `recalc` in < 100 ms
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement `recalc.rs` and `render.rs`.** A full-document pass is acceptable if
  `perf_2000_regions` passes. Add incremental re-evaluation only if it doesn't.
- [ ] **Step 4: Run** `cargo test -p wordcraft-calc` and
  `cargo test -p wordcraft-calc --release -- --ignored perf_2000_regions`. Expected: PASS.
- [ ] **Step 5: Commit**: `git commit -m "calc: dependency-order recalculation, unique names and display"`

### Task 8: Engine commands and automatic recalculation

**Files:**
- Create: `crates/engine/src/cmd/calc.rs`
- Modify:
  - `crates/engine/src/cmd.rs` — register the module the way `equation` is registered
  - `crates/engine/src/catalog.rs` — add the ribbon rows next to `Insert|Symbols|Equation`
  - `crates/engine/Cargo.toml` — add dep `wordcraft-calc`
  - `crates/engine/src/session/run.rs` — after a successful mutating command in `run_command`
- Test: `crates/engine/src/cmd/calc/tests.rs`

**Interfaces:**
- Consumes: `recalc`, `mark_stale`, `defined_names`, `check_new_name` (Task 7); `CalcRegion`, `CalcSettings`, `calc_field_instr`
  (Task 6).
- **Produces these command ids:**

  | Command id | Params | Effect / result |
  |---|---|---|
  | `insert.calculation` | `{ "source": string, "display": bool }` | inserts a calc equation with a fresh id; returns `{ "id": u64 }` |
  | `calc.setSource` | `{ "id": u64, "source": string }` | replaces a region's source |
  | `insert.calcValue` | `{ "name": string, "unit"?: string, "precision"?: u8 }` | inserts a locked WCCALC field |
  | `calc.names` | `{}` | returns every valid name in the document (`defined_names`), for the UI picker |
  | `calc.checkName` | `{ "name": string, "id"?: u64 }` | returns `{ "ok": true }`, or `{ "ok": false, "reason": "duplicate"\|"reserved", "message": string, "existing"?: u64 }`, via `check_new_name` |

  `insert.calculation` and `calc.setSource` also run `check_new_name` on the source's defined
  name. On failure they return a `CmdError` whose message is the `CalcError` text and change
  nothing. The UI checks first with `calc.checkName`, so this is the backstop for the CLI, MCP and
  agents.
  | `calc.recalculate` | `{}` | returns a `RecalcReport` as JSON |
  | `calc.settings` | `{ "system"?: "us"\|"si", "precision"?: u8, "manual"?: bool }` | sets document calc settings |
  | `calc.resolveEdited` | `{ "id": u64, "keep": "source"\|"word" }` | `"source"` re-renders from source; `"word"` sets `calc = None`, leaving a plain equation |

- **Catalog rows:**
  - `Insert|Symbols|Calculation|insert.calculation`
  - `Insert|Symbols|Calculated Value|insert.calcValue`
  - `Formulas|Calculation|Recalculate|calc.recalculate`
  - `Formulas|Calculation|Options|calc.settings`

  Use the tab name the catalog already has for references if `Formulas` doesn't exist; check the
  file. Shortcut: `Mod+Shift+=` for `insert.calculation`, if unbound in `keymap.rs`.
- **Auto-recalculation:** after any successful command whose spec has `mutates == true`, call
  `recalc` when `!settings.manual`, otherwise `mark_stale`. Do this inside the same undo step, so
  undo restores results too.

- [ ] **Step 1: Write failing tests** (use the `run(&mut s, id, json!(…))` helper pattern from
  `crates/engine/src/tests.rs:45`):
  - `insert_and_recalculate` — insert `P`, `A_b`, `σ := P/A_b =`; the third equation's `linear` ends
    `=600 "psi"`
  - `editing_input_updates_result` — `calc.setSource` on P to `3000 lbf` gives `=1200 "psi"`
  - `duplicate_insert_is_refused` — `insert.calculation {source: "P := 2 lbf"}` with P already
    defined returns an error containing `"already defined"`, and the document is unchanged;
    `calc.checkName {name:"P"}` gives `ok:false, reason:"duplicate"`; `{name:"m"}` gives
    `reason:"reserved"`; `{name:"P", id:<P's id>}` gives `ok:true`
  - `duplicate_message_names_page` — with the engine's layout page lookup, the pasted-duplicate
    error message contains `"on page 1"`
  - `undo_restores_results` — undo after that gives `=600 "psi"` again
  - `manual_mode_marks_stale` — after `calc.settings {manual:true}` and an edit, the state is
    `Stale` and `linear` contains `≈`; after `calc.recalculate` it is `Fresh`
  - `resolve_edited_keep_word` — the region becomes a plain equation, and `calc.names` no longer
    lists its name
  - `catalog_has_calc_commands` — the four ids are resolvable via the catalog lookup used in
    existing catalog tests
- [ ] **Step 2: Run them and confirm they fail:** `cargo test -p wordcraft-engine calc`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p wordcraft-engine`. Expected: PASS, including all existing
  tests.
- [ ] **Step 5: Commit**: `git commit -m "engine: calculation commands and automatic recalculation"`

### Task 9: `.docx` round trip

**Files:**
- Create: `crates/docx/src/calc_store.rs`, modelled on `crates/docx/src/fit.rs`:
  - `PART = "customxml/wordcraftcalc.xml"`, `ROOT = "wordcraftCalc"`, `NS = "urn:wordcraft:calc"`,
    `KIND = "wordcraft-calc-v1"`
  - JSON payload `{ settings: CalcSettings, regions: [{ id, source, unit, precision, fingerprint }] }`
- Modify: `crates/doc/src/calc.rs` — add `CalcStore` and `StoredRegion` (see Interfaces)
- Create: `crates/calc/src/docx.rs` with:
  - `pub fn export(doc: &Document) -> (Document, CalcStore)`
  - `pub fn import(doc: &mut Document, store: Option<CalcStore>)`
- Modify: `crates/docx/src/model_bridge.rs`
  - write path, beside the `fit_state` block at line ~404: write the calc store part and its
    manifest/root entries, factoring the shared manifest code out of the fit block into one
    helper used by both
  - read path, beside line ~179: read and remove the part
- Modify: the `.docx` save/open call sites in the engine (find them with
  `grep -rn "wordcraft_docx::" crates/engine/src`) to call `export` before writing and `import`
  after reading. Keep
  `wordcraft-docx` free of `wordcraft-calc`; the store type lives in `wordcraft-doc::calc`.
- Test: `crates/docx/tests/it/calculations.rs` (register it in `crates/docx/tests/it/main.rs`)

**Interfaces:**
- Produces: `pub struct CalcStore { pub settings: CalcSettings, pub regions: Vec<StoredRegion> }`
  and `pub struct StoredRegion { pub id: u64, pub source: String, pub unit: Option<String>, pub precision: Option<u8>, pub fingerprint: u64 }`,
  both in `crates/doc/src/calc.rs`.
- **`export`:**
  - wraps each calc equation in `ControlStart { control: ContentControl { tag: "wordcraft-calc:<id>", kind: rich text, .. } }` … `ControlEnd`, sets `calc = None`
    on the copy, and records the fingerprint
  - the fingerprint is FNV-1a 64 of the equation's `linear` text after rendering
  - WCCALC fields stay as they are (they are already locked fields)
- **`import`:**
  - for each control whose tag starts with `wordcraft-calc:`, looks up the stored region
  - if the fingerprint of the current `linear` matches, it restores `calc` and removes the control
    markers
  - if it differs, it restores `calc` with `state = EditedOutside` and keeps Word's `linear`/`math`
  - a stored region with no matching control is dropped
  - untagged equations are left alone
  - then it calls `recalc(doc, &|_| None)` **except** on `EditedOutside` regions, whose display is preserved

- [ ] **Step 1: Write failing tests:**
  - `calc_round_trips_through_docx` — save a doc with 3 regions and a WCCALC field; reopen;
    regions are live with the same sources and `state == Fresh`; the field result is unchanged
  - `word_sees_plain_equations` — in the saved `word/document.xml`, each `m:oMath` sits inside
    `w:sdt` with `w:tag w:val="wordcraft-calc:<id>"`; the WCCALC field has `w:fldLock="1"`; and
    `customXml/wordcraftcalc.xml` (case-insensitive) exists
  - `edited_in_word_is_flagged` — change the `m:t` text of one equation inside the saved zip
    (string replace), reopen; that region is `EditedOutside`, its text is Word's version, and the
    others are `Fresh`
  - `deleted_in_word_makes_dependents_error` — remove the `w:sdt` defining `A_b`; reopen; `σ` is
    `Error("A_b isn't defined")`
  - `no_store_means_plain` — a docx with a tagged control but no store part opens as a plain
    equation
  - `hostile_store_ignored` — a store with malformed JSON or 1e6 regions opens without panic,
    treated as no store
- [ ] **Step 2: Run them and confirm they fail:** `cargo test -p wordcraft-docx calculations`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p wordcraft-docx` and `cargo test -p wordcraft-calc`.
  Expected: PASS.
- [ ] **Step 5: Commit**: `git commit -m "docx: calculations round trip as tagged equations plus hidden source"`

### Task 10: User interface

**Files:**
- Create: `crates/ui-egui/src/calc_ui.rs`
- Modify:
  - `crates/ui-egui/src/lib.rs` (module)
  - `crates/ui-egui/src/equation_tab.rs` (when the selected equation has `calc`, show a source
    box instead of the built-up editor)
  - `crates/ui-egui/src/ribbon.rs` (new buttons come from the catalog rows of Task 8; wire
    their handlers)
- Test: `crates/ui-egui/src/tests.rs`, following its existing headless-UI test pattern

**Behaviour:**
- **Insert › Calculation** opens a single-line source box at the caret. Enter commits via
  `insert.calculation`; Esc cancels.
- **Unique-name prompt:** before committing (insert or edit), the box calls `calc.checkName`
  with the defined name.
  - If it isn't OK, a prompt shows the message ("P is already defined on page 4 — choose a
    unique name" / "A is a unit, function or constant name — choose another name") with:
    - a "New name" field, pre-filled with the name plus `_1`, `_2` and so on (the first that
      passes `calc.checkName`)
    - **Rename**, which rewrites the defined name in the source and commits
    - **Go to existing** (duplicates only), which moves the caret to the owning region
    - **Cancel**, which returns to the box
  - Nothing is inserted under a clashing name.
- **Typing `:` then `=`** in the box shows `≔`. The box keeps the ASCII `:=`; the parser accepts
  both.
- **Double-clicking a calculation** opens the box with its `source`; committing runs
  `calc.setSource`.
- **Insert › Calculated Value** opens a picker filled from `calc.names`, with optional unit and
  precision fields, and runs `insert.calcValue`.
- **Calculation Options dialog:** unit system radio, precision spinner 0–15, and a manual
  recalculation checkbox. It runs `calc.settings`.
- **An `EditedOutside` region** shows a small bar with "Restore from calculation" and "Keep Word's
  equation". These run `calc.resolveEdited`.
- **The error message** shows as a tooltip on hovering an errored region.

- [ ] **Step 1: Write failing UI tests:**
  - `calc_box_commits_source` — simulate typing `P := 1500 lbf` and Enter; the doc has one calc
    region with that source
  - `double_click_opens_source`
  - `duplicate_name_prompts_for_rename` — with `P` defined, typing `P := 2 lbf` and Enter shows
    the prompt pre-filled `P_1`; clicking Rename inserts `P_1 := 2 lbf`, and the original P is
    untouched
  - `unit_name_prompts_for_rename` — typing `A := 2 in^2` shows the reserved-name message
  - `options_dialog_sets_settings`
- [ ] **Step 2: Run them and confirm they fail:** `cargo test -p wordcraft-ui-egui calc`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p wordcraft-ui-egui`. Expected: PASS.
- [ ] **Step 5: Manual check.**
  - Run `cargo run --release -p wordcraft`.
  - Insert `P := 1500 lbf`, `A_b := 2.5 in^2` and `σ := P/A_b =`; confirm `σ = 600 psi` renders
    built up.
  - Edit P and watch σ update.
  - Take a screenshot with the repo's `ui_shot` example if useful.
- [ ] **Step 6: Commit**: `git commit -m "ui: insert and edit calculations, calculated values, options"`

### Task 11: Sample, dependency pin, full verification

**Files:**
- Create: `crates/calc/tests/flange_bolt_sample.rs` — an end-to-end engine test building a bolt
  stress calculation:
  - inputs: bolt count, diameter, preload in lbf, allowable in ksi
  - a user function `A_t(d) := π/4·d^2`
  - a quoted value in a sentence
  - checks: saving to docx and reopening; changing the preload updates every result and the
    sentence
- Create: `docs/calculations.md` — a short user guide: notation, units, display units,
  Word round trip, edited-in-Word handling
- Modify: `Cargo.toml` / `crates/calc/Cargo.toml`: switch `craft-units` from the path to
  `{ git = "https://github.com/alanrathbun/craft-libs", rev = "<feature/craft-units HEAD>", package = "craft-units" }`
  **only after the user approves pushing** the craft-libs branch. Until then, leave the path
  dependency and note it in the hand-off.

- [ ] **Step 1: Write the sample test, then run it and confirm it passes.**
- [ ] **Step 2: Run the full gate in WordCraft:**
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`

  Expected: all clean.
- [ ] **Step 3: Run the same gate for `craft-units`** in craft-libs: `cargo fmt --check`,
  `cargo clippy -p craft-units --all-targets -- -D warnings`, `cargo test -p craft-units`.
- [ ] **Step 4: Manual Word check (user):** save the sample as `.docx` and open it in
  Microsoft Word. Equations display correctly and nothing looks broken.
- [ ] **Step 5: Commit**: `git commit -m "calc: bolt-stress sample, user guide"`
