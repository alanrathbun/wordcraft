# Calculation Vectors, Matrices and Data Tables Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** WordCraft calculations hold vectors and matrices of unit-checked values, combine them
element by element, index and range over them, and read named column vectors from ordinary
document tables. All of this survives a `.docx` round trip.

**Architecture:**

- **Values:** the evaluator's value becomes "one value or a grid of values". A grid is
  rows × cols, row-major, and each element is today's `Val` (quantity plus temperature kind).
  Element-wise operations map the existing scalar `apply` over elements, so every unit rule holds
  per element.
- **Data tables:** each column of a marked table becomes a *synthetic definition* in the
  recalculation pass, `D_o := [0.84 in; 1.05 in; …]`. It is built from the cell text, so
  ownership, duplicates, cycles, upstream errors and work budgets all reuse the Stage 1
  machinery unchanged.
- **The marker** is a new optional `calc` field on `wordcraft_doc::Table`. In `.docx` it is a
  content control around the table plus an entry in the existing hidden calculation store.

**Tech Stack:** Rust 2024 in the WordCraft workspace. Crates are `wordcraft-calc`, `wordcraft-doc`,
`wordcraft-engine`, `wordcraft-docx` and `wordcraft-ui-egui`, plus `craft-units` (pinned, unchanged).

**Spec:** `docs/superpowers/specs/2026-11-15-calculation-vectors-and-tables-design.md`. Read it
with this plan. It builds on Stage 1, `docs/superpowers/specs/2026-11-14-inline-calculations-design.md`.

**Repo and branch:** `/home/alan/arath/storytold/wordcraft`, branch `feature/inline-calculations`.
Commit there; push nothing without the user's OK.

## Global Constraints

- **`AGENTS.md` rules apply:**
  - no `unsafe`;
  - no panics in non-test code (the calc crate denies `unwrap`/`expect`/`panic`/`unreachable`);
  - bounded recursion and allocation;
  - all source, table text and stored data are hostile input.
- **No agent attribution lines in commits** (storytold rule; it overrides the harness reminder).
- **Before each commit:** run `cargo fmt --check` and
  `cargo clippy -p <touched crates> --all-targets -- -D warnings`. Task 10 runs the full
  workspace test and clippy.
- **Seven tests fail before this work** because craft-i18n data and fonts are missing:
  - engine `swiss_german_and_portugal…` and `review_language_validates…`;
  - ui-egui `rtl_template_titles_are_right_aligned` and `interface_symbols_have_glyphs`;
  - three layout hyphenation tests.

  They are not regressions; every other test must pass.
- **Values pinned by the spec:**
  - arrays hold at most **10,000 elements**;
  - display shows at most **50 rows**, then a `⋮` row and "(N rows)";
  - element numbering `origin` is **0** (default) or 1;
  - lookup tolerance is relative **1e-12**, after unit conversion.
- **Stage 1 documents open unchanged.** Every new serde field has `#[serde(default)]` and is
  skipped when empty or default, so an old file means what it meant.
- **Message wording pinned by the spec, used verbatim:**
  - `shapes don't match: 3 rows × 1 vs 4 rows × 1`
  - `element 2: …`
  - `D_o row 4 isn't a number`
  - `depends on table data with an error`
  - `1.6 in is outside NPS (0.5 in to 1.5 in)`
  - `Calculation data was edited in Word in N tables; results were recalculated`
- **Commands, menus and labels:**
  - **Menu items:** **Use as Calculation Data** / **Stop Using as Calculation Data** on a table's
    right-click menu.
  - **Options dialog:** an "Arrays count from" choice of 0 (Mathcad) or 1.

## Review Focus

1. **`..` must not be read as decimal points.** `i := 0..5`, `x := 1.5..3` and
   `x := 1, 1.5..3` lex as ranges; `1.5` alone is still a number. Test: Task 2
   `range_dots_are_not_decimal_points`.
2. **Table units under the `g` setting.** A heading `m (g)` or a cell `5 g` in a document where
   `g` is gravity is an error naming `gm`, never silently grams. Table cells don't go through the
   evaluator's name lookup, so this needs its own check. Test: Task 6
   `table_g_unit_follows_the_g_setting`.
3. **A copied data table.** Paste duplicates its marker id and its column names. The copy gets a
   fresh id, and its columns are duplicate-name errors. Neither table goes blank, and the first
   keeps working. Test: Task 6 `copied_data_table_is_a_duplicate`.
4. **Empty or junk cells.** A blank cell, a row added in Word and left empty, a thousands
   separator (`1,500` or `1 500`), and `1-1/2-1` are all errors naming the column and row. None
   is ever 0, and `1-1/2` is never 0.5. Tests: Task 6 `cell_values_are_strict`, Task 8
   `table_edited_in_word_is_used_and_noticed`.
5. **Bad indexes.** `D[1.5]`, `D[1 in]`, `D[-1]` and `D[3]` on a 3-vector with origin 0, and
   `M[1]` on a matrix, are errors naming the valid range. None picks a neighbouring element.
   Test: Task 3 `index_must_be_a_whole_number_in_range`.

---

### Task 1: Grid values and element-wise arithmetic

**Files:**
- Create: `crates/calc/src/array.rs` (grid type, broadcasting, element naming)
- Modify: `crates/calc/src/eval.rs` (`Entry::Var`, `Frame.values`, `eval`, `apply` callers, `call` for built-ins, `run_stmt*`, `eval_expr`), `crates/calc/src/lib.rs` (new `CalcError` variants, exports), `crates/calc/src/recalc.rs` (compile against the new outcome type only; scalar behaviour unchanged)
- Test: `crates/calc/src/eval/tests.rs`

