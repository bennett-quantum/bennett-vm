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
    if let Json::Obj(m) = &mut v
        && let Json::Arr(gs) = m.get_mut("gates").unwrap()
        && let Json::Obj(g) = &mut gs[0]
    {
        g.insert("cost".into(), j(r#"{"time":{"var":"eps"}}"#));
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
    if let Json::Obj(m) = &mut v
        && let Json::Obj(rs) = m.get_mut("resources").unwrap()
    {
        rs.insert("gate_count".into(), j(r#"{"kind":"count","unit":"gate"}"#));
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
    if let Json::Obj(m) = &mut bad
        && let Json::Arr(ls) = m.get_mut("leaves").unwrap()
        && let Json::Obj(e) = &mut ls[1]
    {
        e.insert("wires".into(), j("[0,3]"));
    }
    assert!(t.account_json(&bad).unwrap_err().contains("overlap"));
}

#[test]
fn launch_uses_target_scheme_and_rejects_missing_alphabet() {
    // §8.3: else first covering scheme; the executed alphabet is its native leaves.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = repo.join(".tmp/target-launch");
    std::fs::create_dir_all(&tmp).unwrap();
    let object = repo
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/cases/objects/registers.bisa");
    let mut v = base();
    if let Json::Obj(m) = &mut v {
        m.insert("gates".into(),j(r#"[{"op":"h","classes":["*"],"controls":{"min":0,"max":0},"epsilon":0,"cost":{"time":1}},{"op":"s","classes":["*"],"controls":{"min":0,"max":0},"epsilon":0,"cost":{"time":1}},{"op":"cx","classes":["*"],"controls":{"min":1,"max":1},"epsilon":0,"cost":{"time":1}}]"#));
        m.insert("schemes".into(),j(r#"[{"name":"x-via-hssh","realises":{"op":"x","controls":{"min":0,"max":0}},"ancillae":{"count":0,"state":"clean","class":"fast"},"template":[{"gate":"h","wires":["t"]},{"gate":"s","wires":["t"]},{"gate":"s","wires":["t"]},{"gate":"h","wires":["t"]}]}]"#));
    }
    let path = tmp.join("target.json");
    std::fs::write(&path, v.to_jcs()).unwrap();
    let run = |target: bool| {
        let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_bennett-vm"));
        c.arg("run-program")
            .arg(&object)
            .args(["views", "--args", ""]);
        if target {
            c.arg("--target").arg(&path);
        }
        c.output().unwrap()
    };
    let plain = run(false);
    assert!(plain.status.success());
    let lowered = run(true);
    assert!(
        lowered.status.success(),
        "{}",
        String::from_utf8_lossy(&lowered.stderr)
    );
    assert_eq!(plain.stdout, lowered.stdout);
    // No x native or scheme in published target: must fail with exact gate reason.
    std::fs::write(&path, base().to_jcs()).unwrap();
    let bad = run(true);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unsupported gate x"));
}

#[test]
fn imports_capacity_rates_and_parallelism() {
    // §8.3 imports map quantities by factor; §8.4 peak is per-class capacity.
    let mut v = base();
    if let Json::Obj(m) = &mut v {
        if let Json::Obj(rs) = m.get_mut("resources").unwrap() {
            rs.insert("gates".into(), j(r#"{"kind":"count","unit":"gate"}"#));
        }
        m.insert(
            "imports".into(),
            j(r#"{"model":{"version":"0.8","map":{"qty":["gates",2]}}}"#),
        );
        m.insert("classes".into(),j(r#"{"fast":{"capacity":2,"qubit_second":1,"dirty_rate":0.25},"slow":{"capacity":4,"qubit_second":1,"dirty_rate":0.5}}"#));
    }
    let t = target(&v);
    assert_eq!(
        t.import_quantity("model", "0.8", "qty", &r("3")).unwrap(),
        ("gates".into(), r("6"))
    );
    assert!(
        t.import_quantity("model", "0.9", "qty", &r("3"))
            .unwrap_err()
            .contains("version")
    );
    assert_eq!(t.class_for("any", 2, &BTreeMap::new()).unwrap(), "fast");
    assert_eq!(
        t.class_for("any", 1, &BTreeMap::from([("fast".into(), 2)]))
            .unwrap(),
        "slow"
    );
    assert!(
        t.class_for("fast", 3, &BTreeMap::new())
            .unwrap_err()
            .contains("capacity")
    );
    let e = j(
        r#"{"leaves":[],"live_intervals":[{"wire":0,"class":"fast","start":0,"end":2,"dirty":true}]}"#,
    );
    assert_eq!(
        t.account_json(&e)
            .unwrap()
            .get("qubit_second")
            .unwrap()
            .get("fast"),
        Some(&Json::Str("0.5".into()))
    );
}

#[test]
fn static_cost_errors_are_rejected_before_execution() {
    // §8.1: division by zero is a load-time error, even in an unused gate entry.
    let mut v = base();
    if let Json::Obj(m) = &mut v
        && let Json::Arr(gs) = m.get_mut("gates").unwrap()
        && let Json::Obj(g) = &mut gs[0]
    {
        g.insert(
            "cost".into(),
            j(r#"{"time":{"op":"/","args":[1,{"op":"-","args":[{"var":"k"},{"var":"k"}]}]}}"#),
        );
    }
    assert!(
        Target::parse(&v.to_jcs())
            .unwrap_err()
            .contains("division by zero")
    );
}

#[test]
fn jcs_class_order_uses_utf16_not_utf8() {
    // §8.1 RFC8785; §11 first class in JCS order. U+10000 precedes U+E000 in UTF16.
    let mut v = base();
    if let Json::Obj(m) = &mut v {
        m.insert("classes".into(),j("{\"𐀀\":{\"capacity\":2,\"qubit_second\":1,\"dirty_rate\":1},\"\u{e000}\":{\"capacity\":2,\"qubit_second\":1,\"dirty_rate\":1}}"));
        m.insert("gates".into(), Json::Arr(vec![]));
    }
    let raw = v.to_jcs();
    assert!(raw.find("𐀀").unwrap() < raw.find('\u{e000}').unwrap());
    assert_eq!(
        Target::parse(&raw)
            .unwrap()
            .class_for("any", 1, &BTreeMap::new())
            .unwrap(),
        "𐀀"
    );
}

#[test]
fn targeted_c_launch_capacity_and_reverse_metadata() {
    use bennett_vm::launch::*;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read(
        repo.parent()
            .unwrap()
            .join("bennett-isa-spec/conformance/cases/objects/registers.bisa"),
    )
    .unwrap();
    let mut v = base();
    if let Json::Obj(m) = &mut v {
        m.insert(
            "classes".into(),
            j(r#"{"fast":{"capacity":2,"qubit_second":1,"dirty_rate":1}}"#),
        );
        if let Json::Arr(gs) = m.get_mut("gates").unwrap() {
            for op in ["x", "h"] {
                gs.push(j(&format!(r#"{{"op":"{op}","classes":["fast"],"controls":{{"min":0,"max":0}},"epsilon":0,"cost":{{"time":1}}}}"#)));
            }
        }
    }
    let target = v.to_jcs();
    unsafe {
        let mut p = std::ptr::null_mut();
        assert_eq!(
            bennett_load(
                raw.as_ptr(),
                raw.len(),
                target.as_ptr(),
                target.len(),
                &mut p
            ),
            0
        );
        let mut q = 0;
        assert_eq!(bennett_qreg_new(p, c"fast".as_ptr(), 2, &mut q), 0);
        let mut other = 999;
        assert_eq!(bennett_qreg_new(p, c"any".as_ptr(), 1, &mut other), 5);
        assert_eq!(other, 999);
        assert_eq!(bennett_qreg_free(p, q), 0);
        let mut args = std::ptr::null_mut();
        assert_eq!(bennett_args_new(0, &mut args), 0);
        let mut run = 0;
        assert_eq!(
            bennett_run(p, c"views".as_ptr(), args, std::ptr::null(), 0, &mut run),
            0
        );
        let mut result = std::ptr::null_mut();
        let mut request = std::ptr::null_mut();
        assert_eq!(bennett_wait(p, run, &mut result, &mut request), 0);
        assert_eq!(bennett_result_nfaults(result), 0);
        assert_eq!(bennett_reverse(p, run), 0);
        assert_eq!(bennett_run_close(p, run), 0);
        // §7.4 class/liveness metadata is classical history too; capacity restored.
        assert_eq!(bennett_qreg_new(p, c"fast".as_ptr(), 2, &mut q), 0);
        assert_eq!(bennett_qreg_free(p, q), 0);
        bennett_result_free(result);
        bennett_args_free(args);
        assert_eq!(bennett_unload(p), 0);
    }
}
