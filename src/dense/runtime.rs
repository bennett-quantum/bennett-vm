//! Stable logical-wire adapter. Constant wires occupy metadata, not amplitudes.
//! The exact interpreter remains independent; this adapter never keeps a sparse
//! amplitude shadow. Snapshots are explicit diagnostics at API/report boundaries.
//!
//! Discrete decisions (constant wires, §5.6 cleanliness, exact equality) are
//! never decided within the floating-point allowance (beads yt9, ykb). A wire
//! is constant only by an exact certificate:
//! - T0: it is basis metadata (exact by construction);
//! - T1: the operation log shows no live value-changing operation on it since
//!   it was last an exact constant (exact inverse pairs cancel in the log);
//! - T2: the support bitmap shows its other branch is exactly zero.
//!
//! A wire varies only when both branches are certainly nonzero: the computed
//! branch norm, rounded down, exceeds that branch's error bound (T3). Anything
//! else is undecidable and faults loudly; compaction simply leaves such a wire
//! materialized, so it never deletes amplitude the exact state has.
use super::{
    Complex, DenseState, U, gamma, matrix_norm_upper, pair_index, phase_norm_upper,
};
use crate::{bigint::BigUint, exact::Exact, json::Json};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

/// Globally unique operation sequence numbers. Clones share history, so two
/// logs holding the same live sequence numbers hold the same operations.
static SEQ: AtomicU64 = AtomicU64::new(1);
fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Live log entries kept before the oldest are merged into one barrier.
const LOG_LIVE_CAP: usize = 4096;
/// Live entries scanned backwards for an exact inverse to cancel against.
const CANCEL_WALK: usize = 256;

#[derive(Clone, Debug, PartialEq)]
enum OpKind {
    Gate {
        target: usize,
        controls: Vec<usize>,
        m: [[Complex; 2]; 2],
    },
    Flip {
        target: usize,
        controls: Vec<usize>,
    },
    Phase {
        wires: Vec<usize>,
        phase: Complex,
    },
    /// Measurement, preparation, input state, remap or merged history.
    Other,
}

#[derive(Clone, Debug)]
struct Op {
    seq: u64,
    end: u64,
    kind: OpKind,
    /// The binary64 operands are the exact operands of the interpreter.
    exact: bool,
    /// Wires whose basis value the operation may change.
    writes: Vec<usize>,
    /// All wires involved; `None` means every wire (never commutes).
    acts: Option<Vec<usize>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Anchor {
    /// Exact value of the wire when the anchor was set.
    value: bool,
    /// Operations with `end >= seq` may change it.
    seq: u64,
    /// Live operations writing the wire since the anchor.
    count: u64,
}

/// Exact operation-history certificate. Every wire starts as an exact zero
/// (anchor default). Inverse pairs whose exact product is the identity are
/// retired when every live operation between them commutes with them.
#[derive(Clone, Debug, Default)]
struct Log {
    ops: VecDeque<Option<Op>>,
    live: usize,
    anchors: HashMap<usize, Anchor>,
    merged: bool,
}

fn disjoint(a: &[usize], b: &[usize]) -> bool {
    a.iter().all(|x| !b.contains(x))
}

fn sorted(wires: &[usize]) -> Vec<usize> {
    let mut v = wires.to_vec();
    v.sort_unstable();
    v.dedup();
    v
}

// Operations that are diagonal on every shared wire commute.
fn commutes(a: &Op, b: &Op) -> bool {
    if let (OpKind::Flip { target: x, .. }, OpKind::Flip { target: y, .. }) = (&a.kind, &b.kind)
        && x == y
    {
        // Two XORs into one target commute (neither controls the other).
        return true;
    }
    match (&a.acts, &b.acts) {
        (Some(sa), Some(sb)) => disjoint(&a.writes, sb) && disjoint(&b.writes, sa),
        _ => false,
    }
}

/// The interpreter's exact embedding of `QuantumState::apply1` entries:
/// binary64 values are exact dyadics, except that +-FRAC_1_SQRT_2 denotes
/// +-sqrt(1/2).
fn exact_entry(z: Complex) -> Exact {
    let component = |v: f64| {
        if v.abs() == std::f64::consts::FRAC_1_SQRT_2 {
            if v < 0.0 {
                Exact::sqrt_half().neg()
            } else {
                Exact::sqrt_half()
            }
        } else {
            Exact::pair(v, 0.0)
        }
    };
    component(z.re).add(&component(z.im).mul(&Exact::pair(0.0, 1.0)))
}

fn exact_identity_product(new: &[[Complex; 2]; 2], old: &[[Complex; 2]; 2]) -> bool {
    let a = new.map(|r| r.map(exact_entry));
    let b = old.map(|r| r.map(exact_entry));
    (0..2).all(|i| {
        (0..2).all(|j| {
            let x = a[i][0].mul(&b[0][j]).add(&a[i][1].mul(&b[1][j]));
            x == if i == j { Exact::one() } else { Exact::zero() }
        })
    })
}

fn inverse(old: &Op, new: &Op) -> bool {
    if !old.exact || !new.exact {
        return false;
    }
    match (&old.kind, &new.kind) {
        (
            OpKind::Flip {
                target: a,
                controls: ca,
            },
            OpKind::Flip {
                target: b,
                controls: cb,
            },
        ) => a == b && ca == cb,
        (
            OpKind::Gate {
                target: a,
                controls: ca,
                m: ma,
            },
            OpKind::Gate {
                target: b,
                controls: cb,
                m: mb,
            },
        ) => a == b && ca == cb && exact_identity_product(mb, ma),
        (
            OpKind::Phase {
                wires: a,
                phase: pa,
            },
            OpKind::Phase {
                wires: b,
                phase: pb,
            },
        ) => {
            a == b
                && Exact::pair(pa.re, pa.im).mul(&Exact::pair(pb.re, pb.im)) == Exact::one()
        }
        _ => false,
    }
}

impl Log {
    fn anchor(&self, wire: usize) -> Anchor {
        self.anchors.get(&wire).copied().unwrap_or_default()
    }

