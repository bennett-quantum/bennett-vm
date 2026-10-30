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
            std::slice::from_ref(&ws),
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

fn control_object() -> bennett_vm::object::Object {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/control_flow.bisa");
    bennett_vm::load(&std::fs::read_to_string(p).unwrap(), false)
        .unwrap()
        .0
}
#[test]
fn native_qram_accounts_coherent_memory_without_a_private_comparator_library() {
    // §8.1 binds a,w,c in qram costs; §4.6 defines selected-cell XOR/exchange.
    // §8.2 charges each leaf once. Two native loads cost 2+8+0=10 each;
    // 2*popcount(0xdead)=22 image X plus 8 CX give total time 58.
    let mut doc = target("[]").document;
    if let Json::Obj(m) = &mut doc {
        m.insert(
            "classes".into(),
            j(r#"{"fast":{"capacity":64,"dirty_rate":1,"qubit_second":1}}"#),
        );
        m.insert("qram".into(), j(r#"{"load":{"cost":{"time":{"op":"+","args":[{"op":"+","args":[2,{"var":"w"}]},{"var":"c"}]}}},"store":{"cost":{"time":3}},"exch":{"cost":{"time":4}}}"#));
    }
    let t = Target::parse(&doc.to_jcs()).unwrap();
    let obj = control_object();
    for address in 0..2 {
        let mut s = QuantumState::new();
        s.set_target(t.clone()).unwrap();
        let w = s.target_alloc(9, "any").unwrap();
        s.prepare(&[w[8]], &BigUint::from_u64(address));
        action::execute_action_inner(
            &obj,
            &mut s,
            "image",
            &[w[..8].to_vec()],
            &[vec![w[8]]],
            &[],
            &[],
            false,
            &[],
        )
        .unwrap();
        assert_eq!(
            s.constant_value(&w[..8]),
            Some(BigUint::from_u64(if address == 0 { 0xde } else { 0xad }))
        );
        let a = s.target_accounting().unwrap();
        assert_eq!(a.get("time"), Some(&Json::Str("58".into())));
        assert_eq!(
            a.get("peak_qubits").unwrap().get("fast"),
            Some(&Json::Str("33".into()))
        );
        // 25 caller+region wires live throughout 58; 8 load wires for 36.
        assert_eq!(
            a.get("qubit_second").unwrap().get("fast"),
            Some(&Json::Str("1738".into()))
        );
    }
    // Exact memory semantics on an address superposition, preserving phase.
    let mut s = QuantumState::new();
    s.set_target(t).unwrap();
    let w = s.target_alloc(9, "any").unwrap();
    s.set_state(BTreeMap::from([
        (BigUint::from_u64(0), (0.5, 0.0)),
        (BigUint::from_u64(256), (0.0, 0.5)),
    ]));
    let mut expected = s.clone();
    expected.set_state(BTreeMap::from([
        (BigUint::from_u64(0xde), (0.5, 0.0)),
        (BigUint::from_u64(0x1ad), (0.0, 0.5)),
    ]));
    action::execute_action_inner(
        &obj,
        &mut s,
        "image",
        &[w[..8].to_vec()],
        &[vec![w[8]]],
        &[],
        &[],
        false,
        &[],
    )
    .unwrap();
    assert!(s.exact_equals(&expected));
}
#[test]
fn zero_duration_allocations_still_contribute_to_peak() {
    // §8.4: capacity is peak live qubits, even if no positive-time gate occurs.
    let mut s = QuantumState::new();
    s.set_target(target("[]")).unwrap();
    s.target_alloc(3, "any").unwrap();
    let a = s.target_accounting().unwrap();
    assert_eq!(
        a.get("peak_qubits").unwrap().get("fast"),
        Some(&Json::Str("3".into()))
    );
    assert_eq!(a.get("time"), Some(&Json::Str("0".into())));
}
#[test]
fn explicit_borrow_frame_charges_dirty_ownership_for_its_duration() {
    // §8.2: borrowed wires use dirty_rate, replacing the owner's charge.
    // Four CX events take 8 units: two interface wires *8 + borrowed *8/4 =18.
    let mut s = QuantumState::new();
    s.set_target(target("[]")).unwrap();
    let w = s.target_alloc(3, "any").unwrap();
    action::execute_action_with_idle(
        &control_object(),
        &mut s,
        "borrowed",
        &[vec![w[0]]],
        &[vec![w[1]]],
        &[],
        &[],
        false,
        &[],
        &[w[2]],
    )
    .unwrap();
    assert_eq!(
        s.target_accounting()
            .unwrap()
            .get("qubit_second")
            .unwrap()
            .get("fast"),
        Some(&Json::Str("18".into()))
    );
}
#[test]
fn direct_target_actions_refuse_unpublished_row_expansion() {
    // §11: below-gate comparators/scratch belong to the versioned library;
    // calling the Rust API directly must not silently bypass target lowering.
    let mut s = QuantumState::new();
    s.set_target(target("[]")).unwrap();
    let w = s.target_alloc(9, "any").unwrap();
    let e = action::execute_action_inner(
        &control_object(),
        &mut s,
        "image",
        &[w[..8].to_vec()],
        &[vec![w[8]]],
        &[],
        &[],
        false,
        &[],
    )
    .unwrap_err();
    assert!(
        e.contains("unimplemented:") && e.contains("milestone 4") && e.contains("47t"),
        "{e}"
    );
}

#[path = "support/device.rs"]
mod device_fixture;
#[test]
fn classical_and_effect_leaves_are_accounted_once_including_inverse_execution() {
    // §8.2: "Classical instructions and effects take classical_step_time".
    // The entry has eight rows, including one effect; reversal executes eight
    // inverse rows, so makespan is 8 forward and 16 forward+reverse.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let op = repo.join(".tmp/account-device.bisa");
    let tp = repo.join(".tmp/account-device-target.json");
    std::fs::write(&op, device_fixture::block_object(true)).unwrap();
    let mut doc = target("[]").document;
    if let Json::Obj(m) = &mut doc {
        m.insert("ptr_widths_supported".into(), j("[16,32,64]"));
    }
    std::fs::write(&tp, doc.to_jcs()).unwrap();
    for (command, time) in [("run-cc", "8"), ("reverse", "16")] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
            .arg(command)
            .arg(&op)
            .arg("effects")
            .arg("--target")
            .arg(&tp)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v = j(String::from_utf8(out.stdout).unwrap().trim());
        assert_eq!(
            v.get("target_resources").and_then(|a| a.get("time")),
            Some(&Json::Str(time.into()))
        );
    }
}

#[test]
fn action_accuracy_requests_are_consumed_in_generated_order_and_permutations_stay_exact() {
    use action::GateRequest;
    // §8.1 distinguishes requested shares and achieved errors; §4.4 requires
    // every permutation gate (transitively) to use epsilon zero.
    let mut doc = target("[]").document;
    if let Json::Obj(m) = &mut doc {
        m.insert("gates".into(),j(r#"[{"op":"x","classes":["*"],"controls":{"min":0,"max":0},"epsilon":0.001,"cost":{"time":2}},{"op":"x","classes":["*"],"controls":{"min":0,"max":0},"epsilon":0,"cost":{"time":7}}]"#));
    }
    let mut s = QuantumState::new();
    s.set_target(Target::parse(&doc.to_jcs()).unwrap()).unwrap();
    let w = s.target_alloc(2, "any").unwrap();
    let obj = object("x t[0]\nx t[1]");
    let requests = vec![
        GateRequest {
            epsilon_share: r("0.01"),
            realisation: Some("native".into()),
        },
        GateRequest {
            epsilon_share: r("0"),
            realisation: None,
        },
    ];
    action::execute_action_with_requests(
        &obj,
        &mut s,
        "bell",
        std::slice::from_ref(&w),
        &[],
        &[],
        &[],
        false,
        &[],
        &[],
        &requests,
    )
    .unwrap();
    assert_eq!(s.constant_value(&w), Some(BigUint::from_u64(3)));
    assert_eq!(s.target_error(), r("0.001"));
    assert_eq!(
        s.target_accounting().unwrap().get("time"),
        Some(&Json::Str("9".into()))
    );
    // Missing or unused requests reject atomically, not a silent default.
    for requests in [
        &requests[..1],
        &[requests.clone(), vec![requests[0].clone()]].concat()[..],
    ] {
        let before = s.clone();
        let n = s.emitted.len();
        assert!(
            action::execute_action_with_requests(
                &obj,
                &mut s,
                "bell",
                std::slice::from_ref(&w),
                &[],
                &[],
                &[],
                false,
                &[],
                &[],
                requests
            )
            .is_err()
        );
        assert!(s.exact_equals(&before));
        assert_eq!(s.emitted.len(), n);
    }
    let mut s = QuantumState::new();
    s.set_target(target("[]")).unwrap();
    let w = s.target_alloc(4, "any").unwrap();
    let requests = vec![
        GateRequest {
            epsilon_share: r("0.1"),
            realisation: None
        };
        9
    ];
    let e = action::execute_action_with_requests(
        &control_object(),
        &mut s,
        "while3",
        &[vec![w[0]]],
        &[w[1..].to_vec()],
        &[],
        &[],
        false,
        &[],
        &[],
        &requests,
    )
    .unwrap_err();
    assert!(e.contains("epsilon zero"), "{e}");
    assert!(s.emitted.is_empty());
}

#[test]
fn adjoint_allocations_retain_the_declared_class() {
    // §5.3: qalloc <-> qfree; the inverse retains the original allocation's
    // class. §8.4 capacity is per class, so replacing slow by any is observable.
    let obj = object("b = qalloc slow 1\nx b\nx b\nqfree b");
    let mut doc = target("[]").document;
    if let Json::Obj(m) = &mut doc {
        m.insert("classes".into(),j(r#"{"fast":{"capacity":4,"dirty_rate":1,"qubit_second":1},"slow":{"capacity":1,"dirty_rate":1,"qubit_second":1}}"#));
    }
    for adj in [false, true] {
        let mut s = QuantumState::new();
        s.set_target(Target::parse(&doc.to_jcs()).unwrap()).unwrap();
        let w = s.target_alloc(2, "fast").unwrap();
        action::execute_action_inner(
            &obj,
            &mut s,
            "bell",
            std::slice::from_ref(&w),
            &[],
            &[],
            &[],
            adj,
            &[],
        )
        .unwrap();
        assert_eq!(
            s.target_accounting()
                .unwrap()
                .get("peak_qubits")
                .unwrap()
                .get("slow"),
            Some(&Json::Str("1".into()))
        );
    }
}

#[test]
fn native_qram_store_exchange_and_load_obey_address_and_inherited_control() {
    // §4.6: for each selected cell, XOR or exchange under a==i. §3.5
    // additionally predicates every operation. Exercise all two-cell contents,
    // both addresses, all target values and both predicate values.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let raw = std::fs::read_to_string(
        root.join("bennett-isa-spec/reference-interpreter/tests/fixtures/control_flow.bisa"),
    )
    .unwrap();
    let mut doc = target("[]").document;
    if let Json::Obj(m) = &mut doc {
        m.insert("qram".into(),j(r#"{"load":{"cost":{"time":2}},"store":{"cost":{"time":3}},"exch":{"cost":{"time":4}}}"#));
    }
    let t = Target::parse(&doc.to_jcs()).unwrap();
    for (body, mode, duration) in [
        ("qstore m a t", "store", "3"),
        ("qexchm m a t", "exch", "4"),
        ("d = qload m a\nqxor d t\nqunload m a d", "load", "8"),
    ] {
        let obj = bennett_vm::load(&raw.replace("qexchm m a t", body), true)
            .unwrap()
            .0;
        for memory in 0..16 {
            for address in 0..2 {
                for input in 0..4 {
                    for pred in 0..2 {
                        let mut s = QuantumState::new();
                        s.set_target(t.clone()).unwrap();
                        let w = s.target_alloc(8, "any").unwrap();
                        let initial = input | (address << 2) | (memory << 3) | (pred << 7);
                        s.prepare(&w, &BigUint::from_u64(initial));
                        let region = action::RegionWires {
                            a: 1,
                            w: 2,
                            cells: vec![w[3..5].to_vec(), w[5..7].to_vec()],
                        };
                        action::execute_action_inner(
                            &obj,
                            &mut s,
                            "exchange",
                            &[w[..2].to_vec()],
                            &[vec![w[2]]],
                            &[region],
                            &[],
                            false,
                            &[w[7]],
                        )
                        .unwrap();
                        let cell = (memory >> (2 * address)) & 3;
                        let (output, mem) = if pred == 0 {
                            (input, memory)
                        } else {
                            match mode {
                                "load" => (input ^ cell, memory),
                                "store" => (input, memory ^ (input << (2 * address))),
                                _ => (
                                    cell,
                                    (memory & !(3 << (2 * address))) | (input << (2 * address)),
                                ),
                            }
                        };
                        assert_eq!(
                            s.constant_value(&w),
                            Some(BigUint::from_u64(
                                output | (address << 2) | (mem << 3) | (pred << 7)
                            ))
                        );
                        assert_eq!(
                            s.target_accounting().unwrap().get("time"),
                            Some(&Json::Str(duration.into()))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn predicate_depth_and_template_operand_aliases_fail_before_quantum_changes() {
    // §8.3 capabilities limit effective predicates; §3.3 every control wire
    // is distinct from every target, including gates generated by a scheme.
    let mut doc = target("[]").document;
    if let Json::Obj(m) = &mut doc
        && let Json::Obj(c) = m.get_mut("capabilities").unwrap()
    {
        c.insert("max_predicate_depth".into(), j("1"));
    }
    let mut s = QuantumState::new();
    s.set_target(Target::parse(&doc.to_jcs()).unwrap()).unwrap();
    let w = s.target_alloc(3, "any").unwrap();
    let mut g = x(w[0]);
    g.controls = vec![w[1], w[2]];
    g.classes = vec!["fast".into(); 3];
    assert!(
        s.emit_target_gate(g)
            .unwrap_err()
            .contains("predicate depth")
    );
    assert!(s.emitted.is_empty());
    let t = target(
        r#"[{"name":"alias","realises":{"op":"x","controls":{"min":0,"max":0}},"ancillae":{"count":0,"state":"clean","class":"fast"},"template":[{"gate":"cx","wires":["t","t"]}]}]"#,
    );
    assert!(
        t.lower(&x(0), &r("0"), Some("alias"))
            .unwrap_err()
            .contains("disjoint")
    );
}

#[test]
fn launch_snapshot_exposes_actual_accounting_and_keeps_old_snapshots_immutable() {
    // §10 snapshots are a VM extension; §8.2 dictates the five classical steps
    // and wire liveness (occupancy 1+2+2=5, peak 2) in @allocation.
    use bennett_vm::launch::*;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let raw = std::fs::read(
        root.join("bennett-isa-spec/reference-interpreter/tests/fixtures/registers.bisa"),
    )
    .unwrap();
    let mut doc = target("[]").document;
    if let Json::Obj(m) = &mut doc {
        m.insert("gates".into(),j(r#"[{"op":"x","classes":["*"],"controls":{"min":0,"max":"unbounded"},"epsilon":0,"cost":{"time":1}},{"op":"h","classes":["*"],"controls":{"min":0,"max":"unbounded"},"epsilon":0,"cost":{"time":1}},{"op":"cx","classes":["*"],"controls":{"min":1,"max":"unbounded"},"epsilon":0,"cost":{"time":2}}]"#));
    }
    let target = doc.to_jcs();
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
        let mut a = std::ptr::null_mut();
        assert_eq!(bennett_args_new(0, &mut a), 0);
        let mut run = 0;
        assert_eq!(
            bennett_run(p, c"allocation".as_ptr(), a, std::ptr::null(), 0, &mut run),
            0
        );
        bennett_args_free(a);
        let (mut result, mut request) = (std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(bennett_wait(p, run, &mut result, &mut request), 0);
        assert!(request.is_null());
        let mut snapshot = std::ptr::null_mut();
        assert_eq!(bennett_run_snapshot(p, run, &mut snapshot), 0);
        let mut len = 0;
        let bytes = bennett_snapshot_jcs(snapshot, &mut len);
        let saved = std::slice::from_raw_parts(bytes, len).to_vec();
        let v = j(std::str::from_utf8(&saved).unwrap());
        let resources = v.get("target_resources").unwrap();
        assert_eq!(resources.get("time"), Some(&Json::Str("5".into())));
        assert_eq!(
            resources.get("qubit_second").unwrap().get("fast"),
            Some(&Json::Str("5".into()))
        );
        assert_eq!(
            resources.get("peak_qubits").unwrap().get("fast"),
            Some(&Json::Str("2".into()))
        );
        assert_eq!(bennett_reverse(p, run), 0);
        assert_eq!(std::slice::from_raw_parts(bytes, len), &saved);
        let mut reverse = std::ptr::null_mut();
        assert_eq!(bennett_run_snapshot(p, run, &mut reverse), 0);
        let mut rlen = 0;
        let rbytes = bennett_snapshot_jcs(reverse, &mut rlen);
        let v = j(std::str::from_utf8(std::slice::from_raw_parts(rbytes, rlen)).unwrap());
        assert_eq!(
            v.get("target_resources").unwrap().get("time"),
            Some(&Json::Str("10".into()))
        );
        bennett_snapshot_free(reverse);
        bennett_snapshot_free(snapshot);
        bennett_result_free(result);
        assert_eq!(bennett_run_close(p, run), 0);
        assert_eq!(bennett_unload(p), 0);
    }
}