**Interfaces:**
- **Produces in `lib.rs`:**
  - `pub enum Value { One(Quantity), Grid { rows: usize, cols: usize, items: Vec<Quantity> } }`
    (row-major), the public outcome type.
  - `run_stmt(env, s) -> Result<Option<Value>, CalcError>` and
    `eval_expr(env, e) -> Result<Value, CalcError>` (were `Quantity`).
- **Produces in `array.rs`, `pub(crate)`:**
  - `struct Grid { rows: usize, cols: usize, items: Vec<Val> }` with
    `Grid::new(rows, cols, items) -> Result<Grid, CalcError>`. It refuses more than
    `MAX_ELEMENTS = 10_000` and a size that doesn't match.
  - `enum Data { One(Val), Grid(Grid) }` with `Data::value(&self) -> Value` and
    `Data::shape(&self) -> Option<(usize, usize)>`.
  - `fn zip(a: &Data, b: &Data, origin: u8, f: impl FnMut(Val, Val) -> Result<Val, CalcError>) -> Result<Data, CalcError>`
    broadcasts a single value over a grid. Same shapes go element by element; other shapes give
    `Shape`.
  - `fn map(a: &Data, origin: u8, f: impl FnMut(Val) -> Result<Val, CalcError>) -> Result<Data, CalcError>`.
  - `fn element_name(rows, cols, k, origin) -> String`. A vector element is named `2` (one index;
    a row vector counts along its columns), and a matrix element `[1, 2]`, both counted from
    `origin`.
- **Produces in `Env`:**
  - `pub fn define_array(&mut self, name: &str, rows: usize, cols: usize, items: Vec<Quantity>) -> Result<(), CalcError>`;
  - `pub fn set_origin(&mut self, origin: u8)` (0 or 1; anything else is 0);
  - `pub(crate) fn data(&self, name: &str) -> Option<&Data>`, which replaces `value()`.
- **Produces in `CalcError`:**
  - `Shape { left: (usize, usize), right: (usize, usize) }` →
    `"shapes don't match: {l0} rows × {l1} vs {r0} rows × {r1}"`;
  - `Element { at: String, error: Box<CalcError> }` → `"element {at}: {error}"`;
  - `TooLarge` → `"more than 10,000 elements"`;
  - `IndexRange(String)` → `"{0}"`;
  - `UpstreamTable` → `"depends on table data with an error"` (raised from Task 6).
- `run_stmt_in` returns `Result<Option<Data>, CalcError>`. `recalc` takes `Data::One` exactly as
  it took a `Quantity` before; Task 5 adds grid display.

- [ ] **Step 1: Write failing tests**, using `Env::define_array`, since the literal syntax arrives in Task 2:
  - `vectors_add_element_wise`: `D = [1, 2, 3] in` and `t = [0.1, 0.2, 0.3] in` give
    `D + t` = `Grid{3,1}` of 1.1, 2.2, 3.3 in (compared in SI).
  - `single_value_broadcasts`: `2·D` and `D/2` are element-wise; `D^2` has dimension L².
  - `shape_mismatch_is_named`: a 3×1 plus a 4×1 gives
    `Err(Shape { left: (3,1), right: (4,1) })`, displaying
    `"shapes don't match: 3 rows × 1 vs 4 rows × 1"`.
  - `element_error_names_the_element`: `[1 in, 2 lbf] + [1 in, 1 in]` gives a message starting
    `"element 1: "`, and with origin 1 starting `"element 2: "`.
  - `builtins_apply_element_wise`: `sqrt([4, 9])` = `[2, 3]`; `round([1.26, 2.34], 1)` =
    `[1.3, 2.3]`.
  - `user_functions_apply_element_wise`: `f(x) := x^2 + 1` gives `f([1, 2])` = `[2, 5]`.
  - `arrays_are_bounded`: `define_array` of 10,001 items is `Err(TooLarge)`, as is any operation
    whose result would exceed 10,000.
  - `element_operations_count_against_the_budget`: an element-wise op on a 10,000-vector charges
    10,000 steps. With `Pass::new(5_000)` it gives `TooMuchWork`.
- [ ] **Step 2: Run** `cargo test -p wordcraft-calc eval::tests`. Expect a compile failure
  (`define_array` missing).
- [ ] **Step 3: Implement.** Add `Pass::charge(&self, n: u64) -> Result<(), CalcError>` and call it
  once per element in `zip`/`map`. `Frame.values` becomes `&[Data]`. `min`/`max` stay scalar here;
  Task 4 makes them reductions. Every other scalar built-in broadcasts over all its arguments with
  `zip`/`map`. Temperature kinds are kept per element exactly as `eval`'s `Binary` arm does now.
  The offset-literal checks (`refuse_offset_operand`, `offset_literal`) apply to the syntax and
  are unchanged.
- [ ] **Step 4: Run** `cargo test -p wordcraft-calc`. Expect all to pass, Stage 1 tests included.
- [ ] **Step 5: Commit** `calc: grid values with element-wise arithmetic`.

### Task 2: Syntax for literals, indexing, ranges and indexed definitions

**Files:**
- Modify: `crates/calc/src/ast.rs`, `crates/calc/src/parse.rs`, `crates/calc/src/render.rs` (`write_stmt`/`write_expr`, `prec`)
- Test: `crates/calc/src/parse/tests.rs`, `crates/calc/tests/recalc.rs` (source display only)