    fn certified(&self, wire: usize) -> Option<bool> {
        let a = self.anchor(wire);
        (a.count == 0).then_some(a.value)
    }

    fn set_anchor(&mut self, wire: usize, value: bool) {
        self.anchors.insert(
            wire,
            Anchor {
                value,
                seq: next_seq(),
                count: 0,
            },
        );
    }

    fn record(&mut self, kind: OpKind, exact: bool, writes: Vec<usize>, acts: Option<Vec<usize>>) {
        let mut new = Op {
            seq: 0,
            end: 0,
            kind,
            exact,
            writes,
            acts,
        };
        let mut seen = 0;
        for idx in (0..self.ops.len()).rev() {
            let Some(old) = &self.ops[idx] else {
                continue;
            };
            if inverse(old, &new) {
                self.retire(idx);
                return;
            }
            seen += 1;
            if !commutes(old, &new) || seen >= CANCEL_WALK {
                break;
            }
        }
        new.seq = next_seq();
        new.end = new.seq;
        for &w in &new.writes {
            self.anchors.entry(w).or_default().count += 1;
        }
        self.ops.push_back(Some(new));
        self.live += 1;
        if self.live > LOG_LIVE_CAP {
            self.merge();
        }
    }

    fn retire(&mut self, idx: usize) {
        let op = self.ops[idx].take().expect("dense runtime: retiring a dead log entry");
        self.live -= 1;
        for &w in &op.writes {
            let a = self.anchors.entry(w).or_default();
            if op.end >= a.seq {
                assert!(a.count > 0, "dense runtime: log anchor count underflow");
                a.count -= 1;
            }
        }
        while matches!(self.ops.back(), Some(None)) {
            self.ops.pop_back();
        }
        while matches!(self.ops.front(), Some(None)) {
            self.ops.pop_front();
        }
    }

    // Bounded memory: the oldest live half becomes one barrier writing the
    // union of their wires, conservatively spanning their sequence range.
    fn merge(&mut self) {
        let (mut lo, mut hi, mut merged) = (u64::MAX, 0, 0);
        let mut writes = BTreeSet::new();
        while merged < LOG_LIVE_CAP / 2 {
            match self.ops.pop_front() {
                Some(Some(op)) => {
                    lo = lo.min(op.seq);
                    hi = hi.max(op.end);
                    writes.extend(op.writes);
                    merged += 1;
                }
                Some(None) => {}
                None => break,
            }
        }
        let writes: Vec<usize> = writes.into_iter().collect();
        self.ops.push_front(Some(Op {
            seq: lo,
            end: hi,
            kind: OpKind::Other,
            exact: false,
            writes: writes.clone(),
            acts: None,
        }));
        self.live = self.live - merged + 1;
        self.merged = true;
        for w in writes {
            let a = self.anchor(w);
            let count = self
                .ops
                .iter()
                .flatten()
                .filter(|o| o.end >= a.seq && o.writes.contains(&w))
                .count() as u64;
            self.anchors.entry(w).or_default().count = count;
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn live_seqs(&self) -> Vec<u64> {
        self.ops.iter().flatten().map(|o| o.seq).collect()
    }
}

/// Classify a binary64 gate matrix as handed over by `QuantumState`, which
/// passes both `apply1` matrices (exact as given) and binary64 projections
/// of symbolic rx/ry/rz rotations through the same call. Returns
/// (operands are exact, matrix is exactly diagonal).
/// - rx/ry projections have identical diagonal entries (one symbolic value);
/// - rz projections are diag(e^{-ia}, e^{ia}), so a unit first entry forces
///   the identity; every rotation at a subnormal angle projects to I.
fn classify(m: &[[Complex; 2]; 2]) -> (bool, bool) {
    let off = m[0][1] == Complex::ZERO && m[1][0] == Complex::ZERO;
    if off && m[0][0] == Complex::ONE && m[1][1] == Complex::ONE {
        return (false, false);
    }
    if off {
        return (m[0][0] == Complex::ONE || m[0][0] == m[1][1], true);
    }
    (m[0][0] != m[1][1], false)
}

enum Decision {
    Constant(bool),
    Varies,
    Undecidable(String),
}

#[derive(Clone, Debug)]
pub(crate) struct Runtime {
    state: DenseState,
    active: Vec<usize>,
    basis: BigUint,
    projections: u64,
    /// Per packed position p and value v: an upper bound on the norm of the
    /// state error restricted to the basis states with bit p equal to v.
    branch: Vec<[f64; 2]>,
    /// Bit i is set when amplitude i may be nonzero in the exact or the
    /// computed state; a clear bit certifies an exact zero in both.
    support: Vec<u64>,
    log: Log,
}

fn words(len: usize) -> usize {
    len.div_ceil(64)
}

fn set_word_bit(v: &mut [u64], i: usize, on: bool) {
    if on {
        v[i >> 6] |= 1 << (i & 63);
    } else {
        v[i >> 6] &= !(1 << (i & 63));
    }
}

/// Upper bound of |z|.
fn modulus_upper(z: Complex) -> f64 {
    (z.re.hypot(z.im) * (1.0 + gamma(4))).next_up()
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            state: DenseState::new(0),
            active: vec![],
            basis: BigUint::zero(),
            projections: 0,
            branch: vec![],
            support: vec![1],
            log: Log::default(),
        }
    }

