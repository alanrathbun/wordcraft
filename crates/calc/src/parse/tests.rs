use super::parse;
use crate::{CalcError, Expr, Op, Show, Stmt, defined_name, free_names};
use proptest::prelude::*;

fn num(v: f64) -> Expr {
    Expr::Num(v)
}
fn name(n: &str) -> Expr {
    Expr::Name(n.into())
}
fn bin(op: Op, a: Expr, b: Expr) -> Expr {
    Expr::Binary(op, Box::new(a), Box::new(b))
}
fn neg(a: Expr) -> Expr {
    Expr::Unary(Op::Neg, Box::new(a))
}
fn call(n: &str, args: Vec<Expr>) -> Expr {
    Expr::Call(n.into(), args)
}
fn show(unit: Option<&str>) -> Show {
    Show { unit: unit.map(str::to_string) }
}
/// The expression of an `Eval` (the statement must end in `=`).
fn expr_of(src: &str) -> Expr {
    match parse(&format!("{src} =")) {
        Ok(Stmt::Eval { expr, .. }) => expr,
        other => panic!("{src:?} did not parse as an Eval: {other:?}"),
    }
}
fn syntax(src: &str) {
    assert!(matches!(parse(src), Err(CalcError::Syntax(_))), "{src:?} should be a syntax error, got {:?}", parse(src));
}

#[test]
fn define_with_units() {
    assert_eq!(
        parse("P := 1500 lbf"),
        Ok(Stmt::Define { name: "P".into(), params: vec![], body: bin(Op::Mul, num(1500.0), name("lbf")), show: None })
    );
}

#[test]
fn define_and_show() {
    assert_eq!(
        parse("σ := P/A_b ="),
        Ok(Stmt::Define { name: "σ".into(), params: vec![], body: bin(Op::Div, name("P"), name("A_b")), show: Some(show(None)) })
    );
}

#[test]
fn define_and_show_with_unit() {
    let want = Stmt::Define { name: "σ".into(), params: vec![], body: bin(Op::Div, name("P"), name("A_b")), show: Some(show(Some("ksi"))) };
    assert_eq!(parse("σ := P/A_b = ? ksi"), Ok(want.clone()));
    assert_eq!(parse("σ ≔ P/A_b = ksi"), Ok(want));
}

#[test]
fn eval_with_unit() {
    let want = Ok(Stmt::Eval { expr: name("σ"), show: show(Some("ksi")) });
    assert_eq!(parse("σ = ? ksi"), want);
    assert_eq!(parse("σ = ksi"), want);
    assert_eq!(parse("σ ="), Ok(Stmt::Eval { expr: name("σ"), show: show(None) }));
    assert_eq!(parse("σ = ?"), Ok(Stmt::Eval { expr: name("σ"), show: show(None) }));
    assert_eq!(parse("σ =   "), Ok(Stmt::Eval { expr: name("σ"), show: show(None) }));
}

#[test]
fn show_unit_is_verbatim_and_unparsed() {
    for unit in ["lbf/in^2", "kN·m", "°F", "Δ°F", "µm", "m/s^2", "% weird (unit)"] {
        assert_eq!(parse(&format!("x = ? {unit}")), Ok(Stmt::Eval { expr: name("x"), show: show(Some(unit)) }));
    }
}

#[test]
fn function_def() {
    assert_eq!(
        parse("f(x, y) := x^2 + y"),
        Ok(Stmt::Define {
            name: "f".into(),
            params: vec!["x".into(), "y".into()],
            body: bin(Op::Add, bin(Op::Pow, name("x"), num(2.0)), name("y")),
            show: None
        })
    );
}

#[test]
fn subscripted_greek() {
    assert_eq!(
        parse("σ_allow := 20 ksi"),
        Ok(Stmt::Define { name: "σ_allow".into(), params: vec![], body: bin(Op::Mul, num(20.0), name("ksi")), show: None })
    );
    assert_eq!(expr_of("P_1 + A_b"), bin(Op::Add, name("P_1"), name("A_b")));
}

#[test]
fn precedence() {
    assert_eq!(expr_of("a + b*c^2"), bin(Op::Add, name("a"), bin(Op::Mul, name("b"), bin(Op::Pow, name("c"), num(2.0)))));
    assert_eq!(expr_of("a^b^c"), bin(Op::Pow, name("a"), bin(Op::Pow, name("b"), name("c"))));
    assert_eq!(expr_of("a - b - c"), bin(Op::Sub, bin(Op::Sub, name("a"), name("b")), name("c")));
    assert_eq!(expr_of("a / b / c"), bin(Op::Div, bin(Op::Div, name("a"), name("b")), name("c")));
    assert_eq!(expr_of("(a + b) * c"), bin(Op::Mul, bin(Op::Add, name("a"), name("b")), name("c")));
}

