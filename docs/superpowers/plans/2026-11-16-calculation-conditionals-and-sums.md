# Calculation Conditionals, Text Values and Sums Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** WordCraft calculations gain:
- text values ("OK"/"NG"), single or in vectors, including text columns in data tables;
- comparisons and `and`/`or`/`not`;
- one-line piecewise definitions and `if(c, a, b)`, where a branch an element doesn't pick can never fail that element;
- sums with limits, `∑(i = 1..n) …`.

All of these survive a `.docx` round trip.

**Architecture:**

- **Text values.** Text is its own kind of value. `Data` gains `Text` and `TextGrid`, and `Grid` becomes generic over its element type, so the numeric code never sees text.
- **Per-element branching** uses *lenient* evaluation. While a branch is worked out over whole arrays, element-wise operations record each failing element in the grid's `failed` list instead of stopping.
  - The piecewise picks element by element.
  - A failed element only matters if it is picked.
  - Every whole-array consumer (array functions, indexing, storing a result) rejects a grid that still has failed elements.
- **Syntax.** Comparisons, logic, cases and sums are new `Expr` variants. Their displays are built as math nodes directly, like matrix literals today.

**Tech Stack:** Rust 2024 in the WordCraft workspace. The crates are `wordcraft-calc`, `wordcraft-doc` (math nodes, unchanged), `wordcraft-engine` and `wordcraft-ui-egui`, plus `craft-units` (pinned, unchanged).

**Spec:** `docs/superpowers/specs/2026-11-16-calculation-conditionals-and-sums-design.md`. Read it with this plan. It builds on `2026-11-15-calculation-vectors-and-tables-design.md`.

**Repo and branch:** `/home/alan/arath/storytold/wordcraft`, branch `feature/inline-calculations`. Commit there, and push nothing without the user's OK.

## Global Constraints

- **`AGENTS.md` rules apply:**
  - no `unsafe`;
  - no panics in non-test code (the calc crate denies `unwrap`/`expect`/`panic`/`unreachable`);
  - bounded recursion and allocation;
  - all source, table text and stored data are hostile input.
- **Commits carry no agent attribution lines.** This is the storytold rule, and it overrides the harness reminder.
- **Before each commit,** run `cargo fmt --check` and `cargo clippy -p <touched crates> --all-targets -- -D warnings`.
- **Seven tests fail before this work** because craft-i18n data and fonts are missing. They are not regressions; every other test must pass.
  - engine: `swiss_german_and_portugal…` and `review_language_validates…`;
  - ui-egui: `rtl_template_titles_are_right_aligned` and `interface_symbols_have_glyphs`;
  - three layout hyphenation tests.
- **Values pinned by the spec:**
  - a text value is at most **1,000 characters**;
  - comparisons treat values within a relative **1e-12**, after unit conversion, as equal (`|a − b| ≤ 1e-12 · max(|a|, |b|)` on SI values), for all six operators;
  - arrays hold at most 10,000 elements, and display cuts after 50 rows/columns (unchanged).
- **Reserved words:** `if`, `otherwise`, `and`, `or`, `not`.
- **Typed forms accepted:**
  - `<`, `>`, `<=`/`≤`, `>=`/`≥`, `==`, `!=`/`≠`, `∧`, `∨`, `¬`, `∑`;
  - text in `"…"` or `“…”`.
- **Display:**
  - `≤ ≥ ≠ < >` as typed;
  - `==` as a bold `=`;
  - `and`/`or`/`not` as `∧`/`∨`/`¬`;
  - text in straight quotes;
  - cases as a left brace over two columns (value | `if` condition / `otherwise`);
  - a sum as a Word n-ary ∑ with the limits below and above.
- **Stage 1 and vector documents open unchanged.** The new syntax never parsed before, so no stored source changes meaning.
- **Message wording, used verbatim:**
  - `"OK" is text — it can't be used in arithmetic` (the general form is `"{text}" is text — it can't be used {place}`, with the text cut to 40 characters plus `…`)
  - `can't compare "OK" with a number`
  - `can't compare an absolute temperature with a temperature difference — write an average as T_1 + (T_2 − T_1)/2`
  - `a condition must be a comparison or a plain number`
  - `{word} is a reserved word — choose another name`
  - `a list of cases (a if c, b otherwise) can only be the whole right side of a definition`
  - `no case applies (T = …)` (as an element error: `element 2: no case applies (…)`)
  - `element 3 is text but element 0 is a number` (the element names follow the origin)
  - `the sum's index i is already defined — choose another name`
  - `the sum's range is empty — its last value is below its first`
  - `a text needs a closing quote, like "OK"`
  - `a text is at most 1,000 characters`
  - `Material row 4 is empty`
  - `Material row 4 has a quote mark ", which text can't hold`

## Review Focus

These are the five inputs most likely to bite a user that no task's main tests cover. Each line names the task that owns the test.

1. **A stale display of a text result whose text contains `=`, `@`, `&` or `)`.** It must read back and show the same text with `≈` (Task 2).
2. **A data-table column in °F compared with `900 °F`.** The column must count as absolute temperatures, so `T ≤ 900 °F` works per element (Task 7).
3. **An old document that uses `and`, `or`, `not`, `if` or `otherwise` as a variable name.** That calculation shows the reserved-word error, and the rest of the document still calculates (Task 3).
4. **A piecewise inside a user function called with a vector.** For `f(x) := sqrt(x) if x ≥ 0, 0 otherwise`, `f([4, -1, 9])` gives `[2, 0, 3]` (Task 5).
5. **A sum inside a function whose range uses the parameter.** For `S(n) := ∑(i = 1..n) i`, `S(4)` gives 10 (Task 6).

---

### Task 1: Text values in the evaluator

**Files:**
- Modify: `crates/calc/src/ast.rs` (`Expr::Text`, `free_names`)
- Modify: `crates/calc/src/parse.rs` (`Kind::Text`, lexing, `prefix`)
- Modify: `crates/calc/src/array.rs` (generic `Grid<T>`, `Data::Text`/`TextGrid`, `MAX_TEXT_CHARS`)
- Modify: `crates/calc/src/lib.rs` (`Value::Text`/`TextGrid`, `CalcError::TextUse`)
- Modify: `crates/calc/src/eval.rs`, `crates/calc/src/array_funcs.rs`, `crates/calc/src/recalc.rs` (the compile fallout only; recalc formatting is Task 2)
- Test: `crates/calc/src/parse/tests.rs`, `crates/calc/src/eval/tests.rs`

**Interfaces:**
- **Produces:**
  - `Expr::Text(String)`: a text literal, without its quotes.
  - `pub(crate) const MAX_TEXT_CHARS: usize = 1000` in `array.rs`.
  - `pub(crate) struct Grid<T = Val> { rows, cols, items: Vec<T> }`, with `Grid::<T>::new(rows, cols, items) -> Result<Grid<T>, CalcError>`.
  - `pub(crate) enum Data { One(Val), Grid(Grid), Text(String), TextGrid(Grid<String>) }`.
  - `Value::Text(String)` and `Value::TextGrid { rows: usize, cols: usize, items: Vec<String> }`.
  - `CalcError::TextUse { text: String, place: &'static str }`, displayed as `"{text}" is text — it can't be used {place}`, with `text` cut to 40 characters plus `…`.
  - `pub(crate) fn text_use(text: &str, place: &'static str) -> CalcError` in `eval.rs`.
  - `pub(crate) fn mixed(rows, cols, text_at: usize, number_at: usize, origin: u8) -> CalcError` in `array.rs`, giving `element {k} is text but element {j} is a number` as `CalcError::Syntax`.
    - `k` is the later of the two elements and `j` the earlier.
    - When the text comes first, the message reads `element {k} is a number but element {j} is text`.
    - Element names use `element_name`.