    fn position(&self, wire: usize) -> Option<usize> {
        self.active.iter().position(|&w| w == wire)
    }

    fn key(&self, index: usize) -> BigUint {
        let mut key = self.basis.clone();
        for (p, &w) in self.active.iter().enumerate() {
            key.set_bit(w, index >> p & 1 != 0);
        }
        key
    }

    fn support_bit(&self, i: usize) -> bool {
        self.support[i >> 6] >> (i & 63) & 1 != 0
    }

    fn promote(&mut self, wire: usize) -> usize {
        if let Some(p) = self.position(wire) {
            return p;
        }
        let p = self.active.len();
        let old = self.state.len();
        let len = old
            .checked_mul(2)
            .filter(|n| n.checked_mul(16).is_some())
            .expect("dense runtime: state allocation overflows indexing");
        self.state
            .re
            .try_reserve_exact(old)
            .expect("dense runtime: real state allocation failed");
        self.state
            .im
            .try_reserve_exact(old)
            .expect("dense runtime: imaginary state allocation failed");
        self.state.re.resize(len, 0.0);
        self.state.im.resize(len, 0.0);
        self.support.resize(words(len), 0);
        let high = self.basis.bit(wire);
        if high {
            for i in 0..old {
                self.state.re[old + i] = self.state.re[i];
                self.state.im[old + i] = self.state.im[i];
                self.state.re[i] = 0.0;
                self.state.im[i] = 0.0;
                let on = self.support_bit(i);
                set_word_bit(&mut self.support, old + i, on);
                set_word_bit(&mut self.support, i, false);
            }
        }
        // The new coordinate's other branch is exactly zero in both states.
        let b = self.state.bound;
        self.branch.push(if high { [0.0, b] } else { [b, 0.0] });
        self.basis.set_bit(wire, false);
        self.active.push(wire);
        self.state.n += 1;
        p
    }

    /// Branch is exactly zero (in the exact and the computed state).
    fn support_zero(&self, p: usize, value: bool) -> bool {
        let v = usize::from(value);
        let zero = if p >= 6 {
            self.support
                .iter()
                .enumerate()
                .all(|(wi, &w)| (wi >> (p - 6)) & 1 != v || w == 0)
        } else {
            let lanes = (0..64)
                .filter(|j| (j >> p) & 1 == v)
                .fold(0u64, |m, j| m | 1 << j);
            self.support.iter().all(|&w| w & lanes == 0)
        };
        if zero {
            for i in 0..self.state.len() {
                if (i >> p & 1) == v {
                    assert!(
                        self.state.amplitude(i) == Complex::ZERO,
                        "dense runtime: support certificate disagrees with amplitude {i}"
                    );
                }
            }
        }
        zero
    }

    // Squaring a tiny but nonzero input amplitude can underflow. Scaled
    // norms bound the computed branch norm from above and below.
    fn branch_norm_raw(&self, p: usize, value: bool) -> (f64, usize) {
        let mut norm: f64 = 0.0;
        let mut steps = 0usize;
        for i in 0..self.state.len() {
            if (i >> p & 1 != 0) == value {
                let z = self.state.amplitude(i);
                if z != Complex::ZERO {
                    norm = norm.hypot(z.re).hypot(z.im);
                    steps += 2;
                }
            }
        }
        (norm, steps)
    }

    fn branch_norm(&self, p: usize, value: bool) -> f64 {
        let (norm, steps) = self.branch_norm_raw(p, value);
        if norm == 0.0 {
            0.0
        } else {
            (norm * (1.0 + gamma(4 * steps + 8)) + steps as f64 * f64::MIN_POSITIVE).next_up()
        }
    }

    fn branch_norm_lower(&self, p: usize, value: bool) -> f64 {
        let (norm, steps) = self.branch_norm_raw(p, value);
        let low = (norm * (1.0 - gamma(4 * steps + 8)) - steps as f64 * f64::MIN_POSITIVE)
            .next_down();
        low.max(0.0)
    }

    /// Exact branch mass is certainly nonzero.
    fn certainly_nonzero(&self, p: usize, value: bool) -> bool {
        self.branch_norm_lower(p, value) > self.branch[p][usize::from(value)]
    }

    fn decide(&self, wire: usize) -> Decision {
        let Some(p) = self.position(wire) else {
            return Decision::Constant(self.basis.bit(wire));
        };
        if let Some(v) = self.log.certified(wire) {
            assert!(
                !self.certainly_nonzero(p, !v) && !self.support_zero(p, v),
                "dense runtime: internal error: history certificate for wire {wire} = {v} \
                 contradicts the numerical state"
            );
            return Decision::Constant(v);
        }
        if self.support_zero(p, true) {
            return Decision::Constant(false);
        }
        if self.support_zero(p, false) {
            return Decision::Constant(true);
        }
        if self.certainly_nonzero(p, false) && self.certainly_nonzero(p, true) {
            return Decision::Varies;
        }
        let side = |v: bool| {
            format!(
                "|{}> branch norm in [{:e}, {:e}] with error bound {:e}",
                u8::from(v),
                self.branch_norm_lower(p, v),
                self.branch_norm(p, v),
                self.branch[p][usize::from(v)]
            )
        };
        Decision::Undecidable(format!(
            "wire {wire}: {}; {}; no exact history or support certificate",
            side(false),
            side(true)
        ))
    }

