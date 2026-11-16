# Calculations: conditionals, text values and sums — design

Date: 2026-11-16
Branch: `feature/inline-calculations` (fork `alanrathbun/wordcraft`)
Builds on: `2026-11-14-inline-calculations-design.md` (Stage 1) and
`2026-11-15-calculation-vectors-and-tables-design.md` (vectors, matrices and data tables). This is
the if/then part of the planned Stage 2, plus sums. Programs, loops and solve blocks are not part of
it.

## Why

The user named the features they would actually use next: the solver, conditionals, spreadsheet
import/export and sums. They also want to import their Mathcad 15 and Prime worksheets later, so
whatever is built now should be something those sheets can be translated into.

A recount of the 1,025 Mathcad 14 example sheets (`.xmcd`) shows how common each piece is:

| Mathcad feature | Uses |
|---|---|
| `<` / `>` / `≤` / `≥` | 1,677 / 439 / 680 / 258 |
| if-then branches (`ifThen`) | 1,243 |
| `otherwise` | 1,012 |
| `if()` function | 126 sheets |
| summation with limits | 369 |
| `or` / `and` | 169 / 84 |
| if-then branches giving text | 48 sheets |

The everyday use is a code check: a coefficient that depends on temperature, and a pass/fail
verdict per pipe size.

## Decisions made with the user

1. **Checks can give text.** A calculation can produce text such as "OK", "NG" or "Use Schedule 80",
   one per element for vectors. Text is a new kind of value everywhere: display, quoted values,
   vectors and Word. (Chosen over numbers-only checks, and over also coloring NG red with a count
   in the status line.)
2. **Data tables get text columns in this round.** This lets a material table feed lookups:
   `S_allow := lookup(Material, S, "A106 B")`.
3. **Text is its own kind of value, separate from numbers** (approach B below).

## Goals

- **Text values:**
  - **Literals:** `"OK"` in straight or curly quotes. A text can't contain a quote mark.
  - **Vectors of text:** `["A106 B", "A312 TP316"]`.
  - **Function results:** a function can return text (`verdict(m) := "OK" if m ≥ 0 in, "NG" otherwise`).
  - **Equality:** `==` and `≠` compare texts.
  - **Lookup:** `lookup` takes text keys and can return text values.
  - **Size and arrangement:** indexing, `length`, `rows`, `cols` and `transpose` work on text
    vectors.
  - **Everything else is an error:** arithmetic, a unit, `<`, `min`/`max`/`sum`/`mean`,
    `linterp`/`spline`, and `= ?` with a unit. The error says text can't be used there, naming the
    text; it never gives a number.
- **Comparisons** `<`, `>`, `≤`, `≥`, `==`, `≠`:
  - **Typing:** they can be typed as `<=`, `>=` and `!=`, or as the symbols. Backslash shortcuts
    `\le`, `\ge` and `\ne` give the symbols, as in Word.
  - **Units:** units are checked first, so a length against a force is an error. Values are
    compared after conversion: `38.1 mm == 1.5 in` is true and `100 °C == 212 °F` is true.
  - **Result:** a comparison gives 1 (true) or 0 (false), dimensionless, as in Mathcad, so it works
    in arithmetic. For example, `sum(check == "NG")` counts the failed checks.
  - **Arrays:** comparisons work element by element with the same broadcasting as arithmetic.
  - **Chains:** `0 ≤ x ≤ 1` means `0 ≤ x and x ≤ 1`.
  - **One tolerance for all six:** values within the equality tolerance (Design, below) count as
    equal for every operator. So `38.1 mm > 1.5 in` is false and `212 °F ≤ 100 °C` is true, even
    though unit conversion leaves a last-digit difference.
- **Logic:**
  - `and`, `or` and `not`, also typed as `∧`, `∨`, `¬` (shortcuts `\wedge`, `\vee`, `\neg`).
  - They take dimensionless numbers, where nonzero is true, and give 1 or 0.
  - `and` and `or` skip the right side when the left side already decides the result, per element.
- **Piecewise definitions:**
  - **Form:** `Y := 0.4 if T ≤ 900 °F, 0.5 if T ≤ 950 °F, 0.7 otherwise`.
  - **Order:** cases are tried in order and the first true condition wins.
  - **Where it can appear:** a piecewise is allowed only as the whole right side of a definition,
    including a function definition, as with ranges. This keeps its commas from being confused with
    list or argument commas.
  - **No `otherwise`:** if no condition is true, that's an error naming the values tested, never a
    0. For example: "no case applies (T = 1,000 °F)".