**Interfaces:**
- **Produces in the AST:**
  - `Expr::Matrix { rows: usize, cols: usize, items: Vec<Expr> }`, row-major;
  - `Expr::Index(String, Vec<Expr>)`, with one or two indices;
  - `Expr::Range { first: Box<Expr>, second: Option<Box<Expr>>, last: Box<Expr> }`;
  - `Stmt::Define` gains `index: Option<String>`, the range variable of `t[i] := …`.
- `free_names` walks every new node: the indexed name, index expressions, matrix items and range
  ends. For `t[i] := …` it includes `i`.
- **Lexer tokens:** `LBracket`, `RBracket`, `Semi`, `DotDot`.

- [ ] **Step 1: Write failing parser tests:**
  - `literals_parse`:
    - `[1, 2, 3]` is `Matrix{3,1}`, a column, so a one-dimensional list is always a column.
    - `[1, 2; 3, 4]` is `Matrix{2,2}`.
    - `[2 in, 50 mm]` is a two-element column of products.
    - `[0.840, 1.050] in` is `Binary(Mul, Matrix, Name("in"))`.
  - `ragged_rows_are_an_error`: `[1, 2; 3]` gives Syntax `"row 2 has 1 element; row 1 has 2"`.
  - `empty_and_nested_literals_are_errors`: `[]` and `[[1], 2]` are errors.
  - `indexing_needs_no_space`:
    - `D[2]` is `Index("D", [2])` and `M[i, j]` is `Index("M", [i, j])`;
    - `D [2]` is `Binary(Mul, D, Matrix{1,1})`;
    - `f(x)[2]` gives Syntax `"only a name can be indexed, like D[2]"`.
  - `range_dots_are_not_decimal_points`:
    - `i := 0..5` gives `Range{0, None, 5}`;
    - `x := 1.5..3` gives `Range{1.5, None, 3}`;
    - `x := 1, 1.5..3` gives `Range{1, Some(1.5), 3}`;
    - `y := 1.5` still parses as a number.
  - `ranges_are_whole_right_hand_sides`: `x := (0..5)`, `x := 2·0..5`, `0..5 =` and `f(0..5) =`
    each give Syntax `"a range like 0..5 can only be the whole right side of a definition"`.
  - `indexed_definition_head`: `t[i] := 2·D[i]` gives `Define{name: "t", index: Some("i"), …}`;
    `t[i](x) :=` is an error.
  - `free_names_see_new_nodes`: `t[i] := D_o[i]·k + [a, b]` gives free names
    `["D_o", "i", "k", "a", "b"]` in some order without duplicates. Assert as a set.
  - The existing proptest that no source panics keeps passing with `[`, `]`, `;` and `.` added to
    its alphabet.
- [ ] **Step 2: Run** `cargo test -p wordcraft-calc parse`. Expect it to fail.
- [ ] **Step 3: Implement.**
  - `[` is indexing exactly when the previous token is a `Name` ending at this token's start.
  - Number lexing stops before a `.` that is followed by another `.`.
  - A `DotDot` or a top-level `,` is accepted only by a new `Parser::range_rhs`, tried when the
    definition body is parsed.
  - Literals count toward `MAX_DEPTH` and `MAX_HEIGHT` like parentheses.
- [ ] **Step 4: Write failing render tests** in `tests/recalc.rs`. Each checks the linear source
  display of a definition with no `=`:
  - `D := [0.840, 1.050] in` shows `D≔[■(0.84@1.05)] "in"`;
  - `M := [1, 2; 3, 4]` shows `M≔[■(1&2@3&4)]`;
  - `x := D[2]` shows `x≔D_2`;
  - `y := M[i, j]` shows `y≔M_(i,j)`;
  - `z := D_o[i]` shows `z≔〖D_o〗_i`;
  - `i := 0..5` shows `i≔0..5`;
  - `t[i] := D[i]` shows `t_i≔D_i`.
- [ ] **Step 5: Implement in `render.rs`.**
  - A matrix is written `[■(` … `)]`, with `&` between columns and `@` between rows.
  - An index is a subscript; a name that already has a subscript goes inside
    `INVISIBLE_OPEN`/`INVISIBLE_CLOSE`.
  - Run `cargo test -p wordcraft-calc`; expect PASS.

  The evaluator answers the new nodes with Syntax `"not supported yet"` until Task 3. No test
  depends on that.
- [ ] **Step 6: Commit** `calc: syntax for vector and matrix literals, indexing and ranges`.

### Task 3: Evaluating literals, indexes, ranges and indexed definitions; the origin setting

**Files:**
- Modify: `crates/calc/src/eval.rs`, `crates/calc/src/array.rs`, `crates/doc/src/calc.rs` (`CalcSettings.origin`), `crates/calc/src/recalc.rs` (`evaluate` calls `env.set_origin(settings.origin)`)
- Test: `crates/calc/src/eval/tests.rs`, `crates/doc/src/calc/tests.rs`

**Interfaces:**
- **Consumes:** Task 1's `Grid`, `Data`, `zip` and `map`, and Task 2's AST.
- **Produces:** `CalcSettings.origin: u8` with `#[serde(default, skip_serializing_if = "is_zero")]`,
  where `Default` is 0.

