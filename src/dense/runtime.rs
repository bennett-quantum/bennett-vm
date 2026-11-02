//! Stable logical-wire adapter. Constant wires occupy metadata, not amplitudes.
//! The exact interpreter remains independent; this adapter never keeps a sparse
//! amplitude shadow. Snapshots are explicit diagnostics at API/report boundaries.
use super::{Complex, DenseState, U, pair_index};
use crate::{bigint::BigUint, json::Json};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct Runtime {
    state: DenseState,
    active: Vec<usize>,
    basis: BigUint,
    projections: u64,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            state: DenseState::new(0),
            active: vec![],
            basis: BigUint::zero(),
            projections: 0,
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
        if self.basis.bit(wire) {
            for i in 0..old {
                self.state.re[old + i] = self.state.re[i];
                self.state.im[old + i] = self.state.im[i];
                self.state.re[i] = 0.0;
                self.state.im[i] = 0.0;
            }
        }
        self.basis.set_bit(wire, false);
        self.active.push(wire);
        self.state.n += 1;
        p
    }

    fn constant_bit(&self, wire: usize) -> Option<bool> {
        let Some(p) = self.position(wire) else {
            return Some(self.basis.bit(wire));
        };
        let probs = self.state.probabilities(&[p]);
        // Only discard mass bounded by the accumulated amplitude error;
        // no fixed epsilon is used to turn a real superposition into a bit.
        let tolerance = self.state.bound.min(0.25).powi(2);
        let bound = self.state.bound.min(0.25);
        if probs[1] <= tolerance && self.branch_norm(p, true) <= bound {
            Some(false)
        } else if probs[0] <= tolerance && self.branch_norm(p, false) <= bound {
            Some(true)
        } else {
            None
        }
    }

    // Squaring a tiny but nonzero input amplitude can underflow. A
    // scaled norm must confirm a candidate zero before factoring it.
    fn branch_norm(&self, p: usize, value: bool) -> f64 {
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
        if norm == 0.0 {
            0.0
        } else {
            (norm * (1.0 + super::gamma(4 * steps + 8)) + steps as f64 * f64::MIN_POSITIVE)
                .next_up()
        }
    }

    fn remove_constant(&mut self, wire: usize, value: bool) {
        let Some(p) = self.position(wire) else {
            self.basis.set_bit(wire, value);
            return;
        };
        let discarded = self.branch_norm(p, !value);
        let stride = 1usize << p;
        let len = self.state.len() / 2;
        // Inserting a bit makes src >= destination; forward compaction
        // therefore never overwrites an unread source. No second vector.
        for i in 0..len {
            let src = pair_index(i, stride) | if value { stride } else { 0 };
            self.state.re[i] = self.state.re[src];
            self.state.im[i] = self.state.im[src];
        }
        self.state.re.truncate(len);
        self.state.im.truncate(len);
        self.state.re.shrink_to_fit();
        self.state.im.shrink_to_fit();
        self.state.n -= 1;
        self.active.remove(p);
        self.basis.set_bit(wire, value);
        if discarded > 0.0 {
            self.state.bound = (self.state.bound + discarded).next_up();
            self.projections += 1;
        }
    }

    fn compact(&mut self) {
        for wire in self.active.clone() {
            if let Some(value) = self.constant_bit(wire) {
                self.remove_constant(wire, value);
            }
        }
    }

    pub fn release(&mut self, wires: &[usize]) {
        for &wire in wires {
            if let Some(value) = self.constant_bit(wire) {
                self.remove_constant(wire, value);
            }
        }
    }

    pub fn constant_value(&self, wires: &[usize]) -> Option<BigUint> {
        let mut result = BigUint::zero();
        for (b, &wire) in wires.iter().enumerate() {
            result.set_bit(b, self.constant_bit(wire)?);
        }
        Some(result)
    }

    pub fn prepare(&mut self, wires: &[usize], value: &BigUint) {
        assert!(
            self.constant_value(wires).is_some_and(|v| v.is_zero()),
            "dense runtime: preparation requires clean wires"
        );
        self.release(wires);
        for (b, &w) in wires.iter().enumerate() {
            self.basis.set_bit(w, value.bit(b));
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
        for (k, (r, i)) in input {
            let p = self
                .active
                .iter()
                .enumerate()
                .fold(0, |v, (b, &w)| v | (usize::from(k.bit(w)) << b));
            self.state.set_amplitude(p, Complex::new(r, i));
        }
        // State preparation may contain an explicitly unnormalised vector.
        self.state.norm_upper = (self.state.norm2().sqrt() * (1.0 + 64.0 * U)).next_up();
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

    pub fn apply1(
        &mut self,
        target: usize,
        controls: &[usize],
        m: [[Complex; 2]; 2],
        projection_error: bool,
    ) {
        let Some(cs) = self.controls(controls) else {
            self.state.gates += 1;
            return;
        };
        if let Some(p) = self.position(target) {
            if m[0][1] == Complex::ZERO && m[1][0] == Complex::ZERO {
                self.state.diagonal1(p, &cs, m[0][0], m[1][1]);
            } else {
                self.state.apply1(p, &cs, m);
            }
        } else {
            let bit = usize::from(self.basis.bit(target));
            if m[1 - bit][bit] == Complex::ZERO {
                self.state.phase_if_all(&cs, m[bit][bit]);
            } else if m[bit][bit] == Complex::ZERO && cs.is_empty() {
                self.basis.set_bit(target, bit == 0);
                self.state.phase_if_all(&[], m[1 - bit][bit]);
            } else {
                let p = self.promote(target);
                self.state.apply1(p, &cs, m);
            }
        }
        if projection_error {
            // Conversion of exact symbolic gate entries to binary64 is
            // separate from the kernel's arithmetic charge.
            self.state.bound = (self.state.bound
                + 32.0 * U * (self.state.norm_upper + self.state.bound))
                .next_up();
        }
    }

    pub fn flip(&mut self, target: usize, controls: &[usize]) {
        let Some(cs) = self.controls(controls) else {
            self.state.gates += 1;
            return;
        };
        if self.position(target).is_none() && cs.is_empty() {
            self.basis.set_bit(target, !self.basis.bit(target));
            self.state.gates += 1;
        } else {
            let p = self.promote(target);
            self.state.flip(p, &cs);
        }
    }

    pub fn phase(&mut self, wires: &[usize], phase: Complex) {
        if let Some(cs) = self.controls(wires) {
            self.state.phase_if_all(&cs, phase);
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
        let output_basis = loop {
            let first = f(&self.key(0));
            let mut varying = BigUint::zero();
            for i in 1..self.state.len() {
                varying = varying.bitor(&f(&self.key(i)).bitxor(&first));
            }
            let additional: Vec<_> = (0..varying.bits())
                .filter(|&w| varying.bit(w) && self.position(w).is_none())
                .collect();
            if additional.is_empty() {
                break first;
            }
            for w in additional {
                self.promote(w);
            }
        };
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
            loop {
                let next = mapped(self, cursor, &mut f);
                let saved = self.state.amplitude(next);
                self.state.set_amplitude(next, carry);
                carry = saved;
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
        self.state.gates += 1;
        self.compact();
    }

    pub fn measure(&mut self, wires: &[usize], draw: f64) -> (BigUint, String) {
        let packed: Vec<_> = wires.iter().filter_map(|&w| self.position(w)).collect();
        let (outcome, _) = self.state.measure(&packed, draw);
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
        for &wire in wires {
            self.remove_constant(wire, false);
        }
        let bits = (0..wires.len())
            .map(|i| if value.bit(i) { '1' } else { '0' })
            .collect();
        (value, bits)
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
