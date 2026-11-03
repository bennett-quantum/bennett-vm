//! Discrete decisions of the dense engine against the exact interpreter
//! (beads bennett-vm-yt9, bennett-vm-ykb). Ported from the 2026-11-02
//! design review probes (docs/reviews/2026-11-02-design-review.md); each
//! test calls the same public `QuantumState` operations on both engines.
//! The exact interpreter is the oracle. A dense decision that its
//! certificates cannot settle must fail loudly, never guess.
use bennett_vm::action::QuantumState;
use bennett_vm::bigint::BigUint;
use bennett_vm::json::Json;

type Amp = (f64, f64);
const H: f64 = std::f64::consts::FRAC_1_SQRT_2;

fn h() -> [[Amp; 2]; 2] {
    [[(H, 0.), (H, 0.)], [(H, 0.), (-H, 0.)]]
}
fn t_gate() -> [[Amp; 2]; 2] {
    [[(1., 0.), (0., 0.)], [(0., 0.), (H, H)]]
}
fn tdg_gate() -> [[Amp; 2]; 2] {
    [[(1., 0.), (0., 0.)], [(0., 0.), (H, -H)]]
}
fn ry(t: f64) -> [[Amp; 2]; 2] {
    let (s, c) = (t / 2.).sin_cos();
    [[(c, 0.), (-s, 0.)], [(s, 0.), (c, 0.)]]
}
fn bound(s: &QuantumState) -> f64 {
    let e = s.simulation_evidence().unwrap();
    match e.get("error_bound").unwrap() {
        Json::Num(v) => v.parse().unwrap(),
        _ => panic!("error_bound is not a number"),
    }
}
fn engines() -> [QuantumState; 2] {
    [QuantumState::new(), QuantumState::new_dense()]
}

fn assert_undecidable(error: &str) {
    assert!(error.contains("undecidable"), "{error}");
    assert!(error.contains("bennett-vm-k8w"), "{error}");
    assert!(error.contains("exact interpreter"), "{error}");
}

#[test]
fn equality_certifies_actual_inverse_with_nonzero_bound() {
    for mut s in engines() {
        let w = s.alloc_wires(3);
        s.apply1(w[2], &[], h());
        let entry = s.clone();
        s.apply1(w[0], &[], h());
        s.apply1(w[1], &[w[0]], h());
        s.apply1(w[0], &[], t_gate());
        s.apply1(w[0], &[], tdg_gate());
        s.apply1(w[1], &[w[0]], h());
        s.apply1(w[0], &[], h());
        if s.simulation_evidence().is_some() {
            assert!(bound(&s) > 0.0);
        }
        assert_eq!(s.numerically_equals(&entry), Ok(true));
        assert_eq!(entry.numerically_equals(&s), Ok(true));
    }
}

#[test]
fn symbolic_rotation_reversal_keeps_the_exact_interpreter_capability() {
    for mut s in engines() {
        let w = s.alloc_wires(2);
        s.apply1(w[1], &[], h());
        let entry = s.clone();
        for theta in [0.3, -0.3] {
            s.apply_exact(w[0], &[], bennett_vm::exact::Exact::rotation("ry", theta));
        }
        if s.simulation_evidence().is_some() {
            assert_undecidable(&s.numerically_equals(&entry).unwrap_err());
            assert_undecidable(&entry.numerically_equals(&s).unwrap_err());
        } else {
            assert_eq!(s.numerically_equals(&entry), Ok(true));
            assert!(s.is_clean(&[w[0]]).unwrap());
        }
        // A clone still has an exact shared-history equality certificate,
        // even when the history contains operands that cannot be replayed.
        assert_eq!(s.numerically_equals(&s.clone()), Ok(true));
    }
}

#[test]
fn unrecorded_remap_reversal_keeps_the_exact_interpreter_capability() {
    for mut s in engines() {
        // GHZ has two exact amplitudes but thirteen packed coordinates,
        // exceeding the dense remap recording limit (4096 keys).
        let w = s.alloc_wires(13);
        s.apply1(w[0], &[], h());
        for &target in &w[1..] {
            s.flip(target, &[w[0]]);
        }
        let entry = s.clone();
        for _ in 0..2 {
            s.map_basis(|key| {
                let mut out = key.clone();
                out.set_bit(w[12], !key.bit(w[12]));
                out
            });
        }
        if s.simulation_evidence().is_some() {
            assert_undecidable(&s.numerically_equals(&entry).unwrap_err());
            assert_undecidable(&entry.numerically_equals(&s).unwrap_err());
        } else {
            assert_eq!(s.numerically_equals(&entry), Ok(true));
        }
    }
}

