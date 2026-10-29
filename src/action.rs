//! Quantum state simulation (§14.2) for Q actions reached from region
//! calls, plus the structured action executor (§5.3).
//!
//! The state is a sparse superposition over stable wire ids (bit `i`
//! of a basis key is wire `i`; freed wires keep their bit at zero).
//! Amplitudes are `f64` pairs: on a device simulator this serves Born
//! probabilities and basis execution; exact-amplitude output of a
//! standalone `run-action` is milestone 3.

use crate::bigint::BigUint;
use crate::object::{Function, Instr, Object};
use crate::value::{Budget, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub type Amp = (f64, f64);
type OutcomeCache = BTreeMap<(Vec<BigUint>, Vec<Vec<BigUint>>), (BigUint, BigUint)>;
type NormalizedCall<'a> = (bool, Option<&'a str>, &'a str, Vec<String>);

fn c_add(a: Amp, b: Amp) -> Amp {
    (a.0 + b.0, a.1 + b.1)
}
fn c_mul(a: Amp, b: Amp) -> Amp {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

#[derive(Clone, Debug)]
pub struct QuantumState {
    pub amps: BTreeMap<BigUint, Amp>,
    next_wire: usize,
}

impl QuantumState {
    pub fn new() -> QuantumState {
        let mut amps = BTreeMap::new();
        amps.insert(BigUint::zero(), (1.0, 0.0));
        QuantumState { amps, next_wire: 0 }
    }
    pub fn wire_count(&self) -> usize {
        self.next_wire
    }
    /// Reclaim only a clean suffix of simulator ids after actual inverse gates.
    /// This changes allocation bookkeeping, never quantum amplitudes.
    pub fn rewind_clean_wires(&mut self, boundary: usize) -> Result<(), String> {
        if boundary > self.next_wire
            || self
                .amps
                .keys()
                .any(|key| (boundary..self.next_wire).any(|w| key.bit(w)))
        {
            return Err("§7.4: inverse left a released wire unclean".into());
        }
        self.next_wire = boundary;
        Ok(())
    }
    pub fn alloc_wire(&mut self) -> usize {
        let w = self.next_wire;
        self.next_wire += 1;
        w
    }
    pub fn alloc_wires(&mut self, n: usize) -> Vec<usize> {
        (0..n).map(|_| self.alloc_wire()).collect()
    }
    fn prune(&mut self) {
        self.amps.retain(|_, a| !(a.0 == 0.0 && a.1 == 0.0));
        if self.amps.is_empty() {
            self.amps.insert(BigUint::zero(), (0.0, 0.0));
        }
    }
    /// Set basis bits on wires that start at zero in every component.
    pub fn prepare(&mut self, wires: &[usize], value: &BigUint) {
        self.amps = std::mem::take(&mut self.amps)
            .into_iter()
            .map(|(mut k, a)| {
                for (i, w) in wires.iter().enumerate() {
                    if value.bit(i) {
                        k.set_bit(*w, true);
                    }
                }
                (k, a)
            })
            .collect();
    }
    pub fn extract(key: &BigUint, wires: &[usize]) -> BigUint {
        let mut v = BigUint::zero();
        for (i, w) in wires.iter().enumerate() {
            if key.bit(*w) {
                v.set_bit(i, true);
            }
        }
        v
    }
    pub fn constant_value(&self, wires: &[usize]) -> Option<BigUint> {
        let mut seen: Option<BigUint> = None;
        for k in self.amps.keys() {
            let v = Self::extract(k, wires);
            match &seen {
                None => seen = Some(v),
                Some(prev) if *prev == v => {}
                _ => return None,
            }
        }
        seen
    }
    pub fn is_clean(&self, wires: &[usize]) -> bool {
        self.constant_value(wires) == Some(BigUint::zero())
    }
    /// Controlled single-qubit matrix `[[a,b],[c,d]]`.
    pub fn apply1(&mut self, target: usize, controls: &[usize], m: [[Amp; 2]; 2]) {
        let old = std::mem::take(&mut self.amps);
        let mut next: BTreeMap<BigUint, Amp> = BTreeMap::new();
        for (k, amp) in old {
            let enabled = controls.iter().all(|c| k.bit(*c));
            if !enabled {
                let e = next.entry(k).or_insert((0.0, 0.0));
                *e = c_add(*e, amp);
                continue;
            }
            let bit = k.bit(target) as usize;
            for (out_bit, coef) in [(0usize, m[0][bit]), (1usize, m[1][bit])] {
                if coef == (0.0, 0.0) {
                    continue;
                }
                let mut nk = k.clone();
                nk.set_bit(target, out_bit == 1);
                let entry = next.entry(nk).or_insert((0.0, 0.0));
                *entry = c_add(*entry, c_mul(coef, amp));
            }
        }
        self.amps = next;
        self.prune();
    }
    /// Flip `target` where every control is 1 (permutation gate).
    pub fn flip(&mut self, target: usize, controls: &[usize]) {
        self.apply1(
            target,
            controls,
            [[(0.0, 0.0), (1.0, 0.0)], [(1.0, 0.0), (0.0, 0.0)]],
        );
    }
    pub fn phase_if_all(&mut self, wires: &[usize], phase: Amp) {
        for (k, a) in self.amps.iter_mut() {
            if wires.iter().all(|w| k.bit(*w)) {
                *a = c_mul(phase, *a);
            }
        }
    }
    pub fn swap(&mut self, a: usize, b: usize) {
        if a == b {
            return;
        }
        self.amps = std::mem::take(&mut self.amps)
            .into_iter()
            .map(|(mut k, amp)| {
                let (ba, bb) = (k.bit(a), k.bit(b));
                k.set_bit(a, bb);
                k.set_bit(b, ba);
                (k, amp)
            })
            .collect();
    }
    /// Map every basis key through a classical function (branch
    /// enumeration of a permutation segment, NorthStar §4.2).
    pub fn map_basis(&mut self, mut f: impl FnMut(&BigUint) -> BigUint) {
        let old = std::mem::take(&mut self.amps);
        let mut next: BTreeMap<BigUint, Amp> = BTreeMap::new();
        for (k, a) in old {
            let nk = f(&k);
            let e = next.entry(nk).or_insert((0.0, 0.0));
            *e = c_add(*e, a);
        }
        self.amps = next;
        self.prune();
    }
    /// Measure `wires` (wire 0 first in `bits`); `draw` in [0,1) picks
    /// the outcome in ascending value order by cumulative probability.
    /// Returns (value, bits). The measured bits are reset to zero in
    /// the remaining state (the handle is consumed; the post-state
    /// factorises onto the outcome).
    pub fn measure(&mut self, wires: &[usize], draw: f64) -> (BigUint, String) {
        let mut probs: BTreeMap<BigUint, f64> = BTreeMap::new();
        for (k, a) in &self.amps {
            *probs.entry(Self::extract(k, wires)).or_insert(0.0) += a.0 * a.0 + a.1 * a.1;
        }
        let mut cumulative = 0.0;
        let mut chosen: Option<BigUint> = None;
        let mut ordered = probs.iter().collect::<Vec<_>>();
        // CLI draws enumerate outcomes lexically in wire order (wire 0 first).
        ordered.sort_by_key(|(v, _)| (0..wires.len()).map(|i| v.bit(i)).collect::<Vec<_>>());
        for (v, p) in ordered {
            cumulative += p;
            if draw < cumulative {
                chosen = Some(v.clone());
                break;
            }
        }
        let value = chosen
            .or_else(|| probs.keys().last().cloned())
            .unwrap_or_else(BigUint::zero);
        let p = probs.get(&value).copied().unwrap_or(1.0);
        let norm = if p > 0.0 { 1.0 / p.sqrt() } else { 1.0 };
        self.amps = std::mem::take(&mut self.amps)
            .into_iter()
            .filter(|(k, _)| Self::extract(k, wires) == value)
            .map(|(mut k, a)| {
                for w in wires {
                    k.set_bit(*w, false);
                }
                (k, (a.0 * norm, a.1 * norm))
            })
            .collect();
        let mut bits = String::new();
        for i in 0..wires.len() {
            bits.push(if value.bit(i) { '1' } else { '0' });
        }
        (value, bits)
    }
}

impl Default for QuantumState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegionWires {
    /// One wire bundle per cell, little-endian within the cell.
    pub cells: Vec<Vec<usize>>,
    pub a: usize,
    pub w: usize,
}

pub struct Exec<'o> {
    obj: &'o Object,
    state: &'o mut QuantumState,
    bundles: HashMap<String, Vec<usize>>,
    regions: HashMap<String, RegionWires>,
    local_images: HashMap<String, Vec<BigUint>>,
    args: HashMap<String, Value>,
    pred: Vec<usize>,
    borrowed: Vec<String>,
    idle: Vec<usize>,
    func: String,
}

fn diag(rule: &str, msg: impl std::fmt::Display) -> String {
    format!("§{rule}: {msg}")
}

fn parse_wire_operand(token: &str) -> (String, Option<usize>) {
    if let Some(i) = token.find('[')
        && let Some(inner) = token[i + 1..].strip_suffix(']')
        && let Ok(n) = inner.parse::<usize>()
    {
        return (token[..i].to_string(), Some(n));
    }
    (token.to_string(), None)
}

impl<'o> Exec<'o> {
    fn bundle(&self, name: &str) -> Result<Vec<usize>, String> {
        self.bundles.get(name).cloned().ok_or_else(|| {
            diag(
                "5.1",
                format!("unknown wire bundle {name:?} in @{}", self.func),
            )
        })
    }
    fn wire(&self, token: &str) -> Result<usize, String> {
        let (name, idx) = parse_wire_operand(token);
        let b = self.bundle(&name)?;
        let i = idx.unwrap_or(0);
        if idx.is_none() && b.len() != 1 {
            return Err(diag(
                "3.3",
                format!(
                    "wire operand {token:?} in @{} must index a bundle of width {}",
                    self.func,
                    b.len()
                ),
            ));
        }
        b.get(i).copied().ok_or_else(|| {
            diag(
                "3.3",
                format!("wire index in {token:?} is out of range in @{}", self.func),
            )
        })
    }
    fn angle(&self, token: &str) -> Result<f64, String> {
        if let Some(name) = token.strip_prefix('-')
            && self.args.contains_key(name)
        {
            return self.angle(name).map(|v| -v);
        }
        if let Ok(v) = token.parse::<f64>()
            && v.is_finite()
        {
            return Ok(v);
        }
        if let Some(name) = token.strip_prefix('-') {
            return Ok(-self.angle(name)?);
        }
        match self.args.get(token) {
            Some(Value::F64(v)) => Ok(*v),
            Some(Value::Int { bits, .. }) => {
                Ok(bits.to_u64().map(|v| v as f64).unwrap_or(f64::INFINITY))
            }
            _ => Err(diag(
                "4.4",
                format!(
                    "gate angle {token:?} in @{} is neither a finite literal nor a classical argument",
                    self.func
                ),
            )),
        }
    }
    fn controls_plus(&self, extra: &[usize]) -> Vec<usize> {
        let mut v = self.pred.clone();
        v.extend_from_slice(extra);
        v.sort_unstable();
        v.dedup();
        v
    }

    fn gate(&mut self, ins: &Instr) -> Result<(), String> {
        let pred = self.pred.clone();
        match ins.op.as_str() {
            "x" => {
                let t = self.wire(&ins.operands[0])?;
                self.state.flip(t, &pred);
            }
            "cx" => {
                let c = self.wire(&ins.operands[0])?;
                let t = self.wire(&ins.operands[1])?;
                let ctrls = self.controls_plus(&[c]);
                self.state.flip(t, &ctrls);
            }
            "ccx" => {
                let c1 = self.wire(&ins.operands[0])?;
                let c2 = self.wire(&ins.operands[1])?;
                let t = self.wire(&ins.operands[2])?;
                let ctrls = self.controls_plus(&[c1, c2]);
                self.state.flip(t, &ctrls);
            }
            "mcx" => {
                let ctrls_names = crate::validate::list_operand(&ins.operands[0])?;
                let mut ctrls = pred.clone();
                for c in &ctrls_names {
                    ctrls.push(self.wire(c)?);
                }
                let t = self.wire(&ins.operands[1])?;
                ctrls.sort_unstable();
                ctrls.dedup();
                self.state.flip(t, &ctrls);
            }
            "swap" => {
                let a = self.wire(&ins.operands[0])?;
                let b = self.wire(&ins.operands[1])?;
                // Controlled swap through controlled flips is exact only
                // as a basis permutation under predication: use Fredkin
                // decomposition (cx b a; ccx pred,a b; cx b a) when
                // predicated. Here a direct branch remap is exact too.
                if pred.is_empty() {
                    self.state.swap(a, b);
                } else {
                    self.state.map_basis(|k| {
                        let mut nk = k.clone();
                        if pred.iter().all(|p| k.bit(*p)) {
                            let (ba, bb) = (k.bit(a), k.bit(b));
                            nk.set_bit(a, bb);
                            nk.set_bit(b, ba);
                        }
                        nk
                    });
                }
            }
            "cz" => {
                let a = self.wire(&ins.operands[0])?;
                let b = self.wire(&ins.operands[1])?;
                let mut all = pred.clone();
                all.push(a);
                all.push(b);
                self.state.phase_if_all(&all, (-1.0, 0.0));
            }
            "h" => {
                let t = self.wire(&ins.operands[0])?;
                let s = std::f64::consts::FRAC_1_SQRT_2;
                self.state
                    .apply1(t, &pred, [[(s, 0.0), (s, 0.0)], [(s, 0.0), (-s, 0.0)]]);
            }
            "s" => self.diag(&ins.operands[0], (0.0, 1.0))?,
            "sdg" => self.diag(&ins.operands[0], (0.0, -1.0))?,
            "t" => {
                let s = std::f64::consts::FRAC_1_SQRT_2;
                self.diag(&ins.operands[0], (s, s))?;
            }
            "tdg" => {
                let s = std::f64::consts::FRAC_1_SQRT_2;
                self.diag(&ins.operands[0], (s, -s))?;
            }
            "rx" | "ry" | "rz" => {
                let th = self.angle(&ins.operands[0])?;
                let t = self.wire(&ins.operands[1])?;
                let (c, s) = ((th / 2.0).cos(), (th / 2.0).sin());
                let m: [[Amp; 2]; 2] = match ins.op.as_str() {
                    "rx" => [[(c, 0.0), (0.0, -s)], [(0.0, -s), (c, 0.0)]],
                    "ry" => [[(c, 0.0), (-s, 0.0)], [(s, 0.0), (c, 0.0)]],
                    _ => [[(c, -s), (0.0, 0.0)], [(0.0, 0.0), (c, s)]],
                };
                self.state.apply1(t, &pred, m);
            }
            _ => return Err(format!("unimplemented: gate {} (milestone 3)", ins.op)),
        }
        Ok(())
    }

    fn diag(&mut self, token: &str, phase: Amp) -> Result<(), String> {
        let t = self.wire(token)?;
        let pred = self.pred.clone();
        self.state
            .apply1(t, &pred, [[(1.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), phase]]);
        Ok(())
    }

    fn xor_bundle(&mut self, src: &[usize], dst: &[usize]) -> Result<(), String> {
        if src.len() != dst.len() {
            return Err(diag(
                "4.5",
                format!("bundle width mismatch in @{}", self.func),
            ));
        }
        let pred = self.pred.clone();
        for (s, d) in src.iter().zip(dst.iter()) {
            let ctrls = self.controls_plus(&[*s]);
            let _ = pred;
            self.state.flip(*d, &ctrls);
        }
        Ok(())
    }

    fn region(&self, name: &str) -> Result<RegionWires, String> {
        self.regions
            .get(name)
            .cloned()
            .ok_or_else(|| diag("4.6", format!("unknown region {name:?} in @{}", self.func)))
    }

    /// For each cell i, under (a == i): apply `body(cell wires)`.
    fn region_dispatch(
        &mut self,
        region: &RegionWires,
        addr: &[usize],
        value: &[usize],
        mode: &str,
    ) -> Result<(), String> {
        // Exact branchwise form: the controlled operations below are
        // permutations/phases selected by basis comparisons; on a
        // superposition they are defined branch by branch.
        let pred = self.pred.clone();
        match mode {
            "load" => {
                // d ^= cell[a]
                self.state.map_basis(|k| {
                    let mut nk = k.clone();
                    if pred.iter().all(|p| k.bit(*p)) {
                        let a = QuantumState::extract(k, addr)
                            .to_usize()
                            .unwrap_or(usize::MAX);
                        if a < region.cells.len() {
                            for (i, cw) in region.cells[a].iter().enumerate() {
                                if k.bit(*cw) {
                                    let d = value[i];
                                    let cur = nk.bit(d);
                                    nk.set_bit(d, !cur);
                                }
                            }
                        }
                    }
                    nk
                });
            }
            "store" => {
                self.state.map_basis(|k| {
                    let mut nk = k.clone();
                    if pred.iter().all(|p| k.bit(*p)) {
                        let a = QuantumState::extract(k, addr)
                            .to_usize()
                            .unwrap_or(usize::MAX);
                        if a < region.cells.len() {
                            for (i, cw) in region.cells[a].iter().enumerate() {
                                if k.bit(value[i]) {
                                    let cur = nk.bit(*cw);
                                    nk.set_bit(*cw, !cur);
                                }
                            }
                        }
                    }
                    nk
                });
            }
            "exch" => {
                self.state.map_basis(|k| {
                    let mut nk = k.clone();
                    if pred.iter().all(|p| k.bit(*p)) {
                        let a = QuantumState::extract(k, addr)
                            .to_usize()
                            .unwrap_or(usize::MAX);
                        if a < region.cells.len() {
                            for (i, cw) in region.cells[a].iter().enumerate() {
                                let (bv, bc) = (k.bit(value[i]), k.bit(*cw));
                                nk.set_bit(value[i], bc);
                                nk.set_bit(*cw, bv);
                            }
                        }
                    }
                    nk
                });
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn create_region(&mut self, ins: &Instr) -> Result<(), String> {
        let name = ins.operands[0].clone();
        let a: usize = ins.operands[1].parse().map_err(|_| {
            diag(
                "4.6",
                format!("qregion A in @{} is not an integer", self.func),
            )
        })?;
        let w: usize = ins.operands[2].parse().map_err(|_| {
            diag(
                "4.6",
                format!("qregion W in @{} is not an integer", self.func),
            )
        })?;
        if a > 20 {
            return Err(format!(
                "host limit: qregion {name:?} in @{} has 2^{a} cells",
                self.func
            ));
        }
        let ncells = 1usize << a;
        let mut image = vec![BigUint::zero(); ncells];
        if ins.operands.len() == 4 {
            let img = crate::validate::at_name(&ins.operands[3]).unwrap_or("");
            let bytes = crate::sha256::from_hex(self.obj.data.get(img).ok_or_else(|| {
                diag(
                    "4.6",
                    format!("qregion image @{img} undefined in @{}", self.func),
                )
            })?)
            .map_err(|e| diag("16.1", e))?;
            let want_bits = ncells
                .checked_mul(w)
                .ok_or_else(|| "host limit: qregion too large".to_string())?;
            if bytes.len() * 8 != want_bits {
                return Err(diag(
                    "4.6",
                    format!(
                        "qregion {name:?} image @{img} has {} bits, needs {want_bits}",
                        bytes.len() * 8
                    ),
                ));
            }
            let mut all = BigUint::zero();
            for (i, b) in bytes.iter().enumerate() {
                if *b != 0 {
                    all = all.add(&BigUint::from_u64(*b as u64).shl(i * 8));
                }
            }
            for (i, cell) in image.iter_mut().enumerate() {
                *cell = all.shr(i * w).mask(w);
            }
        }
        let mut cells = Vec::new();
        for cell in image.iter().take(ncells) {
            let wires = self.state.alloc_wires(w);
            if !cell.is_zero() {
                // Image gates are unconditional (§3.5).
                self.state.prepare(&wires, cell);
            }
            cells.push(wires);
        }
        self.regions
            .insert(name.clone(), RegionWires { cells, a, w });
        self.local_images.insert(name, image);
        Ok(())
    }

    fn release_local_regions(&mut self) -> Result<(), String> {
        let locals: Vec<String> = self.local_images.keys().cloned().collect();
        for name in locals {
            let region = self.region(&name)?;
            let image = self.local_images.get(&name).cloned().unwrap_or_default();
            for (i, cell) in region.cells.iter().enumerate() {
                let want = &image[i];
                let got = self.state.constant_value(cell);
                if got.as_ref() != Some(want) {
                    return Err(diag(
                        "5.6",
                        format!(
                            "local region {name:?} cell {i} of @{} is not back at its image on every component",
                            self.func
                        ),
                    ));
                }
                self.state.map_basis(|k| {
                    let mut nk = k.clone();
                    for w in cell {
                        nk.set_bit(*w, false);
                    }
                    nk
                });
            }
            self.regions.remove(&name);
        }
        Ok(())
    }

    fn exec_list(&mut self, instrs: &[Instr]) -> Result<(), String> {
        for ins in instrs {
            self.exec_instr(ins)?;
        }
        Ok(())
    }

    fn exec_instr(&mut self, ins: &Instr) -> Result<(), String> {
        let op = ins.op.as_str();
        if is_gate_name(op) {
            return self.gate(ins);
        }
        match op {
            "qalloc" | "qborrow" => {
                let n: usize = ins.operands[1].parse().map_err(|_| {
                    diag(
                        "4.3",
                        format!("{op} size in @{} is not an integer", self.func),
                    )
                })?;
                let wires = if op == "qborrow" {
                    if self.idle.len() < n {
                        return Err(diag(
                            "5.6",
                            "designated idle set lacks requested borrowed wires",
                        ));
                    }
                    self.idle.drain(..n).collect()
                } else {
                    self.state.alloc_wires(n)
                };
                if op == "qborrow" {
                    self.borrowed.push(ins.lhs[0].clone());
                }
                self.bundles.insert(ins.lhs[0].clone(), wires);
                Ok(())
            }
            "qfree" | "qreturn" => {
                let name = base_name(&ins.operands[0]);
                let wires = self.bundle(&name)?;
                if op == "qfree" && !self.state.is_clean(&wires) {
                    let rule = "5.6";
                    return Err(diag(
                        rule,
                        format!(
                            "{op} of {name:?} in @{}: bundle is not clean on every component",
                            self.func
                        ),
                    ));
                }
                if op == "qreturn" {
                    self.idle.extend(wires);
                }
                self.bundles.remove(&name);
                Ok(())
            }
            "qcopy" => {
                let src = self.bundle(&base_name(&ins.operands[0]))?;
                let dst = self.state.alloc_wires(src.len());
                self.bundles.insert(ins.lhs[0].clone(), dst.clone());
                self.xor_bundle(&src, &dst)
            }
            "quncopy" => {
                let src = self.bundle(&base_name(&ins.operands[0]))?;
                let dst_name = base_name(&ins.operands[1]);
                let dst = self.bundle(&dst_name)?;
                self.xor_bundle(&src, &dst)?;
                if !self.state.is_clean(&dst) {
                    return Err(diag(
                        "5.6",
                        format!(
                            "quncopy target {dst_name:?} in @{} is not clean after uncopy",
                            self.func
                        ),
                    ));
                }
                self.bundles.remove(&dst_name);
                Ok(())
            }
            "qxor" => {
                let s = self.bundle(&base_name(&ins.operands[0]))?;
                let t = self.bundle(&base_name(&ins.operands[1]))?;
                self.xor_bundle(&s, &t)
            }
            "qexch" => {
                let a = self.bundle(&base_name(&ins.operands[0]))?;
                let b = self.bundle(&base_name(&ins.operands[1]))?;
                if a.len() != b.len() {
                    return Err(diag(
                        "4.5",
                        format!("qexch width mismatch in @{}", self.func),
                    ));
                }
                let pred = self.pred.clone();
                self.state.map_basis(|k| {
                    let mut nk = k.clone();
                    if pred.iter().all(|p| k.bit(*p)) {
                        for (x, y) in a.iter().zip(b.iter()) {
                            let (bx, by) = (k.bit(*x), k.bit(*y));
                            nk.set_bit(*x, by);
                            nk.set_bit(*y, bx);
                        }
                    }
                    nk
                });
                Ok(())
            }
            "qregion" => Ok(()),
            "qregion_release" => Ok(()),
            "qload" => {
                let region = self.region(&ins.operands[0])?;
                let addr = self.bundle(&base_name(&ins.operands[1]))?;
                let dst = self.state.alloc_wires(region.w);
                self.bundles.insert(ins.lhs[0].clone(), dst.clone());
                self.region_dispatch(&region, &addr, &dst, "load")
            }
            "qunload" => {
                let region = self.region(&ins.operands[0])?;
                let addr = self.bundle(&base_name(&ins.operands[1]))?;
                let dname = base_name(&ins.operands[2]);
                let dst = self.bundle(&dname)?;
                self.region_dispatch(&region, &addr, &dst, "load")?;
                if !self.state.is_clean(&dst) {
                    return Err(diag(
                        "5.6",
                        format!(
                            "qunload bundle {dname:?} in @{} is not clean after unload",
                            self.func
                        ),
                    ));
                }
                self.bundles.remove(&dname);
                Ok(())
            }
            "qstore" => {
                let region = self.region(&ins.operands[0])?;
                let addr = self.bundle(&base_name(&ins.operands[1]))?;
                let v = self.bundle(&base_name(&ins.operands[2]))?;
                self.region_dispatch(&region, &addr, &v, "store")
            }
            "qexchm" => {
                let region = self.region(&ins.operands[0])?;
                let addr = self.bundle(&base_name(&ins.operands[1]))?;
                let v = self.bundle(&base_name(&ins.operands[2]))?;
                self.region_dispatch(&region, &addr, &v, "exch")
            }
            "qpred" => {
                let w = self.wire(&ins.operands[0])?;
                self.pred.push(w);
                Ok(())
            }
            "qunpred" => {
                let w = self.wire(&ins.operands[0])?;
                let popped = self.pred.pop();
                if popped != Some(w) {
                    return Err(diag(
                        "3.5",
                        format!(
                            "qunpred in @{} does not match the predicate stack top",
                            self.func
                        ),
                    ));
                }
                Ok(())
            }
            "qif" => {
                let w = self.wire(&ins.operands[0])?;
                self.pred.push(w);
                self.exec_list(&ins.nested[0].clone())?;
                self.pred.pop();
                let pred = self.pred.clone();
                self.state.flip(w, &pred);
                self.pred.push(w);
                self.exec_list(&ins.nested[1].clone())?;
                self.pred.pop();
                let pred = self.pred.clone();
                self.state.flip(w, &pred);
                Ok(())
            }
            "qloop" => self.exec_qloop(ins, false),
            "qloop_adj" => self.exec_qloop(ins, true),
            "oracle_call" | "action_call" | "adj" | "ctl" => self.exec_region_call(ins),
            "qinplace" => self.exec_qinplace(ins),
            _ => Err(format!(
                "unimplemented: Q instruction {op} in @{} (milestone 3)",
                self.func
            )),
        }
    }

    fn exec_qloop(&mut self, ins: &Instr, adjoint: bool) -> Result<(), String> {
        let n: usize = ins.operands[0].parse().map_err(|_| {
            diag(
                "4.6",
                format!("qloop count in @{} is not an integer", self.func),
            )
        })?;
        let body = ins.nested[0].clone();
        if ins.operands.len() == 1 {
            for _ in 0..n {
                self.exec_list(&body)?;
            }
            return Ok(());
        }
        // while form (§4.6): flags c_i = AND of cond[0..i].
        let cond = self.bundle(&base_name(&ins.operands[2]))?;
        if cond.len() < n {
            return Err(diag(
                "4.6",
                format!(
                    "qloop while in @{} needs {n} condition wires, bundle has {}",
                    self.func,
                    cond.len()
                ),
            ));
        }
        let flags = self.state.alloc_wires(n);
        let pred = self.pred.clone();
        let compute_flags = |st: &mut QuantumState| {
            // cx cond[0] c1; ccx c_i cond[i] c_{i+1} (gates, so predicated)
            if n > 0 {
                let mut ctrls = pred.clone();
                ctrls.push(cond[0]);
                st.flip(flags[0], &ctrls);
                for i in 1..n {
                    let mut c2 = pred.clone();
                    c2.push(flags[i - 1]);
                    c2.push(cond[i]);
                    c2.sort_unstable();
                    c2.dedup();
                    st.flip(flags[i], &c2);
                }
            }
        };
        compute_flags(self.state);
        let order: Vec<usize> = if adjoint {
            (0..n).rev().collect()
        } else {
            (0..n).collect()
        };
        for i in order {
            self.pred.push(flags[i]);
            let r = self.exec_list(&body);
            self.pred.pop();
            r?;
        }
        // Invert the dependency chain from its last flag to its first.
        for i in (1..n).rev() {
            let mut ctrls = pred.clone();
            ctrls.extend([flags[i - 1], cond[i]]);
            self.state.flip(flags[i], &ctrls);
        }
        if n > 0 {
            let mut ctrls = pred.clone();
            ctrls.push(cond[0]);
            self.state.flip(flags[0], &ctrls);
        }
        if !self.state.is_clean(&flags) {
            return Err(diag(
                "4.6",
                format!(
                    "qloop while flags in @{} did not uncompute clean",
                    self.func
                ),
            ));
        }
        Ok(())
    }
}

pub fn base_name(token: &str) -> String {
    parse_wire_operand(token).0
}

pub fn is_gate_name(op: &str) -> bool {
    [
        "x", "h", "s", "sdg", "t", "tdg", "cx", "ccx", "mcx", "swap", "cz", "rx", "ry", "rz",
    ]
    .contains(&op)
}

impl<'o> Exec<'o> {
    fn resolve_args(
        &self,
        types: &[crate::object::Type],
        names: &[String],
    ) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        for (ty, name) in types.iter().zip(names.iter()) {
            match ty {
                crate::object::Type::Token => match self.args.get(name) {
                    Some(Value::Token(b)) => out.push(Value::Token(*b)),
                    _ => {
                        return Err(diag(
                            "4.7",
                            format!(
                                "token argument {name:?} in @{} is not a classical token argument",
                                self.func
                            ),
                        ));
                    }
                },
                crate::object::Type::F64 => match self.args.get(name) {
                    Some(v @ Value::F64(_)) => out.push(v.clone()),
                    _ => {
                        let v: f64 = name.parse().map_err(|_| {
                            diag(
                                "4.7",
                                format!("f64 argument {name:?} in @{} is unresolved", self.func),
                            )
                        })?;
                        out.push(Value::F64(v));
                    }
                },
                crate::object::Type::I(n) => match self.args.get(name) {
                    Some(Value::Int { bits, .. }) => out.push(Value::int(*n, bits.clone())),
                    _ => {
                        let v = BigUint::parse_int(name).map_err(|_| {
                            diag(
                                "4.7",
                                format!(
                                    "classical argument {name:?} in @{} is unresolved",
                                    self.func
                                ),
                            )
                        })?;
                        out.push(Value::int(*n, v));
                    }
                },
                crate::object::Type::Ptr => match self.args.get(name) {
                    Some(Value::Ptr(b)) => out.push(Value::Ptr(b.clone())),
                    Some(Value::Int { bits, .. }) => out.push(Value::Ptr(bits.clone())),
                    _ => {
                        return Err(diag(
                            "4.7",
                            format!("ptr argument {name:?} in @{} is unresolved", self.func),
                        ));
                    }
                },
                _ => {
                    return Err(diag(
                        "4.7",
                        format!(
                            "unsupported classical argument type {} in @{}",
                            ty.signature_text(),
                            self.func
                        ),
                    ));
                }
            }
        }
        Ok(out)
    }

    fn token_arg(&self, name: &str) -> Result<Budget, String> {
        match self.args.get(name) {
            Some(Value::Token(b)) => Ok(*b),
            _ => Err(diag(
                "4.7",
                format!(
                    "token operand {name:?} in @{} is not a token argument",
                    self.func
                ),
            )),
        }
    }

    fn exec_region_call(&mut self, ins: &Instr) -> Result<(), String> {
        // Normalize adj / ctl wrappers.
        let (adjoint, control, call_op, operands) = normalize_call(ins)?;
        let mut pushed = false;
        if let Some(tok) = control {
            let w = self.wire(tok)?;
            self.pred.push(w);
            pushed = true;
        }
        let result = if call_op == "oracle_call" {
            // An oracle is its own adjoint (§4.7).
            self.exec_oracle_call(&operands)
        } else {
            self.exec_action_call(&operands, adjoint)
        };
        if pushed {
            self.pred.pop();
        }
        result
    }

    fn exec_oracle_call(&mut self, operands: &[String]) -> Result<(), String> {
        let callee = crate::validate::at_name(&operands[0])
            .unwrap_or("")
            .to_string();
        let budget = self.token_arg(&operands[1])?;
        let sig = crate::validate::oracle_sig(self.obj, &callee)?;
        let in_names = crate::validate::list_operand(&operands[2])?;
        let mem_names = crate::validate::list_operand(&operands[5])?;
        let arg_names = crate::validate::list_operand(&operands[6])?;
        let args = self.resolve_args(&sig.args, &arg_names)?;
        let in_bundles: Vec<Vec<usize>> = in_names
            .iter()
            .map(|n| self.bundle(&base_name(n)))
            .collect::<Result<_, _>>()?;
        let tgt = self.bundle(&base_name(&operands[3]))?;
        let fr = self.bundle(&base_name(&operands[4]))?;
        let mem_regions: Vec<RegionWires> = mem_names
            .iter()
            .map(|n| self.region(n))
            .collect::<Result<_, _>>()?;
        let obj = self.obj;
        let pred = self.pred.clone();
        let mut first_error: Option<String> = None;
        let mut outcomes: OutcomeCache = Default::default();
        self.state.map_basis(|k| {
            if first_error.is_some() {
                return k.clone();
            }
            let mut nk = k.clone();
            if !pred.iter().all(|p| k.bit(*p)) {
                return nk;
            }
            let inputs: Vec<BigUint> = in_bundles
                .iter()
                .map(|b| QuantumState::extract(k, b))
                .collect();
            let mem_cells: Vec<Vec<BigUint>> = mem_regions
                .iter()
                .map(|r| {
                    r.cells
                        .iter()
                        .map(|c| QuantumState::extract(k, c))
                        .collect()
                })
                .collect();
            let key = (inputs.clone(), mem_cells.clone());
            let outcome = match outcomes.get(&key) {
                Some(o) => o.clone(),
                None => {
                    let mut mem = mem_cells.clone();
                    match crate::oracle::run_oracle(obj, &callee, &inputs, &mut mem, &args, budget)
                    {
                        Ok(o) => {
                            let pair = (o.payload.clone(), o.code.clone());
                            outcomes.insert(key, pair.clone());
                            pair
                        }
                        Err(e) => {
                            first_error = Some(e);
                            return nk;
                        }
                    }
                }
            };
            xor_into(&mut nk, &tgt, &outcome.0);
            xor_into(&mut nk, &fr, &outcome.1);
            nk
        });
        if let Some(e) = first_error {
            return Err(e);
        }
        Ok(())
    }

    fn exec_action_call(&mut self, operands: &[String], adjoint: bool) -> Result<(), String> {
        let callee = crate::validate::at_name(&operands[0])
            .unwrap_or("")
            .to_string();
        let sig = crate::validate::action_sig(self.obj, &callee)?;
        let mut_names = crate::validate::list_operand(&operands[1])?;
        let in_names = crate::validate::list_operand(&operands[2])?;
        let mem_names = crate::validate::list_operand(&operands[3])?;
        let arg_names = crate::validate::list_operand(&operands[4])?;
        let args = self.resolve_args(&sig.args, &arg_names)?;
        let muts: Vec<Vec<usize>> = mut_names
            .iter()
            .map(|n| self.bundle(&base_name(n)))
            .collect::<Result<_, _>>()?;
        let ins_b: Vec<Vec<usize>> = in_names
            .iter()
            .map(|n| self.bundle(&base_name(n)))
            .collect::<Result<_, _>>()?;
        let mems: Vec<RegionWires> = mem_names
            .iter()
            .map(|n| self.region(n))
            .collect::<Result<_, _>>()?;
        let pred = self.pred.clone();
        execute_action_with_idle(
            self.obj, self.state, &callee, &muts, &ins_b, &mems, &args, adjoint, &pred, &self.idle,
        )
    }

    fn exec_qinplace(&mut self, ins: &Instr) -> Result<(), String> {
        // §4.5 expansion, through the oracle_call machinery directly.
        let g = crate::validate::at_name(&ins.operands[0])
            .unwrap_or("")
            .to_string();
        let ginv = crate::validate::at_name(&ins.operands[1])
            .unwrap_or("")
            .to_string();
        let tk_name = ins.operands[2].clone();
        let budget = self.token_arg(&tk_name)?;
        let x_name = base_name(&ins.operands[3]);
        let x = self.bundle(&x_name)?;
        let n = x.len();
        let _ = budget;
        let t = self.state.alloc_wires(n);
        let fsig = crate::validate::oracle_sig(self.obj, &g)?;
        let f = self.state.alloc_wires(fsig.fault);
        self.bundles
            .insert("\u{0}qinplace_t".to_string(), t.clone());
        self.bundles
            .insert("\u{0}qinplace_f".to_string(), f.clone());
        let call = |exec: &mut Exec, oracle: &str| -> Result<(), String> {
            let operands = vec![
                format!("@{oracle}"),
                tk_name.clone(),
                format!("({x_name})"),
                "\u{0}qinplace_t".to_string(),
                "\u{0}qinplace_f".to_string(),
                "()".to_string(),
                "()".to_string(),
            ];
            exec.exec_oracle_call(&operands)
        };
        let r = (|| {
            call(self, &g)?;
            // qexch x t under the predicate
            let pred = self.pred.clone();
            self.state.map_basis(|k| {
                let mut nk = k.clone();
                if pred.iter().all(|p| k.bit(*p)) {
                    for (xw, tw) in x.iter().zip(t.iter()) {
                        let (bx, bt) = (k.bit(*xw), k.bit(*tw));
                        nk.set_bit(*xw, bt);
                        nk.set_bit(*tw, bx);
                    }
                }
                nk
            });
            call(self, &ginv)?;
            if !self.state.is_clean(&t) || !self.state.is_clean(&f) {
                return Err(diag(
                    "4.5",
                    format!(
                        "qinplace workspace in @{} is not clean after the inverse call (the declared inverse pair does not compose to identity, or a fault code was delivered)",
                        self.func
                    ),
                ));
            }
            Ok(())
        })();
        self.bundles.remove("\u{0}qinplace_t");
        self.bundles.remove("\u{0}qinplace_f");
        r
    }
}

pub fn xor_into(key: &mut BigUint, wires: &[usize], value: &BigUint) {
    for (i, w) in wires.iter().enumerate() {
        if value.bit(i) {
            let cur = key.bit(*w);
            key.set_bit(*w, !cur);
        }
    }
}

/// (`adjoint`, control token, call op, call operands)
fn normalize_call(ins: &Instr) -> Result<NormalizedCall<'_>, String> {
    match ins.op.as_str() {
        "oracle_call" | "action_call" => Ok((false, None, ins.op.as_str(), ins.operands.clone())),
        "adj" => {
            let inner = ins.operands.first().ok_or("adj with no call")?;
            Ok((
                true,
                None,
                if inner == "oracle_call" {
                    "oracle_call"
                } else {
                    "action_call"
                },
                ins.operands[1..].to_vec(),
            ))
        }
        "ctl" => {
            let ctrl = ins.operands.first().ok_or("ctl with no control")?;
            let second = ins.operands.get(1).ok_or("ctl with no call")?;
            if second == "adj" {
                let inner = ins.operands.get(2).ok_or("ctl adj with no call")?;
                Ok((
                    true,
                    Some(ctrl.as_str()),
                    if inner == "oracle_call" {
                        "oracle_call"
                    } else {
                        "action_call"
                    },
                    ins.operands[3..].to_vec(),
                ))
            } else {
                Ok((
                    false,
                    Some(ctrl.as_str()),
                    if second == "oracle_call" {
                        "oracle_call"
                    } else {
                        "action_call"
                    },
                    ins.operands[2..].to_vec(),
                ))
            }
        }
        _ => Err(format!("normalize_call on {}", ins.op)),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn execute_action_inner(
    obj: &Object,
    state: &mut QuantumState,
    name: &str,
    muts: &[Vec<usize>],
    ins_b: &[Vec<usize>],
    mems: &[RegionWires],
    args: &[Value],
    adjoint: bool,
    inherited_pred: &[usize],
) -> Result<(), String> {
    execute_action_with_idle(
        obj,
        state,
        name,
        muts,
        ins_b,
        mems,
        args,
        adjoint,
        inherited_pred,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn execute_action_with_idle(
    obj: &Object,
    state: &mut QuantumState,
    name: &str,
    muts: &[Vec<usize>],
    ins_b: &[Vec<usize>],
    mems: &[RegionWires],
    args: &[Value],
    adjoint: bool,
    inherited_pred: &[usize],
    idle: &[usize],
) -> Result<(), String> {
    let f = obj
        .function(name)
        .ok_or_else(|| diag("5.3", format!("action @{name} is imported; executing an unresolved import needs linking (milestone 2)")))?;
    if !matches!(
        f.kind,
        crate::object::FnKind::Action | crate::object::FnKind::PermAction
    ) {
        return Err(diag("5.3", format!("@{name} is not an action")));
    }
    let mut bundles: HashMap<String, Vec<usize>> = HashMap::new();
    for (p, wires) in f.lists[0].iter().zip(muts.iter()) {
        bundles.insert(p.name.clone(), wires.clone());
    }
    for (p, wires) in f.lists[1].iter().zip(ins_b.iter()) {
        bundles.insert(p.name.clone(), wires.clone());
    }
    let mut regions: HashMap<String, RegionWires> = HashMap::new();
    for (p, region) in f.lists[2].iter().zip(mems.iter()) {
        regions.insert(p.name.clone(), region.clone());
    }
    let mut arg_env: HashMap<String, Value> = HashMap::new();
    for (p, v) in f.lists[3].iter().zip(args.iter()) {
        arg_env.insert(p.name.clone(), v.clone());
    }
    let interface_bundles: BTreeSet<String> = bundles.keys().cloned().collect();
    let widths = bundle_widths(obj, f);
    let body_source: Vec<Instr> = f.blocks.iter().flat_map(|b| b.instrs.clone()).collect();
    let body = if adjoint {
        adjoint_list(&body_source, &widths)
    } else {
        body_source.clone()
    };
    let mut ex = Exec {
        obj,
        state,
        bundles,
        regions,
        local_images: HashMap::new(),
        args: arg_env,
        pred: inherited_pred.to_vec(),
        borrowed: Vec::new(),
        idle: idle.to_vec(),
        func: name.to_string(),
    };
    // §4.6: all lexical regions exist at entry, including those inside
    // zero-trip loops or declared after their first use.
    let mut declarations = Vec::new();
    for ins in &body_source {
        collect_qregions(ins, &mut |q| declarations.push(q.clone()));
    }
    for q in declarations {
        ex.create_region(&q)?;
    }
    ex.exec_list(&body)?;
    ex.release_local_regions()?;
    if ex.pred.len() != inherited_pred.len() {
        return Err(diag(
            "3.5",
            format!("predicate stack of @{name} is not restored at exit"),
        ));
    }
    let leaked: Vec<&String> = ex
        .bundles
        .keys()
        .filter(|k| !interface_bundles.contains(*k))
        .collect();
    if !leaked.is_empty() {
        return Err(diag(
            "5.1",
            format!("action @{name} exits with unmatched allocations {leaked:?}"),
        ));
    }
    Ok(())
}

fn collect_qregions(ins: &Instr, out: &mut impl FnMut(&Instr)) {
    if ins.op == "qregion" {
        out(ins);
    }
    for arm in &ins.nested {
        for sub in arm {
            collect_qregions(sub, out);
        }
    }
}

/// Static bundle widths of an action body, for adjoint alloc/free swaps.
fn bundle_widths(_obj: &Object, f: &Function) -> HashMap<String, usize> {
    let mut widths: HashMap<String, usize> = HashMap::new();
    for list in [&f.lists[0], &f.lists[1]] {
        for p in list.iter() {
            if let crate::object::Type::Wire(n) = p.ty {
                widths.insert(p.name.clone(), n);
            }
        }
    }
    fn walk(instrs: &[Instr], widths: &mut HashMap<String, usize>) {
        for ins in instrs {
            match ins.op.as_str() {
                "qalloc" | "qborrow" => {
                    if let (Some(lhs), Some(n)) = (
                        ins.lhs.first(),
                        ins.operands.get(1).and_then(|s| s.parse::<usize>().ok()),
                    ) {
                        widths.insert(lhs.clone(), n);
                    }
                }
                "qcopy" => {
                    if let (Some(lhs), Some(src)) = (ins.lhs.first(), ins.operands.first())
                        && let Some(w) = widths.get(&base_name(src)).copied()
                    {
                        widths.insert(lhs.clone(), w);
                    }
                }
                _ => {}
            }
            for arm in &ins.nested {
                walk(arm, widths);
            }
        }
    }
    for b in &f.blocks {
        walk(&b.instrs, &mut widths);
    }
    widths
}

/// Adjoint of an instruction list (§5.3 table).
fn adjoint_list(instrs: &[Instr], widths: &HashMap<String, usize>) -> Vec<Instr> {
    let mut out = Vec::new();
    for ins in instrs.iter().rev() {
        out.push(adjoint_instr(ins, widths));
    }
    out
}

fn renamed(
    ins: &Instr,
    op: &str,
    lhs: Vec<String>,
    operands: Vec<String>,
    nested: Vec<Vec<Instr>>,
) -> Instr {
    Instr {
        lhs,
        op: op.to_string(),
        operands,
        nested,
        line: ins.line,
        raw: ins.raw.clone(),
    }
}

fn adjoint_instr(ins: &Instr, widths: &HashMap<String, usize>) -> Instr {
    let op = ins.op.as_str();
    match op {
        "s" => renamed(ins, "sdg", vec![], ins.operands.clone(), vec![]),
        "sdg" => renamed(ins, "s", vec![], ins.operands.clone(), vec![]),
        "t" => renamed(ins, "tdg", vec![], ins.operands.clone(), vec![]),
        "tdg" => renamed(ins, "t", vec![], ins.operands.clone(), vec![]),
        "rx" | "ry" | "rz" => {
            let angle = &ins.operands[0];
            let neg = if let Some(rest) = angle.strip_prefix('-') {
                rest.to_string()
            } else {
                format!("-{angle}")
            };
            renamed(ins, op, vec![], vec![neg, ins.operands[1].clone()], vec![])
        }
        "qalloc" => renamed(ins, "qfree", vec![], ins.lhs.clone(), vec![]),
        "qfree" => {
            let name = base_name(&ins.operands[0]);
            let n = widths.get(&name).copied().unwrap_or(1);
            renamed(
                ins,
                "qalloc",
                vec![name],
                vec!["any".into(), n.to_string()],
                vec![],
            )
        }
        "qborrow" => renamed(ins, "qreturn", vec![], ins.lhs.clone(), vec![]),
        "qreturn" => {
            let name = base_name(&ins.operands[0]);
            let n = widths.get(&name).copied().unwrap_or(1);
            renamed(
                ins,
                "qborrow",
                vec![name],
                vec!["any".into(), n.to_string()],
                vec![],
            )
        }
        "qcopy" => renamed(
            ins,
            "quncopy",
            vec![],
            vec![ins.operands[0].clone(), ins.lhs[0].clone()],
            vec![],
        ),
        "quncopy" => renamed(
            ins,
            "qcopy",
            vec![ins.operands[1].clone()],
            vec![ins.operands[0].clone()],
            vec![],
        ),
        "qload" => renamed(
            ins,
            "qunload",
            vec![],
            vec![
                ins.operands[0].clone(),
                ins.operands[1].clone(),
                ins.lhs[0].clone(),
            ],
            vec![],
        ),
        "qunload" => renamed(
            ins,
            "qload",
            vec![ins.operands[2].clone()],
            vec![ins.operands[0].clone(), ins.operands[1].clone()],
            vec![],
        ),
        "qregion" => renamed(ins, "qregion_release", vec![], ins.operands.clone(), vec![]),
        "qinplace" => renamed(
            ins,
            "qinplace",
            vec![],
            vec![
                ins.operands[1].clone(),
                ins.operands[0].clone(),
                ins.operands[2].clone(),
                ins.operands[3].clone(),
            ],
            vec![],
        ),
        "oracle_call" => ins.clone(),
        "action_call" => {
            let mut operands = vec!["action_call".to_string()];
            operands.extend(ins.operands.iter().cloned());
            renamed(ins, "adj", vec![], operands, vec![])
        }
        "adj" => {
            if ins.operands.first().map(|s| s.as_str()) == Some("action_call") {
                renamed(
                    ins,
                    "action_call",
                    vec![],
                    ins.operands[1..].to_vec(),
                    vec![],
                )
            } else {
                ins.clone()
            }
        }
        "ctl" => {
            if ins.operands.get(1).map(|s| s.as_str()) == Some("action_call") {
                let mut operands = vec![ins.operands[0].clone(), "adj".to_string()];
                operands.extend(ins.operands[1..].iter().cloned());
                renamed(ins, "ctl", vec![], operands, vec![])
            } else if ins.operands.get(1).map(|s| s.as_str()) == Some("adj") {
                let mut operands = vec![ins.operands[0].clone()];
                operands.extend(ins.operands[2..].iter().cloned());
                renamed(ins, "ctl", vec![], operands, vec![])
            } else {
                ins.clone()
            }
        }
        "qpred" => renamed(ins, "qunpred", vec![], ins.operands.clone(), vec![]),
        "qunpred" => renamed(ins, "qpred", vec![], ins.operands.clone(), vec![]),
        "qif" => renamed(
            ins,
            "qif",
            vec![],
            ins.operands.clone(),
            vec![
                adjoint_list(&ins.nested[0], widths),
                adjoint_list(&ins.nested[1], widths),
            ],
        ),
        "qloop" => {
            if ins.operands.len() == 3 {
                renamed(
                    ins,
                    "qloop_adj",
                    vec![],
                    ins.operands.clone(),
                    vec![adjoint_list(&ins.nested[0], widths)],
                )
            } else {
                renamed(
                    ins,
                    "qloop",
                    vec![],
                    ins.operands.clone(),
                    vec![adjoint_list(&ins.nested[0], widths)],
                )
            }
        }
        _ => ins.clone(), // x cx ccx mcx swap h cz qxor qexch qstore qexchm
    }
}

/// Execute a permutation action on basis values (`lift.perm_call`).
/// Returns (final muts, final mem cells).
pub fn execute_perm_basis(
    obj: &Object,
    name: &str,
    muts: &[BigUint],
    ins_v: &[BigUint],
    mems: &[Vec<BigUint>],
    args: &[Value],
) -> Result<(Vec<BigUint>, Vec<Vec<BigUint>>), String> {
    let sig = crate::validate::action_sig(obj, name)?;
    let mut state = QuantumState::new();
    let mut_bundles: Vec<Vec<usize>> = sig.muts.iter().map(|w| state.alloc_wires(*w)).collect();
    let in_bundles: Vec<Vec<usize>> = sig.ins.iter().map(|w| state.alloc_wires(*w)).collect();
    for (b, v) in mut_bundles.iter().zip(muts.iter()) {
        state.prepare(b, v);
    }
    for (b, v) in in_bundles.iter().zip(ins_v.iter()) {
        state.prepare(b, v);
    }
    let mut regions = Vec::new();
    for (ri, (a, w, _role)) in sig.mem.iter().enumerate() {
        let mut cells = Vec::new();
        for (ci, _) in (0..(1usize << a)).enumerate() {
            let wires = state.alloc_wires(*w);
            if let Some(v) = mems.get(ri).and_then(|m| m.get(ci))
                && !v.is_zero()
            {
                state.prepare(&wires, v);
            }
            cells.push(wires);
        }
        regions.push(RegionWires {
            cells,
            a: *a,
            w: *w,
        });
    }
    execute_action_inner(
        obj,
        &mut state,
        name,
        &mut_bundles,
        &in_bundles,
        &regions,
        args,
        false,
        &[],
    )?;
    if state.amps.len() != 1 {
        return Err(diag(
            "5.3",
            format!("permutation action @{name} produced a superposition under basis execution"),
        ));
    }
    let key = state.amps.keys().next().cloned().unwrap_or_default();
    let out_muts: Vec<BigUint> = mut_bundles
        .iter()
        .map(|b| QuantumState::extract(&key, b))
        .collect();
    let out_mem: Vec<Vec<BigUint>> = regions
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| QuantumState::extract(&key, c))
                .collect()
        })
        .collect();
    Ok((out_muts, out_mem))
}
