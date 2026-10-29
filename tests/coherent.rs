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
#[test]
fn derived_frame_layout_is_fixed_for_recursive_closure() {
    // §3.6: frame layout fixed per oracle; §3.7: W_cell=max(P,Sc,Did,W_max).
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/sum_db.bisa");
    let (obj, _) = bennett_vm::load(&std::fs::read_to_string(p).unwrap(), false).unwrap();
    let mut m = bennett_vm::step::Machine::new(
        &obj,
        "sum_db",
        &[BigUint::from_u64(1)],
        &[vec![BigUint::from_u64(1); 16]],
        &[],
        bennett_vm::value::Budget {
            steps: 64,
            heap: 4080,
            stack: 512,
            depth: 64,
        },
    )
    .unwrap();
    let mut lengths = std::collections::BTreeSet::new();
    for _ in 0..32 {
        m.forward_step().unwrap();
        lengths.insert(m.frames.last().unwrap().slots.len());
    }
    assert_eq!(
        lengths.len(),
        1,
        "every row, including HALT, has the same clean-padded slots"
    );
    assert!(m.widths_derived.cell >= 32);
}
#[test]
fn borrowed_controls_in_lists_are_not_hidden_from_proof() {
    // §5.6: borrowed wires touched as control or target only inside verified frames.
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/control_flow.bisa");
    let text = std::fs::read_to_string(p)
        .unwrap()
        .replace("cx x b\ncx b t\ncx x b\ncx b t", "mcx (b) t");
    let e = bennett_vm::load(&text, true).unwrap_err();
    assert!(e.contains("§5.6"), "{e}");
}
#[test]
fn every_reached_row_inverse_restores_complete_pre_step_tuple() {
    // §3.6/T2: Π_t⁻¹ restores every location/cell/counter/stack cell and frame,
    // not just the oracle's eventual payload. Exercise zero, one and all-one entries.
    use bennett_vm::{object::FnKind, value::Value};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_none_or(|e| e != "bisa") {
            continue;
        }
        let (obj, _) = bennett_vm::load(&std::fs::read_to_string(&p).unwrap(), false).unwrap();
        for f in obj.functions.iter().filter(|f| f.kind == FnKind::Oracle) {
            let sig = bennett_vm::validate::oracle_sig(&obj, &f.name).unwrap();
            let budget = bennett_vm::cliargs::default_budget(&obj, 128);
            for pattern in 0..3 {
                let val = |w: usize| match pattern {
                    0 => BigUint::zero(),
                    1 => BigUint::from_u64(1),
                    _ => BigUint::from_u64(1).shl(w).sub(&BigUint::from_u64(1)),
                };
                let inputs = sig.ins.iter().map(|w| val(*w)).collect::<Vec<_>>();
                let mem = sig
                    .mem
                    .iter()
                    .map(|(a, w, _)| vec![val(*w); 1usize << a])
                    .collect::<Vec<_>>();
                let args = sig
                    .args
                    .iter()
                    .map(|t| match t {
                        bennett_vm::object::Type::Token => Value::Token(budget),
                        bennett_vm::object::Type::F64 => Value::F64(0.5),
                        bennett_vm::object::Type::I(w) => Value::int(*w, val(*w)),
                        bennett_vm::object::Type::Ptr => Value::Ptr(val(obj.ptr_width)),
                        _ => panic!(),
                    })
                    .collect::<Vec<_>>();
                let mut m =
                    bennett_vm::step::Machine::new(&obj, &f.name, &inputs, &mem, &args, budget)
                        .unwrap();
                for n in 0..128 {
                    let pre = m.state.clone();
                    let frames = m.frames.clone();
                    let steps = m.steps;
                    m.forward_step()
                        .unwrap_or_else(|e| panic!("{} {} row {n}: {e}", p.display(), f.name));
                    m.inverse_step()
                        .unwrap_or_else(|e| panic!("{} {} inverse {n}: {e}", p.display(), f.name));
                    assert_eq!(m.state, pre, "{} {} prefix {n}", p.display(), f.name);
                    assert_eq!(m.frames, frames);
                    assert_eq!(m.steps, steps);
                    m.forward_step().unwrap();
                    checked += 1;
                }
            }
        }
    }
    assert!(checked >= 57 * 3 * 128);
    eprintln!("ROW_INVERSES {checked}");
}
#[test]
fn borrow_frames_are_proved_before_nonpermutation_composition() {
    // §5.6: frames are maximal contiguous permutation-only subcircuits.
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/control_flow.bisa");
    let text=std::fs::read_to_string(p).unwrap().replace("qif p {\nh t\n} {\nx t\n}","b = qborrow any 1\ncx p b\ncx b t\ncx p b\ncx b t\nh t\ncx p b\ncx b t\ncx p b\ncx b t\nqreturn b");
    bennett_vm::load(&text, true).unwrap();
    // A nonidentity frame cannot be excused by a second frame undoing it.
    let bad = text.replace(
        "cx p b\ncx b t\ncx p b\ncx b t\nh t\ncx p b\ncx b t\ncx p b\ncx b t",
        "cx b t\nh t\ncx b t",
    );
    assert!(bennett_vm::load(&bad, true).unwrap_err().contains("§5.6"));
}