- [ ] **Step 1: Write failing tests:**
  - `literal_evaluates`: `D := [0.840, 1.050, 1.315] in` is a 3×1 grid in inches;
    `[2 in, 50 mm]` keeps both elements as lengths.
  - `index_reads_an_element`: with origin 0, `D[0]` = 0.840 in and `M[1, 0]` = 3. With origin 1,
    `D[1]` = 0.840 in.
  - `index_must_be_a_whole_number_in_range` (Review Focus 5). Each of these is an error, and none
    picks a neighbour:
    - `D[1.5]` and `D[1 in]` give Syntax `"an index is a whole number, like D[2]"`;
    - `D[3]` and `D[-1]` give `IndexRange("index 3 is outside D (0 to 2)")`;
    - `M[1]` gives `IndexRange("M is a matrix — give a row and a column, like M[1, 0]")`;
    - `x[0]` with `x` a single value gives `IndexRange("x isn't a vector")`.
  - `ranges_evaluate`:
    - `0..5` is a 6-vector 0…5, so a range includes both ends;
    - `1, 1.5..3` gives 1, 1.5, 2, 2.5, 3;
    - `0 in, 0.5 in..1 in` gives 3 lengths;
    - `5..0` with no step counts down: 5, 4, …, 0.
  - `bad_ranges_are_errors`, each an error:
    - a zero step (`1, 1..3`);
    - a step in the wrong direction (`1, 0..3`);
    - mixed dimensions (`0..5 in`);
    - more than 10,000 elements (`0..20000`).
  - `indexed_definition_fills_a_vector`: with `i := 0..2` and `D` as above,
    `t[i] := 2·D[i]` defines a 3-vector 1.68, 2.1, 2.63 in.
  - `indexed_definition_is_strict`, each an error:
    - with `i := 1..2` and origin 0, element 0 is never set: Syntax
      `"t[0] isn't set by i — the range must cover every element"`;
    - a body giving a grid for one `i`;
    - `i` not a vector of whole numbers.
  - `origin_round_trips_and_defaults`: in the doc crate, `CalcSettings::default().origin == 0`,
    and a Stage 1 JSON settings object with no `origin` deserialises with `origin == 0`.
- [ ] **Step 2: Run** `cargo test -p wordcraft-calc -p wordcraft-doc`. Expect it to fail.
- [ ] **Step 3: Implement.**
  - Evaluate `Expr::Matrix` items and assemble them. An item that is itself a grid is the Syntax
    error `"a matrix element can't be a vector"`.
  - An index is a dimensionless whole number within `1e-9` of an integer, offset by
    `env.origin`.
  - A range's element count is `floor((last − first)/step + 1e-9) + 1`, checked against
    `MAX_ELEMENTS` before allocating.
  - An indexed definition evaluates `body` once per element of `i`, in a frame binding the
    parameter `i` to that element.
- [ ] **Step 4: Run** `cargo test -p wordcraft-calc -p wordcraft-doc`. Expect PASS.
- [ ] **Step 5: Commit** `calc: evaluate vectors, indexes, ranges and indexed definitions; origin setting`.

### Task 4: Array functions

**Files:**
- Create: `crates/calc/src/array_funcs.rs`
- Modify: `crates/calc/src/funcs.rs` (`BUILTINS` gains the new names, so they become reserved names), `crates/calc/src/eval.rs` (`call` dispatches array functions before scalar broadcasting)
- Test: `crates/calc/src/eval/tests.rs`

**Interfaces:**
- **Produces** `pub(crate) fn call_array(name: &str, args: &[Data], origin: u8, pass: &Pass) -> Option<Result<Data, CalcError>>`,
  covering:
  - `length`, `rows`, `cols`;
  - `min` and `max` as reductions over every element of every argument (temperature kinds as
    today);
  - `sum`, `mean`, `dot`, `mmul`, `transpose`;
  - `lookup`, `linterp`, `spline`.

- [ ] **Step 1: Write failing tests:**
  - `size_functions`: on a 3×2 `M`, `length(M)` = 6, `rows(M)` = 3, `cols(M)` = 2. On a single
    value, `length(5 in)` = 1.
  - `reductions`:
    - `min([3, 1, 2] in)` = 1 in;
    - `max(D, 2 in)` = 2 in;
    - `sum([1, 2] lbf)` = 3 lbf;
    - `mean([1, 2, 3] in)` = 2 in;
    - `min([1 in, 2 lbf])` gives a Unit error.
  - `products`:
    - `dot([1, 2] in, [3, 4] lbf)` = 11 lbf·in;
    - `mmul([1, 2; 3, 4], [5; 6])` = `[17; 39]`;
    - `mmul` of 2×3 by 2×3 gives `Shape`;
    - `transpose([1, 2, 3])` is 1×3.
  - `lookup_is_exact`:
    - with `NPS = [0.5, 0.75, 1, 1.5] in` and `T = [30, 45, 60, 90] ft·lbf`,
      `lookup(NPS, T, 1.5 in)` = 90 ft·lbf;
    - `lookup(NPS, T, 38.1 mm)` = 90 ft·lbf, equal after conversion within 1e-12 relative;
    - `lookup(NPS, T, 2 in)` gives Syntax `"2 in isn't in NPS"`, never a nearest guess.
  - `linterp_and_spline_never_extrapolate`:
    - `linterp(NPS, T, 1.25 in)` = 75 ft·lbf;
    - `linterp(NPS, T, 1.6 in)` gives Syntax `"1.6 in is outside NPS (0.5 in to 1.5 in)"`.
      Values in the message use the document's display format;
    - `spline` through points on `y = 2x + 1` at x = 0…4 gives exactly 4 at x = 1.5, because a
      natural spline reproduces a line. Through points on `y = x²` it returns the knot values at
      the knots and lies strictly between the neighbouring knot values at x = 1.5;
    - a vector `x` gives one result per element.
  - `interp_needs_increasing_xs`: `xs` not strictly increasing, or `xs` and `ys` of different
    lengths, is an error naming which.
  - `new_function_names_are_reserved`: `check_name("sum")` is `ReservedName`, and likewise for
    `length`, `rows`, `cols`, `mean`, `dot`, `mmul`, `transpose`, `lookup`, `linterp` and
    `spline`.
  - `products_are_bounded`: `mmul` of a 10,000×1 by a 1×10,000 is `TooLarge` before any work is
    done; `mmul` charges `rows·cols·inner` to the pass.