#[test]
fn mixed_dense_exact_equality_returns_a_diagnostic_in_both_directions() {
    let mut exact = QuantumState::new();
    let mut dense = QuantumState::new_dense();
    exact.alloc_wires(1);
    dense.alloc_wires(1);
    exact.apply1(0, &[], h());
    // The same diagnostic snapshot does not make the exact algebraic
    // amplitude sqrt(1/2) equal to its binary64 projection.
    dense.set_state(exact.amplitudes());
    for error in [
        exact.numerically_equals(&dense).unwrap_err(),
        dense.numerically_equals(&exact).unwrap_err(),
    ] {
        assert_undecidable(&error);
        assert!(error.contains("mixed dense/exact"), "{error}");
    }
}

fn decision_object() -> String {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../bennett-isa-spec/reference-interpreter/tests/fixtures/registers.bisa");
    let raw = std::fs::read_to_string(fixture).unwrap();
    assert!(raw.contains("h t[0]\ncx t[0] t[1]"));
    rehash_object(raw.replace("h t[0]\ncx t[0] t[1]", "ry 0.3 t[0]\nry -0.3 t[0]"))
}

fn rehash_object(mut raw: String) -> String {
    let object = bennett_vm::object::parse(&raw).unwrap();
    let ids = bennett_vm::identity::compute(&object).unwrap();
    for symbol in object.symbols.values() {
        raw = raw.replace(&symbol.stored_hash, &ids.interface[&symbol.name]);
    }
    raw
}

#[test]
fn cli_reports_undecidable_decisions_and_interpreter_still_reverses() {
    let path = std::env::temp_dir().join(format!("dense-decisions-{}.bisa", std::process::id()));
    std::fs::write(&path, decision_object()).unwrap();
    for reverse in [false, true] {
        let run = |dense: bool| {
            let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_bennett-vm"));
            command.args([
                if reverse { "reverse" } else { "run-action" },
                path.to_str().unwrap(),
                "bell",
                "--mut",
                "0",
            ]);
            if dense {
                command.env("BENNETT_VM_ENGINE", "dense");
            } else {
                command.env_remove("BENNETT_VM_ENGINE");
            }
            command.output().unwrap()
        };
        let exact = run(false);
        assert!(
            exact.status.success(),
            "{}",
            String::from_utf8_lossy(&exact.stderr)
        );
        let report = bennett_vm::json::parse(std::str::from_utf8(&exact.stdout).unwrap()).unwrap();
        if reverse {
            assert_eq!(report.get("equals_entry"), Some(&Json::Bool(true)));
        } else {
            assert_eq!(
                report.get("mut").unwrap().as_arr().unwrap(),
                &[Json::u64(0)]
            );
        }
        let dense = run(true);
        assert_eq!(
            dense.status.code(),
            Some(1),
            "device errors must exit normally"
        );
        let error = std::str::from_utf8(&dense.stderr).unwrap();
        assert_undecidable(error);
        assert!(
            error.contains(if reverse { "§7.4" } else { "§5.6" }),
            "{error}"
        );
        assert!(!error.contains("panicked"), "{error}");
        assert!(
            dense.stdout.is_empty(),
            "a fault must not emit a success report"
        );
    }
    std::fs::remove_file(path).unwrap();
}

