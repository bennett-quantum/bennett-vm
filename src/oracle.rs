//! §14.1 reference interpreter: locations and cells are bit-vectors,
//! pseudo-locations read-only, each §4.2 row is its classical function,
//! counters unbounded. This defines `F_B` and the budget unit.

use crate::bigint::{BigUint, sign_extend};
use crate::cliargs;
use crate::json::Json;
use crate::object::{FnKind, Function, Instr, Object, Type};
use crate::sha256::from_hex;
use crate::value::{
    Budget, FAULT_BUDGET_DEPTH, FAULT_BUDGET_HEAP, FAULT_BUDGET_STEPS, FAULT_UB_MEMFAULT,
    FAULT_UB_TRAP, Value, binop, div_fault, divop, icmp,
};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug)]
pub struct Outcome {
    pub payload: BigUint,
    pub code: BigUint,
    pub steps: u64,
}

#[derive(Clone)]
struct AllocEntry {
    base: BigUint,
    size: BigUint,
    live: bool,
    pinned: bool,
}

struct RegionMap {
    base: BigUint,
    /// Byte length of the mapped range.
    len: BigUint,
    region_index: usize,
    w: usize,
}

struct Frame {
    caller: String,
    block: usize,
    instr: usize,
    dest: String,
    saved: Vec<(String, BigUint)>,
    callee: String,
}

enum Flow {
    Next,
    Jump(usize, usize),
    Call(String),
    HaltOk,
    HaltFault(BigUint),
    Return(BigUint),
}

pub struct Machine<'o> {
    obj: &'o Object,
    oracle_name: String,
    locations: HashMap<String, HashMap<String, BigUint>>,
    widths: HashMap<String, HashMap<String, usize>>,
    pseudo: HashMap<String, BigUint>,
    pseudo_widths: HashMap<String, usize>,
    args: HashMap<String, Value>,
    mem: HashMap<BigUint, u8>,
    regions: Vec<Vec<BigUint>>,
    region_maps: Vec<RegionMap>,
    alloctab: Vec<AllocEntry>,
    heap_used: BigUint,
    sp: BigUint,
    depth: u64,
    result_val: BigUint,
    tgt_width: usize,
    fault_width: usize,
    budget: Budget,
    steps: u64,
    call_stack: Vec<Frame>,
    /// Function order of the §3.6 index space for this oracle.
    index_order: Vec<String>,
    index_offsets: HashMap<String, usize>,
}

fn diag(rule: &str, msg: impl std::fmt::Display) -> String {
    format!("§{rule}: {msg}")
}

pub fn parse_const(text: &str, width: usize) -> Result<BigUint, String> {
    if let Some(neg) = text.strip_prefix('-') {
        let mag = BigUint::parse_decimal(neg)
            .map_err(|_| diag("16.1", format!("invalid integer literal {text:?}")))?;
        if mag.is_zero() {
            return Ok(BigUint::zero());
        }
        return Ok(BigUint::from_u64(1)
            .shl(width)
            .sub(&mag.mask(width + 1))
            .mask(width));
    }
    let v = BigUint::parse_decimal(text)
        .map_err(|_| diag("16.1", format!("invalid integer literal {text:?}")))?;
    Ok(v.mask(width))
}

/// Locations of a lifted function in declaration order (§3.7):
/// lproc params, result name, regs; oracle regs only.
pub fn location_widths(f: &Function) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    if f.kind == FnKind::Lproc {
        for p in f.lists[0].iter().chain(f.lists[1].iter()) {
            if let Type::Reg(n) = p.ty {
                out.push((p.name.clone(), n));
            }
        }
    }
    for p in &f.regs {
        if let Type::Reg(n) = p.ty {
            out.push((p.name.clone(), n));
        }
    }
    out
}

/// Transitive lproc closure of an oracle through `lift.call`.
pub fn lproc_closure(obj: &Object, oracle: &Function) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut work: Vec<String> = Vec::new();
    let scan = |f: &Function, work: &mut Vec<String>, seen: &BTreeSet<String>| {
        let mut flat = Vec::new();
        f.flat_instrs(&mut flat);
        for ins in flat {
            if ins.op == "lift.call"
                && let Some(name) = ins
                    .operands
                    .first()
                    .and_then(|t| crate::validate::at_name(t))
                && !seen.contains(name)
            {
                work.push(name.to_string());
            }
        }
    };
    scan(oracle, &mut work, &seen);
    while let Some(name) = work.pop() {
        if seen.insert(name.clone())
            && let Some(f) = obj.function(&name)
        {
            scan(f, &mut work, &seen);
        }
    }
    seen.into_iter().collect()
}

/// §3.6 index order: oracle first, then lproc SCCs ordered by scc hash
/// with members in name (SCC index) order.
fn index_order(obj: &Object, oracle: &Function) -> Result<Vec<String>, String> {
    let closure = lproc_closure(obj, oracle);
    // SCCs among the closure, via identity's reference graph restricted
    // to lift.call edges (references_of also covers data refs, which do
    // not put an lproc into the index space).
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for name in &closure {
        let mut e = Vec::new();
        if let Some(f) = obj.function(name) {
            let mut flat = Vec::new();
            f.flat_instrs(&mut flat);
            for ins in flat {
                if ins.op == "lift.call"
                    && let Some(t) = ins
                        .operands
                        .first()
                        .and_then(|t| crate::validate::at_name(t))
                    && closure.contains(&t.to_string())
                {
                    e.push(t.to_string());
                }
            }
        }
        edges.insert(name.clone(), e);
    }
    // Kosaraju on this small graph.
    fn dfs(
        v: &str,
        edges: &HashMap<String, Vec<String>>,
        seen: &mut BTreeSet<String>,
        order: &mut Vec<String>,
    ) {
        if !seen.insert(v.to_string()) {
            return;
        }
        for w in edges.get(v).cloned().unwrap_or_default() {
            dfs(&w, edges, seen, order);
        }
        order.push(v.to_string());
    }
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for name in &closure {
        dfs(name, &edges, &mut seen, &mut order);
    }
    let mut redges: HashMap<String, Vec<String>> = HashMap::new();
    for (v, ws) in &edges {
        for w in ws {
            redges.entry(w.clone()).or_default().push(v.clone());
        }
    }
    let mut comps: Vec<Vec<String>> = Vec::new();
    let mut seen2 = BTreeSet::new();
    for name in order.iter().rev() {
        if seen2.contains(name) {
            continue;
        }
        let mut comp = Vec::new();
        let mut stack = vec![name.clone()];
        while let Some(v) = stack.pop() {
            if seen2.insert(v.clone()) {
                comp.push(v.clone());
                for w in redges.get(&v).cloned().unwrap_or_default() {
                    stack.push(w);
                }
            }
        }
        comp.sort();
        comps.push(comp);
    }
    let ids = crate::identity::compute(obj)?;
    comps.sort_by(|a, b| {
        let ka = ids.scc_hash_of(&a[0]);
        let kb = ids.scc_hash_of(&b[0]);
        ka.cmp(&kb)
    });
    let mut out = vec![oracle.name.clone()];
    for comp in comps {
        out.extend(comp);
    }
    Ok(out)
}