- [ ] **Step 2: Run** `cargo test -p wordcraft-calc`. Expect it to fail.
- [ ] **Step 3: Implement.** Each function names its argument when it errors, as in
  `"lookup needs xs and ys of the same length (4 and 3)"`. The arguments of the names in the
  messages are the source names when the argument is a bare `Expr::Name`, otherwise `xs`. To do
  that, pass the argument expressions alongside their values.
- [ ] **Step 4: Run** `cargo test -p wordcraft-calc`. Expect PASS.
- [ ] **Step 5: Commit** `calc: array functions (sizes, reductions, products, lookup, interpolation)`.

### Task 5: Showing and quoting vectors

**Files:**
- Modify: `crates/calc/src/render.rs` (`Shown`, `rendered`, `shown_result`), `crates/calc/src/recalc.rs` (`format_value` → `format_data`, `check_show` per element, `mark_stale`, `field_result`, `check_quote_unit`, `defined_names`), `crates/calc/src/eval.rs` (`check_show`), `crates/doc/src/calc.rs` (`QuoteSpec`)
- Test: `crates/calc/tests/recalc.rs`, `crates/doc/src/calc/tests.rs`

**Interfaces:**
- **Produces in `render`:**
  - `pub(crate) enum Shown { One(Formatted), Grid { rows: usize, cols: usize, items: Vec<Formatted>, unit: Option<String>, total_rows: usize } }`.
    `unit` is `Some` when every element shares it, and the items then carry an empty unit.
    `items` holds at most 50 rows.
  - `rendered(stmt, src, shown: Option<&Shown>, state)` changes signature, and
    `shown_result(linear) -> Option<Shown>`.
- **Produces in the doc crate, replacing the tuple API:**
  - `pub struct QuoteSpec { pub name: String, pub unit: Option<String>, pub precision: Option<u8>, pub index: Option<(u32, Option<u32>)> }`;
  - `calc_field_instr(&QuoteSpec) -> String`, whose new switch is `\i 2` or `\i 1,2`;
  - `parse_calc_field(&str) -> Option<QuoteSpec>`.

- [ ] **Step 1: Write failing tests:**
  - `vector_result_shows_as_a_matrix`: `D := [0.840, 1.050] in` and `D =` give linear
    `D=[■(0.84@1.05)] "in"`.
  - `display_unit_applies_to_every_element`: `D = ? mm` gives `D=[■(21.336@26.67)] "mm"`;
    `D = ? lbf` gives a Unit error.
  - `mixed_dimensions_show_each_unit`: `[2 in, 3 lbf] =` gives
    `[■(2 "in"@3 "lbf")]=[■(2 "in"@3 "lbf")]`, with no unit after the bracket.
  - `offset_unit_is_checked_per_element`: with `T := [20 °C, 30 °C]`, `T = ? °F` shows 68 and
    86 °F; `T - [10 °C, 10 °C] = ? °C` gives `NotAbsolute`, since a difference of absolute
    temperatures is a difference.
  - `long_vectors_are_visibly_cut`: `0..59 =` shows the first 50 rows, then a `⋮` row, then
    `"(60 rows)"`. The linear ends `@⋮)] "(60 rows)"`.
  - `stale_vector_keeps_its_values`: after `mark_stale`, the vector display is grey after `≈`
    with the same elements, the truncated form included. This proves `shown_result` reads back
    every form that `rendered` writes. Add a proptest: for random `Shown` grids,
    `shown_result(rendered(…).linear)` gives the same `Shown`.
  - `quoted_vector_is_a_list`, with field results:
    - `WCCALC D` gives `"0.84, 1.05 in"`;
    - `WCCALC D \i 1` gives `"1.05 in"` (origin 0);
    - `WCCALC M` gives `"1, 2; 3, 4"`;
    - a vector of 60 gives its first 50 values then `", … (60 values)"`;
    - `\i 9` out of range gives `"?"`.
  - `quote_spec_round_trips` (doc crate): `calc_field_instr` then `parse_calc_field` returns the
    same `QuoteSpec` for every combination of unit, precision and index. `\i x`, `\i 1,2,3` and a
    repeated `\i` give `None`.
  - `defined_names` lists vector variables too.
- [ ] **Step 2: Run** `cargo test -p wordcraft-calc -p wordcraft-doc`. Expect it to fail.
- [ ] **Step 3: Implement.** Make the engine call sites compile against `QuoteSpec` too: in
  `crates/engine/src/cmd/calc.rs`, `insert_value` builds `QuoteSpec { index: None, … }` until
  Task 7.
- [ ] **Step 4: Run** `cargo test -p wordcraft-calc -p wordcraft-doc -p wordcraft-engine`.
  Expect PASS, apart from the known failures.
- [ ] **Step 5: Commit** `calc: show vector results as matrices and quote vectors`.

### Task 6: Data tables in the model and the recalculation pass