- [ ] **Step 1: Write the failing tests.** In `parse/tests.rs`:

```rust
#[test]
fn text_literals() {
    assert_eq!(body("c := \"OK\""), Expr::Text("OK".into()));
    assert_eq!(body("c := “Use Schedule 80”"), Expr::Text("Use Schedule 80".into()));
    assert_eq!(body("c := [\"A\", \"B\"]"), Expr::Matrix { rows: 2, cols: 1, items: vec![Expr::Text("A".into()), Expr::Text("B".into())] });
    assert_eq!(parse("c := \"OK").unwrap_err().to_string(), "a text needs a closing quote, like \"OK\"");
    assert_eq!(parse(&format!("c := \"{}\"", "x".repeat(1001))).unwrap_err().to_string(), "a text is at most 1,000 characters");
    assert!(parse("c := 2 \"OK\"").is_err(), "text never multiplies by adjacency");
}
```

`body(src)` is the test helper returning a `Define`'s body; add it if missing.

In `eval/tests.rs`:

```rust
#[test]
fn text_values() {
    let mut env = Env::new();
    assert_eq!(run_value(&mut env, "c := \"OK\" ="), Ok(Some(Value::Text("OK".into()))));
    assert_eq!(run_value(&mut env, "v := [\"A\", \"B\"; \"C\", \"D\"] ="), Ok(Some(Value::TextGrid { rows: 2, cols: 2, items: vec!["A".into(), "B".into(), "C".into(), "D".into()] })));
    assert_eq!(run_value(&mut env, "v[1, 0] ="), Ok(Some(Value::Text("C".into()))));
    run_value(&mut env, "w := transpose(v)").unwrap();
    assert_eq!(run_value(&mut env, "w[0, 1] ="), Ok(Some(Value::Text("C".into()))));
    assert_eq!(val("length([\"A\", \"B\", \"C\"])"), Quantity::dimensionless(3.0));
    assert_eq!(err("\"OK\" + 1 ="), "\"OK\" is text — it can't be used in arithmetic");
    assert_eq!(err("-\"OK\" ="), "\"OK\" is text — it can't be used in arithmetic");
    assert_eq!(err("sqrt(\"OK\") ="), "\"OK\" is text — it can't be used in sqrt");
    assert_eq!(err("sum([\"A\", \"B\"]) ="), "\"A\" is text — it can't be used in sum");
    assert_eq!(err("\"OK\" = ? in"), "\"OK\" is text — it can't be used with a unit");
    assert_eq!(err("[\"A\", 2] ="), "element 1 is a number but element 0 is text");
    let long = "y".repeat(60);
    assert_eq!(err(&format!("\"{long}\" + 1 =")), format!("\"{}…\" is text — it can't be used in arithmetic", "y".repeat(40)));
    // Functions pass text through; indexed definitions build text vectors.
    run_value(&mut env, "label(x) := \"size\"").unwrap();
    assert_eq!(run_value(&mut env, "label(2) ="), Ok(Some(Value::Text("size".into()))));
}
```

`err(src)` returns the error's display string; adapt the existing helper if it returns `CalcError`.

- [ ] **Step 2: Run them and watch them fail.**
Run: `cargo test -p wordcraft-calc text_`
Expected: compile errors (`Expr::Text`, `Value::Text` not found).

- [ ] **Step 3: Implement.**
  - **Lexer:**
    - A literal opens with `"` or `“` and closes with `"` or `”`.
    - Its content may not contain `"`, `“` or `”`.
    - It gives `Kind::Text(String)`.
    - The errors are the two pinned messages above.
  - **Parser:**
    - `prefix` turns `Kind::Text` into `Expr::Text`.
    - `starts_factor` is false for text, so `2 "OK"` ends the expression and reports the leftover token.
  - **Grid:** make `Grid` generic, with `Val` as the default type parameter. `zip`/`map`/`build` stay `Grid<Val>` only.
  - **Evaluator:**
    - Arithmetic, unary minus, `|…|`, unit literals, scalar built-ins and ranges given `Data::Text`/`TextGrid` return `text_use`. The place is `in arithmetic`, `in {name}` or `in a range`.
    - Matrix literals are all text (`TextGrid`) or all numbers; anything else is `mixed`.
    - Indexing, `copy` (charged per element), user-function arguments and results, and indexed definitions work on text. An indexed definition mixing the two is `mixed`.
    - `check_display` of text is `text_use(…, "with a unit")`.
  - **Array functions:**
    - `length`, `rows`, `cols` and `transpose` accept text grids.
    - Every other array function given text returns `text_use(first text, "in {name}")`.
    - `lookup` with text is Task 7. Until then it uses the same `text_use` error.
  - **Public values:** `Data::value()` maps to the new `Value` variants.
  - **Recalc fallout:** in `recalc.rs`, for this task only, make `format_data`/`field_result` treat text as an error (`text_use(…, "in a display")`). Task 2 replaces this.

- [ ] **Step 4: Run them and watch them pass.**
Run: `cargo test -p wordcraft-calc`
Expected: PASS (all old tests too).

- [ ] **Step 5: Commit** with the message `feat(calc): text values`.

---

### Task 2: Showing and quoting text

**Files:**
- Modify: `crates/calc/src/render.rs` (`Shown`, `rendered`, `push_grid`, `shown_result`, writing `Expr::Text`)
- Modify: `crates/calc/src/recalc.rs` (`format_data`, `field_result`, `quoted_list`, `check_quote_unit`)
- Test: `crates/calc/src/render/tests.rs`, `crates/calc/tests/recalc.rs`

**Interfaces:**
- **Consumes:** `Data::Text`, `Data::TextGrid`, `Grid<String>` and `text_use` (Task 1).
- **Produces:** `Shown::Text(String)` and `Shown::TextGrid { rows, cols, items: Vec<String>, total_rows, total_cols }`, with the same cutting rule as `Shown::Grid`.
  - **Linear forms:** `="OK"` for a single text, and `=[■("OK"@"NG")]` for a grid, followed by the size note when cut.
  - **Math forms:** each text is one `MRun` whose text includes the straight quote marks, with `sty: Some(MSty::Plain)`.
  - **Text literals in expressions** are written the same way, both linear and as a run.

- [ ] **Step 1: Write the failing tests.** In `render/tests.rs`:

```rust
#[test]
fn text_results_round_trip() {
    for shown in [
        Shown::Text("OK".into()),
        Shown::Text("Use t = 0.2 in (min) & check @ site".into()),
        Shown::TextGrid { rows: 3, cols: 1, items: vec!["OK".into(), "a = b".into(), "x)]".into()], total_rows: 3, total_cols: 1 },
        Shown::TextGrid { rows: 50, cols: 1, items: vec!["NG".into(); 50], total_rows: 60, total_cols: 1 },
    ] {
        assert_eq!(shown_result(&shown_linear(&shown)), Some(shown));
    }
}

#[test]
fn text_shows_in_quotes() {
    let d = display_of("c := \"OK\" =", Some(&Shown::Text("OK".into())));
    assert!(d.linear.ends_with("=\"OK\""), "{}", d.linear);
    assert!(runs(&d.math).iter().any(|r| r.text == "\"OK\"" && r.sty == Some(MSty::Plain)));
}
```

Use the existing helpers; add `display_of`/`runs` if they're missing. They are thin wrappers over `rendered` and a walk collecting `MRun`s.

In `tests/recalc.rs`:

```rust
#[test]
fn text_results_show_and_quote() {
    let mut d = doc_with(&["c := \"OK\" =", "v := [\"OK\", \"NG\", \"OK\"] ="]);
    d.body.push(para_block(para(vec![field(&calc_field_instr(&QuoteSpec::named("c"))), field(&calc_field_instr(&QuoteSpec::named("v"))), field(&calc_field_instr(&QuoteSpec { index: Some((1, None)), ..QuoteSpec::named("v") }))])));
    recalc(&mut d, &no_page);
    assert!(region(&d, 1).1.ends_with("=\"OK\""));
    assert!(region(&d, 2).1.ends_with("=[■(\"OK\"@\"NG\"@\"OK\")]"));
    assert_eq!(field_results(&d), ["OK", "OK, NG, OK", "NG"]);
    assert!(check_quote_unit(&d, "c", "in").unwrap_err().to_string().contains("can't be used with a unit"));
    // Stale: the text result is read back and shown with ≈.
    mark_stale(&mut d);
    assert!(region(&d, 1).1.ends_with("≈\"OK\""));
}
```

`QuoteSpec::named` may not exist; build the `QuoteSpec` literal the way the existing quote tests do. Pin the Review Focus 1 case in the same test:

```rust
    let mut d = doc_with(&["c := \"t = 0.2 & up\" ="]);
    recalc(&mut d, &no_page);
    mark_stale(&mut d);
    assert!(region(&d, 1).1.ends_with("≈\"t = 0.2 & up\""));
```

- [ ] **Step 2: Run them and watch them fail.**
Run: `cargo test -p wordcraft-calc text_`
Expected: FAIL (no `Shown::Text`).

- [ ] **Step 3: Implement.**
  - **Read-back:** `shown_result` finds the result by scanning from the end, because texts can't contain `"`. Then:
    - a quoted run right after the last `=`/`≈` outside quotes is a text;
    - `[■(` followed by quoted cells is a text grid;
    - numeric results read back as today.
  - **Formatting and quoting:**
    - `format_data` maps text to `Shown::Text`/`TextGrid`, cut like numbers.
    - `field_result` quotes text bare. A text grid quotes as `quoted_list` does: `, ` between values, `; ` between matrix rows, then `, … (N values)` past the limit.
    - A text with a quote unit gives `?`.
    - `check_quote_unit` on text returns the `text_use` error.

- [ ] **Step 4: Run them and watch them pass.**
Run: `cargo test -p wordcraft-calc`
Expected: PASS.

- [ ] **Step 5: Commit** with the message `feat(calc): show and quote text results`.

---

### Task 3: Comparisons, logic and reserved words

**Files:**
- Modify: `crates/calc/src/parse.rs` (tokens, keywords, binding powers, comparison chains, `define_head`, the grammar comment)
- Modify: `crates/calc/src/ast.rs` (`Cmp`, `Logic`, `Expr::Compare`/`Logic`/`Not`, `free_names`)
- Modify: `crates/calc/src/eval.rs` (`compare`, `truth`, `check_name`)
- Modify: `crates/calc/src/array_funcs.rs` (move `equal` to a shared `pub(crate) fn nearly_equal(a: f64, b: f64) -> bool`)
- Modify: `crates/calc/src/lib.rs` (`CalcError::Keyword`, `CalcError::NotCondition`, `CalcError::TempCompare`, exports)
- Modify: `crates/calc/src/render.rs` (writing the new expressions)
- Modify: `crates/calc/src/tables.rs` (a heading that is a reserved word)
- Test: `parse/tests.rs`, `eval/tests.rs`, `render/tests.rs`, `tests/recalc.rs`

**Interfaces:**
- **Produces:**
  - `pub enum Cmp { Lt, Gt, Le, Ge, Eq, Ne }`
  - `pub enum Logic { And, Or }`
  - `Expr::Compare { first: Box<Expr>, rest: Vec<(Cmp, Expr)> }`, where `rest` is never empty and a chain `a < b ≤ c` is one `Compare`
  - `Expr::Logic(Logic, Box<Expr>, Box<Expr>)` and `Expr::Not(Box<Expr>)`
  - lexer kinds `Lt, Gt, Le, Ge, EqEq, Ne, And, Or, Not, If, Otherwise`
    - `If` and `Otherwise` are lexed here but parsed in Task 5. Until then they are "unexpected".
  - `CalcError::Keyword(String)`, displayed as `{0} is a reserved word — choose another name`
  - `CalcError::NotCondition`, displayed as `a condition must be a comparison or a plain number`
  - `CalcError::TempCompare`, displayed as `can't compare an absolute temperature with a temperature difference — write an average as T_1 + (T_2 − T_1)/2`
  - `pub(crate) fn text_key(s: &str) -> String`: the text trimmed, with every run of whitespace (including U+00A0) turned into one space
  - `pub(crate) fn truth(cx: &Cx, d: &Data) -> Result<Data, CalcError>`: checks a condition and gives the grid or single value of 1/0. Task 5 reuses it.
- **Binding powers, loosest first:**
  - `OR_BP = (2, 3)`
  - `AND_BP = (4, 5)`
  - the operand of a prefix `not` is parsed at `7`
  - `CMP_BP = (8, 9)`, with the existing `ADD_BP (10, 11)` above it
  - So `not a < b and c ≥ d or e` is `((not (a < b)) and (c ≥ d)) or e`.

- [ ] **Step 1: Write the failing tests.** In `parse/tests.rs`:

```rust
#[test]
fn comparisons_and_logic_parse() {
    let cmp = |a: Expr, rest: Vec<(Cmp, Expr)>| Expr::Compare { first: Box::new(a), rest };
    let n = |s: &str| Expr::Name(s.into());
    assert_eq!(expr("a <= b + 1"), cmp(n("a"), vec![(Cmp::Le, Expr::Binary(Op::Add, Box::new(n("b")), Box::new(Expr::Num(1.0))))]));
    assert_eq!(expr("0 ≤ x < 1"), cmp(Expr::Num(0.0), vec![(Cmp::Le, n("x")), (Cmp::Lt, Expr::Num(1.0))]));
    assert_eq!(expr("a == b"), cmp(n("a"), vec![(Cmp::Eq, n("b"))]));
    assert_eq!(expr("a != b"), expr("a ≠ b"));
    assert_eq!(expr("a >= b"), expr("a ≥ b"));
    assert_eq!(expr("not a < b and c or d"), expr("((¬(a < b)) ∧ c) ∨ d"));
    assert_eq!(expr("a ∧ b"), Expr::Logic(Logic::And, Box::new(n("a")), Box::new(n("b"))));
    // `==` is not the show `=`; a statement can still end in `=`.
    assert!(matches!(parse("a == b ="), Ok(Stmt::Eval { .. })));
    assert_eq!(parse("or := 5").unwrap_err().to_string(), "or is a reserved word — choose another name");
    assert_eq!(parse("f(if) := 5").unwrap_err().to_string(), "if is a reserved word — choose another name");
    assert!(parse("x := 1 ! 2").unwrap_err().to_string().contains("`!`"));
}
```

`expr(src)` parses `src =` and returns the `Eval` expression; add it if missing.

In `eval/tests.rs`:

```rust
#[test]
fn comparisons() {
    let t = |src: &str| val(src).value;
    assert_eq!(t("38.1 mm == 1.5 in"), 1.0);
    assert_eq!(t("38.1 mm > 1.5 in"), 0.0);
    assert_eq!(t("212 °F ≤ 100 °C"), 1.0);
    assert_eq!(t("100 °C == 212 °F"), 1.0);
    assert_eq!(t("0.1 + 0.2 == 0.3"), 1.0);
    assert_eq!(t("1e-20 == 0"), 0.0);
    assert_eq!(t("0 ≤ 0.5 ≤ 1"), 1.0);
    assert_eq!(t("0 ≤ 1.5 ≤ 1"), 0.0);
    assert_eq!(t("\"A106 B\" == \" A106\u{a0} B \""), 1.0);
    assert_eq!(t("\"a\" ≠ \"A\""), 1.0);
    assert!(matches!(run(&mut Env::new(), "1 in < 1 lbf ="), Err(CalcError::Unit(_))));
    assert_eq!(err("\"a\" < \"b\" ="), "\"a\" is text — it can't be used with <");
    assert_eq!(err("\"OK\" == 1 ="), "can't compare \"OK\" with a number");
    let v = vector("[1, 5, 9] > 4");
    assert_eq!(v, vec![0.0, 1.0, 1.0]);
}

#[test]
fn temperature_comparisons() {
    let mut env = Env::new();
    for src in ["T := 950 °F", "T_K := 700 K", "ΔT := 20 Δ°F", "T_1 := 100 °F", "T_2 := 200 °F"] {
        run(&mut env, src).unwrap();
    }
    assert_eq!(run(&mut env, "T ≤ 900 °F =").unwrap().unwrap().value, 0.0);
    assert_eq!(run(&mut env, "T ≤ 800 K =").unwrap().unwrap().value, 1.0); // 950 °F is 783.15 K
    assert_eq!(run(&mut env, "ΔT ≤ 10 Δ°F =").unwrap().unwrap().value, 0.0);
    assert_eq!(run(&mut env, "T_K > ΔT =").unwrap().unwrap().value, 1.0);
    let msg = "can't compare an absolute temperature with a temperature difference — write an average as T_1 + (T_2 − T_1)/2";
    assert_eq!(run(&mut env, "ΔT ≤ 10 °F =").unwrap_err().to_string(), msg);
    assert_eq!(run(&mut env, "(T_1 + T_2)/2 ≤ 900 °F =").unwrap_err().to_string(), msg);
    assert_eq!(run(&mut env, "T_1 + (T_2 - T_1)/2 ≤ 900 °F =").unwrap().unwrap().value, 1.0);
}

#[test]
fn logic() {
    let t = |src: &str| val(src).value;
    assert_eq!(t("1 < 2 and 3 < 4"), 1.0);
    assert_eq!(t("1 > 2 or not 3 > 4"), 1.0);
    assert_eq!(t("0.5 and 1"), 1.0); // nonzero is true
    assert_eq!(err("1 in and 1 ="), "a condition must be a comparison or a plain number");
    assert_eq!(err("\"OK\" or 1 ="), "\"OK\" is text — it can't be used as a condition");
    // A single false left side decides `and` without evaluating the right.
    assert_eq!(t("1 > 2 and undefined_name > 0"), 0.0);
    assert_eq!(t("1 < 2 or undefined_name > 0"), 1.0);
    assert_eq!(vector("[1, 0, 2] > 0 and [1, 1, 0] > 0"), vec![1.0, 0.0, 0.0]);
}
```

`vector(src)` evaluates `src` and returns the grid's plain values; add it if missing.

In `render/tests.rs`, the display test:

```rust
#[test]
fn comparisons_and_logic_display() {
    let lin = |src: &str| display_of(src, None).linear;
    assert!(lin("c := a <= b").contains("a≤b"));
    assert!(lin("c := a != b").contains("a≠b"));
    assert!(lin("c := a and not b or d").contains("a∧¬b∨d"));
    assert!(lin("c := 2·(a < b)").contains("2·(a<b)") || lin("c := 2·(a < b)").contains("2(a<b)"));
    let eq = display_of("c := a == b", None);
    assert!(runs(&eq.math).iter().any(|r| r.text == "=" && r.sty == Some(MSty::Bold)));
}
```

In `tests/recalc.rs`, Review Focus 3:

```rust
#[test]
fn a_reserved_word_name_fails_alone() {
    let mut d = doc_with(&["a := 1", "or := 2", "b := a + 1 ="]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 2).0.state, CalcState::Error("or is a reserved word — choose another name".into()));
    assert_eq!(region(&d, 3).0.state, CalcState::Fresh);
}
```

Also, in `tables.rs` tests, `heading("or (in)")` is `Err(CalcError::Keyword("or".into()))`.

- [ ] **Step 2: Run them and watch them fail.**
Run: `cargo test -p wordcraft-calc compar logic reserved`
Expected: compile errors.

- [ ] **Step 3: Implement.**
  - **Lexing:**
    - `=` followed by `=` is `EqEq`, checked before the lone-`=` stop.
    - `<=`, `>=` and `!=` are single tokens. A lone `!` is `unexpected character `!` (did you mean `!=`?)`.
    - The five words become keywords only as whole names.
  - **Chains:** in `expr_inner`, a comparison token at `CMP_BP` reads `rhs = self.expr(CMP_BP.1)` repeatedly while the next token is a comparison. It builds one `Compare`.
  - **Reserved words:** `define_head` reports a keyword as the name or as a parameter with `CalcError::Keyword`. So do `check_name` and `heading_name`.
  - **Evaluating a comparison:**
    - Evaluate each operand once, then compare element by element, broadcasting with `pair`-style shape rules and charging per element.
    - **Numbers:** dimensions must match (the same error `apply(Op::Sub)` gives). Compare SI values with `nearly_equal`:
      - `<` is `a < b && !eq`;
      - `≤` is `a < b || eq`;
      - `>` and `≥` mirror them.
    - **Temperatures,** checked first: if exactly one side is `Temp::Absolute` and the other is `Temp::Plain` with dimension temperature, return `TempCompare`.
    - **Text:** only `==`/`≠`, after `text_key` on both sides.
      - Any other operator is `text_use(text, "with <")`, using the operator's symbol.
      - Text against a number is `Syntax("can't compare \"{text}\" with a number")`.
    - **A chain** is the `and` of its links.
    - **The result** is dimensionless 1/0 with `Temp::Plain`.
  - **Logic:**
    - `truth` takes dimensionless numbers (nonzero is true). A unit is `NotCondition`; text is `text_use(…, "as a condition")`.
    - When the left side of `and`/`or` is a single value that decides the result, the right side isn't evaluated.
    - For grid left sides, evaluate both sides and combine per element (Task 5 makes the right side lenient).
  - **Names:** `free_names` walks the new nodes.
  - **Display:**
    - Comparisons are written with the symbols, and `and`/`or`/`not` as `∧`/`∨`/`¬`.
    - Parentheses go where a looser operator sits inside a tighter one.
    - `==` is a sentinel node holding one `MRun { text: "=", sty: Some(MSty::Bold) }`. Its linear text is `==`.

- [ ] **Step 4: Run them and watch them pass.**
Run: `cargo test -p wordcraft-calc`
Expected: PASS.

- [ ] **Step 5: Commit** with the message `feat(calc): comparisons, and/or/not, reserved words`.

---

### Task 4: Lenient grids (per-element failures)

**Files:**
- Modify: `crates/calc/src/array.rs` (`Grid.failed`, lenient `zip`/`map`, `Data::strict`)
- Modify: `crates/calc/src/eval.rs` (`Cx.lenient`, `eval_lenient`, the strict boundaries)
- Modify: `crates/calc/src/array_funcs.rs` (strict arguments)
- Test: `crates/calc/src/eval/tests.rs`, plus unit tests in `array.rs`