impl<'o> Machine<'o> {
    fn read(&self, func: &str, token: &str) -> Result<(BigUint, Option<usize>), String> {
        if crate::validate::is_int_literal(token) {
            if token.starts_with('-') {
                return Err(diag(
                    "3.3",
                    format!("negative literal {token:?} in lifted operand position has no width"),
                ));
            }
            let v = BigUint::parse_decimal(token)
                .map_err(|_| diag("16.1", format!("invalid literal {token:?}")))?;
            return Ok((v, None));
        }
        if let Some(wmap) = self.widths.get(func)
            && let Some(w) = wmap.get(token)
        {
            let v = self
                .locations
                .get(func)
                .and_then(|m| m.get(token))
                .cloned()
                .unwrap_or_default();
            return Ok((v, Some(*w)));
        }
        if func == self.oracle_name
            && let Some(v) = self.pseudo.get(token)
        {
            return Ok((v.clone(), self.pseudo_widths.get(token).copied()));
        }
        Err(diag(
            "3.7",
            format!("name {token:?} does not resolve to a location or pseudo-location of @{func}"),
        ))
    }

    fn write(&mut self, func: &str, name: &str, value: BigUint) -> Result<(), String> {
        let width = *self
            .widths
            .get(func)
            .and_then(|m| m.get(name))
            .ok_or_else(|| {
                diag(
                    "3.7",
                    format!("{name:?} is not a writable location of @{func}"),
                )
            })?;
        self.locations
            .entry(func.to_string())
            .or_default()
            .insert(name.to_string(), value.mask(width));
        Ok(())
    }

    fn width_of(&self, func: &str, name: &str) -> Result<usize, String> {
        if let Some(w) = self.widths.get(func).and_then(|m| m.get(name)) {
            return Ok(*w);
        }
        if func == self.oracle_name
            && let Some(w) = self.pseudo_widths.get(name)
        {
            return Ok(*w);
        }
        Err(diag(
            "3.7",
            format!("name {name:?} does not resolve in @{func}"),
        ))
    }

    fn inrange(&self, p: &BigUint, n: &BigUint) -> bool {
        let pn = p.add(n);
        self.alloctab.iter().any(|e| {
            e.live
                && e.base.cmp_big(p) != std::cmp::Ordering::Greater
                && pn.cmp_big(&e.base.add(&e.size)) != std::cmp::Ordering::Greater
        })
    }

    fn read_byte(&self, addr: &BigUint) -> u8 {
        for rm in &self.region_maps {
            if addr.cmp_big(&rm.base) != std::cmp::Ordering::Less
                && addr.cmp_big(&rm.base.add(&rm.len)) == std::cmp::Ordering::Less
            {
                let offset = addr.sub(&rm.base);
                let byte_per_cell = rm.w / 8;
                let cell = offset
                    .divmod(&BigUint::from_u64(byte_per_cell as u64))
                    .0
                    .to_usize()
                    .unwrap_or(usize::MAX);
                let within = offset
                    .divmod(&BigUint::from_u64(byte_per_cell as u64))
                    .1
                    .to_usize()
                    .unwrap_or(0);
                return self
                    .regions
                    .get(rm.region_index)
                    .and_then(|r| r.get(cell))
                    .map(|v| v.shr(within * 8).mask(8).to_u64().unwrap_or(0) as u8)
                    .unwrap_or(0);
            }
        }
        self.mem.get(addr).copied().unwrap_or(0)
    }

    fn write_byte(&mut self, addr: &BigUint, byte: u8) {
        for rm_i in 0..self.region_maps.len() {
            let rm = &self.region_maps[rm_i];
            if addr.cmp_big(&rm.base) != std::cmp::Ordering::Less
                && addr.cmp_big(&rm.base.add(&rm.len)) == std::cmp::Ordering::Less
            {
                let offset = addr.sub(&rm.base);
                let byte_per_cell = rm.w / 8;
                let (cell_b, within_b) = offset.divmod(&BigUint::from_u64(byte_per_cell as u64));
                let cell = cell_b.to_usize().unwrap_or(usize::MAX);
                let within = within_b.to_usize().unwrap_or(0);
                let ridx = rm.region_index;
                let w = rm.w;
                if let Some(region) = self.regions.get_mut(ridx)
                    && let Some(v) = region.get_mut(cell)
                {
                    let mask = BigUint::from_u64(0xff).shl(within * 8);
                    let inv = BigUint::from_u64(1)
                        .shl(w.max(1))
                        .sub(&BigUint::from_u64(1));
                    let cleared = v.bitand(&mask.bitxor(&inv));
                    *v = cleared
                        .bitor(&BigUint::from_u64(byte as u64).shl(within * 8))
                        .mask(w);
                }
                return;
            }
        }
        if byte == 0 {
            self.mem.remove(addr);
        } else {
            self.mem.insert(addr.clone(), byte);
        }
    }

    fn load_bytes(&self, addr: &BigUint, n: usize) -> BigUint {
        let mut v = BigUint::zero();
        for i in 0..n {
            let b = self.read_byte(&addr.add(&BigUint::from_u64(i as u64)));
            if b != 0 {
                v = v.bitor(&BigUint::from_u64(b as u64).shl(i * 8));
            }
        }
        v
    }
    fn store_bytes(&mut self, addr: &BigUint, value: &BigUint, n: usize) {
        for i in 0..n {
            let b = value.shr(i * 8).mask(8).to_u64().unwrap_or(0) as u8;
            self.write_byte(&addr.add(&BigUint::from_u64(i as u64)), b);
        }
    }

