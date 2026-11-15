use super::*;
use crate::para::InlineObject;

fn calc_equation() -> InlineObject {
    InlineObject::Equation {
        linear: "σ_max=P/A".into(),
        display: true,
        math: Default::default(),
        calc: Some(Box::new(CalcRegion {
            id: 7,
            source: "σ_max := P / A".into(),
            unit: Some("ksi".into()),
            precision: Some(2),
            numbered: true,
            state: CalcState::Error("undefined: P".into()),
        })),
    }
}

#[test]
fn instr_round_trip() {
    let instr = calc_field_instr("σ_max", Some("ksi"), Some(2));
    assert_eq!(instr, "WCCALC σ_max \\u ksi \\p 2");
    assert_eq!(parse_calc_field(&instr), Some(("σ_max".into(), Some("ksi".into()), Some(2))));
    assert_eq!(parse_calc_field("PAGE"), None);
    assert_eq!(parse_calc_field("WCCALCX a"), None);
    let huge = format!("WCCALC a \\u {}", "m".repeat(10_000));
    assert_eq!(parse_calc_field(&huge), None);
}

#[test]
fn instr_switches_are_optional() {
    assert_eq!(calc_field_instr("x", None, None), "WCCALC x");
    assert_eq!(parse_calc_field("WCCALC x"), Some(("x".into(), None, None)));
    assert_eq!(calc_field_instr("x", None, Some(0)), "WCCALC x \\p 0");
    assert_eq!(parse_calc_field("WCCALC x \\p 0"), Some(("x".into(), None, Some(0))));
    assert_eq!(parse_calc_field("WCCALC x \\p 15"), Some(("x".into(), None, Some(15))));
    // Switch order doesn't matter.
    assert_eq!(parse_calc_field("WCCALC x \\p 4 \\u m"), Some(("x".into(), Some("m".into()), Some(4))));
}

#[test]
fn instr_unit_with_spaces_is_quoted() {
    let instr = calc_field_instr("M", Some("kN m"), None);
    assert_eq!(instr, "WCCALC M \\u \"kN m\"");
    assert_eq!(parse_calc_field(&instr), Some(("M".into(), Some("kN m".into()), None)));
}

#[test]
fn instr_rejects_malformed() {
    for bad in [
        "WCCALC",
        "WCCALC   ",
        "WCCALC \\u ksi",
        "WCCALC x \\z 1",
        "WCCALC x \\u",
        "WCCALC x \\p",
        "WCCALC x \\p 16",
        "WCCALC x \\p -1",
        "WCCALC x \\p 256",
        "WCCALC x \\p two",
        "WCCALC x \\u a \\u b",
        "WCCALC x \\p 1 \\p 2",
        "WCCALC x extra",
        "WCCALC x \\u \"unterminated",
        "WCCALC x \\u \"\"",
        "WCCALC x \\u \\p",
        "WCCALC x \\u \\p 2",
        "WCCALC \"x y\"",
        "",
    ] {
        assert_eq!(parse_calc_field(bad), None, "{bad:?}");
    }
}

#[test]
fn instr_just_under_limit_parses_and_over_is_rejected() {
    let head = "WCCALC a \\u ";
    let ok = format!("{head}{}", "m".repeat(MAX_INSTR_BYTES - head.len()));
    assert_eq!(ok.len(), MAX_INSTR_BYTES);
    assert!(parse_calc_field(&ok).is_some());
    assert_eq!(parse_calc_field(&format!("{ok}m")), None);
}

#[test]
fn instr_never_panics_on_hostile_input() {
    for s in ["\"", "\\", "WCCALC \\", "WCCALC \"", "WCCALC a \\u \"é", "WCCALC a \\p ٣", "\u{0}\u{0}", "WCCALC a\u{a0}\\p 1"] {
        let _ = parse_calc_field(s);
    }
}

#[test]
fn settings_default() {
    let s = CalcSettings::default();
    assert_eq!((s.system, s.precision, s.manual), (UnitSystem::UsCustomary, 3, false));
    assert_eq!(CalcState::default(), CalcState::Fresh);
}

#[test]
fn plain_equation_serde_unchanged() {
    let plain = InlineObject::Equation { linear: "a=b".into(), display: false, math: Default::default(), calc: None };
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("calc"), "{json}");
    // And a file written before the field existed still loads.
    let back: InlineObject = serde_json::from_str(&json).unwrap();
    assert_eq!(back, plain);
    let legacy: InlineObject = serde_json::from_str(r#"{"type":"equation","linear":"a=b"}"#).unwrap();
    assert!(matches!(legacy, InlineObject::Equation { calc: None, display: false, .. }));
}

#[test]
fn calc_equation_serde_round_trip() {
    let eq = calc_equation();
    let json = serde_json::to_string(&eq).unwrap();
    assert!(json.contains("calc"), "{json}");
    let back: InlineObject = serde_json::from_str(&json).unwrap();
    assert_eq!(back, eq);
    for state in [CalcState::Fresh, CalcState::Stale, CalcState::Error("x".into()), CalcState::EditedOutside] {
        let region = CalcRegion { id: 1, source: "a:=1".into(), unit: None, precision: None, numbered: false, state };
        let back: CalcRegion = serde_json::from_str(&serde_json::to_string(&region).unwrap()).unwrap();
        assert_eq!(back, region);
    }
}

#[test]
fn settings_serde_round_trip_and_partial_load() {
    let s = CalcSettings { system: UnitSystem::Si, precision: 6, manual: true, g: GMeans::Gram };
    assert_eq!(serde_json::from_str::<CalcSettings>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
    assert_eq!(serde_json::from_str::<CalcSettings>("{}").unwrap(), CalcSettings::default());
}

#[test]
fn document_calc_settings_serde() {
    let mut doc = crate::Document::new();
    let plain = serde_json::to_string(&doc).unwrap();
    assert!(!plain.contains("calc_settings"), "an untouched document stays as it was");
    doc.calc_settings = Some(CalcSettings { system: UnitSystem::Si, precision: 5, manual: true, g: GMeans::Gram });
    let back: crate::Document = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(back.calc_settings, doc.calc_settings);
    assert!(!back.same_content(&crate::Document::new()), "calc settings are part of the document's content");
}