- **`if(c, a, b)` function:** for use inside expressions and for pasted Mathcad formulas. It
  evaluates only the branch it picks.
- **Per-element branching:** with vector conditions, each element uses only the branch it picks,
  and a branch that isn't picked can't make that element fail. So `sqrt(x) if x ≥ 0, 0 otherwise`
  works for a vector `x` with negative entries.
  - **Whole arrays stay whole:** inside a branch, `sum(x)`, `x[0]`, `lookup(xs, …)` and the like
    see the whole arrays, exactly as outside one. A branch is worked out over the full arrays, and
    element k of the result takes element k of the branch it picks.
  - **Element failures are kept per element:** an element that fails inside a branch (`1/x` at
    `x = 0`) fails the result only if that element picks that branch.
  - **Whole-branch failures:** a branch that fails as a whole (a shape mismatch, or a lookup of a
    single key with no match) fails the result only if some element picks it. A lookup or
    interpolation over a vector of keys fails per element, so a key outside the table fails only an
    element that picks that branch.
  - Results combine into one vector.
  - A vector that would mix text and numbers is an error naming the element ("element 3 is text but
    element 1 is a number").
  - A single value may be text in one case and a number in another.
- **Sums:**
  - **Form:** `∑(i = 1..n) a[i]·b[i]`, typed with `∑` (shortcut `\sum`). The range uses the same
    rules as range definitions: `first..last` or `first, second..last`, both ends included.
  - **Display:** in the equation and in Word it shows as a real ∑ with the limits below and above.
  - **Local index:** the index belongs to that sum alone.
    - Two sums may both use `i`.
    - It can't reuse a name defined in the document or a parameter of the enclosing function, so it
      never shadows anything. The error asks for another index name.
  - **Terms:** they must be addable under the unit rules and may be vectors of one shape, which add
    element by element.
  - **Empty range:** `1..0` is an error, never a silent 0.
  - **Nesting:** sums can be nested.
- **Data tables with text columns:**
  - **Heading:** a heading `Material (text)` makes a text column. Every other column stays strictly
    numeric, so a typo in a number cell is still an error and never quietly turns into text.
  - **Cells:** a text cell's value is its text, trimmed. A number in a text column is text (`"6"`).
  - **Empty cells:** an empty cell is an error, as in numeric columns.
- **Display:**
  - Text results show in quotes in the equation (`check = "OK"`), as in Mathcad. A text vector
    shows as a bracketed column of quoted texts, cut after 50 rows as numeric vectors are.
  - A piecewise shows in Word as a brace with one line per case:

    ```
           ⎧ 0.4   if T ≤ 900 °F
      Y ≔  ⎨ 0.5   if T ≤ 950 °F      = 0.5
           ⎩ 0.7   otherwise
    ```

  - Comparisons show as `<`, `>`, `≤`, `≥` and `≠`. `==` shows as a bold `=`, as in Mathcad, so it
    can't be mistaken for a result.
  - `and`, `or` and `not` show as `∧`, `∨` and `¬`, as in Mathcad printouts, however they were
    typed. `if(c, a, b)` shows as a function call. (Calls made while planning.)
- **Quoted values:** text is quoted into a sentence without quote marks ("the check is OK"), and a
  text vector as a comma list ("OK, OK, NG").
- **Word round trip:** text, comparisons, piecewise braces and sums are saved as ordinary Word math
  that reads correctly in Word. They reopen live in WordCraft through the existing calculation
  store, and text columns round-trip through the existing table fingerprint.
- **Guide:** `docs/calculations.md` gets a section on conditions, text and sums, with the examples
  below. The shortcut list in "Symbols" gets the new shortcuts.

## Non-goals (this design)

- **Products** (`∏`), solve blocks (`Given`/`Find`, `root`), programs with loops and local
  assignments.
- **Coloring** NG results red, and a failed-check count in the status line.
- **Mathcad's global range-variable sum** (`Σ` over a predefined `i`). The future importer
  translates it to the limits form.
- **Text operations:** joining texts, numbers formatted into text, and case-insensitive matching.
- **Importing Mathcad files and spreadsheets** (later rounds).

## Anti-goals

- **A wrong value that looks right:**
  - text silently becoming a number, or a number silently becoming text;
  - a piecewise with no matching case giving 0;
  - an empty sum giving 0;
  - a comparison across incompatible units giving true or false;
  - a typo in a numeric table cell read as text.
- **An error in an unused branch** failing a result (the `sqrt(x) if x ≥ 0` case).
- **A sum index that silently shadows** a document variable.
- **Breaking existing documents.** Calculations that opened before still mean what they meant.
- **Slower ordinary editing.**

## Constraints

- **Stage 1 constraints still apply:** Rust only, no `unsafe`, no panics in non-test code, hostile
  input bounded, clean room.
- **Bounds:**
  - A text value is at most 1,000 characters.
  - A sum's range counts toward the existing 10,000-element and work-budget limits, so
    `∑(i = 1..10^9)` is "too much work" or too large, never a hang. Each term counts as work.
  - Nesting depth is covered by the existing depth limits.
- **Reserved words:** `if`, `otherwise`, `and`, `or` and `not` become reserved. A name `text` can
  still be a variable; only the heading form `(text)` is special.
  - A document that used one of these words as a variable shows an error naming the word, rather
    than a changed meaning.
  - Such documents are unlikely: these words weren't usable as variables before without looking
    like an operator.

## Approaches considered

**Text values (the user picked B):**

- **A. Each element is a number or text, so one vector can mix both.** Rejected:
  - every arithmetic path has to check each element;
  - mixed vectors break the one-unit display and the interpolation functions.
- **B. Text is its own kind of value.** Chosen. A value is a number, a text, a grid of numbers or a
  grid of text.
  - The numeric code never sees text, so text can't reach arithmetic.
  - Text goes only where it belongs.
  - The cost is a mixing check when assembling piecewise results.
- **C. Text stored as a hidden code number.** Rejected: text could quietly turn into a number in
  arithmetic, which is the exact failure to prevent.

**Conditional syntax:**

- **Chosen:** the one-line piecewise (`a if c, b otherwise`) plus `if(c, a, b)`. It reads like the
  printed brace, and both forms are what Mathcad sheets translate into.
- **Rejected:** a Mathcad-style program block with `if` lines. It needs multi-line editing and local
  assignment, which belong with programs.

**Booleans:**

- **Chosen:** comparisons give 1 and 0 (Mathcad, and works in arithmetic, such as counting).
- **Rejected:** a separate true/false kind, which would break `sum(check == "NG")` and pasted
  Mathcad formulas.

**Sums:**

- **Chosen:** the limits form with a local index. This keeps define-once intact.
- **Rejected:** Mathcad's global range-variable sum, which needs a document-wide `i` and can't nest
  two sums over different ranges cleanly.

## Design

- **Values:**
  - The evaluator's value gains text alongside numbers: a single text, or a grid of texts with the
    same rows × cols layout as numeric grids.
  - Numeric single values and grids are unchanged, so existing arithmetic code paths don't change.
  - The public `Value` gains matching text forms for display and quoting.
- **Lexing:**
  - New tokens: `<`, `>`, `≤`/`<=`, `≥`/`>=`, `==`, `≠`/`!=`, `∧`, `∨`, `¬`, `∑`, and quoted text in
    `"…"` or `“…”`.
  - The words `if`, `otherwise`, `and`, `or` and `not` are keywords. `if` followed by `(` at the
    start of an operand is the function.
  - An unclosed quote is an error.
- **Precedence, loosest first:**
  1. `or`
  2. `and`
  3. `not`
  4. comparisons (chainable)
  5. `+ −`
  6. the existing arithmetic levels

  A keyword never starts an implicit multiplication.
- **Equality tolerance:** `==` is equal within a relative 1e-12 after unit conversion. This is the
  rule `lookup` already uses, so 0 equals only exactly 0. The guide says to compare to zero with a
  tolerance (`abs(x) < 1e-9 m`).
- **Text comparison:** texts are equal when they match exactly after trimming the ends and treating
  any run of spaces (non-breaking spaces included) as one space. Matching is case-sensitive.
- **Temperatures:** the evaluator already marks each value as an absolute temperature (a `°C`/`°F`
  value, or one plus or minus a difference), a bare `K`/`°R` value, or anything else. Comparisons
  use that marker:
  - **An absolute temperature against an absolute temperature or a `K`/`°R` value:** compared as
    temperatures (`T ≤ 900 °F`, `T ≤ 755 K`).
  - **Two values neither of which is an absolute temperature:** compared directly. This covers
    differences (`ΔT ≤ 10 Δ°F`) and anything not a temperature.
  - **An absolute temperature against anything else made from temperatures** (a difference, or an
    average written `(T_1 + T_2)/2`): an error, worded like the existing display error. Comparing
    them on the kelvin scale would give a wrong answer that looks right (`ΔT ≤ 10 °F` would test
    against 260.9 K). The guide's existing advice applies: write an average as
    `T_1 + (T_2 − T_1)/2`.
- **Piecewise and per-element branching:** the per-element rule above is required; how it's
  achieved is left to the builder (for example, evaluating each branch only on the elements that
  pick it).
- **Dependencies:** the names in every branch and condition are dependencies, whichever branch is
  picked, so recalculation stays correct when a condition flips. A sum's index counts as a use of
  that name. Normally nothing in the document defines it, so it links to nothing; when something
  does, the link makes sure that definition has run first, so the "already defined" error always
  appears instead of depending on evaluation order.
  As today, a calculation that depends on a calculation with an error shows "depends on a
  calculation with an error", even if the failed name sits in a branch nothing picks. The per-element
  rule above is about errors while working the branches out, not about failed inputs.
- **How far a sum reaches:** `∑` binds like a product term, as in printed math and Word.
  `∑(i = 1..n) a[i]·b[i] + 1` is `(∑ a[i]·b[i]) + 1`; write `∑(i = 1..n) (a[i] + 1)` for the other
  meaning. The display draws the summed term grouped, so the reading is visible. The Mathcad
  importer follows the same rule.
- **Settings:** none new.

## Success criteria

- **B31.3 check:** the existing B31.3 sample gains `check := "OK" if margin ≥ 0 in, "NG" otherwise`,
  giving one verdict per pipe size.
  - `n_NG := sum(check == "NG")` counts the failures.
  - Editing a `t_nom` cell so a size fails flips that element to "NG" and the count to 1.
- **Material lookup:** a data table with `Material (text)` and `S (ksi)` columns gives
  `S_allow := lookup(Material, S, "A106 B")` = 20 ksi (illustrative values). An unknown material is
  an error.
- **Piecewise coefficient** (syntax illustration): `Y := 0.4 if T ≤ 900 °F, 0.5 if T ≤ 950 °F,
  0.7 otherwise` gives the right value at both sides of each boundary, in °F and °C. It shows as a
  brace in WordCraft and in Word.
- **Per-element safety:** `sqrt(x) if x ≥ 0, 0 otherwise` over a vector with negative entries
  works, and `if(x ≠ 0, 1/x, 0)` doesn't fail on a zero element.
- **Sum:** a bolt-pattern sum `I := ∑(i = 0..length(r) − 1) A_b·r[i]^2` matches a hand-calculated
  value. Nested sums work, and an empty range, a shadowing index and a sum over mixed units are
  each errors.
- **Round trip:** saving to `.docx` and reopening keeps everything live, including a text column
  edited in Word.
- **Errors:** every anti-goal case above is an error, never a value.
- **Performance:** a 1,000-element piecewise check recalculates within the existing 100 ms budget.
- **Tests:** units for lexing, parsing, precedence and chaining, evaluation (text, comparison,
  logic, piecewise, `if()`, sums), display (linear text and Word math), quoting, data tables with
  text columns, and the `.docx` round trip.

## Calls made in the reviewer check (for the user to confirm)

- **Branches work on whole arrays,** with failures kept per element and only counted for the
  elements that pick that branch.
- **All six comparisons share the equality tolerance,** so a check exactly at its limit doesn't
  flip because of unit conversion.
- **An absolute °C/°F temperature can't be compared with a difference or an average** written as
  `(T_1 + T_2)/2`. This is an error, the same rule as showing such a value in °C.
- **A failed input still fails** a calculation that names it in any branch, as today.
- **`∑` covers one product term,** like `·`. Use parentheses to sum more.

## Left to the builder

- **Internal types:** the exact types for text values and grids (for example, generic grids), and
  where the new code lives in `wordcraft-calc`.
- **How per-element branching is done,** as long as an unpicked branch can never fail an element
  and the work budget still counts the work done.
- **Error wording,** following the existing style (plain, names the value or element).
- **The Word math structures** used for the brace, the bold `=` and the ∑, as long as Word shows
  them as described.
- **Whether Insert Symbol lists** the new operators alongside the existing ones (preferably yes).
