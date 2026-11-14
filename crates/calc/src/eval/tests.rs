use super::{Env, check_name, eval_expr, run_stmt};
use crate::{CalcError, Expr, Op, parse};
use craft_units::{Dim, Precision, Quantity, UnitError, UnitSystem, format};

fn run(env: &mut Env, src: &str) -> Result<Option<Quantity>, CalcError> {
    let stmt = parse(src).unwrap_or_else(|e| panic!("{src:?} did not parse: {e}"));
    run_stmt(env, &stmt)
}

/// Run `src` in a fresh env and return its shown value.
fn val(src: &str) -> Quantity {
    shown(&mut Env::new(), src)
}

fn shown(env: &mut Env, src: &str) -> Quantity {
    match run(env, src) {
        Ok(Some(q)) => q,
        other => panic!("{src:?} did not show a value: {other:?}"),
    }
}

fn err(src: &str) -> CalcError {
    match run(&mut Env::new(), src) {
        Err(e) => e,
        other => panic!("{src:?} should fail: {other:?}"),
    }
}

fn close(q: Quantity, value: f64, dim: Dim) {
    assert_eq!(q.dim, dim, "dimension of {q:?}");
    assert!((q.value - value).abs() <= 1e-12 * value.abs().max(1.0), "{} vs {value}", q.value);
}

fn syntax_msg(e: CalcError) -> String {
    match e {
        CalcError::Syntax(m) => m,
        other => panic!("expected a syntax error, got {other:?}"),
    }
}

const AREA: Dim = Dim { exp: [2, 0, 0, 0, 0, 0, 0, 0] };

#[test]
fn stress_example() {
    let mut env = Env::new();
    assert_eq!(run(&mut env, "P := 1500 lbf").ok(), Some(None));
    assert_eq!(run(&mut env, "A_b := 2.5 in^2").ok(), Some(None));
    let q = shown(&mut env, "σ := P/A_b =");
    let f = format(q, None, UnitSystem::UsCustomary, Precision::new(3)).expect("formats");
    assert_eq!((f.number.as_str(), f.unit.as_str()), ("600", "psi"));
}

#[test]
fn mixed_systems() {
    close(val("len := 1 in + 25.4 mm ="), 0.0508, Dim::LENGTH);
}

#[test]
fn unit_mismatch_message() {
    assert!(matches!(err("x := 1 in + 1 lbf ="), CalcError::Unit(UnitError::Mismatch { .. })));
}

#[test]
fn undefined_message() {
    let e = err("y := z + 1 =");
    assert_eq!(e, CalcError::Undefined("z".into()));
    assert_eq!(e.to_string(), "z isn't defined");
}

#[test]
fn user_function() {
    let mut env = Env::new();
    assert_eq!(run(&mut env, "f(x, y) := x^2 + y").ok(), Some(None));
    close(shown(&mut env, "f(3 m, 1 m^2) ="), 10.0, AREA);
}

#[test]
fn unit_names_cannot_be_variables() {
    let mut env = Env::new();
    for src in ["m := 5 kg", "A := 2 in^2", "kN := 1", "sin := 1", "f(m) := m", "min := 1", "π := 3", "e := 2", "pi := 3"] {
        assert!(matches!(run(&mut env, src), Err(CalcError::ReservedName(_))), "{src}");
    }
    assert!(run(&mut env, "A_b := 2 in^2").is_ok());
    close(shown(&mut env, "2 m ="), 2.0, Dim::LENGTH);
    // A refused definition leaves nothing behind.
    assert_eq!(env.names(), vec!["A_b".to_string()]);
}