    fn block_index(&self, func: &Function, name: &str) -> Result<usize, String> {
        func.blocks
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| diag("3.6", format!("unknown block ^{name} in @{}", func.name)))
    }

    fn canonical_block_index(&self, func_name: &str, block_name: &str) -> Result<usize, String> {
        let start = *self
            .index_offsets
            .get(func_name)
            .ok_or_else(|| diag("3.6", format!("@{func_name} is not in the index space")))?;
        let f = self
            .obj
            .function(func_name)
            .ok_or_else(|| diag("3.6", format!("unknown function @{func_name}")))?;
        let mut offset = start;
        for b in &f.blocks {
            if b.name == block_name {
                return Ok(offset);
            }
            offset += b.instrs.len();
        }
        Err(diag(
            "3.6",
            format!("unknown block ^{block_name} in @{func_name}"),
        ))
    }

    fn func_index_range(&self, func_name: &str) -> (usize, usize) {
        let start = *self.index_offsets.get(func_name).unwrap_or(&0);
        let n = self
            .obj
            .function(func_name)
            .map(|f| f.instr_count())
            .unwrap_or(0);
        (start, start + n)
    }

    #[allow(clippy::too_many_lines)]
    fn exec_row(&mut self, func: &Function, ins: &Instr) -> Result<Flow, String> {
        let fname = &func.name;
        let op = ins.op.as_str();
        let base = op.strip_prefix("lift.").unwrap_or(op);
        // const
        if let Some(rest) = base.strip_prefix("const.") {
            let width: usize = rest
                .strip_prefix('i')
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| diag("4.8", format!("malformed {op}")))?;
            let dest = &ins.lhs[0];
            let dw = self.width_of(fname, dest)?;
            if dw != width {
                return Err(diag(
                    "3.3",
                    format!("{op} destination {dest} has width {dw}, not {width}"),
                ));
            }
            let v = parse_const(&ins.operands[0], width)?;
            self.write(fname, dest, v)?;
            return Ok(Flow::Next);
        }
        if [
            "add", "sub", "mul", "and", "or", "xor", "shl", "lshr", "ashr",
        ]
        .contains(&base)
        {
            let (a, aw) = self.read(fname, &ins.operands[0])?;
            let (b, bw) = self.read(fname, &ins.operands[1])?;
            let width = both_widths(aw, bw, fname, op)?;
            let dest = &ins.lhs[0];
            if self.width_of(fname, dest)? != width {
                return Err(diag(
                    "3.3",
                    format!("{op} destination {dest} width disagrees with its operands"),
                ));
            }
            let v = binop(base, &a, &b, width);
            self.write(fname, dest, v)?;
            return Ok(Flow::Next);
        }
        if ["udiv", "urem", "sdiv", "srem"].contains(&base) {
            let (a, aw) = self.read(fname, &ins.operands[0])?;
            let (b, bw) = self.read(fname, &ins.operands[1])?;
            let width = both_widths(aw, bw, fname, op)?;
            if let Some(code) = div_fault(base, &a, &b, width) {
                return Ok(Flow::HaltFault(BigUint::from_u64(code)));
            }
            let dest = &ins.lhs[0];
            if self.width_of(fname, dest)? != width {
                return Err(diag(
                    "3.3",
                    format!("{op} destination {dest} width disagrees with its operands"),
                ));
            }
            let v = divop(base, &a, &b, width);
            self.write(fname, dest, v)?;
            return Ok(Flow::Next);
        }
        if let Some(pred) = base.strip_prefix("icmp.") {
            let (a, aw) = self.read(fname, &ins.operands[0])?;
            let (b, bw) = self.read(fname, &ins.operands[1])?;
            let width = both_widths(aw, bw, fname, op)?;
            let dest = &ins.lhs[0];
            if self.width_of(fname, dest)? != 1 {
                return Err(diag(
                    "4.2",
                    format!("{op} destination {dest} must be reg[1]"),
                ));
            }
            let v = BigUint::from_u64(icmp(pred, &a, &b, width) as u64);
            self.write(fname, dest, v)?;
            return Ok(Flow::Next);
        }
        match op {
            "lift.select" => {
                let (p, _) = self.read(fname, &ins.operands[0])?;
                let (a, aw) = self.read(fname, &ins.operands[1])?;
                let (b, bw) = self.read(fname, &ins.operands[2])?;
                let width = both_widths(aw, bw, fname, op)?;
                let dest = &ins.lhs[0];
                if self.width_of(fname, dest)? != width {
                    return Err(diag(
                        "3.3",
                        format!("lift.select destination {dest} width disagrees with its arms"),
                    ));
                }
                self.write(fname, dest, if p.bit(0) { a } else { b })?;
                Ok(Flow::Next)
            }
            "lift.mov" => {
                let (a, aw) = self.read(fname, &ins.operands[0])?;
                let dest = &ins.lhs[0];
                if self.width_of(fname, dest)? != aw.unwrap_or(0) {
                    return Err(diag(
                        "3.3",
                        format!("lift.mov destination {dest} width disagrees with its operand"),
                    ));
                }
                self.write(fname, dest, a)?;
                Ok(Flow::Next)
            }
            "lift.parmov" => {
                let srcs = crate::validate::list_operand(&ins.operands[0])?;
                let mut values = Vec::new();
                for (s, d) in srcs.iter().zip(ins.lhs.iter()) {
                    let (v, sw) = self.read(fname, s)?;
                    if Some(self.width_of(fname, d)?) != sw {
                        return Err(diag(
                            "3.3",
                            format!("lift.parmov destination {d} width disagrees with source {s}"),
                        ));
                    }
                    values.push(v);
                }
                for (d, v) in ins.lhs.clone().iter().zip(values) {
                    self.write(fname, d, v)?;
                }
                Ok(Flow::Next)
            }
            "lift.label" => {
                let (bname, _) = crate::validate::branch_target(&ins.operands[0])
                    .ok_or_else(|| diag("4.2", "malformed lift.label"))?;
                let idx = self.canonical_block_index(fname, &bname)?;
                let dest = &ins.lhs[0];
                self.write(fname, dest, BigUint::from_u64(idx as u64))?;
                Ok(Flow::Next)
            }
            "lift.addr" => {
                let name = crate::validate::at_name(&ins.operands[0]).unwrap_or("");
                let dest = &ins.lhs[0];
                if self.width_of(fname, dest)? != self.obj.ptr_width {
                    return Err(diag(
                        "3.3",
                        format!("lift.addr destination {dest} must be reg[ptr_width]"),
                    ));
                }
                let value =
                    if let Some(g) = self.obj.layout.as_ref().and_then(|l| l.globals.get(name)) {
                        g.base.clone()
                    } else if let Some(base) = self
                        .obj
                        .layout
                        .as_ref()
                        .and_then(|l| l.interface_regions.get(&self.oracle_name))
                        .and_then(|m| m.get(name))
                    {
                        base.clone()
                    } else {
                        return Err(diag(
                            "4.2",
                            format!("lift.addr @{name} names no global or mapped region"),
                        ));
                    };
                self.write(fname, dest, value)?;
                Ok(Flow::Next)
            }
            "lift.boundscheck" => {
                let (p, pw) = self.read(fname, &ins.operands[0])?;
                if pw != Some(self.obj.ptr_width) {
                    return Err(diag(
                        "3.3",
                        "lift.boundscheck address must have width ptr_width",
                    ));
                }
                let (n, _) = self.read(fname, &ins.operands[1])?;
                if self.inrange(&p, &n) {
                    Ok(Flow::Next)
                } else {
                    Ok(Flow::HaltFault(BigUint::from_u64(FAULT_UB_MEMFAULT)))
                }
            }
            "lift.alloc" => {
                let (n, _) = self.read(fname, &ins.operands[0])?;
                let layout = self
                    .obj
                    .layout
                    .as_ref()
                    .ok_or_else(|| diag("7.2", "lift.alloc in a memoryless object"))?;
                let need = if n.is_zero() {
                    BigUint::from_u64(1)
                } else {
                    n.clone()
                };
                let heap_limit = BigUint::from_u64(self.budget.heap);
                let atop_cap = BigUint::from_u64(1).shl(layout.a_tab);
                let over_heap =
                    self.heap_used.add(&need).cmp_big(&heap_limit) == std::cmp::Ordering::Greater;
                let over_tab = BigUint::from_u64(self.alloctab.len() as u64).cmp_big(&atop_cap)
                    != std::cmp::Ordering::Less;
                if over_heap || over_tab {
                    return Ok(Flow::HaltFault(BigUint::from_u64(FAULT_BUDGET_HEAP)));
                }
                let base = layout.heap_base.add(&self.heap_used);
                self.alloctab.push(AllocEntry {
                    base: base.clone(),
                    size: n,
                    live: true,
                    pinned: false,
                });
                self.heap_used = self.heap_used.add(&need);
                let dest = ins.lhs[0].clone();
                if self.width_of(fname, &dest)? != self.obj.ptr_width {
                    return Err(diag(
                        "3.3",
                        format!("lift.alloc destination {dest} must be reg[ptr_width]"),
                    ));
                }
                self.write(fname, &dest, base)?;
                Ok(Flow::Next)
            }
            "lift.free" => {
                let (p, _) = self.read(fname, &ins.operands[0])?;
                let entry = self
                    .alloctab
                    .iter_mut()
                    .find(|e| e.live && !e.pinned && e.base == p);
                match entry {
                    Some(e) => {
                        e.live = false;
                        Ok(Flow::Next)
                    }
                    None => Ok(Flow::HaltFault(BigUint::from_u64(FAULT_UB_MEMFAULT))),
                }
            }
            "lift.faultif" => {
                let (p, pw) = self.read(fname, &ins.operands[0])?;
                if pw != Some(1) {
                    return Err(diag(
                        "4.2",
                        format!("lift.faultif condition in @{fname} must be reg[1]"),
                    ));
                }
                let code = BigUint::parse_int(&ins.operands[1])
                    .map_err(|_| diag("4.2", "invalid lift.faultif code"))?;
                if !p.is_zero() {
                    Ok(Flow::HaltFault(code))
                } else {
                    Ok(Flow::Next)
                }
            }
            "fault" => {
                let code = BigUint::parse_int(&ins.operands[0])
                    .map_err(|_| diag("4.2", "invalid fault code"))?;
                Ok(Flow::HaltFault(code))
            }
            "lift.retval" => {
                let (v, vw) = self.read(fname, &ins.operands[0])?;
                if vw != Some(self.tgt_width) {
                    return Err(diag(
                        "5.2",
                        format!(
                            "lift.retval operand has width {vw:?}, the target width is {}",
                            self.tgt_width
                        ),
                    ));
                }
                self.result_val = self.result_val.bitxor(&v).mask(self.tgt_width);
                Ok(Flow::HaltOk)
            }
            "lift.ret" => {
                if self.call_stack.is_empty() {
                    return Ok(Flow::HaltFault(BigUint::from_u64(FAULT_UB_TRAP)));
                }
                let (v, _) = self.read(fname, &ins.operands[0])?;
                Ok(Flow::Return(v))
            }
            "qjump" => {
                let (bname, _) = crate::validate::branch_target(&ins.operands[0])
                    .ok_or_else(|| diag("4.2", "malformed qjump"))?;
                Ok(Flow::Jump(self.block_index(func, &bname)?, 0))
            }
            "qbranch" => {
                let (p, pw) = self.read(fname, &ins.operands[0])?;
                if pw != Some(1) {
                    return Err(diag(
                        "4.2",
                        format!("qbranch predicate in @{fname} must be reg[1]"),
                    ));
                }
                let target = if !p.is_zero() {
                    &ins.operands[1]
                } else {
                    &ins.operands[2]
                };
                let (bname, _) = crate::validate::branch_target(target)
                    .ok_or_else(|| diag("4.2", "malformed qbranch"))?;
                Ok(Flow::Jump(self.block_index(func, &bname)?, 0))
            }
            "qjumpr" => {
                let (a, _) = self.read(fname, &ins.operands[0])?;
                let (first, end) = self.func_index_range(fname);
                let idx = a.to_usize();
                match idx {
                    Some(i) if i >= first && i < end => {
                        let mut local = i - first;
                        for (bi, b) in func.blocks.iter().enumerate() {
                            if local < b.instrs.len() {
                                return Ok(Flow::Jump(bi, local));
                            }
                            local -= b.instrs.len();
                        }
                        Ok(Flow::HaltFault(BigUint::from_u64(FAULT_UB_TRAP)))
                    }
                    _ => Ok(Flow::HaltFault(BigUint::from_u64(FAULT_UB_TRAP))),
                }
            }
            "lift.call" => {
                let callee = crate::validate::at_name(&ins.operands[0])
                    .ok_or_else(|| diag("4.2", "malformed lift.call"))?
                    .to_string();
                Ok(Flow::Call(callee))
            }
            "lift.oracle_call" | "lift.oracle_uncall" => {
                self.exec_oracle_call(fname, ins)?;
                Ok(Flow::Next)
            }
            "lift.perm_call" => {
                self.exec_perm_call(fname, ins)?;
                Ok(Flow::Next)
            }
            _ if base.starts_with("zext.")
                || base.starts_with("sext.")
                || base.starts_with("trunc.") =>
            {
                let (to, kind) = if let Some(r) = base.strip_prefix("zext.") {
                    (r, "zext")
                } else if let Some(r) = base.strip_prefix("sext.") {
                    (r, "sext")
                } else {
                    (base.strip_prefix("trunc.").unwrap(), "trunc")
                };
                let m: usize = to
                    .strip_prefix('i')
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| diag("4.8", format!("malformed {op}")))?;
                let (a, aw) = self.read(fname, &ins.operands[0])?;
                let from = aw.ok_or_else(|| diag("3.3", format!("{op} operand needs a width")))?;
                let dest = &ins.lhs[0];
                if self.width_of(fname, dest)? != m {
                    return Err(diag(
                        "3.3",
                        format!("{op} destination {dest} must have width {m}"),
                    ));
                }
                let valid = match kind {
                    "trunc" => m < from,
                    _ => m > from,
                };
                if !valid {
                    return Err(diag(
                        "3.3",
                        format!(
                            "{op} from width {from} to {m} does not strictly {} in @{fname}",
                            if kind == "trunc" { "narrow" } else { "widen" }
                        ),
                    ));
                }
                let v = if kind == "sext" {
                    sign_extend(&a, from, m)
                } else {
                    a.mask(m)
                };
                self.write(fname, dest, v)?;
                Ok(Flow::Next)
            }
            _ if op.starts_with("lift.load.") => {
                let n_bits: usize = op
                    .rsplit('.')
                    .next()
                    .and_then(|t| t.strip_prefix('i'))
                    .and_then(|t| t.parse().ok())
                    .ok_or_else(|| diag("3.3", format!("malformed {op}")))?;
                let (a, aw) = self.read(fname, &ins.operands[0])?;
                if aw != Some(self.obj.ptr_width) {
                    return Err(diag(
                        "3.3",
                        format!("{op} address must have width ptr_width"),
                    ));
                }
                if !self.inrange(&a, &BigUint::from_u64((n_bits / 8) as u64)) {
                    return Ok(Flow::HaltFault(BigUint::from_u64(FAULT_UB_MEMFAULT)));
                }
                let v = self.load_bytes(&a, n_bits / 8);
                let dest = ins.lhs[0].clone();
                if self.width_of(fname, &dest)? != n_bits {
                    return Err(diag(
                        "3.3",
                        format!("{op} destination {dest} must have width {n_bits}"),
                    ));
                }
                self.write(fname, &dest, v)?;
                Ok(Flow::Next)
            }
            _ if op.starts_with("lift.store.") => {
                let n_bits: usize = op
                    .rsplit('.')
                    .next()
                    .and_then(|t| t.strip_prefix('i'))
                    .and_then(|t| t.parse().ok())
                    .ok_or_else(|| diag("3.3", format!("malformed {op}")))?;
                let (a, aw) = self.read(fname, &ins.operands[0])?;
                if aw != Some(self.obj.ptr_width) {
                    return Err(diag(
                        "3.3",
                        format!("{op} address must have width ptr_width"),
                    ));
                }
                let (v, vw) = self.read(fname, &ins.operands[1])?;
                if vw != Some(n_bits) {
                    return Err(diag("3.3", format!("{op} value must have width {n_bits}")));
                }
                if !self.inrange(&a, &BigUint::from_u64((n_bits / 8) as u64)) {
                    return Ok(Flow::HaltFault(BigUint::from_u64(FAULT_UB_MEMFAULT)));
                }
                self.store_bytes(&a, &v, n_bits / 8);
                Ok(Flow::Next)
            }
            _ => Err(format!("unimplemented: lifted row {op} (milestone 3)")),
        }
    }

    fn exec_oracle_call(&mut self, fname: &str, ins: &Instr) -> Result<(), String> {
        let callee = crate::validate::at_name(&ins.operands[0])
            .ok_or_else(|| diag("4.2", "malformed lift.oracle_call"))?
            .to_string();
        let tk = &ins.operands[1];
        let budget = match self.args.get(tk) {
            Some(Value::Token(b) | Value::TokenId(_, b)) => *b,
            _ => {
                return Err(diag(
                    "4.2",
                    format!(
                        "token operand {tk:?} of lift.oracle_call in @{fname} is not a classical token argument of the calling oracle"
                    ),
                ));
            }
        };
        let sig = crate::validate::oracle_sig(self.obj, &callee)?;
        let in_names = crate::validate::list_operand(&ins.operands[2])?;
        let mut inputs = Vec::new();
        for n in &in_names {
            inputs.push(self.read(fname, n)?.0);
        }
        let mem_names = crate::validate::list_operand(&ins.operands[5])?;
        let arg_names = crate::validate::list_operand(&ins.operands[6])?;
        let mut callee_args = Vec::new();
        for n in &arg_names {
            match self.args.get(n) {
                Some(v) => callee_args.push(v.clone()),
                None => {
                    // A literal integer classical argument.
                    let idx = callee_args.len();
                    match sig.args.get(idx) {
                        Some(Type::I(w)) => callee_args.push(Value::int(
                            *w,
                            BigUint::parse_int(n).map_err(|_| {
                                diag("4.2", format!("unresolved classical argument {n:?}"))
                            })?,
                        )),
                        _ => {
                            return Err(diag(
                                "4.2",
                                format!("unresolved classical argument {n:?} in @{fname}"),
                            ));
                        }
                    }
                }
            }
        }
        // Borrow the caller's region cells for the nested invocation.
        let oracle = self
            .obj
            .function(&self.oracle_name)
            .ok_or_else(|| diag("3.7", "oracle vanished"))?;
        let region_order: Vec<String> = oracle.lists[3].iter().map(|p| p.name.clone()).collect();
        let mut taken: Vec<Vec<BigUint>> = Vec::new();
        let mut taken_idx: Vec<usize> = Vec::new();
        for m in &mem_names {
            let idx = region_order.iter().position(|r| r == m).ok_or_else(|| {
                diag(
                    "4.2",
                    format!(
                        "mem operand {m:?} is not an interface region of @{}",
                        self.oracle_name
                    ),
                )
            })?;
            taken_idx.push(idx);
            taken.push(std::mem::take(&mut self.regions[idx]));
        }
        let outcome = run_oracle(self.obj, &callee, &inputs, &mut taken, &callee_args, budget);
        for (cells, idx) in taken.into_iter().zip(taken_idx) {
            self.regions[idx] = cells;
        }
        let outcome = outcome?;
        let tgt = &ins.operands[3];
        let (cur, _) = self.read(fname, tgt)?;
        let new = cur.bitxor(&outcome.payload);
        self.write(fname, tgt, new)?;
        // The callee fault code is masked to its fault width on delivery.
        let fr = &ins.operands[4];
        let (curf, _) = self.read(fname, fr)?;
        let newf = curf.bitxor(&outcome.code.mask(sig.fault));
        self.write(fname, fr, newf)?;
        Ok(())
    }

    fn exec_perm_call(&mut self, fname: &str, ins: &Instr) -> Result<(), String> {
        let callee = crate::validate::at_name(&ins.operands[0])
            .ok_or_else(|| diag("4.2", "malformed lift.perm_call"))?
            .to_string();
        let sig = crate::validate::action_sig(self.obj, &callee)?;
        let mut_names = crate::validate::list_operand(&ins.operands[1])?;
        let in_names = crate::validate::list_operand(&ins.operands[2])?;
        let mem_names = crate::validate::list_operand(&ins.operands[3])?;
        let arg_names = crate::validate::list_operand(&ins.operands[4])?;
        let mut muts = Vec::new();
        for n in &mut_names {
            muts.push(self.read(fname, n)?.0);
        }
        let mut ins_v = Vec::new();
        for n in &in_names {
            ins_v.push(self.read(fname, n)?.0);
        }
        let oracle = self
            .obj
            .function(&self.oracle_name)
            .ok_or_else(|| diag("3.7", "oracle vanished"))?;
        let region_order: Vec<String> = oracle.lists[3].iter().map(|p| p.name.clone()).collect();
        let mut taken: Vec<Vec<BigUint>> = Vec::new();
        let mut taken_idx: Vec<usize> = Vec::new();
        for m in &mem_names {
            let idx = region_order.iter().position(|r| r == m).ok_or_else(|| {
                diag(
                    "4.2",
                    format!("mem operand {m:?} is not an interface region"),
                )
            })?;
            taken_idx.push(idx);
            taken.push(std::mem::take(&mut self.regions[idx]));
        }
        let mut callee_args = Vec::new();
        for n in &arg_names {
            match self.args.get(n) {
                Some(v) => callee_args.push(v.clone()),
                None => {
                    return Err(diag(
                        "4.2",
                        format!("unresolved classical argument {n:?} in @{fname}"),
                    ));
                }
            }
        }
        let result = crate::action::execute_perm_basis(
            self.obj,
            &callee,
            &muts,
            &ins_v,
            &taken,
            &callee_args,
        );
        match result {
            Ok((new_muts, new_mem)) => {
                for (cells, idx) in new_mem.into_iter().zip(taken_idx) {
                    // Only a `mut` slot may change the caller's region;
                    // `in` slots come back unchanged here regardless.
                    self.regions[idx] = cells;
                }
                for (n, v) in mut_names.iter().zip(new_muts) {
                    self.write(fname, &crate::action::base_name(n), v)?;
                }
                let _ = sig;
                Ok(())
            }
            Err(e) => {
                for (cells, idx) in taken.into_iter().zip(taken_idx) {
                    self.regions[idx] = cells;
                }
                Err(e)
            }
        }
    }
}

