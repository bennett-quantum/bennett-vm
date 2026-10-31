use bennett_vm::{
    json::{self, Json},
    target::{self, Rational as R, Target},
};
use std::collections::BTreeMap;
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
#[test]
fn exact_decimal_bucket_boundaries_and_domain() {
    let expr = j(r#"{"table":"bucket","index":"eps_decade"}"#);
    let tables = BTreeMap::from([("bucket".into(), (0..120).map(R::from).collect())]);
    for (eps, want) in [
        ("1", 0),
        ("0.1", 1),
        ("0.1001", 1),
        ("0.0999", 2),
        ("0.001", 3),
        ("0.000999", 4),
        ("0.10000000000000000000000000000000000000000000001", 1),
        ("0.09999999999999999999999999999999999999999999999", 2),
        ("1e-100", 100),
    ] {
        assert_eq!(
            target::eval(
                &expr,
                &BTreeMap::from([("eps".into(), eps.parse().unwrap())]),
                &tables
            )
            .unwrap(),
            R::from(want)
        );
    }
    for eps in ["0", "-0.1", "1.0001"] {
        assert!(
            target::eval(
                &expr,
                &BTreeMap::from([("eps".into(), eps.parse().unwrap())]),
                &tables
            )
            .is_err()
        );
    }
    assert!(target::eval(&expr, &BTreeMap::new(), &tables).is_err());
    let short = BTreeMap::from([("bucket".into(), vec![R::from(0), R::from(1)])]);
    assert!(
        target::eval(
            &expr,
            &BTreeMap::from([("eps".into(), "0.0999".parse().unwrap())]),
            &short
        )
        .unwrap_err()
        .contains("outside array")
    );
}
#[test]
fn exact_entry_prohibits_eps_decade_even_in_nested_cost() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let raw = std::fs::read_to_string(
        root.join("bennett-isa-spec/conformance/cases/E/E-matching-first-entry/target.json"),
    )
    .unwrap();
    let mut t = j(&raw);
    if let Json::Obj(m) = &mut t {
        m.insert("tables".into(), j(r#"{"bucket":[1,2,3]}"#));
        if let Json::Arr(g) = m.get_mut("gates").unwrap()
            && let Json::Obj(g) = &mut g[0]
        {
            g.insert(
                "cost".into(),
                j(r#"{"time":{"op":"+","args":[1,{"table":"bucket","index":"eps_decade"}]}}"#),
            );
        }
    }
    assert!(
        Target::parse(&t.to_jcs())
            .unwrap_err()
            .contains("epsilon zero entry references eps")
    );
    // Achieved epsilon is nonzero; the cost reads requested eps, not achieved epsilon.
    if let Json::Obj(m) = &mut t
        && let Json::Arr(g) = m.get_mut("gates").unwrap()
        && let Json::Obj(g) = &mut g[0]
    {
        g.insert("epsilon".into(), j("0.001"));
    }
    Target::parse(&t.to_jcs()).unwrap();
}