    /// Factor a wire already certified constant. In the exact state the
    /// other branch is zero, so dropping its computed residue does not
    /// increase the state error (the projection is a contraction).
    fn remove_constant(&mut self, wire: usize, value: bool) {
        let Some(p) = self.position(wire) else {
            self.basis.set_bit(wire, value);
            return;
        };
        let discarded = self.branch_norm(p, !value);
        let stride = 1usize << p;
        let len = self.state.len() / 2;
        let mut support = vec![0u64; words(len)];
        // Inserting a bit makes src >= destination; forward compaction
        // therefore never overwrites an unread source. No second vector.
        for i in 0..len {
            let src = pair_index(i, stride) | if value { stride } else { 0 };
            self.state.re[i] = self.state.re[src];
            self.state.im[i] = self.state.im[src];
            set_word_bit(&mut support, i, self.support_bit(src));
        }
        self.state.re.truncate(len);
        self.state.im.truncate(len);
        self.state.re.shrink_to_fit();
        self.state.im.shrink_to_fit();
        self.support = support;
        self.state.n -= 1;
        self.active.remove(p);
        self.branch.remove(p);
        self.basis.set_bit(wire, value);
        if discarded > 0.0 {
            self.projections += 1;
        }
    }

    fn compact(&mut self) {
        for wire in self.active.clone() {
            if let Decision::Constant(value) = self.decide(wire) {
                self.remove_constant(wire, value);
            }
        }
    }

    pub fn release(&mut self, wires: &[usize]) {
        for &wire in wires {
            if self.position(wire).is_some()
                && let Decision::Constant(value) = self.decide(wire)
            {
                self.remove_constant(wire, value);
            }
        }
    }

    /// Exact constant value, `None` when some wire certainly varies.
    ///
    /// # Panics
    /// When the certificates cannot settle the question (fail loud: a
    /// numerical engine never guesses a §5.6 cleanliness answer).
    pub fn constant_value(&self, wires: &[usize]) -> Option<BigUint> {
        let mut result = BigUint::zero();
        let mut undecidable = None;
        for (b, &wire) in wires.iter().enumerate() {
            match self.decide(wire) {
                Decision::Constant(v) => result.set_bit(b, v),
                Decision::Varies => return None,
                Decision::Undecidable(why) => {
                    undecidable.get_or_insert(why);
                }
            }
        }
        if let Some(why) = undecidable {
            panic!(
                "dense runtime: cleanliness/constant value is undecidable within the \
                 numerical error bound ({why}); the dense engine does not guess exact \
                 ISA decisions: rerun with the exact engine (BENNETT_VM_ENGINE unset)"
            );
        }
        Some(result)
    }

    pub fn prepare(&mut self, wires: &[usize], value: &BigUint) {
        assert!(
            self.constant_value(wires).is_some_and(|v| v.is_zero()),
            "dense runtime: preparation requires clean wires"
        );
        self.release(wires);
        let ws = sorted(wires);
        self.log
            .record(OpKind::Other, false, ws.clone(), Some(ws));
        for (b, &w) in wires.iter().enumerate() {
            self.basis.set_bit(w, value.bit(b));
            self.log.set_anchor(w, value.bit(b));
        }
    }

    pub fn set_state(&mut self, input: BTreeMap<BigUint, (f64, f64)>) {
        assert!(!input.is_empty(), "dense runtime: empty input state");
        *self = Self::new();
        self.basis = input.keys().next().unwrap().clone();
        let varying = input
            .keys()
            .fold(BigUint::zero(), |a, k| a.bitor(&k.bitxor(&self.basis)));
        for w in 0..varying.bits() {
            if varying.bit(w) {
                self.promote(w);
            }
        }
        self.state.re.fill(0.0);
        self.state.im.fill(0.0);
        self.support.fill(0);
        for (k, (r, i)) in input {
            let p = self
                .active
                .iter()
                .enumerate()
                .fold(0, |v, (b, &w)| v | (usize::from(k.bit(w)) << b));
            self.state.set_amplitude(p, Complex::new(r, i));
            set_word_bit(&mut self.support, p, r != 0.0 || i != 0.0);
        }
        // State preparation may contain an explicitly unnormalised vector.
        self.state.norm_upper = (self.state.norm2().sqrt() * (1.0 + 64.0 * U)).next_up();
        self.log
            .record(OpKind::Other, false, self.active.clone(), None);
        for w in 0..self.basis.bits() {
            if self.basis.bit(w) {
                self.log.set_anchor(w, true);
            }
        }
    }

    fn controls(&self, wires: &[usize]) -> Option<Vec<usize>> {
        let mut packed = vec![];
        for &wire in wires {
            if let Some(p) = self.position(wire) {
                if !packed.contains(&p) {
                    packed.push(p);
                }
            } else if !self.basis.bit(wire) {
                return None;
            }
        }
        Some(packed)
    }

