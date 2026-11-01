//! Dense engine vs the exact sparse engine (`action::QuantumState`).
//! The exact engine is the semantic reference (epsilon = 0); the dense
//! engine must agree within its own stated floating-point bound.
use bennett_vm::action::QuantumState;
use bennett_vm::dense::{Complex, DenseState, Gate};
use std::collections::BTreeMap;

type Amp = (f64, f64);
fn h_amp() -> [[Amp; 2]; 2] {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    [[(s, 0.0), (s, 0.0)], [(s, 0.0), (-s, 0.0)]]
}
fn h_c() -> [[Complex; 2]; 2] {
    h_amp().map(|r| r.map(|(a, b)| Complex::new(a, b)))
}

fn exact_amps(q: &QuantumState, n: usize) -> BTreeMap<usize, Amp> {
    let mut out: BTreeMap<usize, Amp> = (0..1usize << n).map(|i| (i, (0.0, 0.0))).collect();
    for (k, a) in &q.amps {
        out.insert(k.to_usize().expect("small key"), *a);
    }
    out
}

fn assert_agree(d: &DenseState, q: &QuantumState, n: usize, ctx: &str) {
    let ex = exact_amps(q, n);
    // Account only for the exact engine's final f64 projection here;
    // the dense engine must supply its own forward-error allowance.
    let projection = 8.0 * f64::EPSILON;
    let tol = d.error_bound() + projection;
    let mut distance2 = 0.0;
    for i in 0..1usize << n {
        let z = d.amplitude(i);
        let e = ex[&i];
        distance2 += (z.re - e.0).powi(2) + (z.im - e.1).powi(2);
        assert!(
            (z.re - e.0).abs() <= tol && (z.im - e.1).abs() <= tol,
            "{ctx} amp[{i}]: dense=({},{}) exact=({},{}) bound={:e}",
            z.re,
            z.im,
            e.0,
            e.1,
            d.error_bound()
        );
    }
    assert!(
        distance2.sqrt() <= d.error_bound() + projection * (d.len() as f64).sqrt(),
        "{ctx}: 2-norm distance {} exceeds numerical bound {} plus projection allowance",
        distance2.sqrt(),
        d.error_bound()
    );
}

#[test]
fn dense_agrees_with_exact_engine_on_gate_corpus() {
    let n = 4;
    let mut d = DenseState::new(n);
    let mut q = QuantumState::new();
    let w = q.alloc_wires(n);
    assert_eq!(w, vec![0, 1, 2, 3]);
    // Realistic structured sequence over the named gate alphabet.
    d.apply1(0, &[], h_c());
    q.apply1(0, &[], h_amp());
    d.apply1(2, &[], h_c());
    q.apply1(2, &[], h_amp());
    d.cx(0, 1);
    q.flip(1, &[0]);
    d.t_gate(1);
    {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        q.apply1(1, &[], [[(1.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), (s, s)]]);
    }
    d.flip(3, &[0, 1]);
    q.flip(3, &[0, 1]); // ccx
    d.s_gate(2);
    q.apply1(2, &[], [[(1.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), (0.0, 1.0)]]);
    d.swap(1, 3);
    q.swap(1, 3);
    d.cz(0, 2);
    {
        q.phase_if_all(&[0, 2], (-1.0, 0.0));
    }
    d.phase_if_all(&[0, 1, 2], Complex::new(0.0, -1.0));
    q.phase_if_all(&[0, 1, 2], (0.0, -1.0));
    assert_agree(&d, &q, n, "corpus");
    // Inverse permutation segment returns both to the pre-segment state
    // structure: apply ccx again (self-inverse) on both.
    d.flip(2, &[0, 3]);
    q.flip(2, &[0, 3]);
    d.flip(2, &[0, 3]);
    q.flip(2, &[0, 3]);
    assert_agree(&d, &q, n, "corpus+perm round trip");
}

#[test]
fn dense_measure_agrees_with_exact_engine_measure() {
    // Superposition over wires 0,2 with a marker on wire 1, then measure
    // wires [2, 0] (wire order matters: exact engine enumerates outcomes
    // lexicographically in list order). Sweep draws across [0,1).
    for draw in [0.0f64, 0.1, 0.26, 0.49, 0.51, 0.74, 0.99] {
        let mut d = DenseState::new(3);
        let mut q = QuantumState::new();
        q.alloc_wires(3);
        d.apply1(0, &[], h_c());
        q.apply1(0, &[], h_amp());
        d.apply1(2, &[], h_c());
        q.apply1(2, &[], h_amp());
        d.cx(2, 1);
        q.flip(1, &[2]);
        d.t_gate(0);
        {
            let s = std::f64::consts::FRAC_1_SQRT_2;
            q.apply1(0, &[], [[(1.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), (s, s)]]);
        }
        let (dv, dbits) = d.measure(&[2, 0], draw);
        let (qv, qbits) = q.measure(&[2, 0], draw);
        assert_eq!(dv, qv.to_usize().unwrap(), "draw {draw}: outcome value");
        assert_eq!(dbits, qbits, "draw {draw}: outcome bits");
        assert_agree(&d, &q, 3, "post-measure");
    }
}