fn both_widths(a: Option<usize>, b: Option<usize>, func: &str, op: &str) -> Result<usize, String> {
    match (a, b) {
        (Some(x), Some(y)) if x == y => Ok(x),
        _ => Err(diag(
            "3.3",
            format!("{op} operands in @{func} must have equal widths, got {a:?} and {b:?}"),
        )),
    }
}

/// Run one oracle to `F_B`. `mem` carries one cell vector per `mem`
/// interface slot, in signature order.
pub fn run_oracle(
    obj: &Object,
    name: &str,
    inputs: &[BigUint],
    mem: &mut [Vec<BigUint>],
    args: &[Value],
    budget: Budget,
) -> Result<Outcome, String> {
    cliargs::validate_budget(obj, &budget)?;
    let func = obj.function(name).ok_or_else(|| {
        diag(
            "5.2",
            format!("oracle @{name} is unresolved; executing it needs linking (milestone 2)"),
        )
    })?;
    if func.kind != FnKind::Oracle {
        return Err(diag("5.2", format!("@{name} is not an oracle")));
    }
    let sig = crate::validate::oracle_sig(obj, name)?;
    if inputs.len() != sig.ins.len() {
        return Err(diag("5.1", "oracle input count differs from signature"));
    }
    for (v, w) in inputs.iter().zip(sig.ins.iter()) {
        if v.bits() > *w {
            return Err(diag("3.3", "integer literal exceeds declared width"));
        }
    }
    if mem.len() != sig.mem.len() {
        return Err(diag("5.1", "oracle mem count differs from signature"));
    }
    if args.len() != sig.args.len() {
        return Err(diag(
            "5.1",
            "oracle classical argument count differs from signature",
        ));
    }
    let order = index_order(obj, func)?;
    let mut widths: HashMap<String, HashMap<String, usize>> = HashMap::new();
    let mut locations: HashMap<String, HashMap<String, BigUint>> = HashMap::new();
    let mut index_offsets: HashMap<String, usize> = HashMap::new();
    let mut offset = 0usize;
    for fname in &order {
        let f = obj.function(fname).ok_or_else(|| {
            diag(
                "5.2",
                format!(
                    "lproc @{fname} in the closure of @{name} is unresolved; linking is milestone 2"
                ),
            )
        })?;
        index_offsets.insert(fname.clone(), offset);
        offset += f.instr_count();
        let lw = location_widths(f);
        widths.insert(fname.clone(), lw.iter().cloned().collect());
        locations.insert(
            fname.clone(),
            lw.iter()
                .map(|(n, _)| (n.clone(), BigUint::zero()))
                .collect(),
        );
    }
    let mut pseudo: HashMap<String, BigUint> = HashMap::new();
    let mut pseudo_widths: HashMap<String, usize> = HashMap::new();
    for (p, v) in func.lists[0].iter().zip(inputs.iter()) {
        pseudo.insert(p.name.clone(), v.clone());
        if let Type::Wire(n) = p.ty {
            pseudo_widths.insert(p.name.clone(), n);
        }
    }
    let mut arg_env: HashMap<String, Value> = HashMap::new();
    for (p, v) in func.lists[4].iter().zip(args.iter()) {
        arg_env.insert(p.name.clone(), v.clone());
        match (&p.ty, v) {
            (Type::I(n), Value::Int { bits, .. }) => {
                pseudo.insert(p.name.clone(), bits.clone());
                pseudo_widths.insert(p.name.clone(), *n);
            }
            (Type::Ptr, Value::Ptr(bits)) => {
                pseudo.insert(p.name.clone(), bits.clone());
                pseudo_widths.insert(p.name.clone(), obj.ptr_width);
            }
            (Type::Token, Value::Token(b) | Value::TokenId(_, b)) => {
                cliargs::validate_budget(obj, b)?;
            }
            (Type::F64, Value::F64(_)) => {}
            _ => {
                return Err(diag(
                    "5.1",
                    format!(
                        "classical argument {:?} of @{name} has the wrong value kind",
                        p.name
                    ),
                ));
            }
        }
    }
    // Initial allocator state (§3.7).
    let mut mem_map: HashMap<BigUint, u8> = HashMap::new();
    let mut alloctab: Vec<AllocEntry> = Vec::new();
    let mut region_maps: Vec<RegionMap> = Vec::new();
    let mut regions: Vec<Vec<BigUint>> = mem.to_vec();
    if let Some(layout) = &obj.layout {
        for (gname, g) in &layout.globals {
            alloctab.push(AllocEntry {
                base: g.base.clone(),
                size: g.size.clone(),
                live: true,
                pinned: true,
            });
            let bytes = from_hex(obj.data.get(gname).map(|s| s.as_str()).unwrap_or(""))
                .map_err(|e| diag("16.1", e))?;
            for (i, b) in bytes.into_iter().enumerate() {
                if b != 0 {
                    mem_map.insert(g.base.add(&BigUint::from_u64(i as u64)), b);
                }
            }
        }
        for (i, p) in func.lists[3].iter().enumerate() {
            let (a, w) = match p.ty {
                Type::Region(a, w) => (a, w),
                _ => {
                    return Err(diag(
                        "5.1",
                        format!("mem slot {:?} is not a region", p.name),
                    ));
                }
            };
            if let Some(base) = layout
                .interface_regions
                .get(name)
                .and_then(|m| m.get(&p.name))
            {
                if w % 8 != 0 {
                    return Err(diag(
                        "7.2",
                        format!(
                            "mapped region {:?} requires W to be a whole number of bytes in this executor (W=8 per §7.2)",
                            p.name
                        ),
                    ));
                }
                let len = BigUint::from_u64(1)
                    .shl(a)
                    .mul(&BigUint::from_u64((w / 8) as u64));
                alloctab.push(AllocEntry {
                    base: base.clone(),
                    size: len.clone(),
                    live: true,
                    pinned: true,
                });
                region_maps.push(RegionMap {
                    base: base.clone(),
                    len,
                    region_index: i,
                    w,
                });
            }
        }
    }
    // Cell counts: absent cells read as zero; grow sparse callers.
    for (i, (a, w, _)) in sig.mem.iter().enumerate() {
        let count = 1usize
            .checked_shl(*a as u32)
            .ok_or_else(|| format!("host limit: region of @{name} has 2^{a} cells"))?;
        if regions[i].len() < count {
            regions[i].resize(count, BigUint::zero());
        }
        for cell in regions[i].iter_mut() {
            *cell = cell.mask(*w);
        }
    }
    let mut machine = Machine {
        obj,
        oracle_name: name.to_string(),
        locations,
        widths,
        pseudo,
        pseudo_widths,
        args: arg_env,
        mem: mem_map,
        regions,
        region_maps,
        alloctab,
        heap_used: BigUint::zero(),
        sp: BigUint::zero(),
        depth: 0,
        result_val: BigUint::zero(),
        tgt_width: sig.tgt,
        fault_width: sig.fault,
        budget,
        steps: 0,
        call_stack: Vec::new(),
        index_order: order,
        index_offsets,
    };
    // §5.2's prefix reversal restores every caller-owned `in` cell.
    // The sequential F_B evaluator owns its scratch copy; only the XOR
    // payload/code escape. In particular nested calls must not leak stores.
    machine.run()
}

