use bennett_vm::{
    estimate, invocation,
    json::{self, Json},
    object::Type,
    target::{Event, LiveInterval, Rational as R, Target},
};
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
#[test]
fn exact_upward_34_digit_decimal_witnesses() {
    for (n, d, want) in [
        (0, 1, "0"),
        (2, 1, "2"),
        (1, 8, "0.125"),
        (1, 3, "0.3333333333333333333333333333333334"),
        (2, 3, "0.6666666666666666666666666666666667"),
        (1, 30, "0.03333333333333333333333333333333334"),
        (10, 3, "3.333333333333333333333333333333334"),
    ] {
        let r = R::from(n).div(&R::from(d)).unwrap();
        assert_eq!(r.upper_decimal34().unwrap(), want);
        assert!(want.parse::<R>().unwrap() >= r);
    }
    for s in [
        "0.0000000000000000000000000000000000001",
        "99999999999999999999999999999999999",
        "1e100",
        "1e-100",
    ] {
        let r = s.parse::<R>().unwrap();
        let out = r.upper_decimal34().unwrap();
        assert!(!out.contains('e'));
        assert!(out.parse::<R>().unwrap() >= r);
    }
}
#[test]
fn resolved_arguments_are_bit_exact_and_have_no_defaults() {
    let args = j(r#"["18446744073709551617","0x8000000000000000",["2","0","0","0"]]"#);
    let types = [Type::I(128), Type::F64, Type::Token];
    let values = invocation::decode_args(&args, &types, 32).unwrap();
    assert_eq!(
        values[0].bits().unwrap().to_decimal(),
        "18446744073709551617"
    );
    assert!(matches!(values[1],bennett_vm::value::Value::F64(f) if f.to_bits()==1<<63));
    assert_eq!(
        invocation::params_hash(&args),
        "9f628a694fa7259bf899bfb677aabce1c084e268a738a40f7f643eb6d8c9c093"
    );
    for bad in [
        r#"[]"#,
        r#"["01","0x8000000000000000",["2","0","0","0"]]"#,
        r#"["2","0x8000000000000000",7]"#,
    ] {
        assert!(invocation::decode_args(&j(bad), &types, 32).is_err());
    }
    assert!(invocation::decode_args(&j(r#"["256"]"#), &[Type::I(8)], 32).is_err());
}
#[test]
fn aggregation_retains_classes_escaped_keys_and_exact_rationals() {
    let _root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let mut t = j(&std::fs::read_to_string(
        crate::pins::isa().join("conformance/cases/E/E-matching-first-entry/target.json"),
    )
    .unwrap());
    if let Json::Obj(m) = &mut t {
        m.insert("classes".into(),j(r#"{"A":{"qubit_second":1,"dirty_rate":1,"capacity":"unbounded"},"B]":{"qubit_second":1,"dirty_rate":1,"capacity":"unbounded"},"idle":{"qubit_second":1,"dirty_rate":1,"capacity":"unbounded"}}"#));
        m.insert("gates".into(), j("[]"));
        m.insert("schemes".into(), j("[]"));
    }
    let t = Target::parse(&t.to_jcs()).unwrap();
    let third = R::from(1).div(&R::from(3)).unwrap();
    let live = (0..3)
        .map(|wire| LiveInterval {
            wire,
            class: if wire < 2 { "A" } else { "B]" }.into(),
            start: R::zero(),
            end: third.clone(),
            dirty: false,
        })
        .collect::<Vec<_>>();
    let rows = estimate::resource_rows(
        &t,
        &[Event {
            start: R::zero(),
            duration: third,
            wires: vec![],
            cost: Default::default(),
        }],
        &live,
    )
    .unwrap();
    assert_eq!(rows["peak_qubits[A]"].decimal, "2");
    assert_eq!(rows["peak_qubits[B%5D]"].decimal, "1");
    assert_eq!(rows["peak_qubits[idle]"].decimal, "0");
    assert_eq!(rows["qubit_second[A]"].numerator, "2");
    assert_eq!(rows["qubit_second[A]"].denominator, "3");
    assert_eq!(rows["time"].decimal, "0.3333333333333333333333333333333334");
    assert_eq!(estimate::escape("a%[é]"), "a%25%5B%C3%A9%5D");
}
