//! RFC 8785 §3.2.2.3 / ES2019 §7.1.12.1, independent expected spellings.
use bennett_vm::json::{self, Json, format_f64};

#[test]
fn jcs_number_boundaries_serialize_and_accept() {
    for canonical in [
        "1e-8",
        "2e-7",
        "2e-8",
        "1e-7",
        "0.000001",
        "1e+21",
        "123456789012345680000",
        "0.1",
        "1.5e-7",
        "0",
        "5e-324",
        "1.7976931348623157e+308",
        "1",
        "42",
        "9007199254740991",
        "9007199254740992",
        "100000000000000000000",
    ] {
        for raw in [canonical.to_string(), format!("-{canonical}")] {
            if raw == "-0" {
                continue;
            }
            let value = raw.parse::<f64>().unwrap();
            assert_eq!(format_f64(value), raw, "serialize {raw}");
            // The same byte comparison used by target/object canonical loading.
            assert_eq!(json::parse(&raw).unwrap().to_jcs(), raw, "accept {raw}");
        }
    }
    assert_eq!(format_f64(-0.0), "0");
}

#[test]
fn jcs_noncanonical_number_spellings() {
    for (raw, expected) in [
        ("1e-6", "0.000001"),
        ("1e21", "1e+21"),
        ("1000000000000000000000", "1e+21"),
        ("-0", "0"),
        ("-0.0", "0"),
        ("0.0", "0"),
        ("1.0", "1"),
        ("0.10", "0.1"),
        ("1E-8", "1e-8"),
        ("1e-08", "1e-8"),
        ("0.00000001", "1e-8"),
        // These source decimals cannot survive binary64 round-trip unchanged.
        ("9007199254740993", "9007199254740992"),
        ("-9007199254740993", "-9007199254740992"),
        ("0.10000000000000001", "0.1"),
        ("1e-999", "0"),
    ] {
        let actual = json::parse(raw).unwrap().to_jcs();
        assert_eq!(actual, expected, "normalize {raw}");
        assert_ne!(actual, raw, "must reject {raw} as canonical");
    }
}

#[test]
fn json_number_grammar_and_finiteness() {
    for raw in [
        "01",
        "-01",
        "+1",
        "1.",
        ".1",
        "-.1",
        "1.e2",
        "1e",
        "1e+",
        "--1",
        "1-2",
        "NaN",
        "Infinity",
        "-Infinity",
        "inf",
        "1e999",
        "-1e999",
    ] {
        assert!(json::parse(raw).is_err(), "must reject {raw}");
    }
}

#[test]
fn jcs_rfc8785_appendix_b() {
    // Published IEEE-754 bits and decimal answers, including Note 2 ties-to-even.
    for (bits, expected) in [
        (0x0000000000000000, "0"),
        (0x8000000000000000, "0"),
        (0x0000000000000001, "5e-324"),
        (0x8000000000000001, "-5e-324"),
        (0x7fefffffffffffff, "1.7976931348623157e+308"),
        (0xffefffffffffffff, "-1.7976931348623157e+308"),
        (0x4340000000000000, "9007199254740992"),
        (0xc340000000000000, "-9007199254740992"),
        (0x4430000000000000, "295147905179352830000"),
        (0x44b52d02c7e14af5, "9.999999999999997e+22"),
        (0x44b52d02c7e14af6, "1e+23"),
        (0x44b52d02c7e14af7, "1.0000000000000001e+23"),
        (0x444b1ae4d6e2ef4e, "999999999999999700000"),
        (0x444b1ae4d6e2ef4f, "999999999999999900000"),
        (0x444b1ae4d6e2ef50, "1e+21"),
        (0x3eb0c6f7a0b5ed8c, "9.999999999999997e-7"),
        (0x3eb0c6f7a0b5ed8d, "0.000001"),
        (0x41b3de4355555553, "333333333.3333332"),
        (0x41b3de4355555554, "333333333.33333325"),
        (0x41b3de4355555555, "333333333.3333333"),
        (0x41b3de4355555556, "333333333.3333334"),
        (0x41b3de4355555557, "333333333.33333343"),
        (0xbecbf647612f3696, "-0.0000033333333333333333"),
        (0x43143ff3c1cb0959, "1424953923781206.2"),
        (0xc3143ff3c1cb0959, "-1424953923781206.2"),
    ] {
        let actual = format_f64(f64::from_bits(bits));
        assert_eq!(actual, expected, "{bits:016x}");
        assert_eq!(json::parse(expected).unwrap().to_jcs(), expected);
        if bits << 1 != 0 {
            assert_eq!(actual.parse::<f64>().unwrap().to_bits(), bits);
        }
    }
}

#[test]
fn jcs_nonfinite_emission_errors() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(std::panic::catch_unwind(|| format_f64(value)).is_err());
    }
    for raw in ["NaN", "Infinity", "1e999"] {
        assert!(std::panic::catch_unwind(|| Json::Num(raw.into()).to_jcs()).is_err());
    }
}

#[test]
fn target_accepts_canonical_numbers_and_preserves_exact_costs() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/cases/E/E-jcs-target-hash/target.json");
    let base = std::fs::read_to_string(path).unwrap();
    let mut original = json::parse(&base).unwrap();
    if let Json::Obj(root) = &mut original
        && let Json::Obj(capabilities) = root.get_mut("capabilities").unwrap()
    {
        capabilities.insert("classical_step_time".into(), Json::Str("JCS_NUMBER".into()));
    } else {
        panic!("published fixture must contain capabilities");
    }
    let template = original.to_jcs();
    for (canonical, decimal) in [
        ("1e-8", "0.00000001"),
        ("2e-7", "0.0000002"),
        ("2e-8", "0.00000002"),
        ("0.000001", "0.000001"),
        ("1e+21", "1000000000000000000000"),
    ] {
        let raw = template.replace("\"JCS_NUMBER\"", canonical);
        let target = bennett_vm::target::Target::parse(&raw).unwrap();
        assert_eq!(target.hash, bennett_vm::sha256::sha256_hex(raw.as_bytes()));
        let value = target
            .document
            .get("capabilities")
            .unwrap()
            .get("classical_step_time")
            .unwrap();
        assert_eq!(
            bennett_vm::target::eval(value, &Default::default(), &Default::default()).unwrap(),
            decimal.parse::<bennett_vm::target::Rational>().unwrap()
        );
    }
    for noncanonical in ["1e-6", "1e21", "-0", "1.0", "1E-8", "9007199254740993"] {
        let raw = template.replace("\"JCS_NUMBER\"", noncanonical);
        assert!(
            bennett_vm::target::Target::parse(&raw)
                .unwrap_err()
                .contains("canonical"),
            "must reject {noncanonical}"
        );
    }
}
