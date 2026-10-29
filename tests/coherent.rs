use bennett_vm::{action::QuantumState, bigint::BigUint};
#[test]
fn hadamard_round_trip_has_exact_support() {
    // §4.4 H²=I; §5.1 qfree must be clean on every configuration.
    let mut s = QuantumState::new();
    let w = s.alloc_wire();
    let h = std::f64::consts::FRAC_1_SQRT_2;
    for _ in 0..2 {
        s.apply1(w, &[], [[(h, 0.), (h, 0.)], [(h, 0.), (-h, 0.)]]);
    }
    assert!(s.is_clean(&[w]));
    assert_eq!(s.amps.get(&BigUint::zero()), Some(&(1., 0.)));
}
#[test]
fn standalone_action_is_served() {
    // §14.2: Bell from |00> = (|00>+|11>)/√2, phase included.
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/registers.bisa");
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
        .args(["run-action", p.to_str().unwrap(), "bell", "--mut", "0"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
#[test]
fn literal_step_prefix_is_served() {
    // §3.6: after the inverse step every frame field is clean.
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/inc8.bisa");
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
        .args([
            "reverse",
            p.to_str().unwrap(),
            "inc8",
            "--input",
            "8",
            "--budget",
            "3",
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
#[test]
fn designated_borrow_preserves_entangled_reference() {
    // §5.6: declared operation ⊗ identity on any external system entangled with B.
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/control_flow.bisa");
    let (obj, _) = bennett_vm::load(&std::fs::read_to_string(p).unwrap(), false).unwrap();
    let mut s = QuantumState::new();
    let ws = s.alloc_wires(4); // target, input, borrowed, reference
    let h = std::f64::consts::FRAC_1_SQRT_2;
    s.apply1(ws[2], &[], [[(h, 0.), (h, 0.)], [(h, 0.), (-h, 0.)]]);
    s.flip(ws[3], &[ws[2]]);
    s.flip(ws[1], &[]);
    let mut expected = s.clone();
    expected.flip(ws[0], &[ws[1]]);
    bennett_vm::action::execute_action_with_idle(
        &obj,
        &mut s,
        "borrowed",
        &[vec![ws[0]]],
        &[vec![ws[1]]],
        &[],
        &[],
        false,
        &[],
        &[ws[2]],
    )
    .unwrap();
    assert!(s.exact_equals(&expected));
}
#[test]
fn rotation_inverse_clears_workspace_exactly() {
    // §5.3 U†U=I, with no approximate-zero test at release.
    use bennett_vm::exact::Exact;
    let mut s = QuantumState::new();
    let w = s.alloc_wire();
    for axis in ["rx", "ry", "rz"] {
        s.apply_exact(w, &[], Exact::rotation(axis, 0.123));
        s.apply_exact(w, &[], Exact::rotation(axis, -0.123));
    }
    assert!(s.is_clean(&[w]));
    assert_eq!(s.amps.get(&BigUint::zero()), Some(&(1., 0.)));
}
#[test]
fn exhaustive_borrow_proof_accepts_identity_without_lemma_shape() {
    // §5.6 permits a T3 proof over the full domain, alternatively to the lemma.
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/control_flow.bisa");
    let text = std::fs::read_to_string(p).unwrap();
    let text = text.replace("cx x b\ncx b t\ncx x b\ncx b t", "cx b t\ncx b t");
    bennett_vm::load(&text, true).unwrap();
}