**Files:**
- Modify:
  - `crates/doc/src/calc.rs` (`CalcTable`, `ColumnError`);
  - `crates/doc/src/table.rs` (`Table.calc`, with every `Table { … }` literal the compiler names
    updated);
  - `crates/docx/src/model_bridge.rs` (`calc: None` on read; Task 8 does the real round trip);
  - `crates/calc/src/recalc.rs` (collect tables, the definition list, write-back,
    `calc_unchanged`, id reassignment).
- Create: `crates/calc/src/tables.rs` (heading and cell reading, synthetic statements)
- Test: `crates/calc/tests/recalc.rs`, `crates/calc/src/tables.rs` (unit tests)

**Interfaces:**
- **Produces in the doc crate:**
  - `pub struct CalcTable { pub id: u64, #[serde(default, skip_serializing_if = "Vec::is_empty")] pub errors: Vec<ColumnError> }`;
  - `pub struct ColumnError { pub col: Option<u32>, pub message: String }`, where `None` is an
    error for the whole table;
  - `Table.calc: Option<Box<CalcTable>>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- **Produces in `tables.rs`:**
  - `pub(crate) fn heading(text: &str) -> Result<(String, Option<String>), CalcError>`, so
    `"D_o (in)"` gives `("D_o", Some("in"))`;
  - `pub(crate) fn cell_value(text: &str) -> Result<(f64, Option<String>), String>`;
  - `pub(crate) fn column_stmts(t: &Table, g_is_gravity: bool) -> (Vec<(u32, Result<Stmt, CalcError>)>, Option<CalcError>)`,
    one statement per column, `D_o := Matrix{n,1}` of `Expr::Quantity(value, unit)`, and an error
    for the whole table.
- **Produces in `recalc.rs`:**
  - `pub fn data_tables(doc: &Document) -> Vec<(Vec<u32>, u64)>`, the body path and id of each
    live marked table in document order with repeated ids reassigned. Task 7 uses it.
  - Ownership is decided in document order over regions and columns together. A table's columns
    sit at the table's position, before anything inside it.

- [ ] **Step 1: Write the `tables.rs` unit tests:**
  - `cell_values_are_strict` (Review Focus 4):
    - `0.840` gives `(0.84, None)` and `-3` gives `(-3, None)`;
    - `3/4` gives 0.75;
    - `1-1/2` and `1 1/2` give 1.5, and `-1-1/2` gives −1.5;
    - `1.5 in` gives `(1.5, Some("in"))` and `1-1/2 in` gives `(1.5, Some("in"))`;
    - `1e3` gives 1000;
    - each of these errors: `""`, `"1,500"`, `"1 500"`, `"1-1/2-1"`, `"2+3"`, `"abc"`, `"1/0"`.
  - `headings`: `"D_o (in)"` and `"t_nom"` read; `"2x (in)"`, `"m (kg)"` (reserved) and
    `"D_o (furlongs)"` are errors.
- [ ] **Step 2: Write failing recalc tests.** Build documents with a marked `Table`, using the
  existing `doc_of`/`calc_eq` helpers and a new local `data_table(id, rows: &[&[&str]])` helper:
  - `table_columns_define_vectors`: a table `NPS (in) | D_o (in) | t_nom (in)` with rows
    `1/2 | 0.840 | 0.109`, `3/4 | 1.050 | 0.113` and `1 | 1.315 | 0.133`, plus regions
    `P := 1000 psi`, `S := 20000 psi`, `E := 1`, `Y := 0.4`,
    `t_min := P·D_o/(2·(S·E + P·Y)) =`, `m := t_nom - t_min =` and `min(m) =`.
    The results are:
    - `t_min` = 0.021, 0.026, 0.032 in;
    - `m` = 0.088, 0.087, 0.101 in;
    - `min(m)` = 0.087 in.
  - `editing_a_cell_recalculates`: change `0.113` to `0.120`, and `min(m)` becomes 0.088 in.
  - `bad_cell_names_column_and_row`:
    - a blank `D_o` cell in data row 2 makes the column `D_o row 2 isn't a number`;
    - `t_min` shows `"depends on table data with an error"`;
    - the error is in `CalcTable.errors` as `ColumnError{col: Some(1), …}`;
    - the cell's text is untouched.
  - `cell_unit_must_match_heading`: `50 lbf` under `D_o (in)` is an error for that column;
    `21.3 mm` under it is fine (0.839 in).
  - `table_g_unit_follows_the_g_setting` (Review Focus 2): with `g` as gravity, the heading
    `m (g)` and the cell `5 g` are errors naming `gm`. With the gram setting both read as grams.
  - `merged_cells_are_an_error`: a horizontal or vertical merge anywhere is
    `ColumnError{col: None, message: "a calculation data table can't have merged cells"}`.
  - `duplicate_names_follow_document_order`:
    - a table defining `D_o` before a region `D_o := 1 in` makes the region a `Duplicate`;
    - with the region first, the column is in error;
    - a column named like a function's name is a `Duplicate` too.
  - `copied_data_table_is_a_duplicate` (Review Focus 3): two tables with the same id and headings.
    The second gets a fresh id, its columns are duplicate errors, and the first table's dependents
    still compute.
  - `tracked_deleted_cells_are_skipped`: a cell whose text `0.840` is tracked-deleted and
    `0.850` inserted reads 0.850. A whole table inside a tracked deletion defines nothing.
  - `typing_outside_tables_skips_the_pass`: `calc_unchanged` is true after a prose edit and false
    after a data-cell edit or after a heading edit.
  - `perf_1000_element_vector_chain`: modelled on `perf_2000_regions`. A 1,000-row data table
    column `x`, plus 200 regions `y_k := y_(k-1)·1.001 + x`, recalculates in < 100 ms in release
    and asserts no errors.
