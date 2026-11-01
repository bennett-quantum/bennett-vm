//! Dense kernels vs an independent slow oracle written in this file.
//! The oracle uses an out-of-place, per-basis-state algorithm (like the
//! exact sparse engine), deliberately different from the in-place pair
//! kernels under test.
use bennett_vm::dense::{Complex, DenseState, Gate};
use bennett_vm::exact::Exact;

type C = (f64, f64);
fn cmul(a: C, b: C) -> C {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}
fn cadd(a: C, b: C) -> C {
    (a.0 + b.0, a.1 + b.1)
}
fn cc(m: [[Complex; 2]; 2]) -> [[C; 2]; 2] {
    m.map(|r| r.map(|z| (z.re, z.im)))
}

struct Slow {
    n: usize,
    a: Vec<C>,
}
impl Slow {
    fn from_dense(d: &DenseState) -> Self {
        Self {
            n: d.qubits(),
            a: (0..d.len())
                .map(|i| {
                    let z = d.amplitude(i);
                    (z.re, z.im)
                })
                .collect(),
        }
    }
    fn apply1(&mut self, target: usize, controls: &[usize], m: [[C; 2]; 2]) {
        let mut next = vec![(0.0, 0.0); self.a.len()];
        for (k, &amp) in self.a.iter().enumerate() {
            if controls.iter().all(|&c| (k >> c) & 1 == 1) {
                let bit = (k >> target) & 1;
                for (out, row) in m.iter().enumerate() {
                    let nk = (k & !(1 << target)) | (out << target);
                    next[nk] = cadd(next[nk], cmul(amp, row[bit]));
                }
            } else {
                next[k] = cadd(next[k], amp);
            }
        }
        self.a = next;
    }
    fn map_basis(&mut self, f: impl Fn(usize) -> usize) {
        let mut next = vec![(0.0, 0.0); self.a.len()];
        for (k, &amp) in self.a.iter().enumerate() {
            next[f(k)] = cadd(next[f(k)], amp);
        }
        self.a = next;
    }
    fn phase_where(&mut self, pred: impl Fn(usize) -> bool, p: C) {
        for (k, amp) in self.a.iter_mut().enumerate() {
            if pred(k) {
                *amp = cmul(*amp, p);
            }
        }
    }
    fn prob(&self, wires: &[usize]) -> Vec<f64> {
        let mut p = vec![0.0; 1 << wires.len()];
        for (k, amp) in self.a.iter().enumerate() {
            let mut v = 0usize;
            for (i, &w) in wires.iter().enumerate() {
                v |= ((k >> w) & 1) << i;
            }
            p[v] += amp.0 * amp.0 + amp.1 * amp.1;
        }
        p
    }
    #[allow(dead_code)]
    fn qubits(&self) -> usize {
        self.n
    }
}

fn h() -> [[Complex; 2]; 2] {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    [
        [Complex::new(s, 0.0), Complex::new(s, 0.0)],
        [Complex::new(s, 0.0), Complex::new(-s, 0.0)],
    ]
}
fn u(a: f64, b: f64) -> [[Complex; 2]; 2] {
    // A generic unitary from two angles (Ry then Rz-like phases).
    let (c, s) = (a.cos(), a.sin());
    let p = Complex::new(b.cos(), b.sin());
    [
        [Complex::new(c, 0.0), Complex::new(-s, 0.0)],
        [p.scale(s), p.scale(c)],
    ]
}

/// Deterministic pseudo-random normalised state on n qubits.
fn seeded_state(n: usize, seed: u64) -> DenseState {
    let mut d = DenseState::new(n);
    let mut x = seed | 1;
    let mut next = move || {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((x >> 33) as f64) / (1u64 << 31) as f64 - 1.0
    };
    let mut norm = 0.0;
    let vals: Vec<C> = (0..d.len())
        .map(|_| {
            let v = (next(), next());
            norm += v.0 * v.0 + v.1 * v.1;
            v
        })
        .collect();
    let inv = 1.0 / norm.sqrt();
    for (i, v) in vals.iter().enumerate() {
        d.set_amplitude(i, Complex::new(v.0 * inv, v.1 * inv));
    }
    d
}