    /// Lane-parallel support update over the amplitude pairs of packed
    /// position `p` under packed controls `cs`.
    fn support_pairs(&mut self, p: usize, cs: &[usize], f: impl Fn(u64, u64) -> (u64, u64)) {
        let stride = 1usize << p;
        let cmask: usize = cs.iter().fold(0, |a, &c| a | 1 << c);
        let (low, high) = (cmask & 63, cmask >> 6);
        let lane_ok = (0..64usize)
            .filter(|j| j & low == low)
            .fold(0u64, |m, j| m | 1 << j);
        let s = &mut self.support;
        if stride >= 64 {
            let ws = stride >> 6;
            for wi in 0..s.len() {
                if wi & ws != 0 || wi & high != high {
                    continue;
                }
                let wj = wi | ws;
                let (z0, z1) = (s[wi], s[wj]);
                let (o0, o1) = f(z0, z1);
                s[wi] = (z0 & !lane_ok) | (o0 & lane_ok);
                s[wj] = (z1 & !lane_ok) | (o1 & lane_ok);
            }
        } else {
            let m0 = (0..64usize)
                .filter(|j| j & stride == 0)
                .fold(0u64, |m, j| m | 1 << j)
                & lane_ok;
            let keep = !(m0 | (m0 << stride));
            for (wi, w) in s.iter_mut().enumerate() {
                if wi & high != high {
                    continue;
                }
                let z0 = *w & m0;
                let z1 = (*w >> stride) & m0;
                let (o0, o1) = f(z0, z1);
                *w = (*w & keep) | (o0 & m0) | ((o1 & m0) << stride);
            }
        }
    }

    fn rounding(&self, a: f64, j: usize, b0: f64, r0: f64) -> f64 {
        let absolute = j as f64 * f64::MIN_POSITIVE * (self.state.len() as f64).sqrt();
        (gamma(j) * a * (r0 + b0) + absolute) * (1.0 + 4.0 * U)
    }

    /// A kernel acting within every branch of `p` (p is not its target,
    /// or the kernel is diagonal) maps branch errors by its norm.
    fn charge_within(&mut self, a: f64, j: usize, b0: f64, r0: f64, skip: Option<usize>) {
        let round = self.rounding(a, j, b0, r0);
        let cap = self.state.bound;
        for (p, b) in self.branch.iter_mut().enumerate() {
            if Some(p) == skip {
                continue;
            }
            for v in b.iter_mut() {
                *v = ((a * *v + round) * (1.0 + 4.0 * U)).next_up().min(cap);
            }
        }
    }

    /// A non-diagonal kernel on target position `p` mixes its branches.
    fn charge_target(&mut self, p: usize, m: &[[Complex; 2]; 2], controlled: bool, b0: f64, r0: f64) {
        let a = matrix_norm_upper(m);
        self.charge_within(a, 32, b0, r0, Some(p));
        let round = self.rounding(a, 32, b0, r0);
        let cap = self.state.bound;
        let [e0, e1] = self.branch[p];
        for v in 0..2 {
            let mixed = modulus_upper(m[v][0]) * e0 + modulus_upper(m[v][1]) * e1 + round;
            let keep = if controlled { self.branch[p][v] } else { 0.0 };
            self.branch[p][v] = ((keep + mixed) * (1.0 + 4.0 * U)).next_up().min(cap);
        }
    }

    pub fn apply1(
        &mut self,
        target: usize,
        controls: &[usize],
        m: [[Complex; 2]; 2],
        projection_error: bool,
    ) {
        let Some(cs) = self.controls(controls) else {
            // A control is exact metadata zero: an exact identity.
            self.state.gates += 1;
            return;
        };
        let (exact, diagonal) = classify(&m);
        let cl = sorted(controls);
        let mut acts = cl.clone();
        acts.push(target);
        let acts = sorted(&acts);
        self.log.record(
            OpKind::Gate {
                target,
                controls: cl,
                m,
            },
            exact,
            if diagonal { vec![] } else { vec![target] },
            Some(acts),
        );
        // An entry is exactly zero only if its binary64 operand is exact.
        let zero = |r: usize, c: usize| m[r][c] == Complex::ZERO && (exact || diagonal);
        let (b0, r0) = (self.state.bound, self.state.norm_upper);
        if let Some(p) = self.position(target) {
            if zero(0, 1) && zero(1, 0) {
                self.state.diagonal1(p, &cs, m[0][0], m[1][1]);
                let a = phase_norm_upper(m[0][0]).max(phase_norm_upper(m[1][1]));
                self.charge_within(a, 8, b0, r0, None);
            } else {
                self.general(p, &cs, &m, exact, b0, r0);
            }
        } else {
            let bit = usize::from(self.basis.bit(target));
            if zero(1 - bit, bit) {
                self.state.phase_if_all(&cs, m[bit][bit]);
                self.charge_within(phase_norm_upper(m[bit][bit]), 8, b0, r0, None);
            } else if zero(bit, bit) && cs.is_empty() {
                self.basis.set_bit(target, bit == 0);
                self.state.phase_if_all(&[], m[1 - bit][bit]);
                self.charge_within(phase_norm_upper(m[1 - bit][bit]), 8, b0, r0, None);
            } else {
                let p = self.promote(target);
                self.general(p, &cs, &m, exact, b0, r0);
            }
        }
        if projection_error {
            // Conversion of exact symbolic gate entries to binary64 is
            // separate from the kernel's arithmetic charge.
            let charge = 32.0 * U * (self.state.norm_upper + self.state.bound);
            self.state.bound = (self.state.bound + charge).next_up();
            let cap = self.state.bound;
            for b in &mut self.branch {
                for v in b.iter_mut() {
                    *v = (*v + charge).next_up().min(cap);
                }
            }
        }
    }

