//! Dense state-vector simulation engine (bead bennett-vm-bjm).
//!
//! Design: `docs/design/dense-engine.md`. Summary: `2^n` binary64
//! amplitudes in SoA layout (`re[]`, `im[]`, 16 bytes/amplitude, one
//! state vector + O(1) scratch), in-place qHiPSTER-style pair kernels,
//! gate fusion, permutation and diagonal fast paths, static-partition
//! `std::thread` parallelism, AVX2/FMA with a tested scalar fallback.
//!
//! Qubit `q` is bit `q` of the basis index, matching
//! `crate::action::QuantumState`. Semantics of `apply1`, `flip`,
//! `phase_if_all`, `swap` and `measure` mirror that exact engine; this
//! engine's evidence kind is always `"numerical"` with the accumulated
//! floating-point bound from `error_bound()`, never `"exhaustive"`.
mod runtime;
pub(crate) use runtime::Runtime;
/// Binary64 unit roundoff, `2^-53`.
const U: f64 = 1.110_223_024_625_156_5e-16;
/// States smaller than this run single-threaded (spawn cost dominates).
const THREAD_MIN_ITEMS: usize = 1 << 14;
/// Maximum involved wires in one fused group (local vector <=64).
const FUSE_MAX_WIRES: usize = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };
    pub const ONE: Self = Self { re: 1.0, im: 0.0 };
    pub const I: Self = Self { re: 0.0, im: 1.0 };

    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    /// `exp(i * theta)`.
    pub fn exp_i(theta: f64) -> Self {
        let (s, c) = theta.sin_cos();
        Self::new(c, s)
    }
    pub fn scale(self, k: f64) -> Self {
        Self::new(self.re * k, self.im * k)
    }
    pub fn norm2(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
}

impl std::ops::Add for Complex {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self::new(self.re + b.re, self.im + b.im)
    }
}
impl std::ops::Sub for Complex {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        Self::new(self.re - b.re, self.im - b.im)
    }
}
impl std::ops::Mul for Complex {
    type Output = Self;
    fn mul(self, b: Self) -> Self {
        Self::new(
            self.re * b.re - self.im * b.im,
            self.re * b.im + self.im * b.re,
        )
    }
}
impl std::ops::Neg for Complex {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.re, -self.im)
    }
}

/// One elementary gate, for fused application via `apply_fused`.
#[derive(Clone, Debug)]
pub enum Gate {
    One {
        target: usize,
        controls: Vec<usize>,
        m: [[Complex; 2]; 2],
    },
    Diagonal {
        target: usize,
        controls: Vec<usize>,
        d: [Complex; 2],
    },
    Flip {
        target: usize,
        controls: Vec<usize>,
    },
}

/// Evidence for a dense run: always "numerical", never "exhaustive".
/// Any epsilon = 0 claim can only come from the exact engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Evidence {
    pub kind: &'static str,
    pub error_bound: f64,
    pub qubits: usize,
    pub gates: u64,
    /// Accumulated bound on measurement distribution error (total variation).
    /// The state error after measurement is conditional on its returned outcome.
    pub measurement_error_bound: f64,
}

/// Raw `f64` pointer shared between worker threads.
///
/// Safety: every kernel partitions work into pairwise-disjoint index
/// sets (amplitude pairs, permutation pairs, index ranges) before
/// handing ranges to threads, and each thread only dereferences
/// indices in its own set while the `&mut DenseState` borrow is live.
#[derive(Clone, Copy)]
struct Ptr(*mut f64);
// SAFETY: see type documentation; disjointness is on each kernel.
unsafe impl Send for Ptr {}
// SAFETY: see type documentation; disjointness is on each kernel.
unsafe impl Sync for Ptr {}
impl Ptr {
    /// # Safety
    /// `i` must be in bounds and exclusively owned by the caller.
    unsafe fn get(self, i: usize) -> f64 {
        // SAFETY: caller guarantees per type documentation.
        unsafe { *self.0.add(i) }
    }
    /// # Safety
    /// `i` must be in bounds and exclusively owned by the caller.
    unsafe fn set(self, i: usize, v: f64) {
        // SAFETY: caller guarantees per type documentation.
        unsafe { *self.0.add(i) = v }
    }
}

/// Run `f(lo, hi)` over a static contiguous partition of `0..total`.
fn run_ranges(total: usize, threads: usize, f: impl Fn(usize, usize) + Sync) {
    if total == 0 {
        return;
    }
    let t = threads.min(total);
    if t <= 1 {
        f(0, total);
        return;
    }
    let chunk = total.div_ceil(t);
    std::thread::scope(|s| {
        let mut lo = 0;
        while lo < total {
            let hi = (lo + chunk).min(total);
            let f = &f;
            s.spawn(move || f(lo, hi));
            lo = hi;
        }
    });
}

/// Insert a 0 bit at position `target` in pair index `p`.
fn pair_index(p: usize, stride: usize) -> usize {
    (p & (stride - 1)) | ((p & !(stride - 1)) << 1)
}

#[derive(Clone, Debug)]
pub struct DenseState {
    n: usize,
    re: Vec<f64>,
    im: Vec<f64>,
    threads: usize,
    gates: u64,
    bound: f64,
    norm_upper: f64,
    measurement_bound: f64,
    simd: bool,
}

impl DenseState {
    /// `|0...0>` on `n` qubits, threads from `available_parallelism()`.
    pub fn new(n: usize) -> Self {
        let threads = std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(1);
        Self::with_threads(n, threads)
    }

    /// `|0...0>` on `n` qubits with an explicit worker thread count.
    ///
    /// # Panics
    /// If `threads == 0` or `2^n` does not fit an index (fail loud).
    pub fn with_threads(n: usize, threads: usize) -> Self {
        assert!(threads >= 1, "dense: thread count must be >= 1, got 0");
        assert!(
            n < usize::BITS as usize,
            "dense: 2^{n} amplitudes overflow usize indexing"
        );
        let size = 1usize
            .checked_shl(n as u32)
            .unwrap_or_else(|| panic!("dense: 2^{n} amplitudes overflow usize indexing"));
        assert!(
            size.checked_mul(16).is_some(),
            "dense: state of 2^{n} amplitudes overflows the memory bound"
        );
        let mut st = Self {
            n,
            re: vec![0.0; size],
            im: vec![0.0; size],
            threads,
            gates: 0,
            bound: 0.0,
            norm_upper: 1.0,
            measurement_bound: 0.0,
            simd: true,
        };
        if threads > 1 && size >= THREAD_MIN_ITEMS {
            // First-touch: fault the zero pages in from the same static
            // partition the kernels use (vec![0.0] may be untouched
            // calloc pages).
            let (re, im) = (Ptr(st.re.as_mut_ptr()), Ptr(st.im.as_mut_ptr()));
            run_ranges(size, threads, move |lo, hi| {
                for i in lo..hi {
                    // SAFETY: index ranges are disjoint by partition.
                    unsafe { re.set(i, 0.0) };
                    // SAFETY: index ranges are disjoint by partition.
                    unsafe { im.set(i, 0.0) };
                }
            });
        }
        st.re[0] = 1.0;
        st
    }