fn assert_close(d: &DenseState, s: &Slow, ctx: &str) {
    // The slow f64 oracle also rounds; allow one independent budget,
    // not the predecessor's 1000x multiplier and fixed 1e-12 floor.
    let tol = 2.0 * d.error_bound() + 8.0 * f64::EPSILON;
    for i in 0..d.len() {
        let z = d.amplitude(i);
        assert!(
            (z.re - s.a[i].0).abs() <= tol && (z.im - s.a[i].1).abs() <= tol,
            "{ctx}: amp[{i}] dense=({},{}) slow=({},{}) tol={tol:e}",
            z.re,
            z.im,
            s.a[i].0,
            s.a[i].1
        );
    }
    assert!(
        (d.norm2() - 1.0).abs() < 1e-9,
        "{ctx}: norm drifted: {}",
        d.norm2()
    );
}

#[test]
fn one_qubit_gates_match_slow_oracle_all_targets() {
    for n in [1usize, 2, 3, 5] {
        for target in 0..n {
            let mut d = seeded_state(n, 0xB10 + (n * 16 + target) as u64);
            let mut s = Slow::from_dense(&d);
            for m in [h(), u(0.37, 1.1), u(-1.9, 0.23)] {
                d.apply1(target, &[], m);
                s.apply1(target, &[], cc(m));
                assert_close(&d, &s, "1q");
            }
        }
    }
}

#[test]
fn controlled_gates_match_slow_oracle() {
    let n = 5;
    for (target, controls) in [
        (2usize, vec![0]),
        (0, vec![1, 3]),
        (4, vec![0, 1, 2]),
        (1, vec![4]),
    ] {
        let mut d = seeded_state(n, 0xC0FFEE + target as u64);
        let mut s = Slow::from_dense(&d);
        d.apply1(target, &controls, u(0.9, -0.4));
        s.apply1(target, &controls, cc(u(0.9, -0.4)));
        assert_close(&d, &s, "controlled");
    }
}
#[test]
fn diagonal_and_phase_match_slow_oracle() {
    let n = 4;
    let mut d = seeded_state(n, 42);
    let mut s = Slow::from_dense(&d);
    let (d0, d1) = (Complex::new(0.0, 1.0), Complex::new(-1.0, 0.0));
    d.diagonal1(2, &[0], d0, d1);
    s.phase_where(|k| (k & 1) == 1 && ((k >> 2) & 1) == 0, (0.0, 1.0));
    s.phase_where(|k| (k & 1) == 1 && ((k >> 2) & 1) == 1, (-1.0, 0.0));
    assert_close(&d, &s, "diag");
    let bound0 = d.error_bound();
    d.phase_if_all(&[0, 2, 3], Complex::new(0.6, 0.8));
    s.phase_where(
        |k| [0usize, 2, 3].iter().all(|&w| (k >> w) & 1 == 1),
        (0.6, 0.8),
    );
    assert_close(&d, &s, "phase_if_all");
    assert!(
        d.error_bound() > bound0,
        "diagonal gates must charge the bound"
    );
    for t in 0..n {
        d.apply1(t, &[], h());
        s.apply1(t, &[], cc(h()));
    }
    d.rz(1, 0.7);
    d.s_gate(0);
    d.t_gate(3);
    d.z_gate(2);
    // Matches exact::Exact::rotation("rz", th) = diag(e^{-i th/2}, e^{+i th/2}).
    s.phase_where(|k| ((k >> 1) & 1) == 0, {
        let a = -0.35f64;
        (a.cos(), a.sin())
    });
    s.phase_where(|k| ((k >> 1) & 1) == 1, {
        let a = 0.35f64;
        (a.cos(), a.sin())
    });
    s.phase_where(|k| (k & 1) == 1, (0.0, 1.0));
    s.phase_where(|k| ((k >> 3) & 1) == 1, {
        let a = std::f64::consts::FRAC_PI_4;
        (a.cos(), a.sin())
    });
    s.phase_where(|k| ((k >> 2) & 1) == 1, (-1.0, 0.0));
    assert_close(&d, &s, "named diag");
}