#[test]
fn grover_two_qubit_marks_target_like_exact_engine() {
    // One Grover iteration on 2 qubits marking |11>: exact theory gives
    // probability 1 on |11>. Both engines must show it.
    let run_dense = || {
        let mut d = DenseState::new(2);
        d.apply1(0, &[], h_c());
        d.apply1(1, &[], h_c());
        d.cz(0, 1); // oracle: phase -1 on |11>
        d.apply1(0, &[], h_c());
        d.apply1(1, &[], h_c());
        d.x_gate(0);
        d.x_gate(1);
        d.cz(0, 1);
        d.x_gate(0);
        d.x_gate(1);
        d.apply1(0, &[], h_c());
        d.apply1(1, &[], h_c());
        d
    };
    let d = run_dense();
    let p = d.probabilities(&[0, 1]);
    assert!((p[3] - 1.0).abs() < 1e-12, "Grover p(11) = {}", p[3]);
    let mut q = QuantumState::new();
    q.alloc_wires(2);
    q.apply1(0, &[], h_amp());
    q.apply1(1, &[], h_amp());
    q.phase_if_all(&[0, 1], (-1.0, 0.0));
    q.apply1(0, &[], h_amp());
    q.apply1(1, &[], h_amp());
    q.flip(0, &[]);
    q.flip(1, &[]);
    q.phase_if_all(&[0, 1], (-1.0, 0.0));
    q.flip(0, &[]);
    q.flip(1, &[]);
    q.apply1(0, &[], h_amp());
    q.apply1(1, &[], h_amp());
    assert_agree(&d, &q, 2, "grover");
}

#[test]
fn longer_fused_and_scalar_sequences_stay_within_reported_bound() {
    let n = 4;
    let mut exact = QuantumState::new();
    exact.alloc_wires(n);
    let mut fused = DenseState::with_threads(n, 1);
    let mut scalar = fused.clone();
    scalar.set_simd_enabled(false);
    let mut group = Vec::new();
    for step in 0..128 {
        let target = (step / 3 + step) % n;
        let control = (target + 1) % n;
        let gate = match step % 8 {
            0 | 1 => Gate::One {
                target,
                controls: vec![],
                m: h_c(),
            },
            2 => Gate::Flip {
                target,
                controls: vec![control],
            },
            3 => Gate::Diagonal {
                target,
                controls: vec![],
                d: [
                    Complex::ONE,
                    Complex::new(
                        std::f64::consts::FRAC_1_SQRT_2,
                        -std::f64::consts::FRAC_1_SQRT_2,
                    ),
                ],
            },
            4 => Gate::One {
                target,
                controls: vec![control],
                m: h_c(),
            },
            5 => Gate::Diagonal {
                target,
                controls: vec![control],
                d: [Complex::I, Complex::ONE],
            },
            6 => Gate::One {
                target,
                controls: vec![],
                m: [[Complex::ZERO, -Complex::I], [Complex::I, Complex::ZERO]],
            },
            _ => Gate::One {
                target,
                controls: vec![],
                m: [
                    [Complex::new(0.6, 0.0), Complex::new(-0.8, 0.0)],
                    [Complex::new(0.8, 0.0), Complex::new(0.6, 0.0)],
                ],
            },
        };
        match &gate {
            Gate::One {
                target,
                controls,
                m,
            } => {
                exact.apply1(*target, controls, m.map(|r| r.map(|z| (z.re, z.im))));
                scalar.apply1(*target, controls, *m);
            }
            Gate::Diagonal {
                target,
                controls,
                d,
            } => {
                exact.apply1(
                    *target,
                    controls,
                    [
                        [(d[0].re, d[0].im), (0.0, 0.0)],
                        [(0.0, 0.0), (d[1].re, d[1].im)],
                    ],
                );
                scalar.diagonal1(*target, controls, d[0], d[1]);
            }
            Gate::Flip { target, controls } => {
                exact.flip(*target, controls);
                scalar.flip(*target, controls);
            }
        }
        group.push(gate);
        if group.len() == 8 {
            fused.apply_fused(&group);
            assert_agree(&fused, &exact, n, "long fused");
            assert_agree(&scalar, &exact, n, "long scalar");
            group.clear();
        }
    }
}

#[test]
fn empty_measurement_matches_exact_interpreter() {
    for n in [0, 3] {
        let mut exact = QuantumState::new();
        exact.alloc_wires(n);
        let mut dense = DenseState::with_threads(n, 1);
        if n > 0 {
            exact.apply1(1, &[], h_amp());
            dense.h_gate(1);
        }
        let (value, bits) = exact.measure(&[], 0.7);
        assert_eq!(dense.measure(&[], 0.7), (value.to_usize().unwrap(), bits));
        assert_agree(&dense, &exact, n, "empty measurement");
    }
}

#[test]
fn selected_dense_cli_executes_bell_and_labels_numerical_evidence() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/registers.bisa");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
        .env("BENNETT_VM_ENGINE", "dense")
        .args([
            "run-action",
            fixture.to_str().unwrap(),
            "bell",
            "--mut",
            "0",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "selected dense action failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = bennett_vm::json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let evidence = report
        .get("simulation")
        .expect("dense execution needs simulation provenance");
    assert_eq!(
        evidence.get("kind").and_then(|j| j.as_str()),
        Some("numerical")
    );
    assert!(
        evidence.get("error_bound").is_some(),
        "dense evidence needs its numerical bound"
    );
    let state = report.get("state").unwrap().as_obj().unwrap();
    for bits in ["00", "11"] {
        let amp = state.get(bits).unwrap().as_arr().unwrap();
        let re: f64 = match &amp[0] {
            bennett_vm::json::Json::Num(s) => s.parse().unwrap(),
            other => panic!("invalid amplitude {other:?}"),
        };
        assert!((re - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-14);
    }
}