#[test]
fn trig_takes_angles() {
    close(val("sin(30 deg) ="), 0.5, Dim::ONE);
    close(val("cos(0) ="), 1.0, Dim::ONE);
    assert!(matches!(err("sin(1 m) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
}

#[test]
fn non_finite_results_are_errors() {
    for src in ["10^400 =", "1/0 =", "sqrt(-1) =", "0/0 =", "ln(0) =", "ln(-1) =", "10^300 * 10^300 =", "tan(90 deg) * 10^300 * 10^300 ="] {
        assert_eq!(err(src), CalcError::NonFinite, "{src}");
    }
    // A hand-built non-finite literal is caught too.
    let e = Expr::Num(f64::INFINITY);
    assert_eq!(eval_expr(&Env::new(), &e), Err(CalcError::NonFinite));
}

#[test]
fn recursion_bound() {
    let mut env = Env::new();
    assert!(run(&mut env, "f(x) := f(x)").is_ok());
    assert_eq!(run(&mut env, "f(1) ="), Err(CalcError::TooComplex));
}

#[test]
fn upstream() {
    let mut env = Env::new();
    env.mark_failed("a");
    assert_eq!(run(&mut env, "b := a + 1 ="), Err(CalcError::Upstream));
    assert_eq!(run(&mut env, "a(2) ="), Err(CalcError::Upstream));
    // Defining it afterwards clears the mark.
    env.define_var("a", Quantity::dimensionless(1.0));
    close(shown(&mut env, "a + 1 ="), 2.0, Dim::ONE);
}

#[test]
fn name_resolution_order() {
    let mut env = Env::new();
    // Parameters win over variables; constants and units resolve last.
    env.define_var("x", Quantity::dimensionless(100.0));
    assert_eq!(run(&mut env, "fa(x) := x + 1").ok(), Some(None));
    close(shown(&mut env, "fa(1) ="), 2.0, Dim::ONE);
    close(val("π ="), std::f64::consts::PI, Dim::ONE);
    close(val("pi ="), std::f64::consts::PI, Dim::ONE);
    close(val("e ="), std::f64::consts::E, Dim::ONE);
    close(val("5 min ="), 300.0, Dim::TIME);
}

#[test]
fn functions_see_globals_but_not_the_callers_parameters() {
    let mut env = Env::new();
    run(&mut env, "k := 10").expect("defines");
    run(&mut env, "fa(x) := x * k").expect("defines");
    run(&mut env, "fb(k) := fa(k)").expect("defines");
    close(shown(&mut env, "fb(2) ="), 20.0, Dim::ONE);
    // `fc` has no parameter `y`, and the caller's `y` does not leak in.
    run(&mut env, "fc(x) := x + y").expect("defines");
    run(&mut env, "fd(y) := fc(1)").expect("defines");
    assert_eq!(run(&mut env, "fd(5) ="), Err(CalcError::Undefined("y".into())));
}

#[test]
fn offset_units_only_as_a_lone_literal() {
    close(val("212 °F ="), 373.15, Dim::TEMPERATURE);
    close(val("100 °C ="), 373.15, Dim::TEMPERATURE);
    close(val("-40 °F ="), 233.15, Dim::TEMPERATURE);
    close(val("(5 °C) ="), 278.15, Dim::TEMPERATURE);
    // Two absolute temperatures can't be added or subtracted, but an interval can be added.
    for src in ["100 °C + 5 °C =", "100 °C - 5 °F ="] {
        assert_eq!(err(src), CalcError::Unit(UnitError::AffineTemperature), "{src}");
    }
    close(val("100 °C + 5 Δ°C ="), 378.15, Dim::TEMPERATURE);
    close(val("5 K + 100 °C ="), 378.15, Dim::TEMPERATURE);
    close(val("5 Δ°F + 100 °C ="), 373.15 + 5.0 * 5.0 / 9.0, Dim::TEMPERATURE);
    close(val("100 °C + 5 °R ="), 373.15 + 5.0 * 5.0 / 9.0, Dim::TEMPERATURE);
    // An absolute literal beside a variable is refused; variables alone are not tracked.
    let mut env = Env::new();
    run(&mut env, "T_1 := 100 °C").expect("defines");
    run(&mut env, "T_2 := 20 °C").expect("defines");
    close(shown(&mut env, "T_1 - T_2 ="), 80.0, Dim::TEMPERATURE);
    close(shown(&mut env, "T_1 + 5 K ="), 378.15, Dim::TEMPERATURE);
    for src in ["T_1 + 5 °C =", "100 °C + T_1 =", "T_1 - 5 °F =", "100 °C + 5 m ="] {
        assert_eq!(run(&mut env, src), Err(CalcError::Unit(UnitError::AffineTemperature)), "{src}");
    }
    for src in ["°F =", "1 / °F =", "5 °F / s =", "x := 2 °F^2 =", "3 + °C =", "-(5 °F) =", "(5 °F)^2 =", "2 (5 °F) =", "(5 °F) / 2 ="] {
        assert_eq!(err(src), CalcError::Unit(UnitError::AffineTemperature), "{src}");
    }
    let mut env = Env::new();
    run(&mut env, "t := 3").expect("defines");
    assert_eq!(run(&mut env, "t °F ="), Err(CalcError::Unit(UnitError::AffineTemperature)));
    // Added to an interval, an absolute temperature stays absolute.
    close(val("100 °C + 5 K ="), 378.15, Dim::TEMPERATURE);
    // A plain temperature unit is fine: 10 K, and a rankine.
    close(val("10 K ="), 10.0, Dim::TEMPERATURE);
    // Hand-built quantity literals take the same path.
    let q = eval_expr(&Env::new(), &Expr::Quantity(32.0, "°F".into())).expect("evaluates");
    close(q, 273.15, Dim::TEMPERATURE);
}

#[test]
fn functions_are_not_values() {
    let mut env = Env::new();
    run(&mut env, "f(x) := x").expect("defines");
    run(&mut env, "P := 2").expect("defines");
    assert_eq!(syntax_msg(run(&mut env, "f + 1 =").unwrap_err()), "f is a function — call it like f(x)");
    assert_eq!(syntax_msg(run(&mut env, "f^2 (3) =").unwrap_err()), "f is a function — call it like f(x)");
    assert_eq!(syntax_msg(run(&mut env, "sin + 1 =").unwrap_err()), "sin is a function — call it like sin(x)");
    assert!(syntax_msg(run(&mut env, "P(2) =").unwrap_err()).contains("P isn't a function"));
    assert!(syntax_msg(run(&mut env, "m(2) =").unwrap_err()).contains("m isn't a function"));
    assert_eq!(run(&mut env, "nope(2) ="), Err(CalcError::Undefined("nope".into())));
}

#[test]
fn user_function_arity() {
    let mut env = Env::new();
    run(&mut env, "f(x, y) := x + y").expect("defines");
    assert_eq!(run(&mut env, "f(1) ="), Err(CalcError::Arity("f".into(), 2)));
    assert_eq!(run(&mut env, "f(1, 2, 3) ="), Err(CalcError::Arity("f".into(), 2)));
}

#[test]
fn exponential_work_is_bounded() {
    // Each level calls the one below twice: 2^30 evaluations unless the work is capped.
    let mut env = Env::new();
    run(&mut env, "g0(x) := x").expect("defines");
    for i in 1..=30 {
        run(&mut env, &format!("g{i}(x) := g{}(x) + g{}(x)", i - 1, i - 1)).expect("defines");
    }
    assert_eq!(run(&mut env, "g30(1) ="), Err(CalcError::TooComplex));
}

#[test]
fn deep_expressions_evaluate_and_hostile_depth_is_refused() {
    // Parser-limit shapes: 30 nested parentheses (its nesting limit is 64) and a 1000-term sum
    // (a tree 1000 high) evaluate fine.
    let nested = format!("{}1{} =", "(-".repeat(30), ")".repeat(30));
    assert!(run(&mut Env::new(), &nested).is_ok());
    let sum = format!("{} =", vec!["1"; 1000].join("+"));
    close(val(&sum), 1000.0, Dim::ONE);
    // A hand-built tree far deeper than the parser allows is refused, not a stack overflow.
    let mut e = Expr::Num(1.0);
    for _ in 0..3000 {
        e = Expr::Unary(Op::Neg, Box::new(e));
    }
    assert_eq!(eval_expr(&Env::new(), &e), Err(CalcError::TooComplex));
    // Function bodies nested through calls share one depth budget.
    let mut env = Env::new();
    let tail = " + 1".repeat(440);
    run(&mut env, "n0(x) := x").expect("defines");
    for i in 1..30 {
        run(&mut env, &format!("n{i}(x) := n{}(x){tail}", i - 1)).expect("defines");
    }
    assert_eq!(run(&mut env, "n29(1) ="), Err(CalcError::TooComplex));
    // Shallow enough chains still work.
    close(shown(&mut env, "n3(1) ="), 1.0 + 3.0 * 440.0, Dim::ONE);
}

#[test]
fn show_units_are_validated() {
    close(val("1 in = ? mm"), 0.0254, Dim::LENGTH);
    assert!(matches!(err("1 in = ? lbf"), CalcError::Unit(UnitError::Mismatch { .. })));
    assert!(matches!(err("1 in = ? bogus"), CalcError::Unit(UnitError::UnknownUnit(_))));
    // A failed show leaves a defined name undefined.
    let mut env = Env::new();
    assert!(run(&mut env, "a := 1 in = ? lbf").is_err());
    assert!(env.names().is_empty());
    assert!(syntax_msg(err("f(x) := x =")).contains("function"));
}

#[test]
fn definitions_without_show_return_none() {
    let mut env = Env::new();
    assert_eq!(run(&mut env, "a := 3").ok(), Some(None));
    assert_eq!(env.names(), vec!["a".to_string()]);
    run(&mut env, "f(x) := x").expect("defines");
    assert_eq!(env.names(), vec!["a".to_string(), "f".to_string()]);
}

#[test]
fn powers() {
    close(val("(2 m)^3 ="), 8.0, Dim::LENGTH.powi(3).expect("dim"));
    close(val("2^0.5 ="), 2f64.sqrt(), Dim::ONE);
    close(val("4 m^2 ^ 1 ="), 4.0, AREA);
    assert!(matches!(err("2^(1 m) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
    assert!(matches!(err("(2 m)^0.5 ="), CalcError::Unit(UnitError::NotDimensionless(_))));
    close(val("|-3 m| ="), 3.0, Dim::LENGTH);
}

#[test]
fn builtins_unary() {
    close(val("sqrt(4 m^2) ="), 2.0, Dim::LENGTH);
    close(val("√9 ="), 3.0, Dim::ONE);
    assert!(matches!(err("sqrt(4 m) ="), CalcError::Unit(UnitError::Fractional)));
    close(val("abs(-2 kg) ="), 2.0, Dim::MASS);
    close(val("exp(0) ="), 1.0, Dim::ONE);
    close(val("ln(e) ="), 1.0, Dim::ONE);
    close(val("log(1000) ="), 3.0, Dim::ONE);
    close(val("log(8, 2) ="), 3.0, Dim::ONE);
    close(val("log(1000, 10) ="), 3.0, Dim::ONE);
    close(val("log(81, 3) ="), 4.0, Dim::ONE);
    assert!(matches!(err("exp(1 m) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
    close(val("sinh(0) + cosh(0) + tanh(0) ="), 1.0, Dim::ONE);
    close(val("floor(2.7) + ceil(2.1) ="), 5.0, Dim::ONE);
    assert!(matches!(err("floor(2.7 m) ="), CalcError::Unit(UnitError::NotDimensionless(_))));
}

#[test]
fn inverse_trig_returns_angles() {
    close(val("asin(1) ="), std::f64::consts::FRAC_PI_2, Dim::ANGLE);
    close(val("acos(1) ="), 0.0, Dim::ANGLE);
    close(val("atan(1) ="), std::f64::consts::FRAC_PI_4, Dim::ANGLE);
    close(val("atan2(1 m, 1 m) ="), std::f64::consts::FRAC_PI_4, Dim::ANGLE);
    close(val("atan2(1 m, 1000 mm) ="), std::f64::consts::FRAC_PI_4, Dim::ANGLE);
    assert!(matches!(err("atan2(1 m, 1 s) ="), CalcError::Unit(UnitError::Mismatch { .. })));
    assert_eq!(err("asin(2) ="), CalcError::NonFinite);
    // An angle result goes back into trig.
    close(val("sin(asin(0.5)) ="), 0.5, Dim::ONE);
}

#[test]
fn min_max_round_mod() {
    close(val("min(3 m, 1 ft, 2 m) ="), 0.3048, Dim::LENGTH);
    close(val("max(3 m, 1 ft, 2 m) ="), 3.0, Dim::LENGTH);
    close(val("max(1) ="), 1.0, Dim::ONE);
    assert!(matches!(err("min(1 m, 1 s) ="), CalcError::Unit(UnitError::Mismatch { .. })));
    assert_eq!(err("min() ="), CalcError::Arity("min".into(), 1));
    close(val("round(2.5) ="), 3.0, Dim::ONE);
    close(val("round(-2.5) ="), -3.0, Dim::ONE);
    close(val("round(2.71828, 2) ="), 2.72, Dim::ONE);
    close(val("round(1234, -2) ="), 1200.0, Dim::ONE);
    assert!(syntax_msg(err("round(1.5, 0.5) =")).contains("whole number"));
    assert!(syntax_msg(err("round(1.5, 99) =")).contains("-15 to 15"));
    close(val("mod(7 m, 3 m) ="), 1.0, Dim::LENGTH);
    close(val("mod(-1, 3) ="), 2.0, Dim::ONE);
    close(val("mod(5, -3) ="), -1.0, Dim::ONE);
    // A remainder too small to move the divisor is 0, never exactly the divisor.
    close(val("mod(-1/10^20 m, 3 m) ="), 0.0, Dim::LENGTH);
    assert!(val("mod(-1/10^20, 3) =").value < 3.0);
    assert_eq!(err("mod(5, 0) ="), CalcError::NonFinite);
    assert!(matches!(err("mod(1 m, 1 s) ="), CalcError::Unit(UnitError::Mismatch { .. })));
}

#[test]
fn arity_errors() {
    assert_eq!(err("sin() ="), CalcError::Arity("sin".into(), 1));
    assert_eq!(err("sqrt(1, 2) ="), CalcError::Arity("sqrt".into(), 1));
    assert_eq!(err("atan2(1) ="), CalcError::Arity("atan2".into(), 2));
    assert_eq!(err("mod(1, 2, 3) ="), CalcError::Arity("mod".into(), 2));
    assert_eq!(err("round(1, 2, 3) ="), CalcError::Arity("round".into(), 2));
    assert_eq!(err("log() ="), CalcError::Arity("log".into(), 1));
    assert_eq!(err("log(1, 2, 3) ="), CalcError::Arity("log".into(), 2));
}

#[test]
fn check_name_rules() {
    for bad in ["m", "mm", "kN", "psi", "lbf", "sin", "atan2", "round", "mod", "π", "pi", "e", "min", "max", "°F"] {
        assert_eq!(check_name(bad), Err(CalcError::ReservedName(bad.into())), "{bad}");
    }
    for ok in ["x", "A_b", "σ", "P", "f", "foo", "L1"] {
        assert!(check_name(ok).is_ok(), "{ok}");
    }
}

#[test]
fn redefinition_replaces_the_previous_entry() {
    let mut env = Env::new();
    env.define_var("a", Quantity::dimensionless(1.0));
    env.define_fn("a", vec!["x".into()], Expr::Name("x".into()));
    close(shown(&mut env, "a(4) ="), 4.0, Dim::ONE);
    env.mark_failed("a");
    assert_eq!(run(&mut env, "a(4) ="), Err(CalcError::Upstream));
    assert_eq!(env.names(), vec!["a".to_string()]);
}