#[test]
fn permutation_gates_match_slow_oracle_and_are_exact() {
    let n = 5;
    let mut d = seeded_state(n, 7);
    let mut s = Slow::from_dense(&d);
    let b0 = d.error_bound();
    d.flip(3, &[0, 1]); // ccx shape
    s.map_basis(|k| {
        if (k & 1) == 1 && ((k >> 1) & 1) == 1 {
            k ^ (1 << 3)
        } else {
            k
        }
    });
    assert_close(&d, &s, "ccx");
    d.swap(0, 4);
    s.map_basis(|k| {
        let (a, b) = (k & 1, (k >> 4) & 1);
        (k & !1 & !(1 << 4)) | b | (a << 4)
    });
    assert_close(&d, &s, "swap");
    d.x_gate(2);
    d.cx(4, 1);
    s.map_basis(|k| k ^ (1 << 2));
    s.map_basis(|k| if ((k >> 4) & 1) == 1 { k ^ (1 << 1) } else { k });
    assert_close(&d, &s, "x/cx");
    assert_eq!(
        d.error_bound(),
        b0,
        "permutations are exact: bound must not grow"
    );
    // Permutation round-trip is the identity, bit-for-bit.
    let mut e = seeded_state(n, 99);
    let before: Vec<C> = (0..e.len())
        .map(|i| {
            let z = e.amplitude(i);
            (z.re, z.im)
        })
        .collect();
    e.flip(2, &[0, 3]);
    e.flip(2, &[0, 3]);
    e.swap(1, 4);
    e.swap(1, 4);
    for (i, &want) in before.iter().enumerate() {
        let z = e.amplitude(i);
        assert_eq!((z.re, z.im), want, "perm round-trip amp[{i}]");
    }
}

#[test]
fn xor_oracle_enumeration_matches_slow_oracle_and_memoises() {
    let n = 6;
    let mut d = seeded_state(n, 1234);
    let mut s = Slow::from_dense(&d);
    let f = |x: usize| (x * x + 3 * x + 1) & 0b111;
    let mut calls = 0usize;
    d.apply_xor_oracle(&[0, 1, 2], &[3, 4, 5], |x| {
        calls += 1;
        f(x)
    });
    assert_eq!(
        calls, 8,
        "oracle evaluated once per distinct projected input"
    );
    s.map_basis(|k| {
        let x = k & 0b111;
        let y = ((k >> 3) & 0b111) ^ f(x);
        (k & 0b111) | (y << 3)
    });
    assert_close(&d, &s, "xor_oracle");
    // Involution: applying twice is identity bit-for-bit.
    d.apply_xor_oracle(&[0, 1, 2], &[3, 4, 5], f);
    let orig = seeded_state(n, 1234);
    for i in 0..d.len() {
        assert_eq!(d.amplitude(i), orig.amplitude(i), "oracle^2 amp[{i}]");
    }
}