    fn general(&mut self, p: usize, cs: &[usize], m: &[[Complex; 2]; 2], exact: bool, b0: f64, r0: f64) {
        self.state.apply1(p, cs, *m);
        let nz = |r: usize, c: usize| {
            if !exact || m[r][c] != Complex::ZERO {
                u64::MAX
            } else {
                0
            }
        };
        let (n00, n01, n10, n11) = (nz(0, 0), nz(0, 1), nz(1, 0), nz(1, 1));
        self.support_pairs(p, cs, |z0, z1| {
            ((n00 & z0) | (n01 & z1), (n10 & z0) | (n11 & z1))
        });
        self.charge_target(p, m, !cs.is_empty(), b0, r0);
    }

    pub fn flip(&mut self, target: usize, controls: &[usize]) {
        let Some(cs) = self.controls(controls) else {
            self.state.gates += 1;
            return;
        };
        let cl = sorted(controls);
        let mut acts = cl.clone();
        acts.push(target);
        let acts = sorted(&acts);
        self.log.record(
            OpKind::Flip {
                target,
                controls: cl,
            },
            true,
            vec![target],
            Some(acts),
        );
        if self.position(target).is_none() && cs.is_empty() {
            self.basis.set_bit(target, !self.basis.bit(target));
            self.state.gates += 1;
        } else {
            let p = self.promote(target);
            self.state.flip(p, &cs);
            self.support_pairs(p, &cs, |z0, z1| (z1, z0));
            let [e0, e1] = self.branch[p];
            self.branch[p] = if cs.is_empty() {
                [e1, e0]
            } else {
                let both = (e0.hypot(e1) * (1.0 + 4.0 * U)).next_up().min(self.state.bound);
                [both, both]
            };
        }
    }

    pub fn phase(&mut self, wires: &[usize], phase: Complex) {
        if let Some(cs) = self.controls(wires) {
            let ws = sorted(wires);
            self.log.record(
                OpKind::Phase {
                    wires: ws.clone(),
                    phase,
                },
                true,
                vec![],
                Some(ws),
            );
            let (b0, r0) = (self.state.bound, self.state.norm_upper);
            self.state.phase_if_all(&cs, phase);
            self.charge_within(phase_norm_upper(phase), 8, b0, r0, None);
        } else {
            self.state.gates += 1;
        }
    }

    /// Pure bijective logical remap. Infer a closed packed coordinate space,
    /// factoring uniform output bits into metadata. Cycle leaders need no
    /// visited table or second state. Involutions (all interpreter remaps)
    /// take linear work; general long cycles trade time for bounded scratch.
    pub fn map_basis(&mut self, mut f: impl FnMut(&BigUint) -> BigUint) {
        self.compact();
        let (output_basis, changed) = loop {
            let first = f(&self.key(0));
            let mut varying = BigUint::zero();
            let mut changed = first.bitxor(&self.key(0));
            for i in 1..self.state.len() {
                let key = self.key(i);
                let out = f(&key);
                varying = varying.bitor(&out.bitxor(&first));
                changed = changed.bitor(&out.bitxor(&key));
            }
            let additional: Vec<_> = (0..varying.bits())
                .filter(|&w| varying.bit(w) && self.position(w).is_none())
                .collect();
            if additional.is_empty() {
                break (first, changed);
            }
            for w in additional {
                self.promote(w);
            }
        };
        // A remap that is the identity on the packed domain (which holds the
        // whole exact support) is an exact no-op and is not logged.
        let writes: Vec<usize> = (0..changed.bits()).filter(|&w| changed.bit(w)).collect();
        if !writes.is_empty() {
            self.log.record(OpKind::Other, false, writes, None);
        }
        let mapped = |runtime: &Self, i: usize, f: &mut dyn FnMut(&BigUint) -> BigUint| {
            let k = f(&runtime.key(i));
            runtime
                .active
                .iter()
                .enumerate()
                .fold(0, |v, (b, &w)| v | (usize::from(k.bit(w)) << b))
        };
        let len = self.state.len();
        for root in 0..len {
            let mut cursor = mapped(self, root, &mut f);
            let mut steps = 0;
            while cursor > root {
                cursor = mapped(self, cursor, &mut f);
                steps += 1;
                assert!(steps <= len, "dense runtime: basis remap is not bijective");
            }
            if cursor < root {
                continue;
            }
            let mut carry = self.state.amplitude(root);
            let mut carry_bit = self.support_bit(root);
            loop {
                let next = mapped(self, cursor, &mut f);
                let saved = self.state.amplitude(next);
                let saved_bit = self.support_bit(next);
                self.state.set_amplitude(next, carry);
                set_word_bit(&mut self.support, next, carry_bit);
                carry = saved;
                carry_bit = saved_bit;
                cursor = next;
                if cursor == root {
                    break;
                }
            }
        }
        self.basis = output_basis;
        for &w in &self.active {
            self.basis.set_bit(w, false);
        }
        // Branch structure is not tracked through a general remap.
        let b = self.state.bound;
        for e in &mut self.branch {
            *e = [b, b];
        }
        self.state.gates += 1;
        self.compact();
    }