/// Use a separate process to select the dense engine without racing other
/// tests that create launch resources. The host survives and receives the
/// decision diagnostic through bennett_last_error, not panic containment.
#[test]
fn launch_abi_reports_undecidable_cleanliness_to_the_host() {
    const CHILD: &str = "BENNETT_DENSE_DECISION_ABI_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "launch_abi_reports_undecidable_cleanliness_to_the_host",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env("BENNETT_VM_ENGINE", "dense")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
        return;
    }
    use bennett_vm::launch::*;
    use std::{ffi::CStr, ptr};
    let raw = rehash_object(decision_object().replace(
        "a = qreg.new any 1\nb = qreg.new any 1\nq = qreg.concat a b\nqreg.free q\nret",
        "a = qreg.new any 2\naction_call @bell (a) () () ()\nret",
    ));
    unsafe {
        let mut program = ptr::null_mut();
        assert_eq!(
            bennett_load(raw.as_ptr(), raw.len(), ptr::null(), 0, &mut program),
            0
        );
        let mut args = ptr::null_mut();
        assert_eq!(bennett_args_new(0, &mut args), 0);
        let mut run = 0;
        assert_eq!(
            bennett_run(
                program,
                c"allocation".as_ptr(),
                args,
                ptr::null(),
                0,
                &mut run
            ),
            0
        );
        bennett_args_free(args);
        let mut result = ptr::null_mut();
        let mut request = ptr::null_mut();
        assert_eq!(bennett_wait(program, run, &mut result, &mut request), 0);
        assert!(!result.is_null() && request.is_null());
        let mut snapshot = ptr::null_mut();
        assert_eq!(bennett_run_snapshot(program, run, &mut snapshot), 0);
        let mut len = 0;
        let bytes = bennett_snapshot_jcs(snapshot, &mut len);
        let json = bennett_vm::json::parse(
            std::str::from_utf8(std::slice::from_raw_parts(bytes, len)).unwrap(),
        )
        .unwrap();
        let register = json
            .get("instances")
            .unwrap()
            .as_arr()
            .unwrap()
            .iter()
            .find(|value| value.get("name").and_then(Json::as_str) == Some("a"))
            .unwrap()
            .get("values")
            .unwrap()
            .as_arr()
            .unwrap()[0]
            .get("value")
            .unwrap()
            .as_u64()
            .unwrap();
        bennett_snapshot_free(snapshot);
        bennett_result_free(result);
        assert_eq!(bennett_run_close(program, run), 0);
        for prepare in [true, false] {
            let status = if prepare {
                bennett_qreg_prepare(program, register, [0u8, 0].as_ptr(), 2)
            } else {
                bennett_qreg_free(program, register)
            };
            assert_eq!(status, 7, "undecidable cleanliness is a register fault");
            let error = CStr::from_ptr(bennett_last_error()).to_str().unwrap();
            assert_undecidable(error);
            assert!(!error.contains("internal panic"), "{error}");
        }
        assert_eq!(bennett_unload(program), 0);
    }
}

/// 2000 H on wire 0 (identity overall), then ry(2e-12) on a fresh wire 1:
/// wire 1 has amplitude ~1e-12 on |1>, so it is NOT clean (§5.6 qfree must fault).
fn dirty(mut s: QuantumState) -> (QuantumState, Vec<usize>) {
    let w = s.alloc_wires(2);
    for _ in 0..2000 {
        s.apply1(w[0], &[], h());
    }
    s.apply1(w[1], &[], ry(2e-12));
    (s, w)
}

#[test]
fn d1_dirty_wire_is_not_clean_in_dense() {
    let (e, w) = dirty(QuantumState::new());
    let (d, _) = dirty(QuantumState::new_dense());
    assert!(
        !e.is_clean(&[w[1]]).unwrap(),
        "control: exact interpreter sees the residue"
    );
    assert_eq!(
        d.is_clean(&[w[1]]).unwrap(),
        e.is_clean(&[w[1]]).unwrap(),
        "dense declares a dirty wire clean (B={:e})",
        bound(&d)
    );
    assert_eq!(
        d.constant_value(&[w[1]]).unwrap(),
        e.constant_value(&[w[1]]).unwrap()
    );
}

#[test]
fn d1_control_dense_sees_residue_with_small_bound() {
    let mut d = QuantumState::new_dense();
    let w = d.alloc_wires(1);
    d.apply1(w[0], &[], ry(2e-12));
    assert!(!d.is_clean(&w).unwrap(), "B={:e}", bound(&d));
}

/// Reversal equality (CLI `reverse` equals_entry) delegates to the
/// dense exact decision rather than accepting a distance inside the bound.
#[test]
fn d1b_non_identity_reversal_equals_entry_in_dense() {
    for mut s in engines() {
        let w = s.alloc_wires(2);
        for _ in 0..2000 {
            s.apply1(w[0], &[], h());
        }
        let entry = s.clone();
        s.apply1(w[1], &[], ry(2e-12));
        assert!(
            !s.numerically_equals(&entry).unwrap(),
            "state differing by ry(2e-12) reported equal to entry"
        );
    }
}

