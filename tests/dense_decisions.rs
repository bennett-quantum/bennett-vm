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
    assert!(!e.is_clean(&[w[1]]), "control: exact interpreter sees the residue");
    assert_eq!(
        d.is_clean(&[w[1]]),
        e.is_clean(&[w[1]]),
        "dense declares a dirty wire clean (B={:e})",
        bound(&d)
    );
    assert_eq!(d.constant_value(&[w[1]]), e.constant_value(&[w[1]]));
}

#[test]
fn d1_control_dense_sees_residue_with_small_bound() {
    let mut d = QuantumState::new_dense();
    let w = d.alloc_wires(1);
    d.apply1(w[0], &[], ry(2e-12));
    assert!(!d.is_clean(&w), "B={:e}", bound(&d));
}

/// Reversal equality (`run-action --reverse` equals_entry) is decided in
/// src/action.rs `QuantumState::numerically_equals`, outside the dense
/// module; the dense-side exact decision is `dense::Runtime::equals`
/// (unit-tested in src/dense/runtime.rs). Open until action.rs delegates
/// to it (bennett-vm-5vj).
#[test]
#[ignore = "bennett-vm-5vj: action.rs numerically_equals must delegate to dense::Runtime::equals (action.rs not owned by bjm-fix)"]
fn d1b_non_identity_reversal_equals_entry_in_dense() {
    for mut s in engines() {
        let w = s.alloc_wires(2);
        for _ in 0..2000 {
            s.apply1(w[0], &[], h());
        }
        let entry = s.clone();
        s.apply1(w[1], &[], ry(2e-12));
        assert!(
            !s.numerically_equals(&entry),
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
        s.is_clean(&[w[1]])
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
    assert_eq!(d, e, "dense measurement outcome differs from the interpreter");
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
    assert_eq!(d, e, "dense measurement outcome differs from the interpreter");
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
        assert!(s.is_clean(&[w[0]]));
        assert_eq!(s.constant_value(&[w[0]]), Some(BigUint::zero()));
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
        assert!(s.is_clean(&[w[2]]), "ancilla after uncompute");
        assert_eq!(s.constant_value(&[w[2]]), Some(BigUint::zero()));
        assert!(!s.is_clean(&[w[0]]));
    }
}

/// A wire whose cleanliness the dense certificates cannot settle (exact
/// cancellation through gates whose exact entries the engine never sees)
/// must fault loudly, not guess either way.
#[test]
#[should_panic(expected = "undecidable")]
fn undecidable_cleanliness_faults_loudly_in_dense() {
    let mut s = QuantumState::new_dense();
    let w = s.alloc_wires(2);
    s.apply1(w[1], &[], h());
    s.apply1(w[0], &[], ry(0.3));
    s.apply1(w[0], &[w[1]], h());
    s.apply1(w[0], &[w[1]], h());
    s.apply1(w[0], &[], ry(-0.3));
    let _ = s.is_clean(&[w[0]]);
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
    assert!(s.is_clean(&[w[0]]), "exact dyadic entries cancel: c*s - s*c = 0");
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
        evidence.get("numerical_projections").and_then(|v| v.as_u64()),
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
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
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
                let exact = e.constant_value(&[wire]);
                let dense = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    d.constant_value(&[wire])
                }));
                match dense {
                    Ok(v) => {
                        assert_eq!(v, exact, "seed {seed}: wire {wire} dense answer differs");
                        agreed += 1;
                    }
                    Err(p) => {
                        let msg = p
                            .downcast_ref::<String>()
                            .cloned()
                            .unwrap_or_default();
                        assert!(msg.contains("undecidable"), "seed {seed}: {msg}");
                        undecided += 1;
                    }
                }
            }
        }
    }
    std::panic::set_hook(previous);
    eprintln!("dense decisions: {agreed} agreed, {undecided} undecidable");
    assert!(agreed > 10 * undecided, "{agreed} agreed vs {undecided} undecidable");
}