#[test]
fn fused_groups_match_sequential_and_slow_oracle() {
    let n = 6;
    let base = seeded_state(n, 555);
    let group = vec![
        Gate::One {
            target: 0,
            controls: vec![],
            m: h(),
        },
        Gate::One {
            target: 1,
            controls: vec![0],
            m: u(0.4, 0.9),
        },
        Gate::Diagonal {
            target: 2,
            controls: vec![],
            d: [Complex::ONE, Complex::new(0.0, 1.0)],
        },
        Gate::Flip {
            target: 2,
            controls: vec![0, 1],
        },
        Gate::One {
            target: 0,
            controls: vec![],
            m: u(-0.7, 2.0),
        },
        Gate::One {
            target: 5,
            controls: vec![2],
            m: h(),
        },
    ];
    let mut fused = base.clone();
    fused.apply_fused(&group);
    let mut seq = base.clone();
    let mut slow = Slow::from_dense(&base);
    for g in &group {
        match g {
            Gate::One {
                target,
                controls,
                m,
            } => {
                seq.apply1(*target, controls, *m);
                slow.apply1(*target, controls, cc(*m));
            }
            Gate::Diagonal {
                target,
                controls,
                d,
            } => {
                seq.diagonal1(*target, controls, d[0], d[1]);
                slow.apply1(
                    *target,
                    controls,
                    [
                        [(d[0].re, d[0].im), (0.0, 0.0)],
                        [(0.0, 0.0), (d[1].re, d[1].im)],
                    ],
                );
            }
            Gate::Flip { target, controls } => {
                seq.flip(*target, controls);
                let (t, cs) = (*target, controls.clone());
                slow.map_basis(move |k| {
                    if cs.iter().all(|&c| (k >> c) & 1 == 1) {
                        k ^ (1 << t)
                    } else {
                        k
                    }
                });
            }
        }
    }
    assert_close(&fused, &slow, "fused vs slow");
    for i in 0..fused.len() {
        let tol = fused.error_bound() + seq.error_bound() + 8.0 * f64::EPSILON;
        assert!(
            (fused.amplitude(i).re - seq.amplitude(i).re).abs() <= tol
                && (fused.amplitude(i).im - seq.amplitude(i).im).abs() <= tol,
            "fused vs seq amp[{i}]"
        );
    }
    // Exercise the six-wire fusion limit, including a five-control flip.
    let big: Vec<Gate> = (0..6)
        .map(|t| Gate::One {
            target: t,
            controls: vec![],
            m: u(0.1 * t as f64, 0.0),
        })
        .chain([Gate::Flip {
            target: 5,
            controls: vec![0, 1, 2, 3, 4],
        }])
        .collect();
    let mut d2 = base.clone();
    d2.apply_fused(&big);
    let mut s2 = Slow::from_dense(&base);
    for t in 0..6 {
        s2.apply1(t, &[], cc(u(0.1 * t as f64, 0.0)));
    }
    s2.map_basis(|k| {
        if (0..5).all(|c| (k >> c) & 1 == 1) {
            k ^ (1 << 5)
        } else {
            k
        }
    });
    assert_close(&d2, &s2, "six-wire fusion limit");
}
#[test]
fn fused_contiguous_low_wires_match_slow_oracle() {
    let mut d = seeded_state(6, 31337);
    let mut s = Slow::from_dense(&d);
    let group: Vec<Gate> = (0..8)
        .map(|i| Gate::One {
            target: i % 4,
            controls: if i % 3 == 0 {
                vec![(i + 1) % 4]
            } else {
                vec![]
            },
            m: u(0.13 * i as f64, 0.5),
        })
        .collect();
    d.apply_fused(&group);
    for g in &group {
        if let Gate::One {
            target,
            controls,
            m,
        } = g
        {
            s.apply1(*target, controls, cc(*m));
        }
    }
    assert_close(&d, &s, "fused contiguous");
}