/// After conditioning on an outcome of probability ~1e-20, a wire with a
/// real probability 1e-8 of reading 1 must not be declared clean.
#[test]
fn d2_amplified_bound_factors_real_superposition() {
    let [e, d] = engines().map(|mut s| {
        let w = s.alloc_wires(3);
        for _ in 0..10 {
            s.apply1(w[2], &[], h());
        }
        s.apply1(w[0], &[], ry(std::f64::consts::PI - 2e-10));
        let (v, _) = s.measure(&[w[0]], 0.0);
        assert!(v.is_zero(), "draw 0 selects the unlikely outcome 0");
        s.apply1(w[1], &[], ry(2e-4));
        s.is_clean(&[w[1]]).unwrap()
    });
    assert!(!e);
    assert_eq!(d, e, "dense declares a wire with P(1)=1e-8 clean");
}

/// Conditioning on P = 1e-26 amplifies the a-priori bound; a fresh wire
/// with P(1) = sin^2(0.1) must keep its amplitude, so draw 0.995 (0.005
/// from the boundary) reads 1 as in the interpreter.
#[test]
fn d2b_amplified_bound_deletes_real_amplitude() {
    let [e, d] = engines().map(|mut s| {
        let w = s.alloc_wires(2);
        s.apply1(w[0], &[], ry(std::f64::consts::PI - 2e-13));
        let (v, _) = s.measure(&[w[0]], 0.0);
        assert!(v.is_zero(), "draw 0 selects the unlikely outcome 0");
        s.apply1(w[1], &[], ry(0.2));
        s.measure(&[w[1]], 0.995).1
    });
    assert_eq!(e, "1", "control");
    assert_eq!(
        d, e,
        "dense measurement outcome differs from the interpreter"
    );
}

/// A wire whose |0> branch has exact probability 1e-28 is not factored to
/// constant 1 before measurement: draw 0 reads 0 in both engines.
#[test]
fn d2a_compaction_does_not_factor_real_tiny_branch() {
    let [e, d] = engines().map(|mut s| {
        let w = s.alloc_wires(2);
        for _ in 0..10 {
            s.apply1(w[1], &[], h());
        }
        s.apply1(w[0], &[], ry(std::f64::consts::PI - 2e-14));
        s.measure(&[w[0]], 0.0).1
    });
    assert_eq!(
        d, e,
        "dense measurement outcome differs from the interpreter"
    );
}

/// Control: the state bound holds against the exact interpreter on a
/// Clifford+T+CX circuit.
#[test]
fn d3_control_bound_holds_against_exact() {
    let mut e = QuantumState::new();
    let mut d = QuantumState::new_dense();
    let we = e.alloc_wires(5);
    let _ = d.alloc_wires(5);
    let mut x: u64 = 0x9e3779b97f4a7c15;
    for _ in 0..400 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let q = (x % 5) as usize;
        let c = ((x >> 8) % 5) as usize;
        for s in [&mut e, &mut d] {
            match (x >> 16) % 3 {
                0 => s.apply1(we[q], &[], h()),
                1 => s.apply1(we[q], &[], t_gate()),
                _ => {
                    if c != q {
                        s.flip(we[q], &[we[c]])
                    } else {
                        s.apply1(we[q], &[], h())
                    }
                }
            }
        }
    }
    let (a, b) = (e.amplitudes(), d.amplitudes());
    let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).cloned().collect();
    let mut dist2 = 0.0;
    for k in keys {
        let p = a.get(&k).copied().unwrap_or((0., 0.));
        let q = b.get(&k).copied().unwrap_or((0., 0.));
        dist2 += (p.0 - q.0).powi(2) + (p.1 - q.1).powi(2);
    }
    assert!(dist2.sqrt() <= bound(&d) + 1e-15);
}

// ---- Exact decisions the certificates must still make (no regressions) ----