    pub fn qubits(&self) -> usize {
        self.n
    }
    /// Number of amplitudes, `2^qubits`.
    pub fn len(&self) -> usize {
        self.re.len()
    }
    pub fn is_empty(&self) -> bool {
        self.re.is_empty()
    }
    pub fn threads(&self) -> usize {
        self.threads
    }
    pub fn gate_count(&self) -> u64 {
        self.gates
    }
    /// Exact state allocation: 16 bytes per amplitude (binary64 SoA).
    pub fn memory_bytes(&self) -> usize {
        (self.re.len() + self.im.len()) * 8
    }
    /// Running a priori 2-norm forward error bound (see design doc).
    pub fn error_bound(&self) -> f64 {
        self.bound
    }
    pub fn evidence(&self) -> Evidence {
        Evidence {
            kind: "numerical",
            error_bound: self.bound,
            qubits: self.n,
            gates: self.gates,
            measurement_error_bound: self.measurement_bound,
        }
    }
    pub fn amplitude(&self, i: usize) -> Complex {
        Complex::new(self.re[i], self.im[i])
    }
    /// Overwrite one amplitude during input preparation. Does not
    /// renormalise: error accounting assumes normalized prepared input.
    /// State preparation is exact input, outside the gate error budget.
    pub fn set_amplitude(&mut self, i: usize, v: Complex) {
        assert!(
            v.re.is_finite() && v.im.is_finite(),
            "dense: non-finite prepared amplitude"
        );
        self.re[i] = v.re;
        self.im[i] = v.im;
    }
    pub fn set_simd_enabled(&mut self, on: bool) {
        self.simd = on;
    }
    /// Whether the next eligible kernel would take the SIMD path.
    pub fn simd_active(&self) -> bool {
        self.simd && avx2_fma_detected()
    }

    /// Sum of |amplitude|^2 (balanced reduction, fixed chunk order).
    pub fn norm2(&self) -> f64 {
        self.reduce_probabilities(0, 0, 0)[0]
    }

    fn workers(&self) -> usize {
        if self.len() < THREAD_MIN_ITEMS {
            1
        } else {
            self.threads.min(self.len())
        }
    }
    fn charge(&mut self, amplification: f64, operations: usize) {
        // The gate's spectral-norm bound accounts for binary64 entries
        // which are not perfectly unitary, and for general linear input.
        assert!(
            amplification.is_finite(),
            "dense: error bound overflow in gate norm"
        );
        // A conservative absolute allowance also covers gradual
        // underflow (the relative-error model alone does not).
        let absolute = operations as f64 * f64::MIN_POSITIVE * (self.len() as f64).sqrt();
        self.bound = (amplification * self.bound
            + gamma(operations) * amplification * (self.norm_upper + self.bound)
            + absolute)
            .next_up();
        self.norm_upper = (amplification * self.norm_upper).next_up();
        assert!(
            self.bound.is_finite() && self.norm_upper.is_finite(),
            "dense: error bound overflow"
        );
    }
    fn check_wires(&self, wires: &[usize]) {
        for (i, &w) in wires.iter().enumerate() {
            self.check_wire(w);
            assert!(!wires[..i].contains(&w), "dense: duplicate wire {w}");
        }
    }
    fn check_wire(&self, w: usize) {
        assert!(
            w < self.n,
            "dense: wire {w} out of range for {} qubits",
            self.n
        );
    }
    fn check_controls(&self, target: usize, controls: &[usize]) {
        self.check_wire(target);
        for (j, &c) in controls.iter().enumerate() {
            self.check_wire(c);
            assert!(
                c != target,
                "dense: control wire {c} equals target wire {target}"
            );
            assert!(
                !controls[..j].contains(&c),
                "dense: duplicate control wire {c}"
            );
        }
    }
    fn check_finite(&self, m: &[[Complex; 2]; 2]) {
        for row in m {
            for z in row {
                assert!(
                    z.re.is_finite() && z.im.is_finite(),
                    "dense: non-finite gate matrix entry ({}, {})",
                    z.re,
                    z.im
                );
            }
        }
    }
    fn ptrs(&mut self) -> (Ptr, Ptr) {
        (Ptr(self.re.as_mut_ptr()), Ptr(self.im.as_mut_ptr()))
    }
}

