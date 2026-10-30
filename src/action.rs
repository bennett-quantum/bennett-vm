//! Quantum state simulation (§14.2) for Q actions reached from region
//! calls, plus the structured action executor (§5.3).
//!
//! The state is a sparse superposition over stable wire ids (bit `i`
//! of a basis key is wire `i`; freed wires keep their bit at zero).
//! Exact amplitudes determine support, cleanliness and equality. The binary64
//! cache is used only for Born probabilities and CLI display; permutations are
//! executed by branch enumeration without discarding relative or global phase.

use crate::bigint::BigUint;
use crate::exact::Exact;
use crate::object::{Function, Instr, Object};
use crate::value::{Budget, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

pub type Amp = (f64, f64);
type OutcomeCache = BTreeMap<(Vec<BigUint>, Vec<Vec<BigUint>>), (BigUint, BigUint)>;
type NormalizedCall<'a> = (bool, Option<&'a str>, &'a str, Vec<String>);

/// A resolved logical-gate request, in the known structured expansion order.
/// This diagnostic Rust API does not claim §11 library ordinals or a schedule.
#[derive(Clone, Debug)]
pub struct GateRequest {
    pub epsilon_share: crate::target::Rational,
    pub realisation: Option<String>,
}

#[derive(Clone, Debug)]
pub struct QuantumState {
    pub amps: BTreeMap<BigUint, Amp>,
    exact: BTreeMap<BigUint, Exact>,
    next_wire: usize,
    pub(crate) target: Option<std::sync::Arc<crate::target::Target>>,
    pub(crate) wire_classes: BTreeMap<usize, String>,
    pub emitted: Vec<crate::target::Leaf>,
    events: Vec<crate::target::Event>,
    live: Vec<crate::target::LiveInterval>,
    time: crate::target::Rational,
    peak: BTreeMap<String, usize>,
    dirty: BTreeSet<usize>,
    requests: Option<VecDeque<GateRequest>>,
}

impl QuantumState {
    pub fn new() -> QuantumState {
        let mut amps = BTreeMap::new();
        amps.insert(BigUint::zero(), (1.0, 0.0));
        QuantumState {
            amps,
            exact: BTreeMap::from([(BigUint::zero(), Exact::one())]),
            next_wire: 0,
            target: None,
            wire_classes: BTreeMap::new(),
            emitted: vec![],
            events: vec![],
            live: vec![],
            time: crate::target::Rational::zero(),
            peak: BTreeMap::new(),
            dirty: BTreeSet::new(),
            requests: None,
        }
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
    fn refresh(&mut self) {
        self.exact.retain(|_, a| !a.is_zero());
        self.amps = self
            .exact
            .iter()
            .map(|(k, a)| (k.clone(), a.value()))
            .collect();
    }
    pub fn set_state(&mut self, state: BTreeMap<BigUint, Amp>) {
        self.exact = state
            .into_iter()
            .map(|(k, (r, i))| (k, Exact::pair(r, i)))
            .collect();
        self.refresh();
    }
    pub fn exact_equals(&self, other: &Self) -> bool {
        self.exact == other.exact
    }
    /// Set basis bits on wires that start at zero in every component.
    pub fn prepare(&mut self, wires: &[usize], value: &BigUint) {
        self.exact = std::mem::take(&mut self.exact)
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
        self.refresh();
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
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let m = m.map(|r| {
            r.map(|(re, im)| {
                let component = |v: f64| {
                    if v.abs() == h {
                        if v < 0. {
                            Exact::sqrt_half().neg()
                        } else {
                            Exact::sqrt_half()
                        }
                    } else {
                        Exact::pair(v, 0.)
                    }
                };
                component(re).add(&component(im).mul(&Exact::pair(0., 1.)))
            })
        });
        self.apply_exact(target, controls, m);
    }
    pub fn apply_exact(&mut self, target: usize, controls: &[usize], m: [[Exact; 2]; 2]) {
        let mut next = BTreeMap::<BigUint, Exact>::new();
        for (k, a) in std::mem::take(&mut self.exact) {
            if !controls.iter().all(|c| k.bit(*c)) {
                next.insert(k, a);
                continue;
            }
            let bit = usize::from(k.bit(target));
            for (out, row) in m.iter().enumerate() {
                let mut nk = k.clone();
                nk.set_bit(target, out == 1);
                let v = a.mul(&row[bit]);
                let prev = next.remove(&nk).unwrap_or_else(Exact::zero);
                next.insert(nk, prev.add(&v));
            }
        }
        self.exact = next;
        self.refresh();
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
        for (k, a) in &mut self.exact {
            if wires.iter().all(|w| k.bit(*w)) {
                *a = a.mul(&Exact::pair(phase.0, phase.1));
            }
        }
        self.refresh();
    }
    pub fn swap(&mut self, a: usize, b: usize) {
        self.map_basis(|k| {
            let mut nk = k.clone();
            nk.set_bit(a, k.bit(b));
            nk.set_bit(b, k.bit(a));
            nk
        });
    }
    pub fn map_basis(&mut self, mut f: impl FnMut(&BigUint) -> BigUint) {
        let mut next = BTreeMap::<BigUint, Exact>::new();
        for (k, a) in std::mem::take(&mut self.exact) {
            let nk = f(&k);
            let prev = next.remove(&nk).unwrap_or_else(Exact::zero);
            next.insert(nk, prev.add(&a));
        }
        self.exact = next;
        self.refresh();
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
        self.exact = self
            .amps
            .iter()
            .map(|(k, (r, i))| (k.clone(), Exact::pair(*r, *i)))
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
    local_images: BTreeMap<String, Vec<BigUint>>,
    args: HashMap<String, Value>,
    pred: Vec<usize>,
    borrowed: Vec<String>,
    idle: Vec<usize>,
    func: String,
    exact_required: bool,
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
        if self.state.target.is_some() {
            let op = &ins.op;
            let (names, theta) = if matches!(op.as_str(), "rx" | "ry" | "rz") {
                (
                    vec![ins.operands[1].clone()],
                    Some(
                        self.angle(&ins.operands[0])?
                            .to_string()
                            .parse::<crate::target::Rational>()?,
                    ),
                )
            } else if op == "mcx" {
                let mut names = crate::validate::list_operand(&ins.operands[0])?;
                names.push(ins.operands[1].clone());
                (names, None)
            } else {
                (ins.operands.clone(), None)
            };
            let ids = names
                .iter()
                .map(|n| self.wire(n))
                .collect::<Result<Vec<_>, _>>()?;
            let own = crate::target::own_controls(op, ids.len())?;
            let mut controls = ids[..own].to_vec();
            controls.extend(&self.pred);
            let targets = ids[own..].to_vec();
            let classes = controls
                .iter()
                .chain(&targets)
                .map(|w| {
                    self.state
                        .wire_classes
                        .get(w)
                        .cloned()
                        .ok_or("§8.3: target gate wire has no allocated class".into())
                })
                .collect::<Result<Vec<_>, String>>()?;
            return self.emit_gate_request(crate::target::Gate {
                op: op.clone(),
                controls,
                own_controls: own,
                targets,
                classes,
                theta,
            });
        }
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
                self.state
                    .apply_exact(t, &pred, Exact::rotation(&ins.op, th));
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

    fn emit_gate_request(&mut self, g: crate::target::Gate) -> Result<(), String> {
        let request = if let Some(requests) = &mut self.state.requests {
            requests
                .pop_front()
                .ok_or("§8: missing logical gate request")?
        } else {
            GateRequest {
                epsilon_share: crate::target::Rational::zero(),
                realisation: None,
            }
        };
        if self.exact_required && !request.epsilon_share.is_zero() {
            return Err("§4.4/§5.3: permutation actions require epsilon zero transitively".into());
        }
        self.state.emit_target_gate_with(
            g,
            &request.epsilon_share,
            request.realisation.as_deref(),
            &self.idle,
        )
    }

    fn implicit_gate(
        &mut self,
        op: &str,
        own: &[usize],
        targets: &[usize],
        pred: &[usize],
    ) -> Result<(), String> {
        if self.state.target.is_none() {
            match op {
                "x" | "cx" | "ccx" => {
                    let mut controls = own.to_vec();
                    controls.extend(pred);
                    self.state.flip(targets[0], &controls);
                }
                "swap" => {
                    self.state.map_basis(|k| {
                        let mut n = k.clone();
                        if pred.iter().all(|w| k.bit(*w)) {
                            n.set_bit(targets[0], k.bit(targets[1]));
                            n.set_bit(targets[1], k.bit(targets[0]));
                        }
                        n
                    });
                }
                _ => return Err(format!("unimplemented: implicit gate {op} (milestone 4)")),
            }
            return Ok(());
        }
        let mut controls = own.to_vec();
        controls.extend(pred);
        let classes = controls
            .iter()
            .chain(targets)
            .map(|w| {
                self.state
                    .wire_classes
                    .get(w)
                    .cloned()
                    .ok_or_else(|| "§8.3: implicit gate wire has no class".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.emit_gate_request(crate::target::Gate {
            op: op.into(),
            controls,
            own_controls: own.len(),
            targets: targets.to_vec(),
            classes,
            theta: None,
        })
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
            self.implicit_gate("cx", &[*s], &[*d], &pred)?;
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
        if let Some(t) = self.state.target.clone() {
            t.validate_predicate_depth(self.pred.len())?;
            let qram = crate::target::field(&t.document, "qram")?;
            if qram == &crate::json::Json::Null {
                return Err("unimplemented: quantum-memory comparator expansion (milestone 4); unpublished ref library (bennett-vm-47t)".into());
            }
            let vars = BTreeMap::from([
                ("a".into(), crate::target::Rational::from(region.a as u64)),
                ("w".into(), crate::target::Rational::from(region.w as u64)),
                (
                    "c".into(),
                    crate::target::Rational::from(self.pred.len() as u64),
                ),
            ]);
            let costs = crate::target::field(crate::target::field(qram, mode)?, "cost")?
                .as_obj()
                .unwrap();
            let mut cost = BTreeMap::new();
            for (name, expr) in costs {
                let value = crate::target::eval(expr, &vars, &t.tables)?;
                if value.negative() {
                    return Err("§8.1: negative qram cost".into());
                }
                cost.insert(name.clone(), value);
            }
            let wires = region
                .cells
                .iter()
                .flatten()
                .chain(addr)
                .chain(value)
                .chain(&self.pred)
                .copied()
                .collect();
            self.state
                .record_event(cost["time"].clone(), wires, cost, &[]);
        }
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
        for _ in 0..ncells {
            cells.push(self.state.target_alloc(w, "any")?);
        }
        for (cell, wires) in image.iter().zip(&cells) {
            if !cell.is_zero() {
                // Image gates are unconditional (§3.5).
                for (i, w) in wires.iter().enumerate() {
                    if cell.bit(i) {
                        self.implicit_gate("x", &[], &[*w], &[])?;
                    }
                }
            }
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
            for (i, cell) in region.cells.iter().enumerate().rev() {
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
                for (i, w) in cell.iter().enumerate().rev() {
                    if want.bit(i) {
                        self.implicit_gate("x", &[], &[*w], &[])?;
                    }
                }
            }
            for cell in &region.cells {
                self.state.target_release(cell);
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
                    let requested = &ins.operands[0];
                    let selected = self
                        .idle
                        .iter()
                        .copied()
                        .filter(|w| {
                            self.state.target.is_none()
                                || requested == "any"
                                || self.state.wire_classes.get(w) == Some(requested)
                        })
                        .take(n)
                        .collect::<Vec<_>>();
                    if selected.len() != n {
                        return Err(
                            "§8.3: designated idle set lacks requested borrowed class".into()
                        );
                    }
                    self.idle.retain(|w| !selected.contains(w));
                    self.state.dirty.extend(&selected);
                    selected
                } else {
                    self.state.target_alloc(n, &ins.operands[0])?
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
                if op == "qfree" {
                    self.state.target_release(&wires);
                }
                if op == "qreturn" {
                    for w in &wires {
                        self.state.dirty.remove(w);
                    }
                    self.idle.extend(wires);
                }
                self.bundles.remove(&name);
                Ok(())
            }
            "qcopy" => {
                let src = self.bundle(&base_name(&ins.operands[0]))?;
                let dst = self.state.target_alloc(src.len(), "any")?;
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
                self.state.target_release(&dst);
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
                for (x, y) in a.iter().zip(&b) {
                    self.implicit_gate("swap", &[], &[*x, *y], &pred)?;
                }
                Ok(())
            }
            "qregion" => Ok(()),
            "qregion_release" => Ok(()),
            "qload" => {
                let region = self.region(&ins.operands[0])?;
                let addr = self.bundle(&base_name(&ins.operands[1]))?;
                let dst = self.state.target_alloc(region.w, "any")?;
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
                self.state.target_release(&dst);
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
                self.implicit_gate("x", &[], &[w], &pred)?;
                self.pred.push(w);
                self.exec_list(&ins.nested[1].clone())?;
                self.pred.pop();
                let pred = self.pred.clone();
                self.implicit_gate("x", &[], &[w], &pred)?;
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
        let flags = self.state.target_alloc(n, "any")?;
        let pred = self.pred.clone();
        if n > 0 {
            self.implicit_gate("cx", &[cond[0]], &[flags[0]], &pred)?;
        }
        if adjoint {
            // Reverse body order needs the same forward flags first.
            for i in 1..n {
                self.implicit_gate("ccx", &[flags[i - 1], cond[i]], &[flags[i]], &pred)?;
            }
            for i in (0..n).rev() {
                self.pred.push(flags[i]);
                let result = self.exec_list(&body);
                self.pred.pop();
                result?;
            }
        } else {
            for i in 0..n {
                self.pred.push(flags[i]);
                let result = self.exec_list(&body);
                self.pred.pop();
                result?;
                if i + 1 < n {
                    self.implicit_gate("ccx", &[flags[i], cond[i + 1]], &[flags[i + 1]], &pred)?;
                }
            }
        }
        for i in (1..n).rev() {
            self.implicit_gate("ccx", &[flags[i - 1], cond[i]], &[flags[i]], &pred)?;
        }
        if n > 0 {
            self.implicit_gate("cx", &[cond[0]], &[flags[0]], &pred)?;
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
        self.state.target_release(&flags);
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
                    Some(Value::Token(b) | Value::TokenId(_, b)) => out.push(Value::Token(*b)),
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
            Some(Value::Token(b) | Value::TokenId(_, b)) => Ok(*b),
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
        crate::certificate::verify_inverse_pair(self.obj, &g, &ginv, &budget)?;
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

/// Execute a complete explicit request batch transactionally. Missing/unused
/// requests fail; they never silently fall back. History reversal still applies
/// actual inverse gates and never uses this transaction's saved state.
#[allow(clippy::too_many_arguments)]
pub fn execute_action_with_requests(
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
    requests: &[GateRequest],
) -> Result<(), String> {
    if state.target.is_none() || state.requests.is_some() {
        return Err("§8: explicit requests require a target and cannot nest".into());
    }
    let mut trial = state.clone();
    trial.requests = Some(requests.iter().cloned().collect());
    execute_action_with_idle(
        obj,
        &mut trial,
        name,
        muts,
        ins_b,
        mems,
        args,
        adjoint,
        inherited_pred,
        idle,
    )?;
    if !trial.requests.take().unwrap().is_empty() {
        return Err("§8: unused logical gate requests".into());
    }
    *state = trial;
    Ok(())
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
    let certificate_budget = args
        .iter()
        .find_map(|v| {
            if let Value::Token(b) = v {
                Some(*b)
            } else {
                None
            }
        })
        .unwrap_or_default();
    crate::certificate::verify_embedded(
        obj,
        name,
        args,
        &certificate_budget,
        state.target.as_deref(),
    )?;
    // §9.4: an idle designation excludes the entire active interface and
    // effective control context, even when no instruction happens to borrow.
    let active = muts
        .iter()
        .chain(ins_b)
        .flatten()
        .copied()
        .chain(mems.iter().flat_map(|r| r.cells.iter().flatten().copied()))
        .chain(inherited_pred.iter().copied())
        .collect::<BTreeSet<_>>();
    let unique = idle.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != idle.len()
        || idle.iter().any(|w| {
            *w >= state.next_wire
                || active.contains(w)
                || (state.target.is_some() && !state.wire_classes.contains_key(w))
        })
    {
        return Err(
            "§9.4: invalid designated idle set (duplicates, nonlive or active interface)".into(),
        );
    }
    if let Some(t) = &state.target {
        t.validate_function(f)?;
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
        local_images: BTreeMap::new(),
        args: arg_env,
        pred: inherited_pred.to_vec(),
        borrowed: Vec::new(),
        idle: idle.to_vec(),
        func: name.to_string(),
        exact_required: f.kind == crate::object::FnKind::PermAction,
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
// Width plus declared class: inverse allocation must retain both (§5.3).
fn bundle_widths(_obj: &Object, f: &Function) -> HashMap<String, (usize, String)> {
    let mut widths = HashMap::new();
    for list in [&f.lists[0], &f.lists[1]] {
        for p in list.iter() {
            if let crate::object::Type::Wire(n) = p.ty {
                widths.insert(p.name.clone(), (n, "any".into()));
            }
        }
    }
    fn walk(instrs: &[Instr], widths: &mut HashMap<String, (usize, String)>) {
        for ins in instrs {
            match ins.op.as_str() {
                "qalloc" | "qborrow" => {
                    if let (Some(lhs), Some(n)) = (
                        ins.lhs.first(),
                        ins.operands.get(1).and_then(|s| s.parse::<usize>().ok()),
                    ) {
                        widths.insert(lhs.clone(), (n, ins.operands[0].clone()));
                    }
                }
                "qcopy" => {
                    if let (Some(lhs), Some(src)) = (ins.lhs.first(), ins.operands.first())
                        && let Some(w) = widths.get(&base_name(src)).cloned()
                    {
                        widths.insert(lhs.clone(), (w.0, "any".into()));
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
fn adjoint_list(instrs: &[Instr], widths: &HashMap<String, (usize, String)>) -> Vec<Instr> {
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

fn adjoint_instr(ins: &Instr, widths: &HashMap<String, (usize, String)>) -> Instr {
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
            let (n, class) = widths
                .get(&name)
                .cloned()
                .expect("validated allocation has width/class");
            renamed(
                ins,
                "qalloc",
                vec![name],
                vec![class, n.to_string()],
                vec![],
            )
        }
        "qborrow" => renamed(ins, "qreturn", vec![], ins.lhs.clone(), vec![]),
        "qreturn" => {
            let name = base_name(&ins.operands[0]);
            let (n, class) = widths
                .get(&name)
                .cloned()
                .expect("validated allocation has width/class");
            renamed(
                ins,
                "qborrow",
                vec![name],
                vec![class, n.to_string()],
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

impl QuantumState {
    pub fn target_alloc(&mut self, n: usize, requested: &str) -> Result<Vec<usize>, String> {
        let class = if let Some(t) = &self.target {
            let mut used = BTreeMap::new();
            for c in self.wire_classes.values() {
                *used.entry(c.clone()).or_insert(0) += 1;
            }
            Some(t.class_for(requested, n, &used)?)
        } else {
            if requested != "any" {
                return Err("§8.3: concrete allocation class requires a target".into());
            }
            None
        };
        let wires = self.alloc_wires(n);
        if let Some(c) = class {
            for w in &wires {
                self.wire_classes.insert(*w, c.clone());
            }
        }
        let mut used = BTreeMap::<String, usize>::new();
        for c in self.wire_classes.values() {
            *used.entry(c.clone()).or_default() += 1;
        }
        for (c, n) in used {
            let peak = self.peak.entry(c).or_default();
            *peak = (*peak).max(n);
        }
        Ok(wires)
    }
    pub(crate) fn target_release(&mut self, wires: &[usize]) {
        for w in wires {
            self.wire_classes.remove(w);
        }
    }
    /// Install a target before allocating any live simulator wires.
    pub fn set_target(&mut self, target: crate::target::Target) -> Result<(), String> {
        if self.next_wire != 0 || self.target.is_some() {
            return Err("§8: target must be installed before wire allocation".into());
        }
        self.target = Some(std::sync::Arc::new(target));
        Ok(())
    }
    /// Actual execution accounting, separate from the milestone-5 estimate ABI.
    pub fn target_accounting(&self) -> Result<crate::json::Json, String> {
        let mut result = self
            .target
            .as_ref()
            .ok_or("§8: no target installed")?
            .account(&self.events, &self.live)?;
        // Allocations can have zero duration. Capacity still observes them.
        if let crate::json::Json::Obj(m) = &mut result {
            for (name, resource) in self
                .target
                .as_ref()
                .unwrap()
                .document
                .get("resources")
                .unwrap()
                .as_obj()
                .unwrap()
            {
                if resource.get("kind").and_then(crate::json::Json::as_str) == Some("peak") {
                    let peak = m.get_mut(name).unwrap();
                    if let crate::json::Json::Obj(values) = peak {
                        for (c, n) in &self.peak {
                            let previous = values
                                .get(c)
                                .and_then(crate::json::Json::as_str)
                                .unwrap()
                                .parse::<usize>()
                                .unwrap();
                            values.insert(
                                c.clone(),
                                crate::json::Json::Str((*n).max(previous).to_string()),
                            );
                        }
                    }
                }
            }
        }
        Ok(result)
    }
    pub fn target_error(&self) -> crate::target::Rational {
        self.emitted
            .iter()
            .fold(crate::target::Rational::zero(), |a, l| a.add(&l.epsilon))
    }
    fn record_event(
        &mut self,
        duration: crate::target::Rational,
        wires: Vec<usize>,
        cost: BTreeMap<String, crate::target::Rational>,
        dirty: &[usize],
    ) {
        let end = self.time.add(&duration);
        // Slice each live wire at event boundaries. Borrowing replaces its
        // owner's rate in this slice; it never allocates a second physical wire.
        for (w, class) in &self.wire_classes {
            self.live.push(crate::target::LiveInterval {
                wire: *w,
                class: class.clone(),
                start: self.time.clone(),
                end: end.clone(),
                dirty: dirty.contains(w) || self.dirty.contains(w),
            });
        }
        self.events.push(crate::target::Event {
            start: self.time.clone(),
            duration,
            wires,
            cost,
        });
        self.time = end;
    }
    pub(crate) fn classical_event(&mut self) -> Result<(), String> {
        if let Some(t) = &self.target {
            let duration = crate::target::number(crate::target::field(
                crate::target::field(&t.document, "capabilities")?,
                "classical_step_time",
            )?)?;
            self.record_event(duration, vec![], BTreeMap::new(), &[]);
        }
        Ok(())
    }
    pub fn emit_target_gate(&mut self, g: crate::target::Gate) -> Result<(), String> {
        self.emit_target_gate_with(g, &crate::target::Rational::zero(), None, &[])
    }
    /// Execute a resolved gate request. Only the explicit caller idle pool may
    /// supply dirty scheme scratch (§9.4). The choice/share are not a schedule.
    pub fn emit_target_gate_with(
        &mut self,
        g: crate::target::Gate,
        eps: &crate::target::Rational,
        choice: Option<&str>,
        idle: &[usize],
    ) -> Result<(), String> {
        let target = self.target.clone().ok_or("§8: no target installed")?;
        for (w, c) in g.controls.iter().chain(&g.targets).zip(&g.classes) {
            if self.wire_classes.get(w) != Some(c) {
                return Err("§8.3: gate class differs from allocated wire class".into());
            }
        }
        let lowered = target.lower(&g, eps, choice)?;
        let operands = g
            .controls
            .iter()
            .chain(&g.targets)
            .copied()
            .collect::<BTreeSet<_>>();
        let base = operands.iter().max().copied().unwrap_or(0) + 1;
        let anc = if lowered.dirty && lowered.ancillae > 0 {
            let unique = idle.iter().copied().collect::<BTreeSet<_>>();
            if unique.len() != idle.len()
                || idle
                    .iter()
                    .any(|w| !self.wire_classes.contains_key(w) || operands.contains(w))
            {
                return Err(
                    "§9.4: invalid designated idle set (duplicates, nonlive or active operand)"
                        .into(),
                );
            }
            let anc = idle
                .iter()
                .copied()
                .filter(|w| self.wire_classes.get(w) == lowered.ancilla_class.as_ref())
                .take(lowered.ancillae)
                .collect::<Vec<_>>();
            if anc.len() != lowered.ancillae {
                return Err(
                    "§9.4: designated idle set lacks target-scheme wires of requested class".into(),
                );
            }
            anc
        } else if lowered.ancillae > 0 {
            self.target_alloc(lowered.ancillae, lowered.ancilla_class.as_deref().unwrap())?
        } else {
            vec![]
        };
        // Validate the fully remapped leaves before the first quantum change.
        let mut leaves = lowered.leaves;
        for leaf in &mut leaves {
            for w in leaf.gate.controls.iter_mut().chain(&mut leaf.gate.targets) {
                if *w >= base {
                    *w = anc[*w - base];
                }
            }
        }
        for leaf in leaves {
            self.apply_leaf(&leaf.gate)?;
            let wires = leaf
                .gate
                .controls
                .iter()
                .chain(&leaf.gate.targets)
                .copied()
                .collect();
            self.record_event(
                leaf.cost["time"].clone(),
                wires,
                leaf.cost.clone(),
                if lowered.dirty { &anc } else { &[] },
            );
            self.emitted.push(leaf);
        }
        if !lowered.dirty {
            if !self.is_clean(&anc) {
                return Err("§8.3: target scheme left clean ancillae nonzero".into());
            }
            self.target_release(&anc);
        }
        Ok(())
    }
    fn apply_leaf(&mut self, g: &crate::target::Gate) -> Result<(), String> {
        let t = g.targets[0];
        let c = &g.controls;
        match g.op.as_str() {
            "x" | "cx" | "ccx" | "mcx" => self.flip(t, c),
            "swap" => {
                let b = g.targets[1];
                self.map_basis(|k| {
                    let mut n = k.clone();
                    if c.iter().all(|w| k.bit(*w)) {
                        n.set_bit(t, k.bit(b));
                        n.set_bit(b, k.bit(t));
                    }
                    n
                });
            }
            "cz" => {
                let mut c = c.clone();
                c.extend(&g.targets);
                self.phase_if_all(&c, (-1.0, 0.0));
            }
            "h" => {
                let s = std::f64::consts::FRAC_1_SQRT_2;
                self.apply1(t, c, [[(s, 0.0), (s, 0.0)], [(s, 0.0), (-s, 0.0)]]);
            }
            "s" | "sdg" | "t" | "tdg" => {
                let s = std::f64::consts::FRAC_1_SQRT_2;
                let p = match g.op.as_str() {
                    "s" => (0.0, 1.0),
                    "sdg" => (0.0, -1.0),
                    "t" => (s, s),
                    _ => (s, -s),
                };
                self.apply1(t, c, [[(1.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), p]]);
            }
            "rx" | "ry" | "rz" => {
                let theta = g.theta.as_ref().ok_or("§8.3: missing theta")?.f64()?;
                self.apply_exact(t, c, Exact::rotation(&g.op, theta));
            }
            _ => return Err(format!("§17.2: unsupported native gate {}", g.op)),
        }
        Ok(())
    }
}