#[test]
fn measurement_probabilities_collapse_and_reset() {
    // Bell state on 2 qubits: measuring both wires gives 00 or 11.
    for (draw, want_val, want_bits) in [(0.25f64, 0usize, "00"), (0.75, 3, "11")] {
        let mut d = DenseState::new(2);
        d.apply1(0, &[], h());
        d.cx(0, 1);
        let probs = d.probabilities(&[0, 1]);
        assert!(
            (probs[0] - 0.5).abs() < 1e-15
                && (probs[3] - 0.5).abs() < 1e-15
                && probs[1] < 1e-15
                && probs[2] < 1e-15,
            "Bell probs {probs:?}"
        );
        let (v, bits) = d.measure(&[0, 1], draw);
        assert_eq!((v, bits.as_str()), (want_val, want_bits));
        // Collapsed + measured wires reset to zero, mirroring
        // QuantumState::measure: remaining state is |00>.
        assert!(
            (d.amplitude(0).re - 1.0).abs() < 1e-15,
            "post amp0 {:?}",
            d.amplitude(0)
        );
        for i in 1..4 {
            assert!(d.amplitude(i).re.abs() < 1e-15 && d.amplitude(i).im.abs() < 1e-15);
        }
    }
    // Seeded state: probabilities match the slow oracle; every draw lands
    // on a supported outcome; post-measurement support + norm are right.
    let n = 5;
    let base = seeded_state(n, 2024);
    let slow = Slow::from_dense(&base);
    let wires = [1usize, 3];
    let sp = slow.prob(&wires);
    let dp = base.probabilities(&wires);
    for i in 0..4 {
        assert!(
            (dp[i] - sp[i]).abs() < 1e-12,
            "prob[{i}] {} vs {}",
            dp[i],
            sp[i]
        );
    }
    for draw in [0.0f64, 0.13, 0.5, 0.87, 0.999_999] {
        let mut d = base.clone();
        let (v, bits) = d.measure(&wires, draw);
        assert_eq!(bits.len(), 2);
        assert!(sp[v] > 0.0, "draw {draw} chose unsupported outcome {v}");
        assert_eq!(bits.as_bytes()[0], if v & 1 == 1 { b'1' } else { b'0' });
        assert_eq!(bits.as_bytes()[1], if v & 2 == 2 { b'1' } else { b'0' });
        // Surviving amplitudes: slow oracle filtered to outcome v, measured
        // bits cleared, scaled by 1/sqrt(p).
        let norm = 1.0 / sp[v].sqrt();
        for i in 0..d.len() {
            let z = d.amplitude(i);
            if ((i >> 1) & 1) == 1 || ((i >> 3) & 1) == 1 {
                assert!(
                    z.re.abs() < 1e-15 && z.im.abs() < 1e-15,
                    "measured wire not reset at {i}"
                );
                continue;
            }
            let src = i | ((v & 1) << 1) | (((v >> 1) & 1) << 3);
            assert!(
                (z.re - slow.a[src].0 * norm).abs() < 1e-12
                    && (z.im - slow.a[src].1 * norm).abs() < 1e-12,
                "collapse amp[{i}]"
            );
        }
        assert!((d.norm2() - 1.0).abs() < 1e-12);
    }
}

#[test]
fn multithreaded_equals_singlethreaded_on_larger_state() {
    let n = 17; // above the threading threshold, still fast
    let run = |threads: usize| {
        let mut d = DenseState::with_threads(n, threads);
        for t in 0..n {
            d.apply1(t, &[], h());
        }
        for t in 0..n - 1 {
            d.cx(t, t + 1);
        }
        d.apply1(n - 1, &[0, 5], u(0.3, 0.8));
        d.phase_if_all(&[0, 8, 16], Complex::new(0.0, 1.0));
        d.swap(0, 16);
        d
    };
    let a = run(1);
    let b = run(4);
    assert_eq!(b.threads(), 4);
    for i in 0..a.len() {
        assert!(
            (a.amplitude(i).re - b.amplitude(i).re).abs() < 1e-12
                && (a.amplitude(i).im - b.amplitude(i).im).abs() < 1e-12,
            "thread mismatch amp[{i}]"
        );
    }
    assert!((a.norm2() - 1.0).abs() < 1e-9);
}