impl<'o> Machine<'o> {
    fn run(&mut self) -> Result<Outcome, String> {
        let obj: &'o Object = self.obj;
        let mut current = self.oracle_name.clone();
        let mut block_i = 0usize;
        let mut instr_i = 0usize;
        loop {
            if self.steps >= self.budget.steps {
                return Ok(Outcome {
                    payload: BigUint::zero(),
                    code: BigUint::from_u64(FAULT_BUDGET_STEPS),
                    steps: self.steps,
                });
            }
            let func = obj
                .function(&current)
                .ok_or_else(|| diag("3.7", format!("unknown function @{current}")))?;
            let block = func
                .blocks
                .get(block_i)
                .ok_or_else(|| diag("3.6", format!("pc outside @{current}")))?;
            let ins = block
                .instrs
                .get(instr_i)
                .ok_or_else(|| {
                    diag(
                        "3.6",
                        format!("pc falls off block ^{} of @{current}", block.name),
                    )
                })?
                .clone();
            self.steps += 1;
            match self.exec_row(func, &ins)? {
                Flow::Next => {
                    instr_i += 1;
                }
                Flow::Jump(b, i) => {
                    block_i = b;
                    instr_i = i;
                }
                Flow::HaltOk => {
                    return Ok(Outcome {
                        payload: self.result_val.clone(),
                        code: BigUint::zero(),
                        steps: self.steps,
                    });
                }
                Flow::HaltFault(code) => {
                    return Ok(Outcome {
                        payload: BigUint::zero(),
                        code: code.mask(self.fault_width),
                        steps: self.steps,
                    });
                }
                Flow::Call(callee) => {
                    let callee_f = obj.function(&callee).ok_or_else(|| {
                        diag(
                            "3.7",
                            format!(
                                "lift.call to unresolved lproc @{callee}; linking is milestone 2"
                            ),
                        )
                    })?;
                    let locs = location_widths(callee_f);
                    let r_f = BigUint::from_u64((3 + locs.len()) as u64);
                    let over_depth = self.depth + 1 > self.budget.depth;
                    let over_stack = self
                        .sp
                        .add(&r_f)
                        .cmp_big(&BigUint::from_u64(self.budget.stack))
                        == std::cmp::Ordering::Greater;
                    if over_depth || over_stack {
                        return Ok(Outcome {
                            payload: BigUint::zero(),
                            code: BigUint::from_u64(FAULT_BUDGET_DEPTH),
                            steps: self.steps,
                        });
                    }
                    let arg_names = crate::validate::list_operand(&ins.operands[1])?;
                    let mut arg_vals = Vec::new();
                    for a in &arg_names {
                        arg_vals.push(self.read(&current, a)?.0);
                    }
                    // Save, then zero + bind (§3.7 C1-C5, abstractly).
                    let saved: Vec<(String, BigUint)> = locs
                        .iter()
                        .map(|(n, _)| {
                            (
                                n.clone(),
                                self.locations
                                    .get(&callee)
                                    .and_then(|m| m.get(n))
                                    .cloned()
                                    .unwrap_or_default(),
                            )
                        })
                        .collect();
                    let entry = self.locations.entry(callee.clone()).or_default();
                    for (n, _) in &locs {
                        entry.insert(n.clone(), BigUint::zero());
                    }
                    for (p, v) in callee_f.lists[0].iter().zip(arg_vals) {
                        if let Type::Reg(w) = p.ty {
                            entry.insert(p.name.clone(), v.mask(w));
                        }
                    }
                    self.sp = self.sp.add(&r_f);
                    self.depth += 1;
                    self.call_stack.push(Frame {
                        caller: current.clone(),
                        block: block_i,
                        instr: instr_i + 1,
                        dest: ins.lhs[0].clone(),
                        saved,
                        callee: callee.clone(),
                    });
                    current = callee;
                    block_i = 0;
                    instr_i = 0;
                }
                Flow::Return(v) => {
                    let frame = self
                        .call_stack
                        .pop()
                        .ok_or_else(|| diag("3.7", "lift.ret with an empty stack"))?;
                    let entry = self.locations.entry(frame.callee.clone()).or_default();
                    for (n, saved) in &frame.saved {
                        entry.insert(n.clone(), saved.clone());
                    }
                    self.depth -= 1;
                    let caller = frame.caller.clone();
                    self.write(&caller, &frame.dest, v)?;
                    current = caller;
                    block_i = frame.block;
                    instr_i = frame.instr;
                }
            }
            let _ = &self.index_order;
        }
    }
}