impl DenseState {
    /// Controlled single-qubit matrix `[[a,b],[c,d]]` on `target`,
    /// applied where every control wire is 1 (mirrors
    /// `QuantumState::apply1`). In place, one pass.
    pub fn apply1(&mut self, target: usize, controls: &[usize], m: [[Complex; 2]; 2]) {
        self.check_controls(target, controls);
        self.check_finite(&m);
        let stride = 1usize << target;
        let pairs = self.len() / 2;
        let cmask: usize = controls.iter().fold(0, |acc, &c| acc | (1usize << c));
        let (a, b, c, d) = (m[0][0], m[0][1], m[1][0], m[1][1]);
        let (re, im) = self.ptrs();
        let threads = self.workers();
        if controls.is_empty() && self.simd_active() && stride < 4 && self.len() >= 4 {
            #[cfg(target_arch = "x86_64")]
            run_ranges(self.len() / 4, threads, move |lo, hi| {
                // SAFETY: every four-lane group contains whole pairs
                // for either low wire, and groups are disjoint.
                unsafe { apply1_avx2_low(re, im, lo * 4, (hi - lo) * 4, stride, m) };
            });
            #[cfg(not(target_arch = "x86_64"))]
            unreachable!("dense: AVX2 detected on a non-x86_64 target");
        } else if controls.is_empty() && self.simd_active() && stride >= 4 {
            // AVX2 over groups of 4 consecutive pairs; stride is a
            // power of two >= 4, so a group never crosses a run edge.
            // SAFETY: pair groups are disjoint by partition.
            #[cfg(target_arch = "x86_64")]
            run_ranges(pairs / 4, threads, move |lo, hi| {
                for g in lo..hi {
                    let i0 = pair_index(g * 4, stride);
                    // SAFETY: avx2+fma just detected; indices disjoint.
                    unsafe { apply1_avx2_x4(re, im, i0, stride, a, b, c, d) };
                }
            });
            #[cfg(not(target_arch = "x86_64"))]
            unreachable!("dense: AVX2 detected on a non-x86_64 target");
        } else {
            // Scalar kernel over contiguous runs (the lower half of
            // each 2*stride block, paired with the upper half). Only
            // valid as a run loop when no control sits below the
            // target (then the control test is constant per run);
            // otherwise test per pair.
            let high_controls_only = controls.iter().all(|&cw| cw > target);
            run_ranges(pairs, threads, move |lo, hi| {
                if high_controls_only && stride > 1 {
                    let mut p = lo;
                    while p < hi {
                        let cnt = (stride - (p & (stride - 1))).min(hi - p);
                        let i0 = pair_index(p, stride);
                        if i0 & cmask == cmask {
                            // SAFETY: pairs in [lo,hi) are disjoint.
                            unsafe { apply1_run(re, im, i0, stride, cnt, a, b, c, d) };
                        }
                        p += cnt;
                    }
                } else {
                    for p in lo..hi {
                        let i0 = pair_index(p, stride);
                        if i0 & cmask != cmask {
                            continue;
                        }
                        let i1 = i0 | stride;
                        // SAFETY: amplitude pairs are disjoint.
                        unsafe {
                            let (x0, x1) = (re.get(i0), im.get(i0));
                            let (y0, y1) = (re.get(i1), im.get(i1));
                            re.set(i0, a.re * x0 - a.im * x1 + b.re * y0 - b.im * y1);
                            im.set(i0, a.re * x1 + a.im * x0 + b.re * y1 + b.im * y0);
                            re.set(i1, c.re * x0 - c.im * x1 + d.re * y0 - d.im * y1);
                            im.set(i1, c.re * x1 + c.im * x0 + d.re * y1 + d.im * y0);
                        }
                    }
                }
            });
        }
        self.gates += 1;
        self.charge(matrix_norm_upper(&m), 32);
    }

    /// Diagonal 1-qubit gate `diag(d0, d1)` under all-ones controls.
    pub fn diagonal1(&mut self, target: usize, controls: &[usize], d0: Complex, d1: Complex) {
        self.check_controls(target, controls);
        assert!(
            d0.re.is_finite() && d0.im.is_finite() && d1.re.is_finite() && d1.im.is_finite(),
            "dense: non-finite diagonal entry"
        );
        let bit = 1usize << target;
        let cmask: usize = controls.iter().fold(0, |acc, &c| acc | (1usize << c));
        let size = self.len();
        let (re, im) = self.ptrs();
        run_ranges(size, self.workers(), move |lo, hi| {
            for i in lo..hi {
                if i & cmask != cmask {
                    continue;
                }
                let p = if i & bit == 0 { d0 } else { d1 };
                if p == Complex::ONE {
                    continue;
                }
                // SAFETY: index ranges are disjoint by partition.
                unsafe {
                    let (x0, x1) = (re.get(i), im.get(i));
                    re.set(i, p.re * x0 - p.im * x1);
                    im.set(i, p.re * x1 + p.im * x0);
                }
            }
        });
        self.gates += 1;
        self.charge(phase_norm_upper(d0).max(phase_norm_upper(d1)), 8);
    }

    /// Multiply amplitudes with every wire in `wires` equal to 1 by
    /// `phase` (mirrors `QuantumState::phase_if_all`).
    pub fn phase_if_all(&mut self, wires: &[usize], phase: Complex) {
        for &w in wires {
            self.check_wire(w);
        }
        assert!(
            phase.re.is_finite() && phase.im.is_finite(),
            "dense: non-finite phase"
        );
        let mask: usize = wires.iter().fold(0, |acc, &w| acc | (1usize << w));
        let size = self.len();
        let (re, im) = self.ptrs();
        run_ranges(size, self.workers(), move |lo, hi| {
            for i in lo..hi {
                if i & mask != mask {
                    continue;
                }
                // SAFETY: index ranges are disjoint by partition.
                unsafe {
                    let (x0, x1) = (re.get(i), im.get(i));
                    re.set(i, phase.re * x0 - phase.im * x1);
                    im.set(i, phase.re * x1 + phase.im * x0);
                }
            }
        });
        self.gates += 1;
        self.charge(phase_norm_upper(phase), 8);
    }

    /// Flip `target` where every control is 1 (permutation gate:
    /// `x` with no controls, `cx`, `ccx`, `mcx`). Exact: no complex
    /// arithmetic, no error charge.
    pub fn flip(&mut self, target: usize, controls: &[usize]) {
        self.check_controls(target, controls);
        let stride = 1usize << target;
        let pairs = self.len() / 2;
        let cmask: usize = controls.iter().fold(0, |acc, &c| acc | (1usize << c));
        let (re, im) = self.ptrs();
        run_ranges(pairs, self.workers(), move |lo, hi| {
            for p in lo..hi {
                let i0 = pair_index(p, stride);
                if i0 & cmask != cmask {
                    continue;
                }
                let i1 = i0 | stride;
                // SAFETY: amplitude pairs are disjoint by partition.
                unsafe {
                    let t = re.get(i0);
                    re.set(i0, re.get(i1));
                    re.set(i1, t);
                    let t = im.get(i0);
                    im.set(i0, im.get(i1));
                    im.set(i1, t);
                }
            }
        });
        self.gates += 1;
    }

    /// Swap qubits `a` and `b` (permutation, exact).
    pub fn swap(&mut self, a: usize, b: usize) {
        self.check_wire(a);
        self.check_wire(b);
        self.gates += 1;
        if a == b {
            return;
        }
        let (ba, bb) = (1usize << a, 1usize << b);
        let size = self.len();
        let (re, im) = self.ptrs();
        // Source indices have bit a = 0, bit b = 1; each pair is
        // touched exactly once and pairs are disjoint.
        run_ranges(size, self.workers(), move |lo, hi| {
            for i in lo..hi {
                if i & ba == 0 && i & bb != 0 {
                    let j = i ^ ba ^ bb;
                    // SAFETY: swap pairs are disjoint (Ptr contract).
                    unsafe {
                        let t = re.get(i);
                        re.set(i, re.get(j));
                        re.set(j, t);
                        let t = im.get(i);
                        im.set(i, im.get(j));
                        im.set(j, t);
                    }
                }
            }
        });
    }
}