/// h c; h c; qfree c (fixture r9-loop shape): exactly clean in both.
#[test]
fn inverse_gate_pair_is_exactly_clean() {
    for mut s in engines() {
        let w = s.alloc_wires(2);
        s.apply1(w[1], &[], h());
        s.apply1(w[0], &[], h());
        s.apply1(w[0], &[], t_gate());
        s.apply1(w[0], &[], tdg_gate());
        s.apply1(w[0], &[], h());
        assert!(s.is_clean(&[w[0]]).unwrap());
        assert_eq!(s.constant_value(&[w[0]]).unwrap(), Some(BigUint::zero()));
    }
}

/// Bennett compute, use, uncompute with a non-commuting use in between:
/// the ancilla is exactly clean although no gate pair cancels adjacently.
#[test]
fn compute_phase_uncompute_ancilla_is_exactly_clean() {
    for mut s in engines() {
        let w = s.alloc_wires(4);
        s.apply1(w[0], &[], h());
        s.apply1(w[1], &[], h());
        s.apply1(w[3], &[], h());
        s.flip(w[2], &[w[0], w[1]]);
        s.phase_if_all(&[w[2], w[3]], (-1.0, 0.0));
        s.apply1(w[3], &[w[2]], h());
        s.apply1(w[3], &[w[2]], h());
        s.flip(w[2], &[w[0], w[1]]);
        assert!(s.is_clean(&[w[2]]).unwrap(), "ancilla after uncompute");
        assert_eq!(s.constant_value(&[w[2]]).unwrap(), Some(BigUint::zero()));
        assert!(!s.is_clean(&[w[0]]).unwrap());
    }
}

/// A wire whose cleanliness the dense certificates cannot settle (exact
/// cancellation through gates whose exact entries the engine never sees)
/// must fault loudly, not guess either way.
#[test]
fn undecidable_cleanliness_faults_loudly_in_dense() {
    let mut s = QuantumState::new_dense();
    let w = s.alloc_wires(2);
    s.apply1(w[1], &[], h());
    s.apply1(w[0], &[], ry(0.3));
    s.apply1(w[0], &[w[1]], h());
    s.apply1(w[0], &[w[1]], h());
    s.apply1(w[0], &[], ry(-0.3));
    let outcome = std::panic::catch_unwind(|| s.is_clean(&[w[0]]));
    assert!(
        outcome.is_ok(),
        "an undecidable device decision must not panic"
    );
    assert_undecidable(&outcome.unwrap().unwrap_err());
    assert_undecidable(&s.constant_value(&[w[0]]).unwrap_err());
    let before = s.amplitudes();
    assert_undecidable(&s.prepare(&[w[0]], &BigUint::zero()).unwrap_err());
    assert_eq!(
        s.amplitudes(),
        before,
        "failed decisions must preserve state"
    );
    // Put the undecidable wire at the allocation suffix; including the
    // certainly varying control would instead prove the suffix unclean.
    s.swap(w[0], w[1]);
    let before = s.amplitudes();
    assert_undecidable(&s.rewind_clean_wires(w[1]).unwrap_err());
    assert_eq!(s.amplitudes(), before, "failed rewind must preserve state");
    assert_eq!(s.wire_count(), 2, "failed rewind must preserve allocation");
}

/// The same program decided by the exact interpreter (oracle for the case
/// above): ry(0.3) and ry(-0.3) binary64 entries are not exact inverses.
#[test]
fn undecidable_case_has_an_exact_answer_in_the_interpreter() {
    let mut s = QuantumState::new();
    let w = s.alloc_wires(2);
    s.apply1(w[1], &[], h());
    s.apply1(w[0], &[], ry(0.3));
    s.apply1(w[0], &[w[1]], h());
    s.apply1(w[0], &[w[1]], h());
    s.apply1(w[0], &[], ry(-0.3));
    assert!(
        s.is_clean(&[w[0]]).unwrap(),
        "exact dyadic entries cancel: c*s - s*c = 0"
    );
}

/// Measurement compaction must never delete amplitude that the exact
/// state has: after a dense measurement the remaining state keeps the
/// small but real branch of an unmeasured wire.
#[test]
fn compaction_keeps_small_real_branch_after_unlikely_outcome() {
    let mut d = QuantumState::new_dense();
    let w = d.alloc_wires(3);
    d.apply1(w[0], &[], ry(std::f64::consts::PI - 2e-13));
    let _ = d.measure(&[w[0]], 0.0);
    d.apply1(w[1], &[], ry(0.2));
    let _ = d.measure(&[w[2]], 0.5);
    let snap = d.amplitudes();
    let one: f64 = snap
        .iter()
        .filter(|(k, _)| k.bit(w[1]))
        .map(|(_, a)| a.0 * a.0 + a.1 * a.1)
        .sum();
    assert!((one - (0.1f64).sin().powi(2)).abs() < 1e-3, "P(1) = {one}");
    let evidence = d.simulation_evidence().unwrap();
    assert_eq!(
        evidence
            .get("numerical_projections")
            .and_then(|v| v.as_u64()),
        Some(0),
        "no numerical factoring was needed"
    );
}

