//! Hand-derived execution witnesses, independent of the §11 ref expansion library.
use bennett_vm::{
    action::{self, QuantumState},
    bigint::BigUint,
    json::{self, Json},
    target::{Gate, Rational as R, Target},
};
use std::{collections::BTreeMap, path::Path};
fn r(s: &str) -> R {
    s.parse().unwrap()
}
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
fn target(schemes: &str) -> Target {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/cases/E/E-matching-first-entry/target.json");
    let mut v = j(std::fs::read_to_string(path).unwrap().trim_end());
    if let Json::Obj(m) = &mut v {
        m.insert("gates".into(),j(r#"[{"op":"x","classes":["*"],"controls":{"min":0,"max":"unbounded"},"epsilon":0,"cost":{"time":1}},{"op":"cx","classes":["*"],"controls":{"min":1,"max":"unbounded"},"epsilon":0,"cost":{"time":2}},{"op":"ccx","classes":["*"],"controls":{"min":2,"max":"unbounded"},"epsilon":0,"cost":{"time":3}},{"op":"swap","classes":["*"],"controls":{"min":0,"max":"unbounded"},"epsilon":0,"cost":{"time":4}}]"#));
        m.insert("schemes".into(), j(schemes));
        m.insert(
            "classes".into(),
            j(r#"{"fast":{"capacity":16,"dirty_rate":0.25,"qubit_second":1}}"#),
        );
    }
    Target::parse(&v.to_jcs()).unwrap()
}
fn x(w: usize) -> Gate {
    Gate {
        op: "x".into(),
        controls: vec![],
        own_controls: 0,
        targets: vec![w],
        classes: vec!["fast".into()],
        theta: None,
    }
}
fn object(body: &str) -> bennett_vm::object::Object {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/registers.bisa");
    let raw = std::fs::read_to_string(p).unwrap();
    let raw = raw.replace("h t[0]\ncx t[0] t[1]", body);
    let raw = if body.contains("qloop 2") {
        raw.replace(
            "action @bell (t:wire[2] mut) () () () ()",
            "action @bell (t:wire[2] mut) () () () ()\nregs {  } bounds {2}",
        )
        .replace(
            "\"bounds\":[],\"in\":[],\"kind\":\"action\"",
            "\"bounds\":[2],\"in\":[],\"kind\":\"action\"",
        )
    } else {
        raw
    };
    bennett_vm::load(&raw, true).unwrap().0
}
#[test]
fn structured_bundle_and_branch_gates_are_native_events() {
    // §4.5: copy/XOR expand to cx; exchange expands to swap. §4.6 gives
    // qif's two unconditional-in-the-current-context X toggles, even for empty arms.
    let obj = object(
        "d = qcopy t\nqxor d t\nqxor d t\nqexch d t\nquncopy t d\nqif t[0] {\nx t[1]\n} {\nx t[1]\n}\nqloop 2 {\nx t[1]\n}",
    );
    let target = target("[]");
    target.validate_object(&obj).unwrap();
    for bits in 0..4 {
        let mut s = QuantumState::new();
        s.set_target(target.clone()).unwrap();
        let ws = s.target_alloc(2, "any").unwrap();
        s.prepare(&ws, &BigUint::from_u64(bits));
        action::execute_action_inner(
            &obj,
            &mut s,
            "bell",
            &[ws.clone()],
            &[],
            &[],
            &[],
            false,
            &[],
        )
        .unwrap();
        assert_eq!(s.constant_value(&ws), Some(BigUint::from_u64(bits ^ 2)));
        assert_eq!(s.emitted.len(), 16); // 8 CX, 2 swap, 4 branch X, 2 loop X
        assert!(
            s.emitted
                .iter()
                .all(|l| ["cx", "swap", "x"].contains(&l.gate.op.as_str()))
        );
        let a = s.target_accounting().unwrap();
        assert_eq!(a.get("time"), Some(&Json::Str("30".into()))); // 8*2 + 2*4 + 6*1
        assert_eq!(
            a.get("peak_qubits").unwrap().get("fast"),
            Some(&Json::Str("4".into()))
        );
        // Copy is live for 24 units; caller's two wires for all 30.
        assert_eq!(
            a.get("qubit_second").unwrap().get("fast"),
            Some(&Json::Str("108".into()))
        );
    }
}
#[test]
fn designated_dirty_scheme_preserves_external_entanglement_and_replaces_charge() {
    // §8.3 dirty ancillae; §9.4 designated caller idle set. §8.2: borrowed
    // charge at dirty_rate replaces owner's charge, rather than adding a wire.
    let target = target(
        r#"[{"name":"dirty-x","realises":{"op":"x","controls":{"min":0,"max":0}},"ancillae":{"count":1,"state":"dirty","class":"fast"},"template":[{"gate":"cx","wires":["a[0]","t"]},{"gate":"x","wires":["t"]},{"gate":"cx","wires":["a[0]","t"]}]}]"#,
    );
    let mut s = QuantumState::new();
    s.set_target(target).unwrap();
    let ws = s.target_alloc(3, "any").unwrap(); // target, borrow, external reference
    s.set_state(BTreeMap::from([
        (BigUint::from_u64(0), (0.5, 0.0)),
        (BigUint::from_u64(6), (0.0, 0.5)),
    ]));
    let mut expected = s.clone();
    expected.flip(ws[0], &[]);
    let before = s.clone();
    assert!(
        s.emit_target_gate_with(x(ws[0]), &r("0"), Some("dirty-x"), &[])
            .unwrap_err()
            .contains("designated")
    );
    assert!(s.exact_equals(&before));
    assert!(s.emitted.is_empty());
    s.emit_target_gate_with(x(ws[0]), &r("0"), Some("dirty-x"), &[ws[1]])
        .unwrap();
    assert!(s.exact_equals(&expected));
    let a = s.target_accounting().unwrap();
    assert_eq!(a.get("time"), Some(&Json::Str("5".into())));
    assert_eq!(
        a.get("peak_qubits").unwrap().get("fast"),
        Some(&Json::Str("3".into()))
    );
    assert_eq!(
        a.get("qubit_second").unwrap().get("fast"),
        Some(&Json::Str("11.25".into()))
    );
}
#[test]
fn requested_accuracy_executes_selected_native_leaf_and_charges_achieved_error() {
    // §8.1: entry epsilon <= requested share; charge achieved epsilon.
    let mut t = target("[]").document;
    if let Json::Obj(m) = &mut t {
        m.insert("gates".into(),j(r#"[{"op":"x","classes":["*"],"controls":{"min":0,"max":0},"epsilon":0.02,"cost":{"time":7}},{"op":"x","classes":["*"],"controls":{"min":0,"max":0},"epsilon":0.001,"cost":{"time":2}}]"#));
    }
    let mut s = QuantumState::new();
    s.set_target(Target::parse(&t.to_jcs()).unwrap()).unwrap();
    let w = s.target_alloc(1, "any").unwrap()[0];
    s.emit_target_gate_with(x(w), &r("0.01"), None, &[])
        .unwrap();
    assert_eq!(s.constant_value(&[w]), Some(BigUint::from_u64(1)));
    assert_eq!(s.emitted[0].epsilon, r("0.001"));
    assert_eq!(s.target_error(), r("0.001"));
    assert_eq!(
        s.target_accounting().unwrap().get("time"),
        Some(&Json::Str("2".into()))
    );
    assert!(
        s.emit_target_gate(x(w))
            .unwrap_err()
            .contains("unsupported")
    );
    assert_eq!(s.emitted.len(), 1);
}

#[test]
fn while_flags_follow_listed_order_and_release_capacity() {
    // §4.6: "after iteration i < n: ccx ..."; "flags uncomputed in reverse and freed".
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/control_flow.bisa");
    let obj = bennett_vm::load(&std::fs::read_to_string(p).unwrap(), false)
        .unwrap()
        .0;
    for bits in 0..8 {
        let mut s = QuantumState::new();
        s.set_target(target("[]")).unwrap();
        let ws = s.target_alloc(4, "any").unwrap();
        s.prepare(&ws[1..], &BigUint::from_u64(bits));
        let before = s.clone();
        action::execute_action_inner(
            &obj,
            &mut s,
            "while3",
            &[vec![ws[0]]],
            &[ws[1..].to_vec()],
            &[],
            &[],
            false,
            &[],
        )
        .unwrap();
        let iterations = (0..3).take_while(|i| bits & (1 << i) != 0).count();
        assert_eq!(
            s.constant_value(&[ws[0]]),
            Some(BigUint::from_u64((iterations % 2) as u64))
        );
        assert_eq!(
            s.emitted
                .iter()
                .map(|l| l.gate.op.as_str())
                .collect::<Vec<_>>(),
            vec!["cx", "x", "ccx", "x", "ccx", "x", "ccx", "ccx", "cx"]
        );
        // §5.3 adjoint table: "flags computed forward ... adj A for i = n..1 ... flags uncomputed".
        // This fixes the order, even though other gate orders can give the same unitary.
        action::execute_action_inner(
            &obj,
            &mut s,
            "while3",
            &[vec![ws[0]]],
            &[ws[1..].to_vec()],
            &[],
            &[],
            true,
            &[],
        )
        .unwrap();
        assert!(s.exact_equals(&before));
        assert_eq!(
            s.emitted[9..]
                .iter()
                .map(|l| l.gate.op.as_str())
                .collect::<Vec<_>>(),
            vec!["cx", "ccx", "ccx", "x", "x", "x", "ccx", "ccx", "cx"]
        );
        assert_eq!(s.target_alloc(12, "fast").unwrap().len(), 12);
    }
}
#[test]
fn caller_idle_pool_must_be_disjoint_and_unique() {
    // §9.4: caller's "designated idle set" cannot contain an active interface wire.
    let obj = object("x t[0]");
    let mut s = QuantumState::new();
    s.set_target(target("[]")).unwrap();
    let ws = s.target_alloc(3, "any").unwrap();
    for idle in [vec![ws[0]], vec![ws[2], ws[2]]] {
        let before = s.clone();
        assert!(
            action::execute_action_with_idle(
                &obj,
                &mut s,
                "bell",
                &[ws[..2].to_vec()],
                &[],
                &[],
                &[],
                false,
                &[],
                &idle
            )
            .unwrap_err()
            .contains("idle")
        );
        assert!(s.exact_equals(&before));
        assert!(s.emitted.is_empty());
    }
}
#[test]
fn local_image_creation_and_cleanup_are_unconditional_native_gates() {
    // §4.6: "unconditional allocation, X gates from the data image";
    // "image gates reversed, cells released clean" even with a false inherited predicate.
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/control_flow.bisa");
    let raw = std::fs::read_to_string(p)
        .unwrap()
        .replace("d = qload scratch p\nqxor d t\nqunload scratch p d\n", "");
    let obj = bennett_vm::load(&raw, true).unwrap().0;
    let mut t = target("[]").document;
    if let Json::Obj(m) = &mut t {
        m.insert(
            "classes".into(),
            j(r#"{"fast":{"capacity":32,"dirty_rate":1,"qubit_second":1}}"#),
        );
    }
    let mut s = QuantumState::new();
    s.set_target(Target::parse(&t.to_jcs()).unwrap()).unwrap();
    let ws = s.target_alloc(10, "any").unwrap();
    let before = s.clone();
    action::execute_action_inner(
        &obj,
        &mut s,
        "image",
        &[ws[..8].to_vec()],
        &[vec![ws[8]]],
        &[],
        &[],
        false,
        &[ws[9]],
    )
    .unwrap();
    assert!(s.exact_equals(&before));
    let ones = 0xdead_u32.count_ones() as usize;
    assert_eq!(s.emitted.len(), 2 * ones);
    assert!(s.emitted.iter().all(|l| l.gate.controls.is_empty()));
    assert_eq!(s.target_alloc(22, "fast").unwrap().len(), 22);
}