#[test]
fn scalar_fallback_matches_default_path() {
    let build = |simd: bool| {
        let mut d = DenseState::with_threads(12, 1);
        d.set_simd_enabled(simd);
        for t in 0..12 {
            d.apply1(t, &[], u(0.11 * (t as f64 + 1.0), 0.7));
        }
        d.apply1(5, &[], h());
        d
    };
    let a = build(true);
    let b = build(false);
    let mut distance2 = 0.0;
    for i in 0..a.len() {
        distance2 += (a.amplitude(i) - b.amplitude(i)).norm2();
    }
    assert!(
        distance2.sqrt() <= a.error_bound() + b.error_bound(),
        "SIMD/scalar norm distance {}",
        distance2.sqrt()
    );
}

#[test]
fn evidence_is_numerical_never_exhaustive_and_memory_exact() {
    let mut d = DenseState::new(10);
    assert_eq!(d.memory_bytes(), 16 * 1024);
    assert_eq!(d.evidence().kind, "numerical");
    assert_eq!(d.error_bound(), 0.0);
    d.apply1(3, &[], h());
    let ev = d.evidence();
    assert_eq!(ev.kind, "numerical");
    assert!(
        ev.error_bound > 0.0 && ev.error_bound < 1e-12,
        "bound {}",
        ev.error_bound
    );
    assert_eq!(ev.qubits, 10);
    assert_eq!(ev.gates, 1);
    assert_eq!(d.gate_count(), 1);
    d.flip(1, &[3]);
    assert_eq!(d.gate_count(), 2);
    assert_eq!(
        d.evidence().error_bound,
        ev.error_bound,
        "permutation adds no error"
    );
    // Known state: |0000000000> --H(3)--> amplitudes at 0 and 8.
    assert!((d.probabilities(&[3])[1] - 0.5).abs() < 1e-15);
    assert!((d.probabilities(&[1])[1] - 0.5).abs() < 1e-15);
}

#[test]
#[should_panic]
fn zero_threads_panics() {
    let _ = DenseState::with_threads(4, 0);
}

#[test]
#[should_panic]
fn target_out_of_range_panics() {
    DenseState::new(3).apply1(3, &[], h());
}

#[test]
#[should_panic]
fn control_equals_target_panics() {
    DenseState::new(3).apply1(1, &[1], h());
}

#[test]
#[should_panic]
fn draw_out_of_range_panics() {
    DenseState::new(2).measure(&[0], 1.5);
}

#[test]
fn probability_reduction_retains_small_positive_terms() {
    let mut d = DenseState::with_threads(17, 1);
    for i in 1..d.len() {
        d.set_amplitude(i, Complex::new(2.0f64.powi(-30), 0.0));
    }
    let expected = 1.0 + (d.len() - 1) as f64 * 2.0f64.powi(-60);
    assert!((d.norm2() - expected).abs() <= 2.0 * f64::EPSILON);
    assert!((d.probabilities(&[])[0] - expected).abs() <= 2.0 * f64::EPSILON);
}

#[test]
fn measurement_near_one_never_selects_zero_mass() {
    let mut d = DenseState::with_threads(2, 1);
    d.set_amplitude(0, Complex::ZERO);
    d.set_amplitude(1, Complex::new(1.0 - f64::EPSILON, 0.0));
    assert_eq!(d.measure(&[0, 1], 1.0f64.next_down()), (1, "10".into()));
    assert!((d.norm2() - 1.0).abs() <= 4.0 * f64::EPSILON);
}

#[test]
fn measurement_bound_accounts_for_rare_outcome_conditioning() {
    let mut d = DenseState::with_threads(2, 1);
    d.h_gate(0);
    d.ry(1, 2.0e-6);
    let probability = d.probabilities(&[1])[1];
    let before = d.error_bound();
    assert_eq!(d.measure(&[1], 1.0f64.next_down()).0, 1);
    assert!(
        d.error_bound() >= before / probability.sqrt(),
        "conditional bound {} omitted amplification of {before} by 1/sqrt({probability})",
        d.error_bound()
    );
}