impl DenseState {
    // Named gates matching the exact engine's alphabet and conventions
    // (src/action.rs gate dispatch, src/exact.rs Exact::rotation).
    pub fn x_gate(&mut self, t: usize) {
        self.flip(t, &[]);
    }
    pub fn cx(&mut self, control: usize, target: usize) {
        self.flip(target, &[control]);
    }
    pub fn h_gate(&mut self, t: usize) {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        self.apply1(
            t,
            &[],
            [
                [Complex::new(s, 0.0), Complex::new(s, 0.0)],
                [Complex::new(s, 0.0), Complex::new(-s, 0.0)],
            ],
        );
    }
    pub fn z_gate(&mut self, t: usize) {
        self.diagonal1(t, &[], Complex::ONE, Complex::new(-1.0, 0.0));
    }
    pub fn s_gate(&mut self, t: usize) {
        self.diagonal1(t, &[], Complex::ONE, Complex::I);
    }
    pub fn sdg_gate(&mut self, t: usize) {
        self.diagonal1(t, &[], Complex::ONE, Complex::new(0.0, -1.0));
    }
    pub fn t_gate(&mut self, t: usize) {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        self.diagonal1(t, &[], Complex::ONE, Complex::new(s, s));
    }
    pub fn tdg_gate(&mut self, t: usize) {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        self.diagonal1(t, &[], Complex::ONE, Complex::new(s, -s));
    }
    pub fn cz(&mut self, a: usize, b: usize) {
        self.phase_if_all(&[a, b], Complex::new(-1.0, 0.0));
    }
    /// `rx(theta) = [[c, -i s], [-i s, c]]`, `c = cos(theta/2)`.
    pub fn rx(&mut self, t: usize, theta: f64) {
        let (s, c) = (theta / 2.0).sin_cos();
        self.apply1(
            t,
            &[],
            [
                [Complex::new(c, 0.0), Complex::new(0.0, -s)],
                [Complex::new(0.0, -s), Complex::new(c, 0.0)],
            ],
        );
    }
    /// `ry(theta) = [[c, -s], [s, c]]`, `c = cos(theta/2)`.
    pub fn ry(&mut self, t: usize, theta: f64) {
        let (s, c) = (theta / 2.0).sin_cos();
        self.apply1(
            t,
            &[],
            [
                [Complex::new(c, 0.0), Complex::new(-s, 0.0)],
                [Complex::new(s, 0.0), Complex::new(c, 0.0)],
            ],
        );
    }
    /// `rz(theta) = diag(e^{-i theta/2}, e^{+i theta/2})`.
    pub fn rz(&mut self, t: usize, theta: f64) {
        self.diagonal1(
            t,
            &[],
            Complex::exp_i(-theta / 2.0),
            Complex::exp_i(theta / 2.0),
        );
    }

    /// XOR oracle by branch enumeration: `|x>|y> -> |x>|y XOR f(x)>`,
    /// where `x` is read from `input_wires` (list order = bit order,
    /// first wire = bit 0) and the result is XORed into
    /// `target_wires` the same way. `f` is evaluated exactly once per
    /// distinct projected input (memoised for the call). Exact
    /// permutation: no error charge.
    pub fn apply_xor_oracle(
        &mut self,
        input_wires: &[usize],
        target_wires: &[usize],
        mut f: impl FnMut(usize) -> usize,
    ) {
        self.check_wires(input_wires);
        self.check_wires(target_wires);
        for &w in target_wires {
            assert!(
                !input_wires.contains(&w),
                "dense: oracle target wire {w} is also an input wire"
            );
        }
        let n_in = 1usize
            .checked_shl(input_wires.len() as u32)
            .unwrap_or_else(|| panic!("dense: oracle input space too wide"));
        let tout_max = if target_wires.len() >= usize::BITS as usize {
            usize::MAX
        } else {
            (1usize << target_wires.len()) - 1
        };
        let mut table = Vec::with_capacity(n_in);
        for x in 0..n_in {
            let y = f(x);
            assert!(
                y & !tout_max == 0,
                "dense: oracle output {y} does not fit {} target wires",
                target_wires.len()
            );
            table.push(y);
        }
        let in_w: Vec<usize> = input_wires.to_vec();
        let tg_w: Vec<usize> = target_wires.to_vec();
        let size = self.len();
        let (re, im) = self.ptrs();
        // Partner of i keeps the input bits and XORs f(x) into the
        // target bits: an involution, so disjoint pair swaps.
        run_ranges(size, self.workers(), move |lo, hi| {
            for i in lo..hi {
                let mut x = 0usize;
                for (b, &w) in in_w.iter().enumerate() {
                    x |= ((i >> w) & 1) << b;
                }
                let y = table[x];
                if y == 0 {
                    continue;
                }
                let mut j = i;
                for (b, &w) in tg_w.iter().enumerate() {
                    if (y >> b) & 1 == 1 {
                        j ^= 1usize << w;
                    }
                }
                if i < j {
                    // SAFETY: oracle pairs are disjoint (Ptr contract).
                    unsafe {
                        let t = re.get(i);
                        re.set(i, re.get(j));
                        re.set(j, t);
                        let t = im.get(i);
                        im.set(i, im.get(j));
                        im.set(j, t);
                    }
                }
            }
        });
        self.gates += 1;
    }
}

fn gamma(operations: usize) -> f64 {
    let ku = operations as f64 * U;
    (ku / (1.0 - ku)).next_up()
}

// Bound ||M||_2 through the row-sum norm of the 2x2 Gram matrix.
// The additive margin bounds rounding, including cancellation in
// off-diagonal inner products. Identity control sectors have norm 1.
fn matrix_norm_upper(m: &[[Complex; 2]; 2]) -> f64 {
    let sq = |z: Complex| z.re * z.re + z.im * z.im;
    let g0 = sq(m[0][0]) + sq(m[1][0]);
    let g1 = sq(m[0][1]) + sq(m[1][1]);
    if !g0.is_finite() || !g1.is_finite() {
        return f64::INFINITY;
    }
    let inner =
        |a: Complex, b: Complex| Complex::new(a.re * b.re + a.im * b.im, a.re * b.im - a.im * b.re);
    let off = inner(m[0][0], m[0][1]) + inner(m[1][0], m[1][1]);
    ((g0.max(g1) + off.re.hypot(off.im) + gamma(32) * (g0 + g1)).sqrt() * (1.0 + gamma(4)))
        .next_up()
        .max(1.0)
}

