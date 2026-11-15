# Calculations: vectors, matrices and data tables — design

Date: 2026-11-15
Branch: `feature/inline-calculations` (fork `alanrathbun/wordcraft`)
Builds on: `2026-11-14-inline-calculations-design.md` (Stage 1, shipped) — this is the matrix part
of the planned Stage 2. if/then and programs are not part of it.

## Why

The user's Mathcad sheets regularly evaluate one formula for many cases at once. For example, every
schedule 40 pipe size in a design is checked for minimum wall thickness per ASME B31.3, and a table
of values per pipe size feeds flange-torque calculations. Today WordCraft calculations hold single
values only.

A survey of the user's 1,025 Mathcad 14 example sheets (`.xmcd`; the 1,747 older binary `.mcd`
files weren't readable) found:

- **How many use matrices:** 202 of the sheets.
- **Shape:** nearly all are **single-column vectors** (about 1,900 vectors, against 86
  two-dimensional matrices).
- **Units:** values inside matrices usually carry units.
- **Ranges and indexing:** range variables (`i := 0..n`) appear in 190 sheets and indexed elements
  in 181.
- **Table lookups:** interpolation (`interp`/`cspline`/`linterp`) appears in about 40 sheets, and
  `min`/`max` in 31.
- **Rare features:** vectorize in 9 sheets, transpose in 9, and almost no matrix algebra (inverse,
  determinant, solving systems).
