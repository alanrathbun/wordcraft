use super::{Env, Pass, check_name, eval_expr, run_stmt, run_stmt_in};
use crate::array::{Grid, element_name};
use crate::{CalcError, Expr, Op, Value, parse};
use craft_units::{Dim, Precision, Quantity, UnitError, UnitSystem, format};

fn run(env: &mut Env, src: &str) -> Result<Option<Quantity>, CalcError> {
    run_value(env, src).map(|v| v.map(one))
}

fn run_value(env: &mut Env, src: &str) -> Result<Option<Value>, CalcError> {
    let stmt = parse(src).unwrap_or_else(|e| panic!("{src:?} did not parse: {e}"));
    run_stmt(env, &stmt)
}

/// The single value of `v`.
fn one(v: Value) -> Quantity {
    match v {
        Value::One(q) => q,
        other => panic!("expected a single value, got {other:?}"),
    }
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
    close(val("100 °C - 5 Δ°C ="), 368.15, Dim::TEMPERATURE);
    close(val("100 °C - 5 K ="), 368.15, Dim::TEMPERATURE);
    for src in ["5 K - 100 °C =", "5 Δ°C - 100 °C =", "5 Δ°F - 100 °C ="] {
        assert_eq!(err(src), CalcError::Unit(UnitError::AffineTemperature), "{src}");
    }
    close(val("5 Δ°F + 100 °C ="), 373.15 + 5.0 * 5.0 / 9.0, Dim::TEMPERATURE);
    close(val("100 °C + 5 °R ="), 373.15 + 5.0 * 5.0 / 9.0, Dim::TEMPERATURE);
    // An absolute literal beside a variable is refused; variables carry whether they are absolute
    // (see `offset_display_needs_an_absolute_temperature`).
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
    let q = eval_expr(&Env::new(), &Expr::Quantity(32.0, "°F".into())).map(one).expect("evaluates");
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

#[test]
fn offset_display_needs_an_absolute_temperature() {
    let not_absolute = |u: &str| CalcError::NotAbsolute(u.to_string());
    // A difference shown on an offset scale would be −253.15 °C: refused.
    assert_eq!(err("dT := 20 Δ°C = ? °C"), not_absolute("°C"));
    let mut env = Env::new();
    run(&mut env, "T_1 := 70 °F").expect("defines");
    run(&mut env, "T_2 := 100 °F").expect("defines");
    assert_eq!(run(&mut env, "T_2 - T_1 = ? °F"), Err(not_absolute("°F")));
    assert_eq!(run(&mut env, "T_2 - T_1 = ? °C"), Err(not_absolute("°C")));
    // The difference itself is fine in a difference unit, or a plain one.
    close(shown(&mut env, "T_2 - T_1 = ? Δ°F"), 30.0 * 5.0 / 9.0, Dim::TEMPERATURE);
    close(shown(&mut env, "T_2 - T_1 = ? K"), 30.0 * 5.0 / 9.0, Dim::TEMPERATURE);
    // Absolute temperatures, and an absolute temperature plus or minus a difference, show.
    close(shown(&mut env, "T := 70 °F = ? °C"), 294.261_111_111_111_1, Dim::TEMPERATURE);
    close(shown(&mut env, "T_1 + 5 K = ? °C"), 294.261_111_111_111_1 + 5.0, Dim::TEMPERATURE);
    close(shown(&mut env, "T_1 + (T_2 - T_1)/2 = ? °F"), (85.0 - 32.0) * 5.0 / 9.0 + 273.15, Dim::TEMPERATURE);
    close(shown(&mut env, "T_3 := T_2 - 10 Δ°F = ? °F"), (90.0 - 32.0) * 5.0 / 9.0 + 273.15, Dim::TEMPERATURE);
    close(shown(&mut env, "T_3 = ? °C"), (90.0 - 32.0) * 5.0 / 9.0 + 273.15, Dim::TEMPERATURE);
    close(val("100 °C + 5 K = ? °C"), 378.15, Dim::TEMPERATURE);
    close(val("T_K := 300 K = ? °C"), 300.0, Dim::TEMPERATURE);
    // Anything else made from temperatures is not known to be absolute.
    for src in ["(T_1 + T_2)/2 = ? °C", "2*T_1 = ? °C", "-T_1 + 600 K = ? °F", "5 K + 3 K = ? °C", "|T_1| = ? °C"] {
        assert_eq!(run(&mut env, src), Err(not_absolute(src.rsplit(' ').next().unwrap())), "{src}");
    }
    // User functions pass it through their parameters and result.
    run(&mut env, "warm(t) := t + 5 Δ°C").expect("defines");
    run(&mut env, "twice(t) := 2*t").expect("defines");
    close(shown(&mut env, "warm(T_1) = ? °C"), 294.261_111_111_111_1 + 5.0, Dim::TEMPERATURE);
    assert_eq!(run(&mut env, "twice(T_1) = ? °C"), Err(not_absolute("°C")));
    assert_eq!(run(&mut env, "warm(5 Δ°C) = ? °C"), Err(not_absolute("°C")));
    // The larger of two absolute temperatures is one of them.
    close(shown(&mut env, "max(T_1, T_2) = ? °F"), 310.927_777_777_777_8, Dim::TEMPERATURE);
    assert_eq!(run(&mut env, "max(T_2 - T_1, 1 K) = ? °F"), Err(not_absolute("°F")));
}

#[test]
fn engineering_constants_carry_their_units() {
    let accel = Dim::LENGTH.div(Dim::TIME.powi(2).unwrap()).unwrap();
    close(val("a := g_0 ="), 9.80665, accel);
    close(val("v := c_0 ="), 299_792_458.0, Dim::LENGTH.div(Dim::TIME).unwrap());
    let r = val("r := R_u =");
    assert!((r.value - 8.314_462_618).abs() < 1e-9, "{}", r.value);
    // A weight: 10 kg under standard gravity is 98.0665 N, and 1 lbm weighs 1 lbf.
    close(val("F_w := 10 kg·g_0 ="), 98.0665, val("n_1 := 1 N =").dim);
    let mut env = Env::new();
    let w = shown(&mut env, "F_1 := 1 lbm·g_0 =");
    let f = format(w, Some(&craft_units::parse_unit("lbf").unwrap()), UnitSystem::UsCustomary, Precision::new(6)).unwrap();
    assert_eq!(f.number, "1");
    // The constants are taken: they can't be redefined.
    for name in ["g_0", "c_0", "R_u"] {
        assert!(matches!(check_name(name), Err(CalcError::ReservedName(_))), "{name}");
    }
    assert!(check_name("g_1").is_ok(), "other subscripts are free");
}

#[test]
fn g_is_gravity_when_the_document_says_so() {
    let accel = Dim::LENGTH.div(Dim::TIME.powi(2).unwrap()).unwrap();
    let mut env = Env::new();
    env.set_g_is_gravity(true);
    close(shown(&mut env, "a_1 := g ="), 9.80665, accel);
    close(shown(&mut env, "F_w := 5 kg·g ="), 49.03325, val("n_1 := 1 N =").dim);
    close(shown(&mut env, "m_1 := 5 gm ="), 0.005, Dim::MASS);
    close(shown(&mut env, "m_2 := 5 kg ="), 5.0, Dim::MASS);
    // `g` as a display unit would read as gravity: refused, with the way out named.
    let e = run(&mut env, "m_3 := 5 kg = ? g").expect_err("g is gravity here");
    assert!(e.to_string().contains("gm"), "{e}");
    assert!(run(&mut env, "m_4 := 5 kg = ? gm").is_ok());
    // Off (SI): g is the gram again.
    close(val("m_5 := 5 g ="), 0.005, Dim::MASS);
}

// ---------------------------------------------------------------------------------------------
// Grids (vectors and matrices)
// ---------------------------------------------------------------------------------------------

/// `values` in `unit`, as SI quantities.
fn quantities(values: &[f64], unit: &str) -> Vec<Quantity> {
    let scale = craft_units::parse_unit(unit).expect("a unit");
    values.iter().map(|&v| Quantity::new(v * scale.factor, scale.dim)).collect()
}

/// Define the column vector `name` of `values` in `unit`.
fn column(env: &mut Env, name: &str, values: &[f64], unit: &str) {
    env.define_array(name, values.len(), 1, quantities(values, unit)).expect("defines");
}

/// The grid `src` shows: its shape and its elements.
fn grid(env: &mut Env, src: &str) -> ((usize, usize), Vec<Quantity>) {
    match run_value(env, src) {
        Ok(Some(Value::Grid { rows, cols, items })) => ((rows, cols), items),
        other => panic!("{src:?} did not show a grid: {other:?}"),
    }
}

/// Every element of `items` is close to the matching `want` (SI) and has dimension `dim`.
fn close_all(items: &[Quantity], want: &[f64], dim: Dim) {
    assert_eq!(items.len(), want.len(), "{items:?}");
    for (q, w) in items.iter().zip(want) {
        close(*q, *w, dim);
    }
}

const INCH: f64 = 0.0254;

#[test]
fn vectors_add_element_wise() {
    let mut env = Env::new();
    column(&mut env, "D", &[1.0, 2.0, 3.0], "in");
    column(&mut env, "t", &[0.1, 0.2, 0.3], "in");
    let (shape, items) = grid(&mut env, "D + t =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[1.1 * INCH, 2.2 * INCH, 3.3 * INCH], Dim::LENGTH);
    // A name holding a grid evaluates as that grid.
    assert_eq!(run(&mut env, "D_t := D + t").ok(), Some(None));
    let (shape, items) = grid(&mut env, "D_t - t =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[INCH, 2.0 * INCH, 3.0 * INCH], Dim::LENGTH);
    // A shown unit must fit every element; one that doesn't names the element and defines nothing.
    assert!(run_value(&mut env, "D = ? mm").is_ok());
    let e = run_value(&mut env, "u := D = ? lbf").expect_err("inches aren't force");
    assert!(e.to_string().starts_with("element 0: "), "{e}");
    assert_eq!(run(&mut env, "u ="), Err(CalcError::Undefined("u".into())));
}

#[test]
fn single_value_broadcasts() {
    let mut env = Env::new();
    column(&mut env, "D", &[1.0, 2.0, 3.0], "in");
    let (shape, items) = grid(&mut env, "2·D =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[2.0 * INCH, 4.0 * INCH, 6.0 * INCH], Dim::LENGTH);
    close_all(&grid(&mut env, "D/2 =").1, &[0.5 * INCH, INCH, 1.5 * INCH], Dim::LENGTH);
    close_all(&grid(&mut env, "D^2 =").1, &[INCH * INCH, 4.0 * INCH * INCH, 9.0 * INCH * INCH], AREA);
    close_all(&grid(&mut env, "1 in - D =").1, &[0.0, -INCH, -2.0 * INCH], Dim::LENGTH);
    close_all(&grid(&mut env, "-D =").1, &[-INCH, -2.0 * INCH, -3.0 * INCH], Dim::LENGTH);
    close_all(&grid(&mut env, "|1 in - D| =").1, &[0.0, INCH, 2.0 * INCH], Dim::LENGTH);
    // A single value's unit rules still hold for every element.
    let e = run_value(&mut env, "D + 1 lbf =").expect_err("can't add force to length");
    assert!(e.to_string().starts_with("element 0: "), "{e}");
    // Single values are unchanged: their errors aren't about an element.
    assert!(matches!(err("x := 1 in + 1 lbf ="), CalcError::Unit(UnitError::Mismatch { .. })));
}

#[test]
fn shape_mismatch_is_named() {
    let mut env = Env::new();
    column(&mut env, "D", &[1.0, 2.0, 3.0], "in");
    column(&mut env, "E", &[1.0, 2.0, 3.0, 4.0], "in");
    let e = run_value(&mut env, "D + E =").expect_err("shapes differ");
    assert_eq!(e, CalcError::Shape { left: (3, 1), right: (4, 1) });
    assert_eq!(e.to_string(), "shapes don't match: 3 rows × 1 vs 4 rows × 1");
    // A row isn't a column.
    env.define_array("R", 1, 3, quantities(&[1.0, 2.0, 3.0], "in")).expect("defines");
    assert_eq!(run_value(&mut env, "D·R ="), Err(CalcError::Shape { left: (3, 1), right: (1, 3) }));
}

#[test]
fn element_error_names_the_element() {
    let mixed = vec![quantities(&[1.0], "in")[0], quantities(&[2.0], "lbf")[0]];
    for (origin, at) in [(0, "element 1: "), (1, "element 2: ")] {
        let mut env = Env::new();
        env.set_origin(origin);
        env.define_array("P", 2, 1, mixed.clone()).expect("defines");
        column(&mut env, "Q", &[1.0, 1.0], "in");
        let e = run_value(&mut env, "P + Q =").expect_err("lbf + in");
        assert!(e.to_string().starts_with(at), "origin {origin}: {e}");
        assert!(matches!(e, CalcError::Element { ref error, .. } if matches!(**error, CalcError::Unit(UnitError::Mismatch { .. }))), "{e:?}");
    }
    // A vector element has one index (a row vector counts along its columns); a matrix element two.
    assert_eq!(element_name(3, 1, 2, 0), "2");
    assert_eq!(element_name(1, 3, 2, 1), "3");
    assert_eq!(element_name(2, 3, 4, 0), "[1, 1]");
    assert_eq!(element_name(2, 3, 5, 1), "[2, 3]");
    // An origin other than 0 or 1 is 0.
    let mut env = Env::new();
    env.set_origin(7);
    env.define_array("P", 2, 1, mixed).expect("defines");
    column(&mut env, "Q", &[1.0, 1.0], "in");
    assert!(run_value(&mut env, "P + Q =").expect_err("lbf + in").to_string().starts_with("element 1: "));
}

#[test]
fn builtins_apply_element_wise() {
    let mut env = Env::new();
    column(&mut env, "v", &[4.0, 9.0], "1");
    column(&mut env, "r", &[1.26, 2.34], "1");
    let (shape, items) = grid(&mut env, "sqrt(v) =");
    assert_eq!(shape, (2, 1));
    close_all(&items, &[2.0, 3.0], Dim::ONE);
    close_all(&grid(&mut env, "round(r, 1) =").1, &[1.3, 2.3], Dim::ONE);
    close_all(&grid(&mut env, "mod(v, 4) =").1, &[0.0, 1.0], Dim::ONE);
    close_all(&grid(&mut env, "log(100, v) =").1, &[100f64.ln() / 4f64.ln(), 100f64.ln() / 9f64.ln()], Dim::ONE);
    // Arity is checked before any element is.
    assert_eq!(run_value(&mut env, "sqrt(v, v) ="), Err(CalcError::Arity("sqrt".into(), 1)));
    // A per-element error names the element.
    column(&mut env, "w", &[1.0, -1.0], "1");
    assert!(run_value(&mut env, "sqrt(w) =").expect_err("sqrt(-1)").to_string().starts_with("element 1: "));
    // min and max reduce an array to one value rather than applying element-wise.
    close(shown(&mut env, "max(v) ="), 9.0, Dim::ONE);
    close(shown(&mut env, "min(v, 1) ="), 1.0, Dim::ONE);
}

#[test]
fn user_functions_apply_element_wise() {
    let mut env = Env::new();
    column(&mut env, "v", &[1.0, 2.0], "1");
    run(&mut env, "f(x) := x^2 + 1").expect("defines");
    let (shape, items) = grid(&mut env, "f(v) =");
    assert_eq!(shape, (2, 1));
    close_all(&items, &[2.0, 5.0], Dim::ONE);
}

#[test]
fn arrays_are_bounded() {
    let mut env = Env::new();
    let e = env.define_array("big", 10_001, 1, vec![Quantity::dimensionless(1.0); 10_001]).expect_err("too large");
    assert_eq!(e, CalcError::TooLarge);
    assert_eq!(e.to_string(), "more than 10,000 elements");
    assert!(env.define_array("big", 100, 101, Vec::new()).is_err_and(|e| e == CalcError::TooLarge));
    // The cap is checked before the size, and an overflowing shape is too large rather than a panic.
    assert_eq!(Grid::new(101, 100, Vec::new()), Err(CalcError::TooLarge));
    assert_eq!(Grid::new(usize::MAX, 2, Vec::new()), Err(CalcError::TooLarge));
    // A size that doesn't match the shape, or no elements at all, is refused.
    assert!(Grid::new(2, 2, Vec::new()).is_err());
    assert!(Grid::new(0, 1, Vec::new()).is_err());
    // 10,000 is allowed, and so is arithmetic on it.
    column(&mut env, "D", &vec![1.0; 10_000], "in");
    assert_eq!(grid(&mut env, "D + D =").0, (10_000, 1));
}

#[test]
fn element_operations_count_against_the_budget() {
    let stmt = |src: &str| parse(src).expect("parses");
    let mut env = Env::new();
    column(&mut env, "D", &vec![1.0; 10_000], "in");
    column(&mut env, "N", &vec![4.0; 10_000], "1");
    // Each costs one step per node, 10,000 for reading the grid and 10,000 for the element-wise
    // operation (an operator, a negation, a built-in).
    for (src, nodes) in [("x := 2·D", 3), ("y := -D", 2), ("z := sqrt(N)", 2)] {
        let pass = Pass::new(u64::MAX);
        run_stmt_in(&mut env, &stmt(src), &pass).expect("runs");
        assert_eq!(pass.spent.get(), nodes + 20_000, "{src}");
        // The read fits in this budget; the operation doesn't.
        assert_eq!(run_stmt_in(&mut env, &stmt(src), &Pass::new(15_000)), Err(CalcError::TooMuchWork), "{src}");
    }
    assert_eq!(run_stmt_in(&mut env, &stmt("w := 2·D"), &Pass::new(5_000)), Err(CalcError::TooMuchWork));
    // Single values cost what they did: one step per node.
    let pass = Pass::new(u64::MAX);
    run_stmt_in(&mut env, &stmt("k := 2·3"), &pass).expect("runs");
    assert_eq!(pass.spent.get(), 3);
}

// ---------------------------------------------------------------------------------------------
// Literals, indexes, ranges and indexed definitions
// ---------------------------------------------------------------------------------------------

/// An env holding `D := [0.840, 1.050, 1.315] in` and `M := [1, 2; 3, 4]`, numbered from `origin`.
fn pipes(origin: u8) -> Env {
    let mut env = Env::new();
    env.set_origin(origin);
    run(&mut env, "D := [0.840, 1.050, 1.315] in").expect("defines D");
    run(&mut env, "M := [1, 2; 3, 4]").expect("defines M");
    env
}

fn index_range_msg(e: CalcError) -> String {
    match e {
        CalcError::IndexRange(m) => m,
        other => panic!("expected an index range error, got {other:?}"),
    }
}

#[test]
fn literal_evaluates() {
    let mut env = pipes(0);
    let (shape, items) = grid(&mut env, "D =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[0.840 * INCH, 1.050 * INCH, 1.315 * INCH], Dim::LENGTH);
    // Each element keeps its own unit.
    let (shape, items) = grid(&mut env, "[2 in, 50 mm] =");
    assert_eq!(shape, (2, 1));
    close_all(&items, &[2.0 * INCH, 0.050], Dim::LENGTH);
    // A matrix, row by row.
    let (shape, items) = grid(&mut env, "M =");
    assert_eq!(shape, (2, 2));
    close_all(&items, &[1.0, 2.0, 3.0, 4.0], Dim::ONE);
    // Elements are expressions.
    close_all(&grid(&mut env, "[1 + 1, D[0]/(1 in)] =").1, &[2.0, 0.840], Dim::ONE);
    // An element that is itself a grid is refused, whichever way it is written.
    for src in ["[2 [1]] =", "[D, 1] =", "[1, 2·D] ="] {
        assert_eq!(syntax_msg(run_value(&mut env, src).expect_err(src)), "a matrix element can't be a vector", "{src}");
    }
}

#[test]
fn index_reads_an_element() {
    let mut env = pipes(0);
    close(shown(&mut env, "D[0] ="), 0.840 * INCH, Dim::LENGTH);
    close(shown(&mut env, "D[2] ="), 1.315 * INCH, Dim::LENGTH);
    close(shown(&mut env, "M[1, 0] ="), 3.0, Dim::ONE);
    close(shown(&mut env, "M[0, 1] ="), 2.0, Dim::ONE);
    // An index is an expression, and a column vector may also be given a row and a column.
    close(shown(&mut env, "D[3 - 2] ="), 1.050 * INCH, Dim::LENGTH);
    close(shown(&mut env, "D[1, 0] ="), 1.050 * INCH, Dim::LENGTH);
    // A function's parameter may be indexed.
    run(&mut env, "f(x) := x[1]").expect("defines f");
    close(shown(&mut env, "f(D) ="), 1.050 * INCH, Dim::LENGTH);

    let mut env = pipes(1);
    close(shown(&mut env, "D[1] ="), 0.840 * INCH, Dim::LENGTH);
    close(shown(&mut env, "D[3] ="), 1.315 * INCH, Dim::LENGTH);
    close(shown(&mut env, "M[2, 1] ="), 3.0, Dim::ONE);
}

#[test]
fn index_must_be_a_whole_number_in_range() {
    let mut env = pipes(0);
    for src in ["D[1.5] =", "D[1 in] =", "D[1 rad] =", "D[[0]] =", "M[0.5, 0] =", "M[0, 1 in] ="] {
        assert_eq!(syntax_msg(run_value(&mut env, src).expect_err(src)), "an index is a whole number, like D[2]", "{src}");
    }
    // Within 1e-9 of a whole number is that number.
    close(shown(&mut env, "D[1 + 10^-12] ="), 1.050 * INCH, Dim::LENGTH);
    assert_eq!(index_range_msg(run_value(&mut env, "D[3] =").expect_err("past the end")), "index 3 is outside D (0 to 2)");
    assert_eq!(index_range_msg(run_value(&mut env, "D[-1] =").expect_err("before the start")), "index -1 is outside D (0 to 2)");
    assert_eq!(index_range_msg(run_value(&mut env, "D[10^300] =").expect_err("far past the end")).split(' ').next(), Some("index"));
    assert_eq!(index_range_msg(run_value(&mut env, "M[1] =").expect_err("one index")), "M is a matrix — give a row and a column, like M[1, 0]");
    assert_eq!(
        index_range_msg(run_value(&mut env, "M[2, 0] =").expect_err("no third row")),
        "index [2, 0] is outside M (rows 0 to 1, columns 0 to 1)"
    );
    run(&mut env, "x := 5").expect("defines x");
    assert_eq!(index_range_msg(run_value(&mut env, "x[0] =").expect_err("not a vector")), "x isn't a vector");
    assert_eq!(index_range_msg(run_value(&mut env, "π[0] =").expect_err("not a vector")), "π isn't a vector");
    assert_eq!(run_value(&mut env, "z[0] ="), Err(CalcError::Undefined("z".into())));
    run(&mut env, "f(x) := x").expect("defines f");
    assert!(matches!(run_value(&mut env, "f[0] ="), Err(CalcError::Syntax(_))));
    env.mark_failed("F");
    assert_eq!(run_value(&mut env, "F[0] ="), Err(CalcError::Upstream));
    // Counting from 1, 0 is outside.
    let mut env = pipes(1);
    assert_eq!(index_range_msg(run_value(&mut env, "D[0] =").expect_err("before the start")), "index 0 is outside D (1 to 3)");
}

#[test]
fn ranges_evaluate() {
    let mut env = Env::new();
    // A range includes both ends.
    let (shape, items) = grid(&mut env, "i := 0..5 =");
    assert_eq!(shape, (6, 1));
    close_all(&items, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], Dim::ONE);
    close_all(&grid(&mut env, "x := 1, 1.5..3 =").1, &[1.0, 1.5, 2.0, 2.5, 3.0], Dim::ONE);
    close_all(&grid(&mut env, "len := 0 in, 0.5 in..1 in =").1, &[0.0, 0.5 * INCH, INCH], Dim::LENGTH);
    // With no step, a range counts down as well as up.
    close_all(&grid(&mut env, "c := 5..0 =").1, &[5.0, 4.0, 3.0, 2.0, 1.0, 0.0], Dim::ONE);
    // The ends are expressions; a range of one element is a vector.
    run(&mut env, "n := 3").expect("defines n");
    close_all(&grid(&mut env, "k := 0..n-1 =").1, &[0.0, 1.0, 2.0], Dim::ONE);
    assert_eq!(grid(&mut env, "o := 2..2 =").0, (1, 1));
    // A step that doesn't land on the last value stops short of it.
    close_all(&grid(&mut env, "ev := 0, 2..5 =").1, &[0.0, 2.0, 4.0], Dim::ONE);
    // 10,000 elements is allowed.
    assert_eq!(grid(&mut env, "big := 1..10000 =").0, (10_000, 1));
}

#[test]
fn bad_ranges_are_errors() {
    let mut env = Env::new();
    column(&mut env, "D", &[1.0, 2.0], "in");
    let zero = run(&mut env, "x := 1, 1..3").expect_err("zero step");
    assert!(matches!(zero, CalcError::Syntax(_)), "{zero:?}");
    let away = run(&mut env, "x := 1, 0..3").expect_err("wrong direction");
    assert!(matches!(away, CalcError::Syntax(_)), "{away:?}");
    assert!(matches!(run(&mut env, "x := 0..5 in"), Err(CalcError::Unit(UnitError::Mismatch { .. }))));
    assert!(matches!(run(&mut env, "x := 0 in, 1..5 in"), Err(CalcError::Unit(UnitError::Mismatch { .. }))));
    assert_eq!(run(&mut env, "x := 0..20000"), Err(CalcError::TooLarge));
    // A count too large for any integer is too large too, not a panic or a wrap.
    assert_eq!(run(&mut env, "x := 0, 10^-300..1"), Err(CalcError::TooLarge));
    assert_eq!(run(&mut env, "x := 0, 10^-300..10^300"), Err(CalcError::TooLarge));
    // A range with units needs a step: counting by 1 in SI units would be a wrong-looking answer.
    assert!(matches!(run(&mut env, "x := 0 in..1 in"), Err(CalcError::Syntax(_))));
    // The ends are single values.
    assert!(matches!(run(&mut env, "x := D..5"), Err(CalcError::Syntax(_))));
    // A range defines a plain variable only.
    for src in ["f(x) := 0..x", "t[i] := 0..5"] {
        assert_eq!(syntax_msg(run(&mut env, src).expect_err(src)), "a range defines a variable, like i := 0..5", "{src}");
    }
    let built = Expr::Binary(
        Op::Add,
        Box::new(Expr::Range { first: Box::new(Expr::Num(0.0)), second: None, last: Box::new(Expr::Num(2.0)) }),
        Box::new(Expr::Num(1.0)),
    );
    assert_eq!(syntax_msg(eval_expr(&env, &built).expect_err("a range inside an expression")), "a range defines a variable, like i := 0..5");
    // Nothing failed defined anything.
    assert_eq!(env.names(), vec!["D".to_string()]);
    // The range's elements are charged before they are built.
    let pass = Pass::new(u64::MAX);
    run_stmt_in(&mut env, &parse("r := 0..9999").expect("parses"), &pass).expect("runs");
    assert_eq!(pass.spent.get(), 3 + 10_000);
    assert_eq!(run_stmt_in(&mut env, &parse("r := 0..9999").expect("parses"), &Pass::new(5_000)), Err(CalcError::TooMuchWork));
}

#[test]
fn indexed_definition_fills_a_vector() {
    let mut env = pipes(0);
    run(&mut env, "i := 0..2").expect("defines i");
    assert_eq!(run(&mut env, "t[i] := 2·D[i]"), Ok(None));
    let (shape, items) = grid(&mut env, "t =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[1.68 * INCH, 2.1 * INCH, 2.63 * INCH], Dim::LENGTH);
    // Shown as it is defined; `i` is each element in turn.
    close_all(&grid(&mut env, "sq[i] := i^2 =").1, &[0.0, 1.0, 4.0], Dim::ONE);
    // Counting from 1.
    let mut env = pipes(1);
    run(&mut env, "i := 1..3").expect("defines i");
    run(&mut env, "t[i] := 2·D[i]").expect("defines t");
    close_all(&grid(&mut env, "t =").1, &[1.68 * INCH, 2.1 * INCH, 2.63 * INCH], Dim::LENGTH);
    // 10,000 elements fit in one recalculation's budget: reading D[i] doesn't copy D.
    let mut env = Env::new();
    column(&mut env, "D", &vec![1.0; 10_000], "in");
    run(&mut env, "i := 0..9999").expect("defines i");
    let pass = Pass::new(super::MAX_PASS_STEPS);
    run_stmt_in(&mut env, &parse("t[i] := 2·D[i]").expect("parses"), &pass).expect("fits the budget");
    assert_eq!(grid(&mut env, "t =").0, (10_000, 1));
}

#[test]
fn indexed_definition_is_strict() {
    let mut env = pipes(0);
    run(&mut env, "i := 1..2").expect("defines i");
    assert_eq!(syntax_msg(run(&mut env, "t[i] := 2·D[i]").expect_err("t[0] unset")), "t[0] isn't set by i — the range must cover every element");
    // The body giving a grid for an element is an error.
    run(&mut env, "j := 0..2").expect("defines j");
    assert!(matches!(run(&mut env, "u[j] := D"), Err(CalcError::Syntax(_))));
    // The range variable must be a vector of whole, plain numbers.
    run(&mut env, "hf := 0, 0.5..2").expect("defines hf");
    run(&mut env, "q := 0 in, 1 in..2 in").expect("defines q");
    run(&mut env, "k := 1").expect("defines k");
    for src in ["v[hf] := 1", "v[q] := 1", "v[k] := 1", "v[M] := 1"] {
        assert!(matches!(run(&mut env, src), Err(CalcError::Syntax(_))), "{src}");
    }
    // An index before the origin, or one making the vector too large, is refused before any work.
    run(&mut env, "n := -1..2").expect("defines n");
    assert!(matches!(run(&mut env, "v[n] := 1"), Err(CalcError::IndexRange(_))));
    run(&mut env, "w := [0, 20000]").expect("defines w");
    assert_eq!(run(&mut env, "v[w] := 1"), Err(CalcError::TooLarge));
    assert_eq!(run(&mut env, "v[z] := 1"), Err(CalcError::Undefined("z".into())));
    // An element's error names the element.
    let e = run(&mut env, "v[j] := 1/(j - 1)").expect_err("1/0");
    assert_eq!(e, CalcError::Element { at: "1".into(), error: Box::new(CalcError::NonFinite) });
    // Nothing failed defined anything.
    for name in ["t", "u", "v"] {
        assert!(!env.names().contains(&name.to_string()), "{name}");
    }
}

#[test]
fn nested_indexes_are_bounded_not_a_stack_overflow() {
    let mut env = Env::new();
    run(&mut env, "Z := [0]").expect("defines Z");
    // As deep as the parser allows.
    let src = format!("{}0{} =", "Z[".repeat(60), "]".repeat(60));
    close(shown(&mut env, &src), 0.0, Dim::ONE);
    // Hand-built: just inside the evaluator's depth limit evaluates, far past it is refused. An
    // index or literal level costs less stack than a user-function call level (eval → call →
    // call_user → eval), which already needs about 2.7 MB at the depth limit; 4 MB covers both.
    let deep = move || {
        let nest = |n: usize| (0..n).fold(Expr::Num(0.0), |e, _| Expr::Index("Z".into(), vec![e]));
        assert_eq!(eval_expr(&env, &nest(2000)), Ok(Value::One(Quantity::dimensionless(0.0))));
        assert_eq!(eval_expr(&env, &nest(3000)), Err(CalcError::TooComplex));
        let literals =
            |n: usize| (0..n).fold(Expr::Num(0.0), |e, _| Expr::Index("Z".into(), vec![Expr::Matrix { rows: 1, cols: 1, items: vec![e] }]));
        assert!(eval_expr(&env, &literals(3000)).is_err());
        let matrices = |n: usize| (0..n).fold(Expr::Num(0.0), |e, _| Expr::Matrix { rows: 1, cols: 1, items: vec![e] });
        assert_eq!(eval_expr(&env, &matrices(3000)), Err(CalcError::TooComplex));
    };
    std::thread::Builder::new().stack_size(4 << 20).spawn(deep).expect("spawns").join().expect("no panic");
}

// ---------------------------------------------------------------------------------------------
// Array functions
// ---------------------------------------------------------------------------------------------

/// `q` is close to `want`, dimension and value.
fn close_to(q: Quantity, want: Quantity) {
    close(q, want.value, want.dim);
}

/// One value in `unit`, as an SI quantity.
fn quantity(value: f64, unit: &str) -> Quantity {
    quantities(&[value], unit).into_iter().next().expect("one quantity")
}

/// An env holding `NPS := [0.5, 0.75, 1, 1.5] in` and the flange torques
/// `T := [30, 45, 60, 90] ft·lbf`.
fn flanges() -> Env {
    let mut env = Env::new();
    run(&mut env, "NPS := [0.5, 0.75, 1, 1.5] in").expect("defines NPS");
    run(&mut env, "T := [30, 45, 60, 90] ft·lbf").expect("defines T");
    env
}

#[test]
fn size_functions() {
    let mut env = Env::new();
    run(&mut env, "M := [1, 2; 3, 4; 5, 6]").expect("defines M");
    close(shown(&mut env, "length(M) ="), 6.0, Dim::ONE);
    close(shown(&mut env, "rows(M) ="), 3.0, Dim::ONE);
    close(shown(&mut env, "cols(M) ="), 2.0, Dim::ONE);
    close(shown(&mut env, "length(5 in) ="), 1.0, Dim::ONE);
    assert_eq!(run(&mut env, "length(M, M) ="), Err(CalcError::Arity("length".into(), 1)));
}

#[test]
fn reductions() {
    let mut env = pipes(0);
    close(shown(&mut env, "min([3, 1, 2] in) ="), INCH, Dim::LENGTH);
    close(shown(&mut env, "max(D, 2 in) ="), 2.0 * INCH, Dim::LENGTH);
    close(shown(&mut env, "max(D) ="), 1.315 * INCH, Dim::LENGTH);
    close_to(shown(&mut env, "sum([1, 2] lbf) ="), quantity(3.0, "lbf"));
    close(shown(&mut env, "mean([1, 2, 3] in) ="), 2.0 * INCH, Dim::LENGTH);
    assert!(matches!(run(&mut env, "min([1 in, 2 lbf]) ="), Err(CalcError::Unit(_))));
    assert!(matches!(run(&mut env, "sum([1 in, 2 lbf]) ="), Err(CalcError::Unit(_))));
    assert_eq!(run(&mut env, "sum(D, D) ="), Err(CalcError::Arity("sum".into(), 1)));
}

#[test]
fn products() {
    let mut env = Env::new();
    close_to(shown(&mut env, "dot([1, 2] in, [3, 4] lbf) ="), quantity(11.0, "lbf·in"));
    let (shape, items) = grid(&mut env, "mmul([1, 2; 3, 4], [5; 6]) =");
    assert_eq!(shape, (2, 1));
    close_all(&items, &[17.0, 39.0], Dim::ONE);
    // The inner sizes must agree: the message names the arguments, by name when they are names.
    let e = run_value(&mut env, "mmul([1, 2, 3; 4, 5, 6], [1, 2, 3; 4, 5, 6]) =").expect_err("3 columns by 2 rows");
    assert_eq!(syntax_msg(e), "mmul needs A's columns to match B's rows (3 and 2)");
    run(&mut env, "M := [1, 2, 3; 4, 5, 6]").expect("defines M");
    run(&mut env, "X := [1; 2]").expect("defines X");
    let e = run_value(&mut env, "mmul(M, X) =").expect_err("3 columns by 2 rows");
    assert_eq!(syntax_msg(e), "mmul needs M's columns to match X's rows (3 and 2)");
    let (shape, items) = grid(&mut env, "transpose([1, 2, 3]) =");
    assert_eq!(shape, (1, 3));
    close_all(&items, &[1.0, 2.0, 3.0], Dim::ONE);
    let (shape, items) = grid(&mut env, "transpose([1, 2; 3, 4; 5, 6]) =");
    assert_eq!(shape, (2, 3));
    close_all(&items, &[1.0, 3.0, 5.0, 2.0, 4.0, 6.0], Dim::ONE);
    assert_eq!(syntax_msg(run(&mut env, "dot([1, 2], [1, 2, 3]) =").expect_err("lengths")), "dot needs a and b of the same length (2 and 3)");
}

#[test]
fn lookup_is_exact() {
    let mut env = flanges();
    let t90 = quantity(90.0, "ft·lbf");
    assert_eq!(shown(&mut env, "lookup(NPS, T, 1.5 in) ="), t90);
    close_to(shown(&mut env, "lookup(NPS, T, 38.1 mm) ="), t90);
    assert_eq!(run(&mut env, "lookup(NPS, T, 2 in) ="), Err(CalcError::Syntax("2 in isn't in NPS".into())));
    // A vector x gives one result per element; a miss names the element.
    let (shape, items) = grid(&mut env, "lookup(NPS, T, [0.5, 1] in) =");
    assert_eq!(shape, (2, 1));
    assert_eq!(items, vec![quantity(30.0, "ft·lbf"), quantity(60.0, "ft·lbf")]);
    let e = run_value(&mut env, "lookup(NPS, T, [0.5, 0.6] in) =").expect_err("0.6 in isn't a size");
    assert_eq!(e.to_string(), "element 1: 0.6 in isn't in NPS");
    assert!(matches!(run(&mut env, "lookup(NPS, T, 1 lbf) ="), Err(CalcError::Unit(UnitError::Mismatch { .. }))));
    // A size listed twice matches more than one value: an error, never the first match.
    run(&mut env, "NPS_2 := [0.5, 1.5, 1, 1.5] in").expect("defines NPS_2");
    let e = run(&mut env, "lookup(NPS_2, T, 1.5 in) =").expect_err("two matches");
    assert_eq!(syntax_msg(e), "1.5 in matches more than one value in NPS_2");
    // Equal after conversion counts as a match too.
    let e = run(&mut env, "lookup(NPS_2, T, 38.1 mm) =").expect_err("two matches");
    assert_eq!(syntax_msg(e), "1.5 in matches more than one value in NPS_2");
    close_to(shown(&mut env, "lookup(NPS_2, T, 1 in) ="), quantity(60.0, "ft·lbf"));
}

#[test]
fn linterp_and_spline_never_extrapolate() {
    let mut env = flanges();
    close_to(shown(&mut env, "linterp(NPS, T, 1.25 in) ="), quantity(75.0, "ft·lbf"));
    close_to(shown(&mut env, "linterp(NPS, T, 38.1 mm) ="), quantity(90.0, "ft·lbf"));
    for f in ["linterp", "spline"] {
        let outside = run(&mut env, &format!("{f}(NPS, T, 1.6 in) =")).expect_err("past the end");
        assert_eq!(outside, CalcError::Syntax("1.6 in is outside NPS (0.5 in to 1.5 in)".into()), "{f}");
        let below = run(&mut env, &format!("{f}(NPS, T, 0.25 in) =")).expect_err("before the start");
        assert_eq!(below, CalcError::Syntax("0.25 in is outside NPS (0.5 in to 1.5 in)".into()), "{f}");
    }
    column(&mut env, "X", &[0.0, 1.0, 2.0, 3.0, 4.0], "1");
    column(&mut env, "L", &[1.0, 3.0, 5.0, 7.0, 9.0], "1");
    column(&mut env, "S", &[0.0, 1.0, 4.0, 9.0, 16.0], "1");
    // A natural spline reproduces a line exactly.
    assert_eq!(shown(&mut env, "spline(X, L, 1.5) ="), Quantity::dimensionless(4.0));
    // Through y = x², it gives the knot values at the knots and lies between neighbours elsewhere.
    for k in 0..5 {
        assert_eq!(shown(&mut env, &format!("spline(X, S, {k}) =")).value, f64::from(k * k), "x = {k}");
    }
    let between = shown(&mut env, "spline(X, S, 1.5) =").value;
    assert!(between > 1.0 && between < 4.0, "{between}");
    // The solver itself: the second derivatives through y = x² at 0…4 are 0, 18/7, 12/7, 18/7, 0,
    // so s(1.5) = 2.5 − (3/8)(18/7 + 12/7)/6 = 125/56.
    assert!((between - 125.0 / 56.0).abs() <= 1e-9, "{between}");
    // Uneven spacing (widths 1, 2, 3), where swapping an interval's neighbours' widths changes the
    // answer: through y = x² at 0, 1, 3, 6 the second derivatives are 0, 15/7, 18/7, 0, giving
    // s(2) = 107/28 and s(2.5) = 1367/224 (derived in the task 4 report).
    column(&mut env, "X_u", &[0.0, 1.0, 3.0, 6.0], "1");
    column(&mut env, "S_u", &[0.0, 1.0, 9.0, 36.0], "1");
    for (x, want) in [("2", 107.0 / 28.0), ("2.5", 1367.0 / 224.0)] {
        let got = shown(&mut env, &format!("spline(X_u, S_u, {x}) =")).value;
        assert!((got - want).abs() <= 1e-9, "s({x}) = {got}, want {want}");
    }
    // A vector x gives one result per element.
    let (shape, items) = grid(&mut env, "spline(X, L, [0.5, 1.5, 2.5]) =");
    assert_eq!(shape, (3, 1));
    close_all(&items, &[2.0, 4.0, 6.0], Dim::ONE);
    let (shape, items) = grid(&mut env, "linterp(NPS, T, [0.5, 1.25] in) =");
    assert_eq!(shape, (2, 1));
    close_all(&items, &[quantity(30.0, "ft·lbf").value, quantity(75.0, "ft·lbf").value], quantity(1.0, "ft·lbf").dim);
    let e = run_value(&mut env, "linterp(NPS, T, [1, 1.6] in) =").expect_err("1.6 in is past the end");
    assert_eq!(e.to_string(), "element 1: 1.6 in is outside NPS (0.5 in to 1.5 in)");
}

#[test]
fn interp_needs_increasing_xs() {
    let mut env = flanges();
    column(&mut env, "NPS_2", &[0.5, 1.0, 1.0, 1.5], "in");
    column(&mut env, "T_3", &[30.0, 45.0, 60.0], "ft·lbf");
    for f in ["linterp", "spline"] {
        let e = run(&mut env, &format!("{f}(NPS_2, T, 1.25 in) =")).expect_err("not increasing");
        assert_eq!(syntax_msg(e), format!("{f} needs NPS_2 strictly increasing — element 2 isn't more than element 1"));
        let e = run(&mut env, &format!("{f}(NPS, T_3, 1 in) =")).expect_err("lengths differ");
        assert_eq!(syntax_msg(e), format!("{f} needs NPS and T_3 of the same length (4 and 3)"));
    }
    let e = run(&mut env, "lookup(NPS, T_3, 1 in) =").expect_err("lengths differ");
    assert_eq!(syntax_msg(e), "lookup needs NPS and T_3 of the same length (4 and 3)");
    // An argument that isn't a bare name is called by what it is.
    let e = run(&mut env, "lookup([1, 2, 3, 4], [1, 2, 3], 1) =").expect_err("lengths differ");
    assert_eq!(syntax_msg(e), "lookup needs xs and ys of the same length (4 and 3)");
    let e = run(&mut env, "linterp([1, 3, 2], [1, 2, 3], 1.5) =").expect_err("not increasing");
    assert_eq!(syntax_msg(e), "linterp needs xs strictly increasing — element 2 isn't more than element 1");
}

#[test]
fn new_function_names_are_reserved() {
    for name in ["sum", "length", "rows", "cols", "mean", "dot", "mmul", "transpose", "lookup", "linterp", "spline"] {
        assert_eq!(check_name(name), Err(CalcError::ReservedName(name.into())), "{name}");
    }
}

#[test]
fn products_are_bounded() {
    let stmt = |src: &str| parse(src).expect("parses");
    let mut env = Env::new();
    env.define_array("C", 10_000, 1, vec![Quantity::dimensionless(1.0); 10_000]).expect("defines C");
    env.define_array("R", 1, 10_000, vec![Quantity::dimensionless(1.0); 10_000]).expect("defines R");
    // 10,000 × 10,000 elements: too large, before any of the product is worked out. The pass has
    // paid only for the three nodes and for reading C and R.
    let pass = Pass::new(u64::MAX);
    assert_eq!(run_stmt_in(&mut env, &stmt("p := mmul(C, R)"), &pass), Err(CalcError::TooLarge));
    assert_eq!(pass.spent.get(), 3 + 20_000);
    // A product charges rows · cols · inner.
    env.define_array("A", 2, 3, vec![Quantity::dimensionless(1.0); 6]).expect("defines A");
    env.define_array("B", 3, 2, vec![Quantity::dimensionless(1.0); 6]).expect("defines B");
    let pass = Pass::new(u64::MAX);
    run_stmt_in(&mut env, &stmt("q := mmul(A, B)"), &pass).expect("runs");
    assert_eq!(pass.spent.get(), 3 + 12 + 2 * 2 * 3);
    // One element, but 10,000 products to sum.
    let pass = Pass::new(u64::MAX);
    run_stmt_in(&mut env, &stmt("r := mmul(R, C)"), &pass).expect("runs");
    assert_eq!(pass.spent.get(), 3 + 20_000 + 10_000);
    assert_eq!(run_stmt_in(&mut env, &stmt("r := mmul(R, C)"), &Pass::new(25_000)), Err(CalcError::TooMuchWork));
}

#[test]
fn array_messages_use_the_display_format() {
    let mut env = flanges();
    env.set_display(UnitSystem::Si, Precision::new(4));
    let e = run(&mut env, "linterp(NPS, T, 1.6 in) =").expect_err("past the end");
    assert_eq!(syntax_msg(e), "0.0406 m is outside NPS (0.0127 m to 0.0381 m)");
}

#[test]
fn reductions_of_temperatures_keep_what_they_stand_for() {
    let mut env = Env::new();
    // The smallest, largest or mean of temperatures is one; a sum isn't.
    for src in ["min([300 K, 310 K]) = ? °C", "max([300 K, 310 K], 305 K) = ? °C", "mean([300 K, 310 K]) = ? °C"] {
        assert!(run(&mut env, src).is_ok(), "{src}");
    }
    assert_eq!(run(&mut env, "sum([300 K, 310 K]) = ? °C"), Err(CalcError::NotAbsolute("°C".into())));
    assert_eq!(run(&mut env, "max([300 K, 310 K], 5 Δ°C) = ? °C"), Err(CalcError::NotAbsolute("°C".into())));
}

#[test]
fn array_messages_always_show_a_unit() {
    let mut env = flanges();
    // 10^307 m is too many inches to show: it shows in metres rather than as a bare number.
    let e = run(&mut env, "linterp(NPS, T, 10^307 m) =").expect_err("past the end");
    assert_eq!(syntax_msg(e), "1×10³⁰⁷ m is outside NPS (0.5 in to 1.5 in)");
}