- [ ] **Step 3: Run** `cargo test -p wordcraft-calc`. Expect it to fail.
- [ ] **Step 4: Implement.**
  - **Collecting:** `collect` also gathers marked tables (body tables at any depth, not inside a
    tracked deletion) as `FoundTable { path, id, stmts, error }`.
  - **One definition list:** `Index::build` works over `enum Def { Region(usize), Column { table: usize, col: u32 } }`,
    sorted by position.
  - **Dependants:** a failed column marks its name failed with `UpstreamTable` for users.
  - **Write-back:** column outcomes go to `CalcTable.errors`, and the table is touched only when
    its errors change.
  - **Cell text:** each cell's `Paragraph::final_text()` (no objects) over its blocks, trimmed. A
    cell containing a nested table, a calculation or any object is an error for that column.
  - **Ids:** table ids are their own id space; a repeated id gets the next one above the largest.
- [ ] **Step 5: Run** `cargo test -p wordcraft-calc`, then
  `cargo test -p wordcraft-calc --release perf_`. Expect PASS.
- [ ] **Step 6: Commit** `calc: document tables as calculation data`.

### Task 7: Engine commands

**Files:**
- Modify: `crates/engine/src/cmd/calc.rs` (new command, `settings`, `insert_value`, `has_calc`, `fresh_id`)
- Test: `crates/engine/src/cmd/calc/tests.rs`

**Interfaces:**
- **Consumes:** Task 6's `Table.calc` and `data_tables`, and Task 5's `QuoteSpec`.
- **Produces commands:**
  - `calc.dataTable`, with params `{"on": bool}`. It acts on the innermost body table at the
    caret and answers `{"id": n}` or `{"on": false}`. Outside a body table it fails with
    `"Put the caret in a table in the main text"`.
  - `calc.settings` accepts `"origin": 0|1`; anything else is
    ``Params("`origin` is 0 or 1")``. Its answer includes `origin`.
  - `insert.calcValue` accepts `"index": [i] | [i, j]` and refuses an index on a name whose
    value is not a grid.

- [ ] **Step 1: Write failing tests:**
  - `mark_table_defines_columns`: insert a 2×2 table via the existing table command, type
    `D (in)`, `1/2` and so on into its cells, then run `calc.dataTable {"on": true}`. A region
    `D =` shows the vector. Running `{"on": false}` makes the region `D isn't defined`.
  - `undo_unmark_restores` (undo of mark and unmark): after unmarking, `edit.undo` brings the
    results back, and undoing the mark leaves `D` undefined.
  - `origin_setting_reindexes`: `x := D[1]` changes value when `origin` switches 0 → 1. In manual
    mode the switch marks results stale.
  - `quoting_an_element`: `insert.calcValue {"name": "D", "index": [0]}` shows the first value;
    on a single-value name the command fails with `"x isn't a vector"`.
  - `has_calc_counts_data_tables`: a document whose only calculation content is a marked table is
    recalculated after a cell edit.
  - `catalog_has_calc_commands` is extended with `calc.dataTable`.
- [ ] **Step 2: Run** `cargo test -p wordcraft-engine calc`. Expect it to fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p wordcraft-engine`. Expect PASS, apart from the known
  failures.
- [ ] **Step 5: Commit** `engine: calc.dataTable, origin setting and indexed quoted values`.

### Task 8: `.docx` round trip of data tables

**Files:**
- Modify: `crates/doc/src/calc.rs` (`CalcStore.tables`, `StoredTable`, `CALC_TABLE_TAG_PREFIX`), `crates/calc/src/docx.rs` (`export`, `import`, `has_calc_controls`), `crates/calc/src/recalc.rs` (`RecalcReport.tables_edited`), `crates/engine/src/cmd/calc.rs` (`edited_notice`)
- Test: `crates/docx/tests/it/calculations.rs`, `crates/engine/src/cmd/calc/tests.rs`

**Interfaces:**
- **Produces in the doc crate:**
  - `pub const CALC_TABLE_TAG_PREFIX: &str = "wordcraft-calc-table:";`;
  - `pub struct StoredTable { pub id: u64, pub fingerprint: u64 }`;
  - `CalcStore.tables: Vec<StoredTable>` with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`,
    and `is_empty` checks it too.
- **Produces in `RecalcReport`:** `tables_edited: usize`.
- **Fingerprint:** `fnv1a64` over each cell's trimmed `final_text`, with `\u{1f}` between cells
  and `\u{1e}` between rows.

- [ ] **Step 1: Write failing tests:**
  - `data_table_round_trips_through_docx`: the B31.3 document from Task 6 is saved and reopened.
    - The table is marked again with the same id.
    - Results are equal.
    - `document.xml` holds a `w:sdt` with `w:tag w:val="wordcraft-calc-table:<id>"` around a
      `w:tbl`.
    - The table cells hold exactly the typed text.
  - `word_sees_a_plain_table`: in `document.xml`, the data table has no field codes or extra
    runs; the only addition is the wrapping `w:sdt`.
  - `table_edited_in_word_is_used_and_noticed` (Review Focus 4): use `edit_document` to change
    `0.113` to `0.120` and to add an empty row.
    - On reopen the new value is used.
    - The empty row makes `t_nom row 4 isn't a number`.
    - `tables_edited == 1`.
  - `table_without_store_stays_plain`: a tagged table with no store opens as a plain table, with
    its control removed.
  - In engine tests, `edited_table_notice`: opening gives the notice
    `"Calculation data was edited in Word in 1 table; results were recalculated"` (`tables` for
    N ≠ 1). When both kinds of edit are present it is appended to the existing equations notice.
  - The origin setting round-trips through the store.
