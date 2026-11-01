//! Benchmark harness for the dense state-vector engine (bennett-vm-bjm).
//! Std-only. Compares a naive interleaved single-thread baseline
//! (written here, independent of the library) against
//! `bennett_vm::dense::DenseState` at several thread counts, reporting
//! gates/s, bytes/amplitude and effective GB/s (32 bytes of traffic
//! per amplitude per gate: one read + one write of 16 bytes). For
//! fused groups this is equivalent traffic, including cache reuse,
//! not a measurement of physical DRAM bandwidth.
//!
//! Usage: `cargo run --release --example dense_bench -- --qubits 20,24,28`
//! Options: `--qubits <list>` (default 20,24), `--reps <n>` (override
//! per-size repetition count), `--no-naive` (skip the baseline),
//! `--targets <list>` (restrict target qubits, useful for sparse n=28 runs).
//! The machine is shared: load average is printed before and after,
//! and the best of up to 3 timed batches is reported.
use bennett_vm::dense::{Complex, DenseState, Gate};
use std::time::Instant;

// Generic unitary (same shape as the test-suite `u` gate).
fn gate_u() -> [[Complex; 2]; 2] {
    let (a, b) = (0.37f64, 1.1f64);
    let (c, s) = (a.cos(), a.sin());
    let p = Complex::new(b.cos(), b.sin());
    [
        [Complex::new(c, 0.0), Complex::new(-s, 0.0)],
        [p.scale(s), p.scale(c)],
    ]
}

/// Naive baseline: interleaved amplitudes, one thread, per-pair index
/// arithmetic, fresh temporaries per pair. Deliberately unsophisticated.
struct Naive {
    a: Vec<[f64; 2]>,
}
impl Naive {
    fn new(n: usize) -> Self {
        let mut a = vec![[0.0, 0.0]; 1usize << n];
        a[0] = [1.0, 0.0];
        Self { a }
    }
    fn apply1(&mut self, target: usize, m: [[Complex; 2]; 2]) {
        let stride = 1usize << target;
        let pairs = self.a.len() / 2;
        for p in 0..pairs {
            let i0 = (p & (stride - 1)) | ((p & !(stride - 1)) << 1);
            let i1 = i0 | stride;
            let (x, y) = (self.a[i0], self.a[i1]);
            self.a[i0] = [
                m[0][0].re * x[0] - m[0][0].im * x[1] + m[0][1].re * y[0] - m[0][1].im * y[1],
                m[0][0].re * x[1] + m[0][0].im * x[0] + m[0][1].re * y[1] + m[0][1].im * y[0],
            ];
            self.a[i1] = [
                m[1][0].re * x[0] - m[1][0].im * x[1] + m[1][1].re * y[0] - m[1][1].im * y[1],
                m[1][0].re * x[1] + m[1][0].im * x[0] + m[1][1].re * y[1] + m[1][1].im * y[0],
            ];
        }
    }
    fn checksum(&self) -> f64 {
        self.a.iter().map(|v| v[0] * v[0] + v[1] * v[1]).sum()
    }
    fn memory_bytes(&self) -> usize {
        self.a.len() * 16
    }
}

fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|_| "unavailable".into())
}

fn best_gates_per_sec<T>(
    state: &mut T,
    reps: usize,
    batches: usize,
    mut one_batch: impl FnMut(&mut T),
    checksum: impl Fn(&T) -> f64,
) -> (f64, f64) {
    // Norm reductions are outside the timed interval. Check every batch,
    // including warmup, rather than allowing an invalid early batch.
    let check = |state: &T| {
        let norm = checksum(state);
        assert!((norm - 1.0).abs() < 1e-6, "bench sanity: norm2={norm}");
        norm
    };
    one_batch(state);
    check(state);
    let mut best = 0.0f64;
    let mut norm = 0.0;
    for _ in 0..batches {
        let t = Instant::now();
        one_batch(state);
        let dt = t.elapsed().as_secs_f64();
        norm = check(state);
        best = best.max(reps as f64 / dt);
    }
    (best, norm)
}

fn reps_for(n: usize) -> (usize, usize) {
    // (gates per batch, batches) keeping total work bounded on a
    // shared machine; 28 qubits is a 4 GiB state, used sparingly.
    match n {
        0..=20 => (50, 3),
        21..=24 => (10, 3),
        25..=27 => (4, 2),
        _ => (2, 1),
    }
}

