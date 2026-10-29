//! Hand-derived §8 witnesses; the reference CLI has no target evaluator.
use bennett_vm::{
    json::{self, Json},
    target::{self, Gate, Rational as R, Target},
};
use std::{collections::BTreeMap, path::Path};
fn r(s: &str) -> R {
    s.parse().unwrap()
}
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
fn published(case: &str, file: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(format!(
                "bennett-isa-spec/conformance/cases/E/{case}/{file}"
            )),
    )
    .unwrap()
}
fn base() -> Json {
    j(published("E-matching-first-entry", "target.json").trim_end())
}
fn target(v: &Json) -> Target {
    Target::parse(&v.to_jcs()).unwrap()
}
fn gate(c: usize) -> Gate {
    Gate {
        op: "cx".into(),
        controls: (0..c).collect(),
        own_controls: 1,
        targets: vec![c],
        classes: vec!["fast".into(); c + 1],
        theta: None,
    }
}
#[test]
fn exact_decimal_ast_and_load_errors() {
    // §8.1: numbers are exact rationals from decimal text, not binary64.
    let vars = BTreeMap::from([("eps".into(), r("0.001")), ("k".into(), r("1"))]);
    let tables = BTreeMap::new();
    let eval = |s: &str| target::eval(&j(s), &vars, &tables);
    assert_eq!(eval(r#"{"op":"log2inv_eps","args":[]}"#).unwrap(), r("10"));
    assert_eq!(eval(r#"{"op":"+","args":[0.1,0.2]}"#).unwrap(), r("0.3"));
    assert_eq!(eval(r#"{"op":"ceil","args":[-0.25]}"#).unwrap(), r("0"));
    assert_eq!(
        eval(r#"{"op":"/","args":[1,{"op":"-","args":[{"var":"k"},{"var":"k"}]}]}"#).unwrap_err(),
        "§8.1: division by zero"
    );
    assert!(
        eval(r#"{"var":"theta"}"#)
            .unwrap_err()
            .contains("unbound theta")
    );
    assert_eq!(
        r("100000000000000000000000000000000000000.1").add(&r("0.2")),
        r("100000000000000000000000000000000000000.3")
    );
    let vars = BTreeMap::from([("eps".into(), r("2"))]);
    assert!(
        target::eval(&j(r#"{"var":"eps"}"#), &vars, &tables)
            .unwrap_err()
            .contains("eps outside")
    );
    let tables = BTreeMap::from([("cost".into(), vec![r("1"), r("2")])]);
    assert!(
        target::eval(
            &j(r#"{"table":"cost","index":"k"}"#),
            &BTreeMap::from([("k".into(), r("2"))]),
            &tables
        )
        .unwrap_err()
        .contains("table index")
    );
}
#[test]
fn canonical_target_schema_and_hash() {
    // §8.1: SHA-256 of JCS bytes. Published file's newline is NOT JCS.
    let raw = published("E-jcs-target-hash", "target.json");
    assert!(Target::parse(&raw).unwrap_err().contains("canonical"));
    let t = Target::parse(raw.trim_end()).unwrap();
    assert_eq!(
        t.hash,
        bennett_vm::sha256::sha256_hex(raw.trim_end().as_bytes())
    );
    let mut v = base();
    if let Json::Obj(m) = &mut v {
        m.insert("vendor_extension".into(), Json::Bool(true));
    }
    assert!(
        Target::parse(&v.to_jcs())
            .unwrap_err()
            .contains("unknown field")
    );
    let raw = published("E-mbu-must-be-false", "target.json");
    assert!(
        Target::parse(raw.trim_end())
            .unwrap_err()
            .contains("mbu must be false")
    );
    // The staged epsilon-zero file actually has no eps reference: spec says accept.
    Target::parse(published("E-epsilon-zero-entry", "target.json").trim_end()).unwrap();
    let mut v = base();
    if let Json::Obj(m) = &mut v {
        if let Json::Arr(gs) = m.get_mut("gates").unwrap() {
            if let Json::Obj(g) = &mut gs[0] {
                g.insert("cost".into(), j(r#"{"time":{"var":"eps"}}"#));
            }
        }
    }
    assert!(
        Target::parse(&v.to_jcs())
            .unwrap_err()
            .contains("epsilon zero entry references eps")
    );
}
#[test]
fn matching_controls_and_first_entry_accuracy() {
    // §8.3 total c includes own and inherited controls; first matching entry wins.
    let t = target(&base());
    let leaves = t.lower(&gate(1), &r("0.01"), None).unwrap();
    assert_eq!(leaves.leaves.len(), 1);
    assert_eq!(leaves.leaves[0].cost["time"], r("1"));
    assert!(
        t.lower(&gate(2), &r("0.01"), None)
            .unwrap_err()
            .contains("unsupported")
    );
    let mut v = base();
    if let Json::Obj(m) = &mut v {
        m.insert("gates".into(),j(r#"[{"op":"cx","classes":["fast"],"controls":{"min":1,"max":1},"epsilon":0.02,"cost":{"time":9}},{"op":"cx","classes":["*"],"controls":{"min":1,"max":1},"epsilon":0.001,"cost":{"time":2}}]"#));
    }
    let t = target(&v);
    assert_eq!(
        t.lower(&gate(1), &r("0.01"), None).unwrap().leaves[0].cost["time"],
        r("2")
    );
    assert_eq!(
        t.lower(&gate(1), &r("0.1"), None).unwrap().leaves[0].cost["time"],
        r("9")
    );
}
#[test]
fn scheme_expands_to_native_leaves_without_double_charge() {
    // §8.3 repeats [from,to); schemes do not nest; author owes correctness.
    let mut v = base();
    if let Json::Obj(m) = &mut v {
        m.insert(
            "schemes".into(),
            Json::Arr(vec![j(
                published("E-scheme-expansion", "scheme.json").trim_end()
            )]),
        );
    }
    let t = target(&v);
    let mut g = gate(3);
    g.op = "mcx".into();
    g.own_controls = 3;
    let lowered = t.lower(&g, &r("0"), None).unwrap();
    assert_eq!(lowered.leaves.len(), 3);
    assert_eq!(
        lowered
            .leaves
            .iter()
            .map(|g| g.cost["time"].clone())
            .fold(r("0"), |a, b| a.add(&b)),
        r("3")
    );
    // E-scheme-correctness-negative: parity differs from conjunction at 1000.
    let mut bits = [true, false, false, false];
    for leaf in lowered.leaves {
        if leaf.gate.controls.iter().all(|&c| bits[c]) {
            bits[leaf.gate.targets[0]] ^= true;
        }
    }
    assert!(bits[3]); // reference mcx leaves target false; never claim correctness proof.
}
#[test]
fn event_aggregation_liveness_overlap_and_capacity() {
    // §8.2: makespan, leaf count once, live-interval occupancy and peak.
    let mut v = base();
    if let Json::Obj(m) = &mut v {
        if let Json::Obj(rs) = m.get_mut("resources").unwrap() {
            rs.insert("gate_count".into(), j(r#"{"kind":"count","unit":"gate"}"#));
        }
    }
    let t = target(&v);
    let events = j(published("E-aggregation", "events.json").trim_end());
    let result = t.account_json(&events).unwrap();
    assert_eq!(result.get("gate_count"), Some(&Json::Str("2".into())));
    assert_eq!(result.get("time"), Some(&Json::Str("3".into())));
    assert_eq!(
        result.get("peak_qubits").unwrap().get("fast"),
        Some(&Json::Str("2".into()))
    );
    assert_eq!(
        result.get("qubit_second").unwrap().get("fast"),
        Some(&Json::Str("4".into()))
    );
    let overlap = j(
        r#"{"leaves":[{"start":0,"duration":1,"wires":[0,1],"cost":{}},{"start":0,"duration":1,"wires":[2,3],"cost":{}}],"live_intervals":[{"wire":0,"class":"fast","start":0,"end":1},{"wire":1,"class":"fast","start":0,"end":1},{"wire":2,"class":"fast","start":0,"end":1},{"wire":3,"class":"fast","start":0,"end":1}]}"#,
    );
    assert_eq!(
        t.account_json(&overlap)
            .unwrap()
            .get("peak_qubits")
            .unwrap()
            .get("fast"),
        Some(&Json::Str("4".into()))
    );
    let mut bad = overlap.clone();
    if let Json::Obj(m) = &mut bad {
        if let Json::Arr(ls) = m.get_mut("leaves").unwrap() {
            if let Json::Obj(e) = &mut ls[1] {
                e.insert("wires".into(), j("[0,3]"));
            }
        }
    }
    assert!(t.account_json(&bad).unwrap_err().contains("overlap"));
}