pub fn cmd_run_oracle(args: &[String]) -> Result<(), String> {
    let opts = cliargs::scan_opts(args, &["input", "budget", "mem", "arg"], &[])?;
    if opts.positional.len() != 2 {
        return Err("run-oracle needs <object> and <oracle>".into());
    }
    let text = cliargs::read_object(&opts.positional[0])?;
    let (obj, _) = crate::load(&text, false)?;
    let name = cliargs::entry_name(&opts.positional[1]).to_string();
    let func = obj.function(&name);
    match func {
        Some(f) if f.kind == FnKind::Oracle => {}
        Some(_) => return Err(diag("5.2", format!("@{name} is not an oracle"))),
        None => return Err(diag("5.2", format!("unknown oracle @{name}"))),
    }
    let sig = crate::validate::oracle_sig(&obj, &name)?;
    let input_text = opts
        .one("input")?
        .ok_or("run-oracle requires --input <x[,y...]>")?;
    let inputs: Vec<BigUint> = if input_text.is_empty() {
        Vec::new()
    } else {
        input_text
            .split(',')
            .map(|t| {
                BigUint::parse_int(t.trim()).map_err(|_| format!("invalid oracle input {t:?}"))
            })
            .collect::<Result<_, _>>()?
    };
    if inputs.len() != sig.ins.len() {
        return Err(diag("5.1", "oracle input count differs from signature"));
    }
    for (v, w) in inputs.iter().zip(sig.ins.iter()) {
        if v.bits() > *w {
            return Err(diag("3.3", "integer literal exceeds declared width"));
        }
    }
    let budget = match opts.one("budget")? {
        Some(t) => cliargs::parse_budget(t, &obj)?,
        None => cliargs::default_budget(&obj, 1_000_000),
    };
    let mut mem: Vec<Vec<BigUint>> = Vec::new();
    let mut mem_given: std::collections::BTreeMap<String, Vec<BigUint>> = Default::default();
    for kv in opts.all("mem") {
        let (rname, hex) = kv
            .split_once('=')
            .ok_or_else(|| format!("invalid --mem {kv:?} (want name=hex)"))?;
        let bytes = from_hex(hex).map_err(|e| format!("invalid --mem hex: {e}"))?;
        mem_given.insert(
            rname.to_string(),
            bytes_to_cells(&bytes, rname, &obj, &name)?,
        );
    }
    let oracle = obj.function(&name).unwrap();
    for (p, (a, w, _)) in oracle.lists[3].iter().zip(sig.mem.iter()) {
        match mem_given.remove(&p.name) {
            Some(cells) => mem.push(cells),
            None => {
                let _ = w;
                mem.push(vec![BigUint::zero(); 1usize << *a]);
            }
        }
    }
    if let Some((unknown, _)) = mem_given.iter().next() {
        return Err(diag(
            "5.1",
            format!("--mem names {unknown:?}, which is not an interface region of @{name}"),
        ));
    }
    let mut arg_values: Vec<Value> = Vec::new();
    let raw_args = opts.all("arg");
    if raw_args.len() != sig.args.len() {
        return Err(diag(
            "5.1",
            "oracle classical argument count differs from signature",
        ));
    }
    for (raw, ty) in raw_args.iter().zip(sig.args.iter()) {
        let parsed = cliargs::parse_raw_arg(raw, &budget)?;
        let value = match (ty, parsed) {
            (Type::I(n), cliargs::RawArg::Int(t)) => {
                Value::int(*n, cliargs::bind_int(&t, *n, &format!("i{n}"))?)
            }
            (Type::Ptr, cliargs::RawArg::Int(t)) => {
                Value::Ptr(cliargs::bind_int(&t, obj.ptr_width, "ptr")?)
            }
            (Type::F64, cliargs::RawArg::F64(v)) => Value::F64(v),
            (Type::Token, cliargs::RawArg::Token(b)) => {
                cliargs::validate_budget(&obj, &b)?;
                Value::Token(b)
            }
            _ => {
                return Err(diag(
                    "10",
                    format!(
                        "E_ARG: argument {raw:?} does not match {}",
                        ty.signature_text()
                    ),
                ));
            }
        };
        arg_values.push(value);
    }
    let outcome = run_oracle(&obj, &name, &inputs, &mut mem, &arg_values, budget)?;
    println!(
        "{}",
        Json::obj(vec![
            ("code", Json::u(&outcome.code)),
            ("payload", Json::u(&outcome.payload)),
            ("steps", Json::u64(outcome.steps)),
        ])
        .to_jcs()
    );
    Ok(())
}