fn phase_norm_upper(z: Complex) -> f64 {
    (z.re.hypot(z.im) * (1.0 + gamma(4))).next_up().max(1.0)
}

fn sum_partials(parts: &[[f64; 2]]) -> [f64; 2] {
    if parts.len() == 1 {
        return parts[0];
    }
    let mid = parts.len() / 2;
    let a = sum_partials(&parts[..mid]);
    let b = sum_partials(&parts[mid..]);
    [a[0] + b[0], a[1] + b[1]]
}

fn sum_split(
    re: &[f64],
    im: &[f64],
    start: usize,
    mask: usize,
    value: usize,
    bit: usize,
) -> [f64; 2] {
    if re.len() <= 32 {
        let mut out = [0.0; 2];
        for i in 0..re.len() {
            let index = start + i;
            if index & mask == value {
                out[usize::from(index & bit != 0)] += re[i] * re[i] + im[i] * im[i];
            }
        }
        return out;
    }
    let mid = re.len() / 2;
    let a = sum_split(&re[..mid], &im[..mid], start, mask, value, bit);
    let b = sum_split(&re[mid..], &im[mid..], start + mid, mask, value, bit);
    [a[0] + b[0], a[1] + b[1]]
}

// Power-of-two slices let fixed outcome bits prune entire subtrees.
// Across all outcome bins, the leaves visit each amplitude once.
fn sum_outcome(re: &[f64], im: &[f64], mask: usize, value: usize) -> f64 {
    if re.len() == 1 {
        return re[0] * re[0] + im[0] * im[0];
    }
    let mid = re.len() / 2;
    if mask & mid != 0 {
        if value & mid == 0 {
            sum_outcome(&re[..mid], &im[..mid], mask, value)
        } else {
            sum_outcome(&re[mid..], &im[mid..], mask, value)
        }
    } else {
        sum_outcome(&re[..mid], &im[..mid], mask, value)
            + sum_outcome(&re[mid..], &im[mid..], mask, value)
    }
}

impl DenseState {
    // A pair of probabilities for a prefix and its next wire. Each
    // leaf sums at most 32 entries, then all sums form a balanced tree.
    // Scratch is O(workers * log(amplitudes)), independent of outcomes.
    fn reduce_probabilities(&self, mask: usize, value: usize, bit: usize) -> [f64; 2] {
        if self.workers() == 1 {
            return sum_split(&self.re, &self.im, 0, mask, value, bit);
        }
        let chunk = self.len().div_ceil(self.workers());
        let parts: Vec<[f64; 2]> = std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .re
                .chunks(chunk)
                .zip(self.im.chunks(chunk))
                .enumerate()
                .map(|(i, (re, im))| {
                    scope.spawn(move || sum_split(re, im, i * chunk, mask, value, bit))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("dense: probability worker panicked"))
                .collect()
        });
        sum_partials(&parts)
    }

    /// Born probabilities indexed by outcome value (bit b = wires[b]).
    /// This explicit diagnostic allocates its returned outcome table;
    /// `measure` does not allocate a table or a second state vector.
    pub fn probabilities(&self, wires: &[usize]) -> Vec<f64> {
        self.check_wires(wires);
        let size = 1usize << wires.len();
        let mask = wires.iter().fold(0, |m, &w| m | (1usize << w));
        let mut out = vec![0.0; size];
        let ptr = Ptr(out.as_mut_ptr());
        run_ranges(size, self.workers(), |lo, hi| {
            for v in lo..hi {
                let bits = wires
                    .iter()
                    .enumerate()
                    .fold(0, |m, (b, &w)| m | (((v >> b) & 1) << w));
                let p = sum_outcome(&self.re, &self.im, mask, bits);
                // SAFETY: each output bin has exactly one worker.
                unsafe { ptr.set(v, p) };
            }
        });
        out
    }

    /// Measure in lexicographic wire-list order, collapse, and reset
    /// measured wires to zero, as in `QuantumState::measure`.
    /// Randomness and K17 draw ordering belong to the caller.
    ///
    /// Balanced prefix reductions sample without an exponential outcome
    /// table. Draws are normalised to the computed positive mass, so
    /// norm drift cannot select a zero-probability outcome near draw=1.
    /// The post-measurement state bound is conditional on this outcome;
    /// evidence also reports measurement distribution uncertainty.
    pub fn measure(&mut self, wires: &[usize], draw: f64) -> (usize, String) {
        self.measure_projected(wires, draw, &[])
    }

    /// `measure` with optional per-wire branch error bounds:
    /// `branch[b][v]` bounds the norm of the state error restricted to
    /// `wires[b] == v`. The projection onto the chosen outcome then has
    /// error at most `min(B, min_b branch[b][outcome_b])`, which replaces
    /// B in the conditional-state bound. Empty `branch` uses B alone.
    pub(crate) fn measure_projected(
        &mut self,
        wires: &[usize],
        draw: f64,
        branch: &[[f64; 2]],
    ) -> (usize, String) {
        assert!(
            branch.is_empty() || branch.len() == wires.len(),
            "dense: measurement branch bounds do not match the measured wires"
        );
        self.check_wires(wires);
        assert!(
            (0.0..1.0).contains(&draw),
            "dense: measure draw {draw} outside [0, 1)"
        );
        let mut conditional_draw = draw;
        let (mut measured_mask, mut chosen_bits, mut value) = (0usize, 0usize, 0usize);
        // Empty registers, including a zero-wire state, have the one
        // outcome (0, ""), just as in the exact interpreter.
        let mut p = if wires.is_empty() { self.norm2() } else { 0.0 };
        if wires.is_empty() {
            assert!(
                p > 0.0 && p.is_finite(),
                "dense: measurement has zero or non-finite norm"
            );
        }
        let eta = gamma(2 * self.n + 64);
        let absolute = 4.0 * self.len() as f64 * f64::MIN_POSITIVE;
        let before = self.bound;
        // For normalized ideal input, normalized approximate-state
        // measurement differs by at most 2B+B^2 in total variation.
        self.measurement_bound = (self.measurement_bound
            + 2.0 * before
            + before * before
            + absolute
            + (wires.len() as f64 + 1.0) * 8.0 * eta)
            .min(1.0)
            .next_up()
            .min(1.0);
        for (b, &w) in wires.iter().enumerate() {
            let bit = 1usize << w;
            let [p0, p1] = self.reduce_probabilities(measured_mask, chosen_bits, bit);
            let total = p0 + p1;
            assert!(
                total > 0.0 && total.is_finite(),
                "dense: measurement has zero or non-finite norm"
            );
            let position = conditional_draw * total;
            let one = p1 > 0.0 && (p0 == 0.0 || position >= p0);
            if one {
                chosen_bits |= bit;
                value |= 1usize << b;
                p = p1;
                conditional_draw = ((position - p0) / p1).clamp(0.0, 1.0f64.next_down());
            } else {
                p = p0;
                conditional_draw = (position / p0).clamp(0.0, 1.0f64.next_down());
            }
            measured_mask |= bit;
        }
        let scale = 1.0 / p.sqrt();
        assert!(
            scale.is_finite(),
            "dense: measurement normalization overflow"
        );
        let size = self.len();
        let threads = self.workers();
        let (re, im) = self.ptrs();
        // Destination indices have every measured bit zero. Surviving
        // sources are untouched until the second pass, so reads cannot
        // race with another worker's writes. If chosen_bits == 0, the
        // source is the destination itself and has one owner.
        run_ranges(size, threads, move |lo, hi| {
            for i in lo..hi {
                unsafe {
                    if i & measured_mask == 0 {
                        let src = i | chosen_bits;
                        let (xr, xi) = (re.get(src), im.get(src));
                        re.set(i, xr * scale);
                        im.set(i, xi * scale);
                    } else if i & measured_mask != chosen_bits {
                        re.set(i, 0.0);
                        im.set(i, 0.0);
                    }
                }
            }
        });
        if chosen_bits != 0 {
            run_ranges(size, threads, move |lo, hi| {
                for i in lo..hi {
                    if i & measured_mask == chosen_bits {
                        // SAFETY: index ranges are disjoint; prior pass joined.
                        unsafe {
                            re.set(i, 0.0);
                            im.set(i, 0.0);
                        }
                    }
                }
            });
        }
        // ||normalize(Px)-normalize(Py)|| <= 2||x-y||/||Py||.
        // Subtract the reduction's relative and absolute error from
        // p to bound its true projected norm. Include sqrt, divide and
        // amplitude-scaling roundoff, rather than charging O(n*u) alone.
        let probability_error = eta * p + absolute;
        let lower_p = (p - probability_error).max(0.0);
        let projected = branch
            .iter()
            .enumerate()
            .fold(before, |m, (b, e)| m.min(e[value >> b & 1]));
        let conditional_error = if lower_p > 0.0 {
            2.0 * projected / lower_p.sqrt()
        } else {
            // At underflow-scale mass, use the distance bound from
            // two vector norms instead of claiming relative accuracy.
            1.0 + ((p + probability_error) / p).sqrt()
        };
        self.bound = (conditional_error + 8.0 * (eta + absolute / p)).next_up();
        self.norm_upper = 1.0;
        let bits = (0..wires.len())
            .map(|b| if value >> b & 1 == 1 { '1' } else { '0' })
            .collect();
        (value, bits)
    }
}