/// Property: on random circuits (exact-operand gates, rotation-shaped
/// matrices whose exact entries the dense engine cannot see, permutations,
/// phases and measurements), every constant/cleanliness answer the dense
/// engine gives equals the interpreter's; otherwise it faults with
/// "undecidable". Never a different answer.
#[test]
fn random_circuits_dense_decisions_match_interpreter_or_fault() {
    let n = 5;
    let (mut agreed, mut undecided) = (0u64, 0u64);
    for seed in 0..120u64 {
        let mut x = seed.wrapping_mul(0x9e3779b97f4a7c15) | 1;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut e = QuantumState::new();
        let mut d = QuantumState::new_dense();
        let w = e.alloc_wires(n);
        d.alloc_wires(n);
        for _ in 0..40 {
            let r = next();
            let q = (r % n as u64) as usize;
            let c = ((r >> 8) % n as u64) as usize;
            let c2 = ((r >> 16) % n as u64) as usize;
            let mut diverged = false;
            for s in [&mut e, &mut d] {
                match (r >> 24) % 10 {
                    0 | 1 => s.apply1(w[q], &[], h()),
                    2 => s.apply1(w[q], &[], t_gate()),
                    3 => s.apply1(w[q], &[], tdg_gate()),
                    4 => s.apply1(w[q], &[], ry(((r >> 32) % 7) as f64 * 0.25 - 0.75)),
                    5 if c != q => s.flip(w[q], &[w[c]]),
                    5 if c2 != q => {
                        let (src, dst) = (w[c2], w[q]);
                        s.map_basis(|k| {
                            let mut out = k.clone();
                            if k.bit(src) {
                                out.set_bit(dst, !k.bit(dst));
                            }
                            out
                        })
                    }
                    6 if c != q && c2 != q && c2 != c => s.flip(w[q], &[w[c], w[c2]]),
                    7 if c != q => s.apply1(w[q], &[w[c]], h()),
                    8 => s.phase_if_all(&[w[q], w[c]], (-1.0, 0.0)),
                    9 if (r >> 40) % 3 == 0 => {}
                    _ => s.flip(w[q], &[]),
                }
            }
            if (r >> 24) % 10 == 9 && (r >> 40) % 3 == 0 {
                let draw = ((r >> 44) % 1000) as f64 / 1000.0;
                let a = e.measure(&[w[q]], draw);
                let b = d.measure(&[w[q]], draw);
                diverged = a != b;
            }
            if diverged {
                break; // sampled outcome differs at a draw boundary (documented caveat)
            }
            for &wire in &w {
                let exact = e.constant_value(&[wire]).unwrap();
                let dense = d.constant_value(&[wire]);
                match dense {
                    Ok(v) => {
                        assert_eq!(v, exact, "seed {seed}: wire {wire} dense answer differs");
                        agreed += 1;
                    }
                    Err(msg) => {
                        assert_undecidable(&msg);
                        undecided += 1;
                    }
                }
            }
        }
    }
    eprintln!("dense decisions: {agreed} agreed, {undecided} undecidable");
    assert!(
        agreed > 10 * undecided,
        "{agreed} agreed vs {undecided} undecidable"
    );
}

fn s_gate() -> [[Amp; 2]; 2] {
    [[(1., 0.), (0., 0.)], [(0., 0.), (0., 1.)]]
}