- [ ] **Step 2: Run** `cargo test -p wordcraft-docx calculations` and
  `cargo test -p wordcraft-engine calc`. Expect them to fail.
- [ ] **Step 3: Implement.** Wrap the table using `Table.controls`: push the control onto
  `controls.open` and increment `controls.close`. Strip it the same way on import. If the shared
  model can't carry a table-level control through the writer, stop and report it as a blocker
  rather than changing the docx crate's model.
- [ ] **Step 4: Run** both suites. Expect PASS.
- [ ] **Step 5: Commit** `calc: data tables survive the .docx round trip`.

### Task 9: User interface

**Files:**
- Modify: `crates/ui-egui/src/canvas/context_menu.rs` (table items), `crates/ui-egui/src/calc_ui.rs` (`OptionsForm.origin`, `ValueForm.index`, table error marker)
- Test: `crates/ui-egui/src/tests/calc.rs`

**Interfaces:**
- **Consumes:** Task 7's commands, and `wordcraft_engine::cmd::table_mouse::cell_rects`
  (`crates/engine/src/cmd/table_mouse.rs:296`) for where to draw the marker.

- [ ] **Step 1: Write failing UI tests**, using the existing harness pattern in `tests/calc.rs`
  (`get_by_label(...).click()`):
  - `table_menu_marks_calculation_data`:
    - right-click in a table shows **Use as Calculation Data**;
    - clicking it marks the table;
    - the menu then shows **Stop Using as Calculation Data**.
  - `options_choose_array_origin`: the Calculation Options dialog has "Arrays count from" with
    "0 (Mathcad)" and "1"; choosing 1 sends `"origin": 1`.
  - `calculated_value_takes_an_index`: the Calculated Value form has an optional "Element" box.
    Entering `1` sends `"index": [1]`, entering `1, 0` sends `[1, 0]`, and other text is refused
    in the form.
  - `table_error_is_marked_on_screen`:
    - a data table with a column error draws a red `⚠` at the top-right of that heading cell, or
      of the first cell for a table-wide error;
    - hovering it shows the message;
    - the document's text is unchanged.
- [ ] **Step 2: Run** `cargo test -p wordcraft-ui-egui calc`. Expect it to fail.
- [ ] **Step 3: Implement.**
  - The menu items sit with the other table items, shown for any table pick when the table is in
    the main text.
  - The marker is painted like the edited-calculation bar: an overlay positioned through the
    layout, never written into the document.
- [ ] **Step 4: Run** `cargo test -p wordcraft-ui-egui`. Expect PASS, apart from the known
  failures.
- [ ] **Step 5: Commit** `ui: calculation data tables, array origin and element quoting`.

### Task 10: Guide, worked examples and final verification

**Files:**
- Modify: `docs/calculations.md` (new section "Vectors, matrices and data tables")
- Create: `crates/engine/src/cmd/calc/pipe_wall_sample.rs`, registered next to `flange_bolt_sample` in `crates/engine/src/cmd/calc.rs`

- [ ] **Step 1: Write the sample tests**, driving the engine like `flange_bolt_sample` does:
  - `pipe_wall_sample`: the B31.3 document from Task 6, built through commands (insert a table,
    type cells, `calc.dataTable`, insert calculations). Assert:
    - the `t_min`, `m` and `min(m)` values;
    - changing `t_nom` for 3/4" to `0.120` gives `min(m)` = 0.088 in;
    - a `.docx` save and reopen through the engine keeps it live.
  - `flange_torque_sample`: a table `NPS (in) | T_flange (ft·lbf)` with rows `1/2 | 30`,
    `3/4 | 45`, `1 | 60` and `1-1/2 | 90`.
    - `lookup(NPS, T_flange, 1.5 in) = ? ft·lbf` shows 90.
    - `linterp(NPS, T_flange, 1.25 in) = ? ft·lbf` shows 75.
    - `linterp(NPS, T_flange, 1.6 in) =` shows the error
      `"1.6 in is outside NPS (0.5 in to 1.5 in)"`.
- [ ] **Step 2: Run** `cargo test -p wordcraft-engine sample`. Expect PASS. A failure here is a
  bug in Tasks 1–8: fix it there with a test.
- [ ] **Step 3: Write the guide section.** Cover the following and nothing else:
  - typed vectors and matrices (a list is a column), and per-element units;
  - element-wise arithmetic, with the note that a pasted Mathcad formula relying on vector ×
    vector = dot product must use `dot()`;
  - indexing and the "Arrays count from" option;
  - ranges and `t[i] :=`;
  - the function table;
  - data tables, as a worked B31.3 example: heading form, accepted cell values with `1-1/2`, and
    errors;
  - how results and quoted values show, including the 50-row cut;
  - Word behaviour: a plain table, and edits used with a status notice;
  - the new reserved names (`sum`, `length`, `rows`, `cols`, `mean`, `dot`, `mmul`,
    `transpose`, `lookup`, `linterp`, `spline`).
- [ ] **Step 4: Run the full verification:**
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`

  Expect only the seven known environment failures.
- [ ] **Step 5: Build the app**, so the user's desktop shortcut runs this version:
  `cargo build --release -p wordcraft`.
- [ ] **Step 6: Commit** `docs: vectors, matrices and data tables in the calculations guide`.