impl DenseState {
    /// Apply consecutive gates in one cache-blocked traversal when
    /// their involved-wire union has at most six wires. Low wires use
    /// 16 KiB resident tiles; scattered wires use a 64-amplitude local
    /// vector. Gate order and individual gate error charges are kept.
    /// Larger unions execute sequentially (same result, more passes).
    pub fn apply_fused(&mut self, gates: &[Gate]) {
        if gates.is_empty() {
            return;
        }
        for g in gates {
            match g {
                Gate::One {
                    target,
                    controls,
                    m,
                } => {
                    self.check_controls(*target, controls);
                    self.check_finite(m);
                }
                Gate::Diagonal {
                    target,
                    controls,
                    d,
                } => {
                    self.check_controls(*target, controls);
                    assert!(
                        d.iter().all(|z| z.re.is_finite() && z.im.is_finite()),
                        "dense: non-finite fused diagonal entry"
                    );
                }
                Gate::Flip { target, controls } => self.check_controls(*target, controls),
            }
        }
        let mut wires: Vec<usize> = Vec::new();
        for g in gates {
            let (t, cs) = match g {
                Gate::One {
                    target, controls, ..
                } => (*target, controls),
                Gate::Diagonal {
                    target, controls, ..
                } => (*target, controls),
                Gate::Flip { target, controls } => (*target, controls),
            };
            for w in std::iter::once(t).chain(cs.iter().copied()) {
                if !wires.contains(&w) {
                    wires.push(w);
                }
            }
        }
        if wires.len() > FUSE_MAX_WIRES {
            for g in gates {
                match g {
                    Gate::One {
                        target,
                        controls,
                        m,
                    } => self.apply1(*target, controls, *m),
                    Gate::Diagonal {
                        target,
                        controls,
                        d,
                    } => self.diagonal1(*target, controls, d[0], d[1]),
                    Gate::Flip { target, controls } => self.flip(*target, controls),
                }
            }
            return;
        }
        wires.sort_unstable();
        let pos = |w: usize| wires.iter().position(|&x| x == w).expect("fused wire");
        let k = wires.len();
        let dim = 1usize << k;
        // qHiPSTER-style fusion: apply the original sparse gates while
        // a tile stays in cache, avoiding a dense 2^k by 2^k matvec.
        // Gates keep their original order; no matrix construction error.
        if wires[k - 1] < 10 {
            let block_len = 1usize << self.n.min(10);
            let blocks = self.len() / block_len;
            let simd = self.simd_active();
            let threads = self.workers();
            let (re, im) = self.ptrs();
            run_ranges(blocks, threads, |lo, hi| {
                for block in lo..hi {
                    for gate in gates {
                        // SAFETY: each worker owns whole tiles, and every
                        // involved wire is strictly below the tile width.
                        unsafe {
                            apply_gate_tile(re, im, block * block_len, block_len, gate, simd)
                        };
                    }
                }
            });
        } else {
            let mut spread = [0usize; 1 << FUSE_MAX_WIRES];
            for (local, s) in spread[..dim].iter_mut().enumerate() {
                for (b, &w) in wires.iter().enumerate() {
                    *s |= ((local >> b) & 1) << w;
                }
            }
            let free: Vec<usize> = (0..self.n).filter(|w| !wires.contains(w)).collect();
            let outer_count = self.len() >> k;
            let threads = self.workers();
            let (re, im) = self.ptrs();
            run_ranges(outer_count, threads, |lo, hi| {
                let mut vin = [Complex::ZERO; 1 << FUSE_MAX_WIRES];
                for o in lo..hi {
                    let base = free
                        .iter()
                        .enumerate()
                        .fold(0, |v, (b, &w)| v | (((o >> b) & 1) << w));
                    for l in 0..dim {
                        // SAFETY: outer indices partition disjoint blocks.
                        vin[l] = unsafe {
                            Complex::new(re.get(base | spread[l]), im.get(base | spread[l]))
                        };
                    }
                    for gate in gates {
                        apply_gate_local(&mut vin[..dim], gate, &pos);
                    }
                    for l in 0..dim {
                        // SAFETY: the same disjoint blocks are written back.
                        unsafe {
                            re.set(base | spread[l], vin[l].re);
                            im.set(base | spread[l], vin[l].im);
                        }
                    }
                }
            });
        }

        self.gates += gates.len() as u64;
        for g in gates {
            match g {
                Gate::One { m, .. } => self.charge(matrix_norm_upper(m), 32),
                Gate::Diagonal { d, .. } => {
                    self.charge(phase_norm_upper(d[0]).max(phase_norm_upper(d[1])), 8)
                }
                Gate::Flip { .. } => (),
            }
        }
    }
}