/// Conformance case objects/interference: h c; ctl c (x s s x s s = -I) t;
/// h c. The controlled -I is Z on c, so c ends exactly 1, but no gate pair
/// cancels and the support certificate cannot see the phase interference.
/// The exact fallback (replay through the interpreter) must decide it
/// (full gate at 1ec2f7b faulted "undecidable" here).
#[test]
fn algebraic_interference_is_decided_by_exact_replay() {
    for mut s in engines() {
        let w = s.alloc_wires(2);
        s.prepare(&w, &BigUint::zero()).unwrap();
        let (c, t) = (w[0], w[1]);
        s.apply1(c, &[], h());
        s.flip(t, &[c]);
        s.apply1(t, &[c], s_gate());
        s.apply1(t, &[c], s_gate());
        s.flip(t, &[c]);
        s.apply1(t, &[c], s_gate());
        s.apply1(t, &[c], s_gate());
        s.apply1(c, &[], h());
        assert_eq!(s.constant_value(&w).unwrap(), Some(BigUint::from_u64(1)));
    }
}

/// actions.bisa bell --adjoint then reverse (differential `reverse`): the
/// control returned to |0> by H.H is still materialized when the next cx
/// arrives, so the target's cleanliness needs the exact fallback.
#[test]
fn bell_adjoint_reverse_cleanliness_is_decided_exactly() {
    for mut s in engines() {
        let w = s.alloc_wires(4);
        s.flip(w[1], &[w[0]]);
        s.apply1(w[0], &[], h());
        s.apply1(w[0], &[], h());
        s.flip(w[1], &[w[0]]);
        assert!(s.is_clean(&w).unwrap());
    }
}

/// Measurements replay with their draws; a replayed history decides exactly.
#[test]
fn exact_replay_through_a_measurement() {
    for mut s in engines() {
        let w = s.alloc_wires(3);
        s.apply1(w[0], &[], h());
        s.apply1(w[2], &[], h());
        let _ = s.measure(&[w[2]], 0.7);
        s.flip(w[1], &[w[0]]);
        s.apply1(w[1], &[w[0]], s_gate());
        s.apply1(w[1], &[w[0]], s_gate());
        s.apply1(w[1], &[w[0]], s_gate());
        s.apply1(w[1], &[w[0]], s_gate());
        s.flip(w[1], &[w[0]]);
        assert!(s.is_clean(&[w[1]]).unwrap());
    }
}

/// Fixture r9-codex-oracle: h c; oracle d ^= c twice (remaps); h c; qfree.
#[test]
fn remap_compute_uncompute_between_inverse_gates_is_exactly_clean() {
    for mut s in engines() {
        let w = s.alloc_wires(7);
        let (c, d) = (w[1], w[2]);
        s.apply1(c, &[], h());
        for _ in 0..2 {
            s.map_basis(|k| {
                let mut out = k.clone();
                if k.bit(c) {
                    out.set_bit(d, !k.bit(d));
                }
                out
            });
        }
        s.apply1(c, &[], h());
        assert!(s.is_clean(&w[3..]).unwrap());
        assert!(s.is_clean(&[d]).unwrap());
        assert!(s.is_clean(&[c]).unwrap());
    }
}

/// Fixture r9-sol-qinplace (reverse): h c; xor c->s, swap c<->s, xor c->s
/// (the identity on the support, not on the packed domain); h c. The
/// released wire must be exactly factored: no residue may remain
/// materialized on a released wire (the full gate saw distance 1 in the
/// CLI's projected state when it did).
#[test]
fn support_restricted_remaps_compose_and_release_factors_exactly() {
    for mut s in engines() {
        let w = s.alloc_wires(3);
        let (t, c, k) = (w[0], w[1], w[2]);
        s.set_state([(BigUint::zero(), (0.0, 1.0))].into_iter().collect());
        s.apply1(c, &[], h());
        let xor = |k_: &BigUint| {
            let mut out = k_.clone();
            if k_.bit(c) {
                out.set_bit(k, !k_.bit(k));
            }
            out
        };
        s.map_basis(xor);
        s.map_basis(|key| {
            let mut out = key.clone();
            out.set_bit(c, key.bit(k));
            out.set_bit(k, key.bit(c));
            out
        });
        s.map_basis(xor);
        s.apply1(c, &[], h());
        assert!(s.is_clean(&[c, k]).unwrap());
        s.rewind_clean_wires(c).expect("released wires are clean");
        let snap = s.amplitudes();
        assert_eq!(snap.len(), 1, "released wires keep no residue: {snap:?}");
        assert!(
            snap.keys()
                .all(|key| !key.bit(c) && !key.bit(k) && !key.bit(t))
        );
    }
}