    pub fn measure(&mut self, wires: &[usize], draw: f64) -> (BigUint, String) {
        // Factor exactly certified constants (an actual inverse can leave a
        // computed residue on a wire the exact state has constant) before a
        // boundary draw, notably zero, can sample that residue.
        self.compact();
        let packed: Vec<_> = wires.iter().filter_map(|&w| self.position(w)).collect();
        let bounds: Vec<[f64; 2]> = packed.iter().map(|&p| self.branch[p]).collect();
        let (outcome, _) = self.state.measure_projected(&packed, draw, &bounds);
        let (mask, chosen) = packed
            .iter()
            .enumerate()
            .fold((0usize, 0usize), |(m, c), (b, &p)| {
                (m | 1 << p, c | ((outcome >> b & 1) << p))
            });
        let len = self.state.len();
        let mut support = vec![0u64; words(len)];
        for i in 0..len {
            if i & mask == 0 {
                set_word_bit(&mut support, i, self.support_bit(i | chosen));
            }
        }
        self.support = support;
        let b = self.state.bound;
        for e in &mut self.branch {
            *e = [b, b];
        }
        let mut value = BigUint::zero();
        let mut b = 0;
        for (i, &wire) in wires.iter().enumerate() {
            let bit = if self.position(wire).is_some() {
                let v = outcome >> b & 1 != 0;
                b += 1;
                v
            } else {
                self.basis.bit(wire)
            };
            value.set_bit(i, bit);
        }
        self.log.record(OpKind::Other, false, sorted(wires), None);
        for &wire in wires {
            self.log.set_anchor(wire, false);
            self.remove_constant(wire, false);
        }
        let bits = (0..wires.len())
            .map(|i| if value.bit(i) { '1' } else { '0' })
            .collect();
        (value, bits)
    }

    /// Exact equality of the two represented exact states.
    ///
    /// True only by an exact certificate (identical live operation history
    /// from shared ancestry, or two error-free equal vectors); false only
    /// when a distance, rounded down, exceeds the error bounds in total or
    /// on some wire branch.
    ///
    /// # Panics
    /// When neither can be established (fail loud, never guess).
    // Consumed by QuantumState::numerically_equals once bennett-vm-5vj lands
    // (src/action.rs is outside this change); unit-tested below.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn equals(&self, other: &Self) -> bool {
        if !self.log.merged && !other.log.merged && self.log.live_seqs() == other.log.live_seqs()
        {
            return true;
        }
        let a = self.snapshot();
        let b = other.snapshot();
        let wires: BTreeSet<usize> = self
            .active
            .iter()
            .chain(&other.active)
            .copied()
            .collect();
        let keys: BTreeSet<&BigUint> = a.keys().chain(b.keys()).collect();
        let mut total: f64 = 0.0;
        let mut per: BTreeMap<(usize, bool), f64> = BTreeMap::new();
        for k in &keys {
            let x = a.get(*k).copied().unwrap_or((0.0, 0.0));
            let y = b.get(*k).copied().unwrap_or((0.0, 0.0));
            let (dr, di) = (x.0 - y.0, x.1 - y.1);
            if dr == 0.0 && di == 0.0 {
                continue;
            }
            total = total.hypot(dr).hypot(di);
            for &w in &wires {
                let e = per.entry((w, k.bit(w))).or_default();
                *e = e.hypot(dr).hypot(di);
            }
        }
        // Each difference is exact or within one rounding of the true one.
        let lower = |d: f64| {
            ((d * (1.0 - gamma(4 * keys.len() + 8))) - 2.0 * keys.len() as f64 * f64::MIN_POSITIVE)
                .next_down()
                .max(0.0)
        };
        let ba = self.state.bound;
        let bb = other.state.bound;
        if lower(total) > ((ba + bb) * (1.0 + 4.0 * U)).next_up() {
            return false;
        }
        for (&(w, v), &d) in &per {
            let bound = |r: &Self| match r.position(w) {
                Some(p) => r.branch[p][usize::from(v)],
                None if r.basis.bit(w) == v => r.state.bound,
                None => 0.0,
            };
            if lower(d) > ((bound(self) + bound(other)) * (1.0 + 4.0 * U)).next_up() {
                return false;
            }
        }
        if total == 0.0 && ba == 0.0 && bb == 0.0 {
            return true;
        }
        panic!(
            "dense runtime: exact state equality is undecidable within the numerical \
             error bound (distance {total:e}, bounds {ba:e} + {bb:e}); the dense engine \
             does not guess exact ISA decisions: rerun with the exact engine"
        );
    }

    pub fn snapshot(&self) -> BTreeMap<BigUint, (f64, f64)> {
        (0..self.state.len())
            .filter_map(|i| {
                let z = self.state.amplitude(i);
                (z != Complex::ZERO).then(|| (self.key(i), (z.re, z.im)))
            })
            .collect()
    }

    pub fn bound(&self) -> f64 {
        self.state.bound
    }