#[test]
fn unary_minus_binds_looser_than_pow() {
    assert_eq!(expr_of("-x^2"), neg(bin(Op::Pow, name("x"), num(2.0))));
    assert_eq!(expr_of("2^-x"), bin(Op::Pow, num(2.0), neg(name("x"))));
    assert_eq!(expr_of("-a*b"), bin(Op::Mul, neg(name("a")), name("b")));
    assert_eq!(expr_of("a - -b"), bin(Op::Sub, name("a"), neg(name("b"))));
    assert_eq!(expr_of("+a"), name("a"));
    assert_eq!(expr_of("a \u{2212} b"), bin(Op::Sub, name("a"), name("b")));
}

#[test]
fn free_names_excludes_params() {
    let s = parse("f(x) := x*k + g(x)").unwrap();
    assert_eq!(free_names(&s), vec!["k".to_string(), "g".to_string()]);
    assert_eq!(defined_name(&s), Some("f"));
}

#[test]
fn free_names_in_first_use_order_without_duplicates() {
    let s = parse("c := b*a + |b| + sqrt(d) + a").unwrap();
    assert_eq!(free_names(&s), vec!["b", "a", "sqrt", "d"]);
    let s = parse("x + y*x =").unwrap();
    assert_eq!(free_names(&s), vec!["x", "y"]);
    assert_eq!(defined_name(&s), None);
    assert!(free_names(&parse("2 + 3 =").unwrap()).is_empty());
}

#[test]
fn hostile_sources_error_not_panic() {
    assert_eq!(parse(&"(".repeat(200)), Err(CalcError::TooComplex));
    assert_eq!(parse(&format!("x:={}", "1+".repeat(60_000))), Err(CalcError::TooComplex));
    assert!(matches!(parse(":= ="), Err(CalcError::Syntax(_))));
    assert!(matches!(parse("\u{0}"), Err(CalcError::Syntax(_))));
}

#[test]
fn source_size_limit_is_16_kib() {
    let at_limit = format!("x := 1{}", " ".repeat(16 * 1024 - 6));
    assert_eq!(at_limit.len(), 16 * 1024);
    assert!(parse(&at_limit).is_ok());
    assert_eq!(parse(&format!("{at_limit} ")), Err(CalcError::TooComplex));
}

#[test]
fn depth_limits() {
    // Each shape is accepted up to the limit (64 levels of nesting or tree height) and refused
    // at one more.
    let ok = |src: String| assert!(parse(&src).is_ok(), "{} should parse", &src[..src.len().min(40)]);
    let too_complex = |src: String| assert_eq!(parse(&src), Err(CalcError::TooComplex), "{}", &src[..src.len().min(40)]);

    let parens = |n: usize| format!("{}x{} =", "(".repeat(n), ")".repeat(n));
    ok(parens(64));
    too_complex(parens(65));

    // A flat chain builds a left-leaning tree: k additions are k + 1 levels tall.
    let chain = |k: usize| format!("x := {}1", "1+".repeat(k));
    ok(chain(63));
    too_complex(chain(64));

    let negations = |k: usize| format!("x := {}1", "-".repeat(k));
    ok(negations(63));
    too_complex(negations(64));

    let powers = |k: usize| format!("x := {}1", "2^".repeat(k));
    ok(powers(63));
    too_complex(powers(64));

    let abs = |k: usize| format!("x := {}1{}", "|".repeat(k), "|".repeat(k));
    ok(abs(63));
    too_complex(abs(64));

    let calls = |k: usize| format!("x := {}1{}", "f(".repeat(k), ")".repeat(k));
    ok(calls(63));
    too_complex(calls(64));

    // Far beyond any limit.
    too_complex(parens(5_000));
    too_complex(chain(5_000));
}

#[test]
fn token_count_is_capped() {
    // Under 16 kB of source, but more than 10 000 tokens.
    let src = format!("x := {}1", "-".repeat(12_000));
    assert!(src.len() < 16 * 1024);
    assert_eq!(parse(&src), Err(CalcError::TooComplex));
    assert_eq!(parse(&",".repeat(12_000)), Err(CalcError::TooComplex));
}

#[test]
fn numbers() {
    assert_eq!(expr_of("1.5e3"), num(1500.0));
    assert_eq!(expr_of("2E-4"), num(2e-4));
    assert_eq!(expr_of("1e+2"), num(100.0));
    assert_eq!(expr_of(".5"), num(0.5));
    assert_eq!(expr_of("7."), num(7.0));
    assert_eq!(expr_of("2.75"), num(2.75));
    syntax("1e999 =");
    syntax("1.5.3 =");
    syntax(". =");
}