#[test]
fn fused_long_sequence_accounts_for_every_gate() {
    let group = vec![
        Gate::One {
            target: 0,
            controls: vec![],
            m: h()
        };
        1000
    ];
    let mut fused = DenseState::with_threads(1, 1);
    let mut seq = fused.clone();
    fused.apply_fused(&group);
    for _ in 0..group.len() {
        seq.h_gate(0);
    }
    assert!(fused.error_bound() >= seq.error_bound());
    for i in 0..fused.len() {
        assert!(
            (fused.amplitude(i).re - seq.amplitude(i).re).abs()
                <= fused.error_bound() + seq.error_bound()
        );
    }
}

#[test]
#[should_panic(expected = "zero or non-finite norm")]
fn measurement_of_zero_state_panics() {
    let mut d = DenseState::with_threads(2, 1);
    d.set_amplitude(0, Complex::ZERO);
    d.measure(&[0], 0.5);
}

#[test]
#[should_panic(expected = "duplicate wire")]
fn measurement_duplicate_wires_panics() {
    DenseState::with_threads(2, 1).measure(&[0, 0], 0.5);
}

#[test]
#[should_panic(expected = "overflow")]
fn huge_qubit_count_does_not_truncate() {
    // Also catches truncation through `n as u32` on 64-bit hosts.
    let n = 1usize.checked_shl(32).unwrap_or(usize::MAX);
    DenseState::with_threads(n, 1);
}

#[test]
fn threaded_measurement_and_all_wire_reset_match_single_thread() {
    let mut one = seeded_state(17, 89723);
    // Use explicit counts regardless of the host CPU count.
    let mut four = DenseState::with_threads(17, 4);
    for i in 0..one.len() {
        four.set_amplitude(i, one.amplitude(i));
    }
    for (wires, draw) in [(&[16usize, 2, 0][..], 0.718), (&[1usize, 15][..], 0.111)] {
        assert_eq!(one.measure(wires, draw), four.measure(wires, draw));
        let mut distance2 = 0.0;
        for i in 0..one.len() {
            let delta = one.amplitude(i) - four.amplitude(i);
            distance2 += delta.norm2();
        }
        assert!(distance2.sqrt() <= one.error_bound() + four.error_bound());
        assert!((four.norm2() - 1.0).abs() < 32.0 * f64::EPSILON);
    }
    let wires: Vec<_> = (0..17).rev().collect();
    four.measure(&wires, 0.921);
    assert!((four.amplitude(0).norm2() - 1.0).abs() < 32.0 * f64::EPSILON);
    assert!((four.norm2() - four.amplitude(0).norm2()).abs() < f64::EPSILON);
    assert!(four.evidence().measurement_error_bound > 0.0);
}

#[test]
fn scattered_and_oversized_fusion_match_independent_oracle() {
    let mut d = seeded_state(17, 7231);
    let mut slow = Slow::from_dense(&d);
    for targets in [vec![0, 10, 16], vec![0, 1, 2, 3, 4, 10, 16]] {
        let mut group: Vec<_> = targets
            .iter()
            .enumerate()
            .map(|(i, &t)| Gate::One {
                target: t,
                controls: if i == 0 { vec![] } else { vec![targets[i - 1]] },
                m: u(0.13 * (i + 1) as f64, -0.7),
            })
            .collect();
        group.push(Gate::Diagonal {
            target: 16,
            controls: vec![0],
            d: [Complex::I, Complex::ONE],
        });
        group.push(Gate::Flip {
            target: 10,
            controls: vec![0, 16],
        });
        d.apply_fused(&group);
        for g in group {
            match g {
                Gate::One {
                    target,
                    controls,
                    m,
                } => slow.apply1(target, &controls, cc(m)),
                Gate::Diagonal {
                    target,
                    controls,
                    d,
                } => slow.apply1(
                    target,
                    &controls,
                    [
                        [(d[0].re, d[0].im), (0.0, 0.0)],
                        [(0.0, 0.0), (d[1].re, d[1].im)],
                    ],
                ),
                Gate::Flip { target, controls } => slow.map_basis(|i| {
                    if controls.iter().all(|&w| i >> w & 1 == 1) {
                        i ^ (1 << target)
                    } else {
                        i
                    }
                }),
            }
        }
        assert_close(&d, &slow, "scattered/oversized fusion");
    }
}