/// Apply a gate within a resident cache tile. All wires are tile-local.
/// # Safety
/// Caller exclusively owns the complete tile and has detected AVX2/FMA
/// when `simd` is true. All gate wires must be below log2(len).
unsafe fn apply_gate_tile(re: Ptr, im: Ptr, start: usize, len: usize, g: &Gate, simd: bool) {
    let (target, controls) = match g {
        Gate::One {
            target, controls, ..
        }
        | Gate::Diagonal {
            target, controls, ..
        }
        | Gate::Flip { target, controls } => (*target, controls),
    };
    let stride = 1usize << target;
    let cmask = controls.iter().fold(0, |m, &w| m | (1usize << w));
    // SAFETY: every index below is inside this tile; pairs are disjoint.
    unsafe {
        match g {
            Gate::One { m, .. } if cmask == 0 => {
                if simd && stride < 4 && len >= 4 {
                    #[cfg(target_arch = "x86_64")]
                    apply1_avx2_low(re, im, start, len, stride, *m);
                    #[cfg(not(target_arch = "x86_64"))]
                    unreachable!("dense: SIMD tile on unsupported architecture");
                    return;
                }
                for block in (start..start + len).step_by(2 * stride) {
                    if simd && stride >= 4 {
                        #[cfg(target_arch = "x86_64")]
                        for i in (block..block + stride).step_by(4) {
                            apply1_avx2_x4(re, im, i, stride, m[0][0], m[0][1], m[1][0], m[1][1]);
                        }
                        #[cfg(not(target_arch = "x86_64"))]
                        unreachable!("dense: SIMD tile on unsupported architecture");
                    } else {
                        apply1_run(
                            re, im, block, stride, stride, m[0][0], m[0][1], m[1][0], m[1][1],
                        );
                    }
                }
            }
            Gate::One { m, .. } => {
                for pair in 0..len / 2 {
                    let i = start + pair_index(pair, stride);
                    if i & cmask != cmask {
                        continue;
                    }
                    let j = i | stride;
                    let (x, y) = (
                        Complex::new(re.get(i), im.get(i)),
                        Complex::new(re.get(j), im.get(j)),
                    );
                    let a = m[0][0] * x + m[0][1] * y;
                    let b = m[1][0] * x + m[1][1] * y;
                    re.set(i, a.re);
                    im.set(i, a.im);
                    re.set(j, b.re);
                    im.set(j, b.im);
                }
            }
            Gate::Diagonal { d, .. } => {
                for i in start..start + len {
                    if i & cmask == cmask {
                        let z =
                            Complex::new(re.get(i), im.get(i)) * d[usize::from(i & stride != 0)];
                        re.set(i, z.re);
                        im.set(i, z.im);
                    }
                }
            }
            Gate::Flip { .. } => {
                for pair in 0..len / 2 {
                    let i = start + pair_index(pair, stride);
                    if i & cmask == cmask {
                        let j = i | stride;
                        let (r, a) = (re.get(i), im.get(i));
                        re.set(i, re.get(j));
                        im.set(i, im.get(j));
                        re.set(j, r);
                        im.set(j, a);
                    }
                }
            }
        }
    }
}

/// Apply one gate to a gathered local vector (scattered-wire fusion).
fn apply_gate_local(v: &mut [Complex], g: &Gate, pos: &impl Fn(usize) -> usize) {
    let dim = v.len();
    match g {
        Gate::One {
            target,
            controls,
            m,
        } => {
            let stride = 1usize << pos(*target);
            let cmask: usize = controls.iter().fold(0, |acc, &c| acc | (1usize << pos(c)));
            for p in 0..dim / 2 {
                let i0 = pair_index(p, stride);
                if i0 & cmask != cmask {
                    continue;
                }
                let (x, y) = (v[i0], v[i0 | stride]);
                v[i0] = m[0][0] * x + m[0][1] * y;
                v[i0 | stride] = m[1][0] * x + m[1][1] * y;
            }
        }
        Gate::Diagonal {
            target,
            controls,
            d,
        } => {
            let bit = 1usize << pos(*target);
            let cmask: usize = controls.iter().fold(0, |acc, &c| acc | (1usize << pos(c)));
            for (i, amp) in v.iter_mut().enumerate() {
                if i & cmask == cmask {
                    *amp = *amp * if i & bit == 0 { d[0] } else { d[1] };
                }
            }
        }
        Gate::Flip { target, controls } => {
            let stride = 1usize << pos(*target);
            let cmask: usize = controls.iter().fold(0, |acc, &c| acc | (1usize << pos(c)));
            for p in 0..dim / 2 {
                let i0 = pair_index(p, stride);
                if i0 & cmask == cmask {
                    v.swap(i0, i0 | stride);
                }
            }
        }
    }
}