#[test]
fn number_then_e_is_a_name_not_an_exponent() {
    assert_eq!(expr_of("2eV"), bin(Op::Mul, num(2.0), name("eV")));
    assert_eq!(expr_of("2e"), bin(Op::Mul, num(2.0), name("e")));
    assert_eq!(expr_of("2e3 m"), bin(Op::Mul, num(2000.0), name("m")));
}

#[test]
fn implicit_multiplication() {
    assert_eq!(expr_of("2 P"), bin(Op::Mul, num(2.0), name("P")));
    assert_eq!(expr_of("2P"), bin(Op::Mul, num(2.0), name("P")));
    assert_eq!(expr_of("P A"), bin(Op::Mul, name("P"), name("A")));
    assert_eq!(expr_of("2 (a + b)"), bin(Op::Mul, num(2.0), bin(Op::Add, name("a"), name("b"))));
    assert_eq!(expr_of("(a)(b)"), bin(Op::Mul, name("a"), name("b")));
    assert_eq!(expr_of("a b c"), bin(Op::Mul, bin(Op::Mul, name("a"), name("b")), name("c")));
    assert_eq!(expr_of("2x^2"), bin(Op::Mul, num(2.0), bin(Op::Pow, name("x"), num(2.0))));
    assert_eq!(expr_of("x^2 y"), bin(Op::Mul, bin(Op::Pow, name("x"), num(2.0)), name("y")));
    assert_eq!(expr_of("a + 2 b"), bin(Op::Add, name("a"), bin(Op::Mul, num(2.0), name("b"))));
    // A minus after a factor is subtraction, not a negative factor.
    assert_eq!(expr_of("a -b"), bin(Op::Sub, name("a"), name("b")));
}

#[test]
fn multiplication_and_division_symbols() {
    let want = bin(Op::Mul, name("a"), name("b"));
    for sym in ["*", "·", "×", "\u{22C5}"] {
        assert_eq!(expr_of(&format!("a {sym} b")), want, "{sym}");
    }
    assert_eq!(expr_of("a ÷ b"), bin(Op::Div, name("a"), name("b")));
    assert_eq!(expr_of("5 kN·m"), bin(Op::Mul, bin(Op::Mul, num(5.0), name("kN")), name("m")));
    assert_eq!(expr_of("10 lbf/in^2"), bin(Op::Div, bin(Op::Mul, num(10.0), name("lbf")), bin(Op::Pow, name("in"), num(2.0))));
}

#[test]
fn call_needs_the_parenthesis_right_after_the_name() {
    assert_eq!(expr_of("f(x)"), call("f", vec![name("x")]));
    assert_eq!(expr_of("f (x)"), bin(Op::Mul, name("f"), name("x")));
    assert_eq!(expr_of("max(a, 2 b, c)"), call("max", vec![name("a"), bin(Op::Mul, num(2.0), name("b")), name("c")]));
    assert_eq!(expr_of("f()"), call("f", vec![]));
    assert_eq!(expr_of("sin(x)^2"), bin(Op::Pow, call("sin", vec![name("x")]), num(2.0)));
    assert_eq!(expr_of("2 f(x)"), bin(Op::Mul, num(2.0), call("f", vec![name("x")])));
    assert_eq!(expr_of("f(g(x), 1)"), call("f", vec![call("g", vec![name("x")]), num(1.0)]));
    syntax("f(a b c, ) =");
    syntax("f(a,, b) =");
    syntax("f(a =");
}

#[test]
fn square_root_symbol() {
    assert_eq!(expr_of("√x"), call("sqrt", vec![name("x")]));
    assert_eq!(expr_of("√(a + b)"), call("sqrt", vec![bin(Op::Add, name("a"), name("b"))]));
    assert_eq!(expr_of("√x^2"), bin(Op::Pow, call("sqrt", vec![name("x")]), num(2.0)));
    assert_eq!(expr_of("2√x"), bin(Op::Mul, num(2.0), call("sqrt", vec![name("x")])));
    assert_eq!(expr_of("√√x"), call("sqrt", vec![call("sqrt", vec![name("x")])]));
    syntax("√ =");
}

#[test]
fn absolute_value() {
    assert_eq!(expr_of("|x|"), Expr::Abs(Box::new(name("x"))));
    assert_eq!(expr_of("|a - b|"), Expr::Abs(Box::new(bin(Op::Sub, name("a"), name("b")))));
    assert_eq!(expr_of("2|x|"), bin(Op::Mul, num(2.0), Expr::Abs(Box::new(name("x")))));
    assert_eq!(expr_of("|a| |b|"), bin(Op::Mul, Expr::Abs(Box::new(name("a"))), Expr::Abs(Box::new(name("b")))));
    assert_eq!(expr_of("|a| + |b|"), bin(Op::Add, Expr::Abs(Box::new(name("a"))), Expr::Abs(Box::new(name("b")))));
    assert_eq!(expr_of("|(|x|)|"), Expr::Abs(Box::new(Expr::Abs(Box::new(name("x"))))));
    assert_eq!(expr_of("|f(|x|, 1)|"), Expr::Abs(Box::new(call("f", vec![Expr::Abs(Box::new(name("x"))), num(1.0)]))));
    syntax("|x =");
    syntax("x| =");
    syntax("|| =");
}