fn bytes_to_cells(
    bytes: &[u8],
    rname: &str,
    obj: &Object,
    oracle_name: &str,
) -> Result<Vec<BigUint>, String> {
    let oracle = obj
        .function(oracle_name)
        .ok_or_else(|| diag("5.2", format!("unknown oracle @{oracle_name}")))?;
    let param = oracle.lists[3]
        .iter()
        .find(|p| p.name == rname)
        .ok_or_else(|| {
            diag(
                "5.1",
                format!(
                    "--mem names {rname:?}, which is not an interface region of @{oracle_name}"
                ),
            )
        })?;
    let (a, w) = match param.ty {
        Type::Region(a, w) => (a, w),
        _ => return Err(diag("5.1", format!("{rname:?} is not a region"))),
    };
    let ncells = 1usize
        .checked_shl(a as u32)
        .ok_or_else(|| format!("host limit: region {rname:?} has 2^{a} cells"))?;
    let want_bits = ncells * w;
    if bytes.len() * 8 != want_bits {
        return Err(diag(
            "5.1",
            format!(
                "--mem {rname} supplies {} bytes, the region needs exactly {}",
                bytes.len(),
                want_bits / 8
            ),
        ));
    }
    let mut all = BigUint::zero();
    for (i, b) in bytes.iter().enumerate() {
        if *b != 0 {
            all = all.add(&BigUint::from_u64(*b as u64).shl(i * 8));
        }
    }
    Ok((0..ncells).map(|i| all.shr(i * w).mask(w)).collect())
}