**Interfaces:**
- **Consumes:** generic `Grid<T>` (Task 1).
- **Produces:**
  - **`Grid<T>` gains `failed: Vec<(usize, CalcError)>`.** It is sorted by element, holds no `TooMuchWork`/`TooComplex`, and failed slots hold a placeholder item. `Grid::new` leaves it empty.
  - **`array::zip(a, b, origin, lenient: bool, f)` and `array::map(a, origin, lenient: bool, f)`:**
    - An input element in `failed` passes its error to the output element without calling `f`.
    - When `lenient`, an element error is recorded in `failed`.
    - Otherwise, the first error (an input failure, or `f`'s) is returned as `element_error`, as today.
  - **`pub(crate) fn strict(self, origin: u8) -> Result<Data, CalcError>` on `Data`:** the data unchanged when nothing failed. Otherwise it is `element_error` of the first failed element.
  - **`Cx` gains `lenient: Cell<u32>`.**
  - **`pub(crate) fn eval_lenient(cx: &Cx, fr: &Frame, e: &Expr, depth: usize) -> Result<Data, CalcError>`:** evaluates with `lenient` raised, then restores it, even on error.
  - **The strict boundaries.** Each of these calls `strict` on what it takes in:
    - every argument of `call_array`;
    - the base grid of an index;
    - each body result of an indexed definition;
    - the result of `run_stmt_in` and `eval_expr`;
    - `check_display`.

    User-function arguments, names and copies pass failed elements through.

- [ ] **Step 1: Write the failing tests.** In `eval/tests.rs`, with an internal helper that runs `eval_lenient` on a parsed expression in an env:

```rust
#[test]
fn lenient_grids_keep_element_failures() {
    let mut env = Env::new();
    run(&mut env, "x := [1, 0, 2]").unwrap();
    let d = lenient(&env, "1/x");
    let Data::Grid(g) = &d else { panic!("{d:?}") };
    assert_eq!(g.failed.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [1]);
    // Element-wise work carries the failure along; whole-array use refuses it.
    let Data::Grid(g) = lenient(&env, "2·(1/x) + 1") else { panic!() };
    assert_eq!(g.failed.len(), 1);
    assert_eq!(g.items[2].q.value, 2.0);
    assert!(lenient_err(&env, "sum(1/x)").starts_with("element 1:"));
    // Strict evaluation is unchanged.
    assert!(err_in(&mut env, "1/x =").starts_with("element 1:"));
}
```

`lenient`/`lenient_err` build a `Cx`, call `eval_lenient`, and return the `Data` or the error string. In `array.rs` tests, `map(…, true, …)` keeps going past an error, and `map(…, false, …)` on a grid with `failed` returns that element's error.

- [ ] **Step 2: Run them and watch them fail.**
Run: `cargo test -p wordcraft-calc lenient`
Expected: compile errors.

- [ ] **Step 3: Implement as the interfaces say.** `each`, `pair`, `call_each` and comparisons pass `cx.lenient.get() > 0`.

- [ ] **Step 4: Run them and watch them pass.**
Run: `cargo test -p wordcraft-calc` (all old tests too) and `cargo test -p wordcraft-calc --release perf_ -- --include-ignored`
Expected: PASS; the perf tests stay inside their budgets.

- [ ] **Step 5: Commit** with the message `feat(calc): lenient grids keep per-element failures`.

---

### Task 5: Piecewise definitions, `if()` and per-element short-circuit

**Files:**
- Modify: `crates/calc/src/parse.rs` (cases in `body`, the `if(` call, the misplaced-case error)
- Modify: `crates/calc/src/ast.rs` (`Expr::Cases`, `free_names`)
- Modify: `crates/calc/src/eval.rs` (`cases`, `if` in `call`, `and`/`or` with grid left sides)
- Modify: `crates/calc/src/render.rs` (the brace)
- Test: `parse/tests.rs`, `eval/tests.rs`, `render/tests.rs`

**Interfaces:**
- **Consumes:**
  - from Task 3: `truth`, `Cmp`, and the `If`/`Otherwise` kinds;
  - from Task 4: `eval_lenient`, `Grid.failed` and `Data::strict`;
  - from Task 1: `array::mixed`.
- **Produces:**
  - **`Expr::Cases { cases: Vec<(Expr, Expr)>, otherwise: Option<Box<Expr>> }`,** where each case pair is `(value, condition)`.
  - **`if(c, a, b)`** parses to `Expr::Call("if".into(), vec![c, a, b])` when `if` touches `(`. It is evaluated as `Cases { cases: [(a, c)], otherwise: Some(b) }`, and the wrong argument count is `Arity("if", 3)`.
  - **`free_names` never lists `if`.**
  - **`CalcError::Syntax` with the pinned message** `a list of cases (a if c, b otherwise) can only be the whole right side of a definition`. It applies to `if`/`otherwise` anywhere else, including in an `Eval` statement.

**The algorithm.** This is the one the signature doesn't determine.

```text
scalar mode: for each case in order:
    c = truth(eval(cond))       # lenient only if the caller is lenient
    if c is a single value: if true → return eval(value); else continue
    else → switch to vector mode with c's shape S; that case decides its true elements
vector mode:
    decided[k] = which case (or otherwise) element k takes; undecided starts as all not yet decided
    each later condition, only while some element is undecided:
        evaluated with eval_lenient; shape must be S or a single value (else Shape error);
        a failure of the whole condition → return it (every undecided element reaches it);
        a failed element k that is still undecided → element k fails;
        true elements become decided
    undecided elements take otherwise, or fail with "no case applies (…)"
    each value branch picked by at least one element: evaluated with eval_lenient;
        a whole-branch failure → return it; shape must be S or a single value;
        element k takes element k (or the single value); a failed element k → element k fails
    assemble: all numbers → Grid<Val> keeping each Val's Temp; all text → Grid<String>;
        a mix → array::mixed
    element failures: returned as element_error, or, when the caller is lenient,
        recorded in the result's failed list
```

The no-case message is `no case applies (` followed by up to 3 `name = value` entries for the variables (not units, functions or constants) named in the conditions, then `)`.
- Values use `craft_units::format` in the document's unit system and precision.
- For a vector, the element is the one that failed.
- Text is shown quoted.
- With no variables, it is just `no case applies`.

**`and`/`or` with a grid left side:** the right side is evaluated with `eval_lenient`. Element k:
- if the left side decides it, it is that result;
- otherwise it takes `truth` of the right side's element k, where a failed element fails element k.

- [ ] **Step 1: Write the failing tests.** In `parse/tests.rs`:

```rust
#[test]
fn cases_parse() {
    let Stmt::Define { body: Expr::Cases { cases, otherwise }, show, .. } = parse("Y := 0.4 if T ≤ 900 °F, 0.5 if T ≤ 950 °F, 0.7 otherwise =").unwrap() else { panic!() };
    assert_eq!(cases.len(), 2);
    assert_eq!(otherwise.as_deref(), Some(&Expr::Num(0.7)));
    assert!(show.is_some());
    assert!(matches!(body("c := \"OK\" if m ≥ 0 in"), Expr::Cases { otherwise: None, .. }));
    assert!(matches!(body("y := if(x > 0, 1, 2)"), Expr::Call(n, a) if n == "if" && a.len() == 3));
    let misplaced = "a list of cases (a if c, b otherwise) can only be the whole right side of a definition";
    assert_eq!(parse("y := (1 if a, 2 otherwise) + 1").unwrap_err().to_string(), misplaced);
    assert_eq!(parse("1 if a, 2 otherwise =").unwrap_err().to_string(), misplaced);
    assert!(parse("y := 1 otherwise").is_err());
    assert!(parse("y := 1 if a, 2 otherwise, 3 if b").is_err());
    assert!(!free_names(&parse("y := if(x > 0, 1, 2)").unwrap()).contains(&"if".to_string()));
}
```

In `eval/tests.rs`:

```rust
#[test]
fn piecewise() {
    let mut env = Env::new();
    let y = |env: &mut Env, t: &str| {
        run(env, &format!("T_{} := {t}", t.len())).unwrap();
        run(env, &format!("Y_{} := 0.4 if T_{} ≤ 900 °F, 0.5 if T_{} ≤ 950 °F, 0.7 otherwise =", t.len(), t.len(), t.len())).unwrap().unwrap().value
    };
    assert_eq!(y(&mut env, "900 °F"), 0.4);
    assert_eq!(y(&mut env, "900.1 °F"), 0.5);
    assert_eq!(y(&mut env, "482.2222222222222 °C"), 0.4); // 900 °F
    assert_eq!(y(&mut env, "1000 °F"), 0.7);
    run(&mut env, "T := 1000 °F").unwrap();
    let e = run(&mut env, "Z := 0.4 if T ≤ 900 °F, 0.5 if T ≤ 950 °F").unwrap_err().to_string();
    assert!(e.starts_with("no case applies (T = 1"), "{e}");
}

#[test]
fn per_element_branches() {
    let mut env = Env::new();
    run(&mut env, "x := [4, -1, 9, 0]").unwrap();
    assert_eq!(vector_in(&mut env, "r := sqrt(x) if x ≥ 0, 0 otherwise ="), vec![2.0, 0.0, 3.0, 0.0]);
    assert_eq!(vector_in(&mut env, "q := if(x ≠ 0, 1/x, 0) ="), vec![0.25, -1.0, 1.0 / 9.0, 0.0]);
    assert_eq!(vector_in(&mut env, "a := x ≠ 0 and 1/x > 0.2 ="), vec![1.0, 0.0, 0.0, 0.0]);
    // Whole arrays stay whole inside a branch.
    assert_eq!(vector_in(&mut env, "s := sum(x) if x > 0, 0 otherwise ="), vec![12.0, 0.0, 12.0, 0.0]);
    // A picked element that fails fails; an unpicked one doesn't.
    assert!(err_in(&mut env, "b := 1/x if x ≥ 0, 0 otherwise =").starts_with("element 3:"));
    // Text verdicts per element, and counting them.
    run(&mut env, "m := [0.1, -0.2, 0.3] in").unwrap();
    run(&mut env, "check := \"OK\" if m ≥ 0 in, \"NG\" otherwise").unwrap();
    assert_eq!(run_value(&mut env, "check ="), Ok(Some(Value::TextGrid { rows: 3, cols: 1, items: vec!["OK".into(), "NG".into(), "OK".into()] })));
    assert_eq!(run(&mut env, "sum(check == \"NG\") =").unwrap().unwrap().value, 1.0);
    assert_eq!(err_in(&mut env, "mix := \"OK\" if m ≥ 0 in, 0 otherwise ="), "element 1 is a number but element 0 is text");
    // No otherwise: the unmatched element names itself.
    assert!(err_in(&mut env, "z := 1 if m > 0 in =").starts_with("element 1: no case applies (m = "));
    // A single value may be text in one case and a number in another.
    assert_eq!(run_value(&mut env, "w := \"none\" if 1 > 2, 5 otherwise ="), Ok(Some(Value::One(Quantity::dimensionless(5.0)))));
    // Review Focus 4: inside a user function called with a vector.
    run(&mut env, "f(v) := sqrt(v) if v ≥ 0, 0 otherwise").unwrap();
    assert_eq!(vector_in(&mut env, "f([4, -1, 9]) ="), vec![2.0, 0.0, 3.0]);
    // A nested piecewise inside a lenient branch keeps its element failures to itself: g has no
    // case for x ≤ 0, but those elements never pick the branch that calls it.
    run(&mut env, "g(v) := 1 if v > 5, 2 if v > 0").unwrap();
    assert_eq!(vector_in(&mut env, "n := g(x) if x > 0, 0 otherwise ="), vec![2.0, 0.0, 1.0, 0.0]);
}
```

`vector_in(env, src)` and `err_in(env, src)` run in a given env; add them if missing.

In `render/tests.rs`:

```rust
#[test]
fn cases_show_as_a_brace() {
    let d = display_of("Y := 0.4 if T ≤ 900 °F, 0.7 otherwise", None);
    let brace = find_node(&d.math, |n| matches!(n, MNode::Delim { beg: Some('{'), end: None, .. }));
    let Some(MNode::Delim { elems, .. }) = brace else { panic!("{:?}", d.math) };
    assert!(matches!(elems.first().and_then(|e| e.first()), Some(MNode::Matrix { rows, .. }) if rows.len() == 2 && rows.iter().all(|r| r.len() == 2)));
    let words: Vec<String> = runs(&d.math).into_iter().map(|r| r.text).collect();
    assert!(words.iter().any(|w| w.trim() == "if") && words.iter().any(|w| w.trim() == "otherwise"));
    assert!(display_of("y := if(x > 0, 1, 2)", None).linear.contains("if(x>0,1,2)"));
}
```

`find_node` is a depth-first search helper; add it to the test module.

- [ ] **Step 2: Run them and watch them fail.**
Run: `cargo test -p wordcraft-calc cases piecewise per_element`
Expected: FAIL.

- [ ] **Step 3: Implement as described.**
  - **Parsing:** `body()` reads `value if cond` pairs separated by `,`, ending with `value otherwise` or the last case.
    - A condition is a whole expression at the top level, so `in_range_end` is false and commas end it.
    - `If`/`Otherwise` met anywhere else is the misplaced message.
  - **The brace display:** it is built like `Out::matrix`, as a sentinel node: `Delim { beg: Some('{'), end: None, elems: [[Matrix { rows: [[value], [Run("if "), cond…]] … }]] }`.
    - `"if "` and `"otherwise"` are upright plain runs.
    - The linear text is `{■(v&"if " c@…)┤`.

- [ ] **Step 4: Run them and watch them pass.**
Run: `cargo test -p wordcraft-calc`
Expected: PASS.

- [ ] **Step 5: Commit** with the message `feat(calc): piecewise definitions and if()`.

---

### Task 6: Sums with limits

**Files:**
- Modify: `crates/calc/src/parse.rs` (`Kind::Sum`, `Kind::SumEq`, parsing `∑(…)`)
- Modify: `crates/calc/src/ast.rs` (`Expr::Sum`, `free_names`)
- Modify: `crates/calc/src/eval.rs` (`sum`, and `range` taking the current frame)
- Modify: `crates/calc/src/render.rs` (the n-ary display)
- Test: `parse/tests.rs`, `eval/tests.rs`, `render/tests.rs`, `tests/recalc.rs`

**Interfaces:**
- **Consumes:** `range` (existing), changed to `range(cx, fr, first, second, last)` so a sum's ends can use parameters and outer indexes. Definitions pass `&Frame::GLOBAL` as before.
- **Produces:**
  - `Expr::Sum { index: String, first: Box<Expr>, second: Option<Box<Expr>>, last: Box<Expr>, body: Box<Expr> }`.
  - **Lexing:** `∑` (U+2211) is `Kind::Sum`. `=` is `Kind::SumEq` instead of the statement's show `=` when the three tokens before it are `Sum`, `LParen`, `Name`.
  - **Parsing:** `∑(i = first..last) body` or `∑(i = first, second..last) body`.
    - The ends are parsed like range values.
    - `body` is parsed with `self.expr(MUL_BP.0)`, so it covers one product term: `∑(…) a·b + 1` is `(∑ a·b) + 1`.
  - **`free_names`:** names in the ends and the body, without the index inside the body. The index itself is listed as used (the spec's dependency rule).

**Evaluation rules:**
- **The index name:**
  - `check_name(index)` must pass (no units, built-ins, constants or keywords);
  - it must not be a variable, a function, a failed name, a parameter or an enclosing sum's index.

  Otherwise the error is `the sum's index {i} is already defined — choose another name`.
- **The range:**
  - its values come from `range`;
  - with no `second` and `last < first`, the error is `the sum's range is empty — its last value is below its first`.
- **The terms:**
  - the body is evaluated once per index value, with the index bound in the frame;
  - terms add with `apply(Op::Add)` element by element (`pair`), so units must agree and shapes must match;
  - text is `text_use(…, "in a sum")`;
  - work is charged per term.

- [ ] **Step 1: Write the failing tests.** In `eval/tests.rs`:

```rust
#[test]
fn sums() {
    let mut env = Env::new();
    for src in ["A_b := 0.5 in^2", "r := [2, 3, 4] in", "a := [1, 2, 3]", "b := [4, 5, 6]"] {
        run(&mut env, src).unwrap();
    }
    assert_eq!(run(&mut env, "∑(i = 0..2) a[i]·b[i] =").unwrap().unwrap().value, 32.0);
    assert_eq!(run(&mut env, "∑(i = 0..2) a[i]·b[i] + 1 =").unwrap().unwrap().value, 33.0);
    let i = run(&mut env, "I := ∑(i = 0..length(r) - 1) A_b·r[i]^2 = ? in^4").unwrap().unwrap();
    assert!((i.value - 0.5 * (4.0 + 9.0 + 16.0) * 0.0254_f64.powi(4)).abs() < 1e-15);
    assert_eq!(run(&mut env, "∑(i = 1..3) ∑(j = 1..i) j =").unwrap().unwrap().value, 10.0);
    assert_eq!(run(&mut env, "∑(k = 1, 3..7) k =").unwrap().unwrap().value, 16.0);
    // Two sums may share an index; a defined name may not be one.
    assert_eq!(run(&mut env, "∑(i = 1..2) i + ∑(i = 1..3) i =").unwrap().unwrap().value, 9.0);
    assert_eq!(run(&mut env, "∑(a = 1..2) a =").unwrap_err().to_string(), "the sum's index a is already defined — choose another name");
    assert_eq!(run(&mut env, "∑(i = 1..2) ∑(i = 1..2) i =").unwrap_err().to_string(), "the sum's index i is already defined — choose another name");
    assert!(matches!(run(&mut env, "∑(m = 1..2) m ="), Err(CalcError::ReservedName(_))));
    assert_eq!(run(&mut env, "∑(i = 1..0) i =").unwrap_err().to_string(), "the sum's range is empty — its last value is below its first");
    assert!(run(&mut env, "∑(i = 0..2) (a[i] + r[i]) =").is_err(), "mixed units");
    assert!(matches!(run(&mut env, "∑(i = 1..1e9) i ="), Err(CalcError::TooLarge | CalcError::TooMuchWork)));
    // Review Focus 5: a parameter in the range.
    run(&mut env, "S(n) := ∑(i = 1..n) i").unwrap();
    assert_eq!(run(&mut env, "S(4) =").unwrap().unwrap().value, 10.0);
}
```

In `render/tests.rs`:

```rust
#[test]
fn sums_show_as_nary() {
    let d = display_of("I := ∑(i = 0..n) a[i]·b[i]", None);
    let Some(MNode::Nary { chr: '∑', sub, sup, e, .. }) = find_node(&d.math, |n| matches!(n, MNode::Nary { .. })) else { panic!("{:?}", d.math) };
    assert!(!sub.is_empty() && !sup.is_empty() && !e.is_empty());
}
```

In `tests/recalc.rs`, the dependency rule:

```rust
#[test]
fn a_sum_index_defined_elsewhere_is_reported() {
    let mut d = doc_with(&["s := ∑(i = 1..3) i =", "i := 0..5"]);
    recalc(&mut d, &no_page);
    assert_eq!(region(&d, 1).0.state, CalcState::Error("the sum's index i is already defined — choose another name".into()));
}
```

- [ ] **Step 2: Run them and watch them fail.**
Run: `cargo test -p wordcraft-calc sum`
Expected: FAIL.

- [ ] **Step 3: Implement as the interfaces say.**
  - The display is a sentinel node `Nary { chr: '∑', sub: [index, "=", first(, second)], sup: [last], e: [body] }`.
  - The linear text is `∑_(i=first)^last▒〖body〗`.
  - Also update the grammar comment at the top of `parse.rs` for everything Tasks 3–6 added.

- [ ] **Step 4: Run them and watch them pass.**
Run: `cargo test -p wordcraft-calc`
Expected: PASS.

- [ ] **Step 5: Commit** with the message `feat(calc): sums with limits`.

---

### Task 7: Text columns in data tables, and text lookup

**Files:**
- Modify: `crates/calc/src/tables.rs` (`heading`, `column`)
- Modify: `crates/calc/src/array_funcs.rs` (`lookup`)
- Test: `tables.rs` tests, `tests/recalc.rs`

**Interfaces:**
- **Consumes:** `Expr::Text` and `Grid<String>` (Task 1), `text_key` (Task 3).
- **Produces:**
  - `pub(crate) enum ColumnKind { Plain, Unit(String), Text }`.
  - `heading(text) -> Result<(String, ColumnKind), CalcError>`. The unit `text`, in any case, is `ColumnKind::Text`.
  - **A text column's definition** is `Expr::Matrix` of `Expr::Text`.
  - **What a text cell holds:** its `cell_text`, trimmed, with these errors:
    - an empty cell is `{name} row {k} is empty`;
    - a `"` is `{name} row {k} has a quote mark ", which text can't hold`;
    - more than 1,000 characters is `{name} row {k} is longer than 1,000 characters`.
  - **`lookup(xs, ys, x)`:**
    - `xs` may be a text vector, with `x` a text or text vector, matched with `text_key`.
    - `ys` may be text, and the result follows `ys`'s kind.
    - A text `x` against numeric `xs` (or the reverse) is `can't compare "…" with a number`.
    - The messages follow the numeric ones: `"A106 C" isn't in Material` and `"A106 B" matches more than one value in Material`.

- [ ] **Step 1: Write the failing tests.** In `tests/recalc.rs`:

```rust
const MATERIALS: &[&[&str]] = &[&["Material (text)", "S (ksi)"], &["A106 B", "20"], &["A312 TP316", "20"], &["A333 6", "20"]];

#[test]
fn text_columns_feed_lookups() {
    let mut d = doc_of(vec![
        table_block(data_table(1, MATERIALS)),
        para_block(para(vec![calc_eq(8, "S_allow := lookup(Material, S, \"A106 B\") =")])),
        para_block(para(vec![calc_eq(9, "grade := lookup(S, Material, 20 ksi) =")])),
        para_block(para(vec![calc_eq(10, "bad := lookup(Material, S, \"A106 C\") =")])),
    ]);
    recalc(&mut d, &no_page);
    assert!(region(&d, 8).1.ends_with("=20 \"ksi\""), "{}", region(&d, 8).1);
    assert!(matches!(region(&d, 9).0.state, CalcState::Error(ref m) if m.contains("matches more than one value in S")));
    assert_eq!(region(&d, 10).0.state, CalcState::Error("\"A106 C\" isn't in Material".into()));
    set_cell(&mut d, 0, 2, 0, "");
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors, [ColumnError { col: Some(0), message: "Material row 2 is empty".into() }]);
    set_cell(&mut d, 0, 2, 0, "1/2\" pipe");
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors[0].message, "Material row 2 has a quote mark \", which text can't hold");
    // A number in a text column is text; a numeric column stays strict.
    set_cell(&mut d, 0, 2, 0, "6");
    set_cell(&mut d, 0, 3, 1, "2O");
    recalc(&mut d, &no_page);
    assert_eq!(table_calc(&d, 0).errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(), ["S row 3 isn't a number"]);
}

#[test]
fn a_fahrenheit_column_compares_as_temperatures() {
    // Review Focus 2.
    let mut d = doc_of(vec![table_block(data_table(1, &[&["T (°F)"], &["800"], &["950"]])), para_block(para(vec![calc_eq(8, "Y := 0.4 if T ≤ 900 °F, 0.7 otherwise =")]))]);
    recalc(&mut d, &no_page);
    assert!(region(&d, 8).1.ends_with("=[■(0.4@0.7)]"), "{}", region(&d, 8).1);
}
```

In `tables.rs` tests, `heading("Material (Text)")` is `Ok(("Material".into(), ColumnKind::Text))`.

- [ ] **Step 2: Run them and watch them fail.**
Run: `cargo test -p wordcraft-calc text_columns fahrenheit`
Expected: FAIL.

- [ ] **Step 3: Implement as the interfaces say.**

- [ ] **Step 4: Run them and watch them pass.**
Run: `cargo test -p wordcraft-calc`
Expected: PASS.

- [ ] **Step 5: Commit** with the message `feat(calc): text columns in data tables and text lookup`.

---

### Task 8: Typing aids and the guide

**Files:**
- Modify: `crates/ui-egui/src/calc_ui.rs` (`SHORTCUTS`, the Insert Symbol "Operators" row)
- Modify: `docs/calculations.md` ("Symbols", and a new section "Conditions, text and sums" after "Vectors, matrices and data tables")
- Test: `crates/ui-egui/src/tests/calc.rs`

**Interfaces:**
- **Produces:**
  - shortcuts `\le` → `≤`, `\ge` → `≥`, `\ne` → `≠`, `\sum` → `∑`, `\wedge` → `∧`, `\vee` → `∨`, `\neg` → `¬`;
  - the Operators row gains `≤ ≥ ≠ ∑ ∧ ∨ ¬`.

- [ ] **Step 1: Write the failing test** in `tests/calc.rs`:

```rust
#[test]
fn condition_shortcuts_expand() {
    for (typed, symbol) in [("\\le ", "≤"), ("\\ge ", "≥"), ("\\ne ", "≠"), ("\\sum ", "∑"), ("\\wedge ", "∧"), ("\\vee ", "∨"), ("\\neg ", "¬")] {
        let n = typed.chars().count();
        assert_eq!(calc_ui::expand_shortcut(typed, n), Some((symbol.to_string(), 1)), "{typed}");
    }
    assert_eq!(calc_ui::expand_shortcut("\\nu ", 4), Some(("ν".to_string(), 1)), "Greek still wins its own names");
}
```

- [ ] **Step 2: Run it and watch it fail.**
Run: `cargo test -p wordcraft-ui-egui condition_shortcuts`
Expected: FAIL.

- [ ] **Step 3: Implement the shortcuts and the Operators row.** Then write the guide section, in US spelling and in the guide's existing style. It covers:
  - **Text values:** quotes; what text can and can't do; display and quoting.
  - **Comparisons:** typed forms; units first; the equality tolerance with the `38.1 mm == 1.5 in` example; comparing to zero with a tolerance (`abs(x) < 1e-9 m`); chains; the temperature rule and the average advice.
  - **Logic:** `and`/`or`/`not`, shown as `∧`/`∨`/`¬`.
  - **Cases:** the B31.3 `Y` example (marked illustrative); first true case wins; no `otherwise` is an error; per-element branching with the `sqrt(x) if x ≥ 0` example; mixing text and numbers.
  - **`if(c, a, b)`.**
  - **Sums:** the bolt-pattern example; the local index; one product term, so use parentheses to sum more; an empty range is an error.
  - **Text columns:** `Material (text)` and the lookup example.
  - **The check-and-count pattern:** `check := "OK" if margin ≥ 0 in, "NG" otherwise` and `n_NG := sum(check == "NG")`.
  - **Reserved words.**

  Add the seven shortcuts to "Symbols".

- [ ] **Step 4: Run it and watch it pass.**
Run: `cargo test -p wordcraft-ui-egui condition_shortcuts`
Expected: PASS.

- [ ] **Step 5: Commit** with the message `feat(ui): condition and sum shortcuts; guide for conditions, text and sums`.

---

### Task 9: Samples, round trip, performance and full verification

**Files:**
- Modify: `crates/engine/src/cmd/calc/pipe_wall_sample.rs` (the check, the count, a materials sample)
- Modify: `crates/calc/tests/recalc.rs` (`perf_1000_element_piecewise`)
- Test: these files

**Interfaces:**
- **Consumes:** everything above, through engine commands (`calc`, `data_table`, `retype_cell` and the `file.open` round trip in `pipe_wall_sample.rs`).

- [ ] **Step 1: Write the failing tests.**
  - **The B31.3 sample:** in `pipe_wall_sample.rs`, add `check := "OK" if margin ≥ 0 in, "NG" otherwise =` and `n_NG := sum(check == "NG") =` to `pipe_wall()`'s sources. Update the region count to 9 in `pipe_wall_sample`, and add:

```rust
#[test]
fn pipe_wall_check_flips_per_size() {
    let mut s = pipe_wall();
    shows(&s, "check", "=[■(\"OK\"@\"OK\"@\"OK\")]");
    shows(&s, "n_NG", "=0");
    retype_cell(&mut s, 2, 2, "0.020");
    shows(&s, "check", "=[■(\"OK\"@\"NG\"@\"OK\")]");
    shows(&s, "n_NG", "=1");
}

#[test]
fn materials_sample_reopens_live_with_a_word_edit() {
    let mut s = session();
    data_table(&mut s, &[&["Material (text)", "S (ksi)"], &["A106 B", "20"], &["A312 TP316", "20"]]);
    calc(&mut s, "S_allow := lookup(Material, S, \"A312 TP316\") =");
    calc(&mut s, "Y := 0.4 if S_allow ≤ 20 ksi, 0.7 otherwise =");
    calc(&mut s, "Σ_S := ∑(i = 0..length(S) - 1) S[i] =");
    shows(&s, "S_allow", "=20 \"ksi\"");
    let bytes = crate::io::save_bytes("materials.docx", &s.doc).unwrap();
    let mut back = session();
    run(&mut back, "file.open", json!({"path": "materials.docx", "data": crate::cmd::insert::base64_encode(&bytes)}));
    assert!(!back.status.contains("edited in Word"), "{}", back.status);
    for (r, linear) in regions(&s) {
        assert_eq!(region(&back, &r.source).1, linear, "{}", r.source);
    }
    retype_cell(&mut back, 2, 1, "16.7");
    shows(&back, "S_allow", "=16.7 \"ksi\"");
    shows(&back, "Y", "=0.4");
}
```

  - **Word edits of a text column:** mirror the existing test at `crates/engine/src/cmd/calc/tests.rs:1289` with a text cell changed (`A106 B` → `A106 C`) in the saved Word document. The status must read `Calculation data was edited in Word in 1 table; results were recalculated`, and a lookup of `"A106 B"` becomes an error.
  - **Performance:** in `tests/recalc.rs`, add `perf_1000_element_piecewise`, shaped like `perf_1000_element_vector_chain`:
    - a 1,000-row `m (in)` column;
    - `check := "OK" if m ≥ 10 in, "NG" otherwise`, `n := sum(check == "NG")`, and 50 further piecewise regions `y_k := m·1.001 if m > k in, m otherwise`;
    - a cell edit then recalculates in under 100 ms (release).

- [ ] **Step 2: Run them and watch them fail or pass.**
Run: `cargo test -p wordcraft-engine pipe_wall materials`
Expected: PASS once Tasks 1–7 are in. A failure here is a real integration bug: fix it in the owning crate and add a unit test there.

- [ ] **Step 3: Run the full verification.**
Run: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo test -p wordcraft-calc --release perf_ -- --include-ignored`
Expected:
  - fmt and clippy are clean;
  - only the seven known environment failures fail;
  - the perf tests pass.

  Also run `cargo build --release -p wordcraft`.

- [ ] **Step 4: Commit** with the message `test: conditions, text and sums in the samples; round trip and perf`.