fn report(
    engine: &str,
    n: usize,
    target: usize,
    threads: usize,
    gates_s: f64,
    bytes: usize,
    checksum: f64,
) {
    let amps = bytes as f64 / 16.0;
    let gbs = gates_s * amps * 32.0 / 1e9;
    println!(
        "{engine:<18} n={n:>2} target={target:>2} threads={threads:>2}  {gates_s:>10.2} gates/s  {} bytes/amp  {gbs:>7.2} GB/s  norm2={checksum:.12} loadavg={}",
        bytes / (1usize << n),
        loadavg()
    );
    assert!(
        (checksum - 1.0).abs() < 1e-6,
        "bench sanity: norm2 drifted to {checksum} for {engine} n={n} target={target}"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut qubits: Vec<usize> = vec![20, 24];
    let mut reps_override: Option<usize> = None;
    let mut naive = true;
    let mut targets_override: Option<Vec<usize>> = None;
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--qubits" => {
                let v = it.next().expect("--qubits needs a value");
                qubits = v
                    .split(',')
                    .map(|s| s.parse().expect("qubit count"))
                    .collect();
            }
            "--reps" => {
                reps_override = Some(
                    it.next()
                        .expect("--reps needs a value")
                        .parse()
                        .expect("reps"),
                )
            }
            "--no-naive" => naive = false,
            "--targets" => {
                targets_override = Some(
                    it.next()
                        .expect("--targets needs a value")
                        .split(',')
                        .map(|s| s.parse().expect("target qubit"))
                        .collect(),
                )
            }
            other => panic!("unknown argument {other}"),
        }
    }
    let all_threads = std::thread::available_parallelism()
        .map(|v| v.get())
        .unwrap_or(1);
    println!("dense_bench: loadavg at start: {}", loadavg());
    println!("dense_bench: available_parallelism = {all_threads}");
    let m = gate_u();

    for &n in &qubits {
        let (reps0, batches) = reps_for(n);
        let reps = reps_override.unwrap_or(reps0);
        assert!(
            n >= 4 && n < (usize::BITS - 4) as usize,
            "benchmark qubits must be in 4..usize::BITS-4"
        );
        assert!(reps > 0, "benchmark repetitions must be positive");
        let targets = targets_override
            .clone()
            .unwrap_or_else(|| vec![0usize, n / 2, n - 1]);
        assert!(targets.iter().all(|&t| t < n), "target out of range");
        println!(
            "-- n = {n} ({} amplitudes, {} MiB state), reps/batch = {reps}, batches = {batches}",
            1usize << n,
            (16usize << n) >> 20
        );

        if naive {
            for &t in &targets {
                let mut st = Naive::new(n);
                // Put the state in superposition so no branch is trivial.
                st.apply1(n - 1, gate_u());
                let bytes = st.memory_bytes();
                let (rate, chk) = best_gates_per_sec(
                    &mut st,
                    reps,
                    batches,
                    |st| {
                        for _ in 0..reps {
                            st.apply1(t, m);
                        }
                    },
                    Naive::checksum,
                );
                report("naive-AoS", n, t, 1, rate, bytes, chk);
                drop(st);
            }
        }

        let thread_counts: Vec<usize> = if all_threads > 1 {
            vec![1, all_threads]
        } else {
            vec![1]
        };
        for threads in thread_counts.iter().copied() {
            for &t in &targets {
                let mut st = DenseState::with_threads(n, threads);
                st.apply1(n - 1, &[], m);
                let bytes = st.memory_bytes();
                let (rate, chk) = best_gates_per_sec(
                    &mut st,
                    reps,
                    batches,
                    |st| {
                        for _ in 0..reps {
                            st.apply1(t, &[], m);
                        }
                    },
                    DenseState::norm2,
                );
                report(
                    if st.simd_active() {
                        "dense-SoA+AVX2"
                    } else {
                        "dense-SoA"
                    },
                    n,
                    t,
                    threads,
                    rate,
                    bytes,
                    chk,
                );
                drop(st);
            }
            if threads == 1 {
                // Scalar fallback rate, for the SIMD comparison.
                let t = n - 1;
                let mut st = DenseState::with_threads(n, 1);
                st.set_simd_enabled(false);
                st.apply1(n - 1, &[], m);
                let bytes = st.memory_bytes();
                let (rate, chk) = best_gates_per_sec(
                    &mut st,
                    reps,
                    batches,
                    |st| {
                        for _ in 0..reps {
                            st.apply1(t, &[], m);
                        }
                    },
                    DenseState::norm2,
                );
                report("dense-SoA-scalar", n, t, 1, rate, bytes, chk);
                drop(st);
            }
        }

        // Fused group: 8 gates on the lowest 4 qubits in one pass,
        // counted as 8 gates. Plus the same gates applied sequentially
        // for comparison.
        let group: Vec<Gate> = (0..8)
            .map(|i| Gate::One {
                target: i % 4,
                controls: vec![],
                m: gate_u(),
            })
            .collect();
        let thread_counts: Vec<usize> = if all_threads > 1 {
            vec![1, all_threads]
        } else {
            vec![1]
        };
        for threads in thread_counts.iter().copied() {
            let mut fused = DenseState::with_threads(n, threads);
            fused.apply1(n - 1, &[], m);
            let bytes = fused.memory_bytes();
            let (rate, chk) = best_gates_per_sec(
                &mut fused,
                reps * group.len(),
                batches,
                |st| {
                    for _ in 0..reps {
                        st.apply_fused(&group);
                    }
                },
                DenseState::norm2,
            );
            report("dense-fused8", n, 3, threads, rate, bytes, chk);
            // Keep only a bounded sample before dropping the 4 GiB
            // state at n=28; fused and sequential never coexist.
            let sample: Vec<_> = (0..fused.len())
                .step_by((fused.len() / 1024).max(1))
                .map(|i| (i, fused.amplitude(i)))
                .collect();
            let bound = fused.error_bound();
            drop(fused);
            let mut seq = DenseState::with_threads(n, threads);
            seq.apply1(n - 1, &[], m);
            let (rate, chk) = best_gates_per_sec(
                &mut seq,
                reps * group.len(),
                batches,
                |st| {
                    for _ in 0..reps {
                        for g in &group {
                            if let Gate::One { target, m, .. } = g {
                                st.apply1(*target, &[], *m);
                            }
                        }
                    }
                },
                DenseState::norm2,
            );
            report("dense-seq8", n, 3, threads, rate, bytes, chk);
            // Fused and sequential must agree (bench doubles as a check).
            for (i, x) in sample {
                let y = seq.amplitude(i);
                assert!(
                    (x.re - y.re).hypot(x.im - y.im) <= bound + seq.error_bound(),
                    "fused vs sequential mismatch at amp[{i}]"
                );
            }
            drop(seq);
        }
    }
    println!("dense_bench: loadavg at end: {}", loadavg());
}