#[test]
#[should_panic(expected = "error bound")]
fn overflowing_error_bound_fails_loudly() {
    let mut d = DenseState::with_threads(1, 1);
    d.apply1(
        0,
        &[],
        [
            [Complex::new(f64::MAX, 0.0), Complex::ZERO],
            [Complex::ZERO, Complex::ONE],
        ],
    );
}

/// Independent slow algorithm using the crate's exact scalar arithmetic.
/// Every input basis contribution goes into a new output vector; this
/// shares neither the dense pair traversal nor QuantumState's map code.
struct ExactOracle(Vec<Exact>);
impl ExactOracle {
    fn apply1(&mut self, target: usize, controls: &[usize], matrix: [[Exact; 2]; 2]) {
        let mut next = vec![Exact::zero(); self.0.len()];
        for (basis, amplitude) in self.0.iter().enumerate() {
            if controls.iter().all(|&w| basis >> w & 1 == 1) {
                let input = basis >> target & 1;
                for (output, row) in matrix.iter().enumerate() {
                    let index = (basis & !(1 << target)) | (output << target);
                    next[index] = next[index].add(&amplitude.mul(&row[input]));
                }
            } else {
                next[basis] = next[basis].add(amplitude);
            }
        }
        self.0 = next;
    }
}

#[test]
fn fused_kernel_families_match_in_test_exact_oracle() {
    let mut d = DenseState::with_threads(4, 1);
    let mut oracle = ExactOracle(vec![Exact::zero(); d.len()]);
    oracle.0[0] = Exact::one();
    let group = [
        Gate::One {
            target: 0,
            controls: vec![],
            m: h(),
        },
        Gate::One {
            target: 2,
            controls: vec![0],
            m: h(),
        },
        Gate::Diagonal {
            target: 2,
            controls: vec![],
            d: [Complex::ONE, Complex::I],
        },
        Gate::Flip {
            target: 3,
            controls: vec![0, 2],
        },
        Gate::One {
            target: 1,
            controls: vec![],
            m: u(0.41, -0.23),
        },
    ];
    for _ in 0..4 {
        d.apply_fused(&group);
        for g in &group {
            let (target, controls, matrix) = match g {
                Gate::One {
                    target,
                    controls,
                    m,
                } => (*target, controls, *m),
                Gate::Diagonal {
                    target,
                    controls,
                    d,
                } => (
                    *target,
                    controls,
                    [[d[0], Complex::ZERO], [Complex::ZERO, d[1]]],
                ),
                Gate::Flip { target, controls } => (
                    *target,
                    controls,
                    [[Complex::ZERO, Complex::ONE], [Complex::ONE, Complex::ZERO]],
                ),
            };
            // Preserve symbolic sqrt(1/2) for the Hadamard fixtures;
            // every other supplied binary64 entry is an exact dyadic.
            let exact = matrix.map(|row| {
                row.map(|z| {
                    if z.im == 0.0 && z.re.abs() == std::f64::consts::FRAC_1_SQRT_2 {
                        let v = Exact::sqrt_half();
                        if z.re < 0.0 { v.neg() } else { v }
                    } else {
                        Exact::pair(z.re, z.im)
                    }
                })
            });
            oracle.apply1(target, controls, exact);
        }
        let mut distance2 = 0.0;
        for (i, amplitude) in oracle.0.iter().enumerate() {
            let (r, im) = amplitude.value();
            distance2 += (d.amplitude(i) - Complex::new(r, im)).norm2();
        }
        let projection = 8.0 * f64::EPSILON * (d.len() as f64).sqrt();
        assert!(distance2.sqrt() <= d.error_bound() + projection);
    }
}