/// Scalar 1-qubit gate over `cnt` consecutive pairs starting at
/// `i0` (lower run) and `i0 + stride` (upper run). Written over
/// slices so the optimiser sees unit-stride streams.
///
/// # Safety
/// The `2 * cnt` amplitudes in the two runs must be in bounds and
/// exclusively owned by this call.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
unsafe fn apply1_run(
    re: Ptr,
    im: Ptr,
    i0: usize,
    stride: usize,
    cnt: usize,
    a: Complex,
    b: Complex,
    c: Complex,
    d: Complex,
) {
    // SAFETY: per function contract.
    let (xr, xi, yr, yi) = unsafe {
        (
            std::slice::from_raw_parts_mut(re.0.add(i0), cnt),
            std::slice::from_raw_parts_mut(im.0.add(i0), cnt),
            std::slice::from_raw_parts_mut(re.0.add(i0 + stride), cnt),
            std::slice::from_raw_parts_mut(im.0.add(i0 + stride), cnt),
        )
    };
    for (((xr, xi), yr), yi) in xr
        .iter_mut()
        .zip(xi.iter_mut())
        .zip(yr.iter_mut())
        .zip(yi.iter_mut())
    {
        let (x0, x1, y0, y1) = (*xr, *xi, *yr, *yi);
        *xr = a.re * x0 - a.im * x1 + b.re * y0 - b.im * y1;
        *xi = a.re * x1 + a.im * x0 + b.re * y1 + b.im * y0;
        *yr = c.re * x0 - c.im * x1 + d.re * y0 - d.im * y1;
        *yi = c.re * x1 + c.im * x0 + d.re * y1 + d.im * y0;
    }
}

fn avx2_fma_detected() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Low-wire AVX2 kernel: four lanes hold two complete pairs. A fixed
/// permutation exchanges the partner lanes, avoiding strided gathers.
/// # Safety
/// AVX2/FMA must be detected. start and len are multiples of four, the
/// range is exclusively owned, and stride is either one or two.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn apply1_avx2_low(
    re: Ptr,
    im: Ptr,
    start: usize,
    len: usize,
    stride: usize,
    m: [[Complex; 2]; 2],
) {
    use std::arch::x86_64::*;
    unsafe {
        let lanes = |lo: f64, hi: f64| {
            if stride == 1 {
                _mm256_setr_pd(lo, hi, lo, hi)
            } else {
                _mm256_setr_pd(lo, lo, hi, hi)
            }
        };
        let ar = lanes(m[0][0].re, m[1][1].re);
        let ai = lanes(m[0][0].im, m[1][1].im);
        let br = lanes(m[0][1].re, m[1][0].re);
        let bi = lanes(m[0][1].im, m[1][0].im);
        for i in (start..start + len).step_by(4) {
            let rp = re.0.add(i);
            let ip = im.0.add(i);
            let xr = _mm256_loadu_pd(rp);
            let xi = _mm256_loadu_pd(ip);
            let (yr, yi) = if stride == 1 {
                (
                    _mm256_permute4x64_pd::<0xb1>(xr),
                    _mm256_permute4x64_pd::<0xb1>(xi),
                )
            } else {
                (
                    _mm256_permute4x64_pd::<0x4e>(xr),
                    _mm256_permute4x64_pd::<0x4e>(xi),
                )
            };
            let r = _mm256_fnmadd_pd(
                bi,
                yi,
                _mm256_fmadd_pd(br, yr, _mm256_fnmadd_pd(ai, xi, _mm256_mul_pd(ar, xr))),
            );
            let a = _mm256_fmadd_pd(
                bi,
                yr,
                _mm256_fmadd_pd(br, yi, _mm256_fmadd_pd(ai, xr, _mm256_mul_pd(ar, xi))),
            );
            _mm256_storeu_pd(rp, r);
            _mm256_storeu_pd(ip, a);
        }
    }
}

/// Four amplitude pairs of a 1-qubit gate with AVX2/FMA. `i0` and
/// `i0 + stride` each start a contiguous run of 4 amplitudes.
///
/// # Safety
/// Caller must have detected AVX2+FMA; the 8 touched amplitudes must
/// be in bounds and exclusively owned by this call.
#[cfg(target_arch = "x86_64")]
#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "avx2,fma")]
unsafe fn apply1_avx2_x4(
    re: Ptr,
    im: Ptr,
    i0: usize,
    stride: usize,
    a: Complex,
    b: Complex,
    c: Complex,
    d: Complex,
) {
    use std::arch::x86_64::*;
    // SAFETY: per function contract.
    unsafe {
        let (rp, ip) = (re.0.add(i0), im.0.add(i0));
        let xr = _mm256_loadu_pd(rp);
        let xi = _mm256_loadu_pd(ip);
        let yr = _mm256_loadu_pd(rp.add(stride));
        let yi = _mm256_loadu_pd(ip.add(stride));
        let (ar, ai) = (_mm256_set1_pd(a.re), _mm256_set1_pd(a.im));
        let (br, bi) = (_mm256_set1_pd(b.re), _mm256_set1_pd(b.im));
        let (cr, ci) = (_mm256_set1_pd(c.re), _mm256_set1_pd(c.im));
        let (dr, di) = (_mm256_set1_pd(d.re), _mm256_set1_pd(d.im));
        let lo_r = _mm256_fnmadd_pd(
            bi,
            yi,
            _mm256_fmadd_pd(br, yr, _mm256_fnmadd_pd(ai, xi, _mm256_mul_pd(ar, xr))),
        );
        let lo_i = _mm256_fmadd_pd(
            bi,
            yr,
            _mm256_fmadd_pd(br, yi, _mm256_fmadd_pd(ai, xr, _mm256_mul_pd(ar, xi))),
        );
        let hi_r = _mm256_fnmadd_pd(
            di,
            yi,
            _mm256_fmadd_pd(dr, yr, _mm256_fnmadd_pd(ci, xi, _mm256_mul_pd(cr, xr))),
        );
        let hi_i = _mm256_fmadd_pd(
            di,
            yr,
            _mm256_fmadd_pd(dr, yi, _mm256_fmadd_pd(ci, xr, _mm256_mul_pd(cr, xi))),
        );
        _mm256_storeu_pd(rp, lo_r);
        _mm256_storeu_pd(ip, lo_i);
        _mm256_storeu_pd(rp.add(stride), hi_r);
        _mm256_storeu_pd(ip.add(stride), hi_i);
    }
}