- **Element numbering:** 195 of the 202 count from 0 (Mathcad's default).

## Decisions made with the user

1. **Element-wise by default.** Same-shape vectors and matrices combine element by element with
   `+ − · / ^`, so a formula written for one pipe gives one result per pipe size with no vectorize
   operator. Mathcad's default (vector × vector = dot product) silently gives one number when
   vectorize is forgotten, which is the project's anti-goal. Dot and matrix products are explicit
   functions.
2. **Document tables are the data source for tables; short vectors can also be typed** in the
   calculation box.
3. **Each element carries its own value and unit.** A vector is a grid of the same unit-checked
   values single calculations use. Mixed units are allowed where they make sense.
4. **Each column of a data table becomes a vector named by its heading.** A heading like
   `D_o (in)` defines the vector `D_o` in inches, under the same global define-once naming rules as
   calculations.

## Goals

- **Typed vectors and matrices:**
  - `D := [0.840, 1.050, 1.315] in` is a column vector of 3.
  - `M := [1, 2; 3, 4]` is a 2×2 matrix, with `;` starting a new row.
  - A one-dimensional bracket list is always a **column** vector, the engineering convention and
    Mathcad's.
  - A unit after the bracket applies to every element; an element can carry its own unit
    (`[2 in, 50 mm]`).
- **Element-wise arithmetic:**
  - `+ − · / ^` work between same-shape arrays, or between an array and a single value
    (broadcast).
  - Different shapes are an error naming both: "shapes don't match: 3 rows × 1 vs 4 rows × 1".
  - Every unit rule applies per element; one element in error makes the whole result an error
    that names the element ("element 2: can't add lbf to in").
- **Built-in scalar functions apply element-wise** (`sqrt`, `sin`, `ln`, `abs`, `round`…). **User
  functions** given array arguments evaluate element-wise naturally, because their bodies use
  element-wise operators.
- **Indexing:**
  - `D[2]` and `M[i, j]`, displayed as subscripts.
  - Numbering starts at **0** by default (Mathcad), as a per-document setting `origin` of 0 or 1,
    stored with the calculation settings like `g`.
  - An out-of-range index is an error naming the valid range.
- **Ranges:**
  - `i := 0..5` and `x := 1, 1.5..3` (first, second..last) define a range, which is a vector.
  - **Indexed definition** `t[i] := f(D[i])`, with `i` a range, defines the whole vector `t` in one
    statement. This keeps Mathcad's common range-and-index pattern working, alongside the simpler
    element-wise form. It is still a single definition of `t` (define-once holds).
- **Array functions:**
  - **Size:** `length(v)`, `rows(M)`, `cols(M)`.
  - **Reductions:** `min`/`max` (over arrays, and over several arguments as today), `sum(v)`,
    `mean(v)`.
  - **Products:** `dot(a, b)`, `mmul(A, B)` (matrix product), `transpose(A)`.
  - **Lookup:** `lookup(xs, ys, x)` takes the `ys` value whose `xs` equals `x`. No match is an
    error, never a nearest guess.
  - **Interpolation:** `linterp(xs, ys, x)` interpolates linearly, and `spline(xs, ys, x)` uses a
    natural cubic spline.
    - `xs` must be strictly increasing with one dimension.
    - Outside `xs`, both are an error ("1.6 in is outside NPS (0.5 in to 1.5 in)") rather than an
      extrapolation.
    - `x` may itself be a vector, giving one result per element.
- **Data tables:**
  - **Marking:** a normal document table is marked as calculation data from its right-click menu
    (**Use as Calculation Data / Stop Using as Calculation Data**).
  - **Headings:** row 1 holds the headings `Name (unit)`, or `Name` alone for dimensionless values.
    Each column defines a global vector of the remaining rows' values, read as numbers in that
    unit.
  - **Cell values:** a cell may carry its own unit (`1.5 in`), which must match the heading's
    dimension. A thousands separator in a number is refused, as in calculations.
  - **Editing:** a cell edit recalculates like any edit.
  - **Errors:**
    - A heading that isn't a valid, unique, non-reserved name is an error for that column.
    - So is an empty or non-numeric cell; for example, "D_o row 4 isn't a number".
    - Dependents show "depends on table data with an error". Nothing guesses.
  - **Placement:** a table can sit anywhere in the body (define-anywhere), and table cells inside a
    tracked deletion are skipped.
- **Display:**
  - A vector or matrix result is drawn as a bracketed matrix with each element formatted.
    `= ? unit` applies to every element; otherwise each element uses its natural unit. When all
    elements share a dimension, one unit is shown once after the bracket (`=[■(0.109@0.113@0.133)] "in"`).
  - Long vectors show in full up to 50 rows; beyond that they show the first 50 rows, a `⋮`, and
    "(N rows)". The truncation is never silent.
- **Quoted values:** a quoted vector shows a comma list ("0.109, 0.113, 0.133 in"). `name[i]` quotes
  one element (Insert › Calculated Value offers an index box for arrays).
- **Word round trip:**
  - **Vector results:** saved as ordinary Word matrices.
  - **Data tables:** saved as ordinary Word tables, tagged (a content control around the table, tag
    `wordcraft-calc-table:<id>`) and listed in the hidden calculation store with a fingerprint of
    their cell text.
  - **Reopening in WordCraft:** the table is data again. If a reviewer changed table values in
    Word, the new values are used (they are inputs), and the status line says "Calculation data was
    edited in Word in N tables; results were recalculated", so it never happens unnoticed.

## Non-goals (this design)

- if/then, programs and loops (the rest of Stage 2).
- Solve blocks and `lsolve`; inverse and determinant (rare in the user's sheets — later).
- Excel connections (Stage 5); importing `.mcd`/`.xmcd` worksheets (Stage 6).
- Plots; complex numbers; three-dimensional arrays.
- Mathcad's vectorize operator. Element-wise is already the default, so there's nothing to
  vectorize. Pasted Mathcad formulas that relied on a dot product must use `dot()`, and the guide
  says so.

## Anti-goals

- **A wrong number that looks right:**
  - Vector × vector silently becoming a dot product.
  - An interpolation extrapolating or a lookup guessing a nearest value.
  - A truncated display that hides rows.
  - A data cell silently read as 0.
- **A table in Word that looks broken,** or table data lost or changed silently in the round trip.
- **Slower ordinary editing.** The unchanged-signature skip from Stage 1 must cover tables, so
  typing prose in a document with data tables doesn't recalculate.

## Constraints

- Everything in the Stage 1 constraints: Rust only, no `unsafe`, no panics in non-test code,
  hostile input bounded, clean room.
- **Bounds:**
  - Arrays up to 10,000 elements.
  - Data tables within WordCraft's existing table limits.
  - The per-pass work budget counts element operations, so a hostile 10,000 × 10,000 product
    errors as "too much work" instead of hanging.
- Document compatibility: Stage 1 documents open unchanged. New settings (`origin`) default so old
  files mean what they meant.

## Approaches considered

**Value model:**
- **A. Each element is a full unit-checked value. Chosen.** Element-wise operations reuse the
  existing scalar arithmetic per element, so every unit rule applies automatically.
- **B. One unit per column.** Leaner, but it can't express mixed units and means a second set of
  rules where unit mistakes could slip through. Rejected.
- **GridCraft's array engine.** It has no units. Rejected.

**Table link:**
- **1. Each column is a vector named by its heading. Chosen.** No new syntax, it reads naturally,
  and the global naming rules apply.
- **2. The whole table is one named matrix, with columns picked by index.** Rejected: column
  meaning hides behind numbers.

**Typed matrix notation:** `[a, b; c, d]` (rows split by `;`) is chosen over Word's `■(a&b@c&d)`:
it is easier to type, and the display still uses Word's matrix.

## Design

- **Values:** the evaluator's value becomes "a single value or an array (rows × cols) of values".
  The rows × cols layout is row-major. Each element is the existing value (quantity plus
  temperature kind). Element-wise operations map the existing scalar operation over elements,
  stopping at the first element error.
- **Parsing:** new syntax is `[ … ]` literals, postfix `[i]` / `[i, j]` indexing, `a..b` and
  `a, b..c` ranges, and an indexed definition head `t[i] :=`.
  - `[` after a name with no space is indexing; otherwise it starts a literal.
  - The display renders indexing as a subscript and literals as Word matrices, drawn from the
    syntax tree as today, so the displayed structure matches the evaluated one.
- **Data tables in the model:** a table carries an optional calculation-data marker (an id and a
  state, like a calculation region), saved in WordCraft's native format and, through the content
  control plus store, in `.docx`.
  - Recalculation collects marked tables with calculation regions, in document order, and reads
    their headings and cells.
  - Each column is a definition node in the existing dependency graph, owned by its table, so
    duplicate names, cycles, use-before-define and edited-in-Word all work unchanged.
- **Settings:** `CalcSettings.origin: u8` (0 or 1, default 0), in Calculation Options.

## Success criteria

- **B31.3 example:** a pipe-size table (NPS, D_o, t_nom) and inputs P, S, E, Y give a `t_min`
  vector (one value per size), a margin vector `t_nom − t_min`, and `min()` of the margins.
  - Editing a table cell updates everything.
  - Saving to `.docx` and reopening keeps it all live.
- **Flange example:** a torque table by size gives `lookup(NPS, T_flange, 1.5 in)` and a
  `linterp` result. A size outside the table is an error, not a number.
- **Typed vectors:** typed vectors, indexing (origin 0 and 1), ranges with indexed definition,
  shape-mismatch and per-element unit errors, and truncated long-vector display all behave as
  described.
- **Performance:** a 1,000-element vector chain recalculates within the existing 100 ms budget.
- **Tests:** units for each function, parser, evaluator, recalculation with tables, `.docx` round
  trip with a table edited in Word, and UI marking of a table.

## Calls made in the reviewer check (for the user to confirm)

- **Ranges include both ends** (`0..5` is 0,1,2,3,4,5), as in Mathcad. A range is allowed only as
  the whole right-hand side of a definition, so its commas can't be confused with list or argument
  commas.
- **Table cells are not calculation expressions.** A cell holds:
  - a decimal number (`0.840`);
  - a fraction (`3/4`);
  - a mixed number written `1-1/2` or `1 1/2` (= 1.5, the pipe-size convention);
  - optionally followed by a unit.

  Anything else is an error. Reading `1-1/2` as arithmetic would silently give 0.5.
- **Merged cells** in a data table, or a heading row that is not the first row, are an error for
  the table.

## Left to the builder

- The exact internal types and where array code lives in `wordcraft-calc`.
- How a table error is shown on the table itself (e.g. a hover marker on the heading cell). It must
  be visible on screen and never written into the user's table text.
- The spline boundary handling beyond "natural", and the interpolation tolerance for "equals" in
  `lookup` (exact after unit conversion, with a relative 1e-12 tolerance).
- Whether `transpose` also gets a `ᵀ` postfix.
