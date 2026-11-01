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

use std::collections::BTreeMap;

/// Binary64 unit roundoff, `2^-53`.
const U: f64 = 1.110_223_024_625_156_5e-16;
/// Work items below this run single-threaded (spawn cost dominates).
const THREAD_MIN_ITEMS: usize = 1 << 14;
/// Maximum wires in one fused group (matrix is at most 64x64).
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
    let t = if total < THREAD_MIN_ITEMS {
        1
    } else {
        threads.min(total)
    };
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
        }
    }
    pub fn amplitude(&self, i: usize) -> Complex {
        Complex::new(self.re[i], self.im[i])
    }
    /// Overwrite one amplitude. Does NOT renormalise and does not
    /// charge the error bound: state preparation is exact input.
    pub fn set_amplitude(&mut self, i: usize, v: Complex) {
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

    /// Sum of |amplitude|^2 (parallel chunked reduction, fixed order).
    pub fn norm2(&self) -> f64 {
        let size = self.len();
        let t = if size < THREAD_MIN_ITEMS {
            1
        } else {
            self.threads
        };
        if t <= 1 {
            return (0..size)
                .map(|i| self.re[i] * self.re[i] + self.im[i] * self.im[i])
                .sum();
        }
        let chunk = size.div_ceil(t);
        let partials: Vec<f64> = std::thread::scope(|s| {
            let mut handles = Vec::new();
            let mut lo = 0;
            while lo < size {
                let hi = (lo + chunk).min(size);
                let (re, im) = (&self.re, &self.im);
                handles.push(
                    s.spawn(move || (lo..hi).map(|i| re[i] * re[i] + im[i] * im[i]).sum::<f64>()),
                );
                lo = hi;
            }
            handles
                .into_iter()
                .map(|h| h.join().expect("dense: norm worker panicked"))
                .collect()
        });
        partials.iter().sum()
    }

    fn charge(&mut self, k: f64) {
        self.bound += k * U * (1.0 + self.bound);
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
        let threads = self.threads;
        if controls.is_empty() && self.simd && stride >= 4 && avx2_fma_detected() {
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
            run_ranges(pairs, threads, move |lo, hi| {
                for p in lo..hi {
                    let i0 = pair_index(p, stride);
                    if i0 & cmask != cmask {
                        continue;
                    }
                    let i1 = i0 | stride;
                    // SAFETY: amplitude pairs are disjoint by partition.
                    unsafe {
                        let (x0, x1) = (re.get(i0), im.get(i0));
                        let (y0, y1) = (re.get(i1), im.get(i1));
                        re.set(i0, a.re * x0 - a.im * x1 + b.re * y0 - b.im * y1);
                        im.set(i0, a.re * x1 + a.im * x0 + b.re * y1 + b.im * y0);
                        re.set(i1, c.re * x0 - c.im * x1 + d.re * y0 - d.im * y1);
                        im.set(i1, c.re * x1 + c.im * x0 + d.re * y1 + d.im * y0);
                    }
                }
            });
        }
        self.gates += 1;
        self.charge(8.0);
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
        run_ranges(size, self.threads, move |lo, hi| {
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
        self.charge(4.0);
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
        run_ranges(size, self.threads, move |lo, hi| {
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
        self.charge(4.0);
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
        run_ranges(pairs, self.threads, move |lo, hi| {
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
        run_ranges(size, self.threads, move |lo, hi| {
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
        for &w in input_wires.iter().chain(target_wires) {
            self.check_wire(w);
        }
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
        let mut memo: BTreeMap<usize, usize> = BTreeMap::new();
        let mut table = Vec::with_capacity(n_in);
        for x in 0..n_in {
            let y = *memo.entry(x).or_insert_with(|| f(x));
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
        run_ranges(size, self.threads, move |lo, hi| {
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

/// Extract the value on `wires` (list order = bit order) from index `i`.
fn extract_wires(i: usize, wires: &[usize]) -> usize {
    let mut v = 0usize;
    for (b, &w) in wires.iter().enumerate() {
        v |= ((i >> w) & 1) << b;
    }
    v
}

impl DenseState {
    fn outcome_probs_sparse(&self, wires: &[usize]) -> BTreeMap<usize, f64> {
        for &w in wires {
            self.check_wire(w);
        }
        let size = self.len();
        let t = if size < THREAD_MIN_ITEMS {
            1
        } else {
            self.threads
        };
        let partial = |lo: usize, hi: usize, re: &[f64], im: &[f64]| {
            let mut m: BTreeMap<usize, f64> = BTreeMap::new();
            for i in lo..hi {
                *m.entry(extract_wires(i, wires)).or_insert(0.0) += re[i] * re[i] + im[i] * im[i];
            }
            m
        };
        if t <= 1 {
            return partial(0, size, &self.re, &self.im);
        }
        let chunk = size.div_ceil(t);
        let parts: Vec<BTreeMap<usize, f64>> = std::thread::scope(|s| {
            let mut handles = Vec::new();
            let mut lo = 0;
            while lo < size {
                let hi = (lo + chunk).min(size);
                let (re, im) = (&self.re, &self.im);
                handles.push(s.spawn(move || partial(lo, hi, re, im)));
                lo = hi;
            }
            handles
                .into_iter()
                .map(|h| h.join().expect("dense: probability worker panicked"))
                .collect()
        });
        // Merge in fixed chunk order: deterministic for a thread count.
        let mut out: BTreeMap<usize, f64> = BTreeMap::new();
        for part in parts {
            for (k, v) in part {
                *out.entry(k).or_insert(0.0) += v;
            }
        }
        out
    }

    /// Born probabilities for `wires`, indexed by outcome value (bit
    /// `b` of the index is `wires[b]`).
    pub fn probabilities(&self, wires: &[usize]) -> Vec<f64> {
        assert!(
            wires.len() <= 31,
            "dense: probability table for {} wires is too large to materialise",
            wires.len()
        );
        let sparse = self.outcome_probs_sparse(wires);
        let mut p = vec![0.0; 1usize << wires.len()];
        for (v, prob) in sparse {
            p[v] = prob;
        }
        p
    }

    /// Measure `wires` with caller-supplied `draw` in `[0, 1)`.
    ///
    /// Mirrors `QuantumState::measure`: outcomes are enumerated in
    /// lexicographic order in wire-list order (first listed wire is
    /// most significant for ordering); the first outcome whose
    /// cumulative probability exceeds `draw` wins; the state collapses
    /// and renormalises, and the measured wires are reset to 0 in the
    /// remaining state. Returns `(value, bits)` with `bits[b]` the
    /// outcome of `wires[b]`. This engine never draws randomness:
    /// K17 draw order is the caller's concern.
    pub fn measure(&mut self, wires: &[usize], draw: f64) -> (usize, String) {
        assert!(!wires.is_empty(), "dense: measure needs at least one wire");
        assert!(
            (0.0..1.0).contains(&draw),
            "dense: measure draw {draw} outside [0, 1)"
        );
        let probs = self.outcome_probs_sparse(wires);
        let k = wires.len();
        let mut ordered: Vec<usize> = probs.keys().copied().collect();
        ordered.sort_by_key(|&v| {
            // Lexicographic in wire order = ascending bit-reversed value.
            let mut r = 0usize;
            for b in 0..k {
                r |= ((v >> b) & 1) << (k - 1 - b);
            }
            r
        });
        let mut cumulative = 0.0;
        let mut chosen: Option<usize> = None;
        for v in ordered {
            cumulative += probs[&v];
            if draw < cumulative {
                chosen = Some(v);
                break;
            }
        }
        let value = chosen.or_else(|| probs.keys().max().copied()).unwrap_or(0);
        let p = probs.get(&value).copied().unwrap_or(1.0);
        let scale = if p > 0.0 { 1.0 / p.sqrt() } else { 1.0 };
        // Bits of `value` spread back onto the measured wires.
        let mut chosen_bits = 0usize;
        for (b, &w) in wires.iter().enumerate() {
            if (value >> b) & 1 == 1 {
                chosen_bits |= 1usize << w;
            }
        }
        let measured_mask: usize = wires.iter().fold(0, |acc, &w| acc | (1usize << w));
        let wires_v: Vec<usize> = wires.to_vec();
        let size = self.len();
        let (re, im) = self.ptrs();
        let threads = self.threads;
        // Pass 1: destination indices (measured bits all 0) receive
        // the scaled surviving amplitude; non-surviving,
        // non-destination indices are zeroed. Surviving sources are
        // left untouched until pass 2 (they are read here).
        run_ranges(size, threads, move |lo, hi| {
            for i in lo..hi {
                let m = extract_wires(i, &wires_v);
                // SAFETY: writes are to i only; the only read of a
                // foreign index is a surviving source, not written
                // in this pass (Ptr contract, two-pass collapse).
                unsafe {
                    if i & measured_mask == 0 {
                        let src = i | chosen_bits;
                        let (x0, x1) = (re.get(src), im.get(src));
                        re.set(i, x0 * scale);
                        im.set(i, x1 * scale);
                    } else if m != value {
                        re.set(i, 0.0);
                        im.set(i, 0.0);
                    }
                }
            }
        });
        // Pass 2: vacate the surviving sources (their content moved).
        if value != 0 {
            let wires_v: Vec<usize> = wires.to_vec();
            run_ranges(size, threads, move |lo, hi| {
                for i in lo..hi {
                    if extract_wires(i, &wires_v) == value {
                        // SAFETY: index ranges are disjoint.
                        unsafe {
                            re.set(i, 0.0);
                            im.set(i, 0.0);
                        }
                    }
                }
            });
        }
        self.charge(self.n as f64 + 8.0);
        let bits: String = (0..k)
            .map(|b| if (value >> b) & 1 == 1 { '1' } else { '0' })
            .collect();
        (value, bits)
    }
}

impl DenseState {
    /// Apply a consecutive group of gates in ONE pass when their
    /// involved-wire union has at most 6 wires: the group's unitary
    /// is built once (at most 64x64) and applied block-wise, so each
    /// amplitude is touched once per group instead of once per gate.
    /// Larger unions fall back to sequential application (same
    /// result, more passes).
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
        // Build U (row-major, row = out, col = in) by applying the
        // gates in order to the identity's columns.
        let mut u = vec![Complex::ZERO; dim * dim];
        for c in 0..dim {
            u[c * dim + c] = Complex::ONE;
        }
        let mut col = vec![Complex::ZERO; dim];
        for c in 0..dim {
            for r in 0..dim {
                col[r] = u[r * dim + c];
            }
            for g in gates {
                apply_gate_local(&mut col, g, &pos);
            }
            for r in 0..dim {
                u[r * dim + c] = col[r];
            }
        }
        // Spread tables between local (fused) indices and global ones.
        let mut spread = vec![0usize; dim];
        for (local, s) in spread.iter_mut().enumerate() {
            for (b, &w) in wires.iter().enumerate() {
                if (local >> b) & 1 == 1 {
                    *s |= 1usize << w;
                }
            }
        }
        let free: Vec<usize> = (0..self.n).filter(|w| !wires.contains(w)).collect();
        let outer_count = self.len() >> k;
        let (re, im) = self.ptrs();
        run_ranges(outer_count, self.threads, move |lo, hi| {
            let mut idxs = vec![0usize; dim];
            let mut vin = vec![Complex::ZERO; dim];
            for o in lo..hi {
                let mut base = 0usize;
                for (b, &w) in free.iter().enumerate() {
                    if (o >> b) & 1 == 1 {
                        base |= 1usize << w;
                    }
                }
                for l in 0..dim {
                    idxs[l] = base | spread[l];
                    // SAFETY: one outer block per item; blocks are
                    // disjoint sets of amplitude indices.
                    vin[l] = unsafe { Complex::new(re.get(idxs[l]), im.get(idxs[l])) };
                }
                for r in 0..dim {
                    let mut acc = Complex::ZERO;
                    for c in 0..dim {
                        acc = acc + u[r * dim + c] * vin[c];
                    }
                    // SAFETY: disjoint fused blocks (above).
                    unsafe {
                        re.set(idxs[r], acc.re);
                        im.set(idxs[r], acc.im);
                    }
                }
            }
        });
        self.gates += gates.len() as u64;
        self.charge(8.0 * dim as f64);
    }
}

/// Apply one gate to a dense local vector (fused-matrix construction).
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