    pub fn evidence(&self, logical_wires: usize) -> Json {
        Json::obj(vec![
            ("kind", Json::Str("numerical".into())),
            ("engine", Json::Str("dense".into())),
            ("error_bound", Json::Num(self.state.bound.to_string())),
            (
                "measurement_error_bound",
                Json::Num(self.state.measurement_bound.to_string()),
            ),
            ("metric", Json::Str("state_2_norm".into())),
            ("qubits", Json::u64(logical_wires as u64)),
            ("packed_qubits", Json::u64(self.active.len() as u64)),
            ("state_bytes", Json::u64(self.state.memory_bytes() as u64)),
            ("gates", Json::u64(self.state.gates)),
            ("numerical_projections", Json::u64(self.projections)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const S: f64 = std::f64::consts::FRAC_1_SQRT_2;
    fn c(re: f64) -> Complex {
        Complex::new(re, 0.0)
    }
    fn h() -> [[Complex; 2]; 2] {
        [[c(S), c(S)], [c(S), c(-S)]]
    }
    fn t(sign: f64) -> [[Complex; 2]; 2] {
        [[c(1.0), c(0.0)], [c(0.0), Complex::new(S, sign * S)]]
    }
    fn ry(theta: f64) -> [[Complex; 2]; 2] {
        let (s, k) = (theta / 2.0).sin_cos();
        [[c(k), c(-s)], [c(s), c(k)]]
    }

    /// Review probe d1b at the dense API: a state differing from its entry
    /// by ry(2e-12) after 2000 H is certainly unequal (per-branch bound).
    #[test]
    fn equals_refutes_tiny_difference_on_a_fresh_wire_branch() {
        let mut r = Runtime::new();
        for _ in 0..2000 {
            r.apply1(0, &[], h(), true);
        }
        let entry = r.clone();
        r.apply1(1, &[], ry(2e-12), true);
        assert!(r.bound() > 1e-12, "the global bound alone cannot refute");
        assert!(!r.equals(&entry));
        assert!(!entry.equals(&r));
    }

    /// An actual inverse of exact-operand gates cancels in the history
    /// certificate, so reversal equality is exact although B > 0.
    #[test]
    fn equals_certifies_actual_inverse_by_history() {
        let mut r = Runtime::new();
        r.apply1(2, &[], h(), true);
        let entry = r.clone();
        r.apply1(0, &[], h(), true);
        r.apply1(1, &[0], h(), true);
        r.flip(3, &[0, 1]);
        r.apply1(0, &[], t(1.0), true);
        r.phase(&[0, 3], c(-1.0));
        r.phase(&[0, 3], c(-1.0));
        r.apply1(0, &[], t(-1.0), true);
        r.flip(3, &[0, 1]);
        r.apply1(1, &[0], h(), true);
        r.apply1(0, &[], h(), true);
        assert!(r.bound() > 0.0);
        assert!(r.equals(&entry));
        assert!(entry.equals(&r));
    }

    /// Without a certificate and within the bound: fail loudly.
    #[test]
    #[should_panic(expected = "undecidable")]
    fn equals_faults_when_undecidable() {
        let mut r = Runtime::new();
        r.apply1(1, &[], h(), true);
        let entry = r.clone();
        r.apply1(0, &[], ry(0.3), true);
        r.apply1(0, &[], ry(-0.3), true);
        let _ = r.equals(&entry);
    }

    /// Independent runtimes with identical error-free vectors are equal;
    /// a distinct permutation is refuted.
    #[test]
    fn equals_on_error_free_states() {
        let mut a = Runtime::new();
        let mut b = Runtime::new();
        a.flip(4, &[]);
        b.flip(4, &[]);
        assert!(a.equals(&b));
        b.flip(5, &[]);
        assert!(!a.equals(&b));
    }

    /// Branch bounds stay sound: per-wire branch error against an exact
    /// reference after mixing, controls, permutations and conditioning.
    #[test]
    fn branch_bounds_cover_exact_branch_error() {
        let mut e = crate::action::QuantumState::new();
        let wires = e.alloc_wires(4);
        let mut r = Runtime::new();
        let amp = |m: [[Complex; 2]; 2]| m.map(|row| row.map(|z| (z.re, z.im)));
        let ops: Vec<(usize, Vec<usize>, [[Complex; 2]; 2])> = vec![
            (0, vec![], h()),
            (1, vec![0], h()),
            (2, vec![], ry(1.0)),
            (3, vec![1, 2], h()),
            (0, vec![], t(1.0)),
            (2, vec![3], ry(-0.5)),
        ];
        for _ in 0..30 {
            for (target, controls, m) in &ops {
                r.apply1(*target, controls, *m, true);
                e.apply1(wires[*target], controls, amp(*m));
            }
            r.flip(1, &[3]);
            e.flip(wires[1], &[wires[3]]);
        }
        let (re, rd) = (e.amplitudes(), r.snapshot());
        for (p, &w) in r.active.iter().enumerate() {
            for v in [false, true] {
                let keys: BTreeSet<&BigUint> =
                    re.keys().chain(rd.keys()).filter(|k| k.bit(w) == v).collect();
                let mut d2 = 0.0;
                for k in keys {
                    let x = re.get(k).copied().unwrap_or((0.0, 0.0));
                    let y = rd.get(k).copied().unwrap_or((0.0, 0.0));
                    d2 += (x.0 - y.0).powi(2) + (x.1 - y.1).powi(2);
                }
                // The exact reference itself is shown in binary64.
                let allowance = 8.0 * f64::EPSILON * 4.0;
                assert!(
                    d2.sqrt() <= r.branch[p][usize::from(v)] + allowance,
                    "wire {w} branch {v}: {:e} > {:e}",
                    d2.sqrt(),
                    r.branch[p][usize::from(v)]
                );
            }
        }
    }
}