#[test]
fn unit_names_lex_as_names() {
    assert_eq!(expr_of("20 °F"), bin(Op::Mul, num(20.0), name("°F")));
    assert_eq!(expr_of("5 Δ°F"), bin(Op::Mul, num(5.0), name("Δ°F")));
    assert_eq!(expr_of("5 ∆°C"), bin(Op::Mul, num(5.0), name("∆°C")));
    assert_eq!(expr_of("3 µm"), bin(Op::Mul, num(3.0), name("µm")));
    assert_eq!(expr_of("3 μm"), bin(Op::Mul, num(3.0), name("μm")));
    assert_eq!(expr_of("2 Ω"), bin(Op::Mul, num(2.0), name("Ω")));
    assert_eq!(expr_of("α_1 + x2"), bin(Op::Add, name("α_1"), name("x2")));
}

#[test]
fn definition_heads() {
    assert_eq!(defined_name(&parse("A_b := 2 in^2").unwrap()), Some("A_b"));
    assert_eq!(defined_name(&parse("g(a) := a").unwrap()), Some("g"));
    // Whitespace and the ≔ spelling are fine; a define with show keeps both.
    let s = parse("  g ( a )≔a  =  ").unwrap();
    assert_eq!(s, Stmt::Define { name: "g".into(), params: vec!["a".into()], body: name("a"), show: Some(show(None)) });
    syntax("f() := 1");
    syntax("f(1) := 1");
    syntax("f(x, x) := 1");
    syntax("f(x y) := 1");
    syntax("f(x := 1");
    syntax("f(x) + 1 := 2");
    syntax("1 := 2");
    syntax("a b := 2");
    syntax("x := ");
    syntax(":= 3");
    syntax("x := y := 3");
    syntax("x := 3 + =");
}

#[test]
fn malformed_statements_are_syntax_errors() {
    for src in
        ["", "   ", "=", "a +", "a + + =", "* a =", "a b", "a + b", "(a + b =", "a + b) =", "a $ b =", "a : b =", "a ) b =", "()", "( ) =", "a := ("]
    {
        let r = parse(src);
        assert!(matches!(r, Err(CalcError::Syntax(_))), "{src:?} gave {r:?}");
    }
}

#[test]
fn syntax_messages_are_readable() {
    let msg = |src: &str| match parse(src) {
        Err(CalcError::Syntax(m)) => m,
        other => panic!("{other:?}"),
    };
    assert_eq!(msg("a $ b ="), "unexpected character `$`");
    assert!(msg("\u{0}").contains("unexpected character"));
    assert!(!msg("a + b").is_empty());
}

#[test]
fn quantity_variant_is_never_produced() {
    fn has_quantity(e: &Expr) -> bool {
        match e {
            Expr::Quantity(..) => true,
            Expr::Num(_) | Expr::Name(_) => false,
            Expr::Unary(_, a) | Expr::Abs(a) => has_quantity(a),
            Expr::Binary(_, a, b) => has_quantity(a) || has_quantity(b),
            Expr::Call(_, args) => args.iter().any(has_quantity),
        }
    }
    assert!(!has_quantity(&expr_of("5 kN·m + 2 ksi")));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_text_never_panics(src in "\\PC{0,200}") {
        let _ = parse(&src);
    }

    #[test]
    fn operator_soup_never_panics(src in "[a-zA-Z0-9_ ().,|:=?√·×*/^+\\-≔°Δµσ]{0,300}") {
        if let Ok(s) = parse(&src) {
            let _ = free_names(&s);
            let _ = defined_name(&s);
        }
    }
}

#[test]
fn error_messages() {
    assert_eq!(CalcError::Undefined("k".into()).to_string(), "k isn't defined");
    assert_eq!(CalcError::Arity("sqrt".into(), 1).to_string(), "sqrt expects 1 argument(s)");
    assert_eq!(CalcError::Duplicate { name: "P".into(), page: Some(3) }.to_string(), "P is already defined on page 3 — choose a unique name");
    assert_eq!(
        CalcError::Duplicate { name: "P".into(), page: None }.to_string(),
        "P is already defined elsewhere in the document — choose a unique name"
    );
    assert_eq!(CalcError::Circular("a, b".into()).to_string(), "a, b depend on each other");
    assert_eq!(CalcError::from(craft_units::UnitError::Overflow).to_string(), "dimension exponent overflow");
}
