//! Literal basis realization of §3.6, with §3.7 activation cells.
//! Assignment history contains displaced values, never whole-state snapshots.
use crate::{
    bigint::{BigUint, sign_extend},
    object::{Instr, Object, Type},
    oracle::Outcome,
    value::{Budget, Value, binop, div_fault, divop, icmp},
};
use std::collections::BTreeMap;
fn u(n: usize) -> BigUint {
    BigUint::from_u64(n as u64)
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Loc(String, String),
    Mem(BigUint),
    Tab(usize),
    Stack(usize),
    Counter(usize),
    Result,
    Code,
    Fault,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Configuration {
    cells: BTreeMap<Key, BigUint>,
    pub pc: usize,
    pub pc_prime: usize,
}
impl Configuration {
    fn get(&self, k: &Key) -> BigUint {
        self.cells.get(k).cloned().unwrap_or_default()
    }
    fn set(&mut self, k: Key, v: BigUint) {
        if v.is_zero() {
            self.cells.remove(&k);
        } else {
            self.cells.insert(k, v);
        }
    }
    fn xor(&mut self, k: Key, v: &BigUint) {
        self.set(k.clone(), self.get(&k).bitxor(v));
    }
    fn exchange(&mut self, k: Key, v: &mut BigUint) {
        let old = self.get(&k);
        self.set(k, v.clone());
        *v = old;
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedWidths {
    pub pc: usize,
    pub heap: usize,
    pub atop: usize,
    pub stack: usize,
    pub depth: usize,
    pub cell: usize,
    pub slots: Vec<usize>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub old_pc: usize,
    pub slots: Vec<BigUint>,
    pub flag: bool,
    pub succ: usize,
}
#[derive(Clone)]
struct Row {
    flag: bool,
    code: BigUint,
    succ: usize,
    assign: Vec<(Key, BigUint)>,
}
pub struct Machine<'a> {
    obj: &'a Object,
    name: String,
    budget: Budget,
    args: BTreeMap<String, Value>,
    rows: Vec<(String, Instr)>,
    labels: BTreeMap<(String, String), usize>,
    ranges: BTreeMap<String, (usize, usize)>,
    locations: Vec<(String, String, usize)>,
    widths: BTreeMap<(String, String), usize>,
    pseudo: BTreeMap<(String, String), BigUint>,
    pub widths_derived: DerivedWidths,
    pub state: Configuration,
    pub frames: Vec<Frame>,
    pub steps: u64,
}
impl<'a> Machine<'a> {
    pub fn new(
        obj: &'a Object,
        name: &str,
        inputs: &[BigUint],
        mem: &[Vec<BigUint>],
        args: &[Value],
        budget: Budget,
    ) -> Result<Self, String> {
        crate::cliargs::validate_budget(obj, &budget)?;
        let f = obj.function(name).ok_or("§5.2: unknown oracle")?;
        let sig = crate::validate::oracle_sig(obj, name)?;
        if inputs.len() != sig.ins.len()
            || mem.len() != sig.mem.len()
            || args.len() != sig.args.len()
        {
            return Err("§5.1: interface counts differ".into());
        }
        let order = crate::oracle::index_order(obj, f)?;
        let mut rows = vec![];
        let mut labels = BTreeMap::new();
        let mut ranges = BTreeMap::new();
        let mut locations = vec![];
        let mut widths = BTreeMap::new();
        let mut pseudo = BTreeMap::new();
        for fname in order {
            let ff = obj.function(&fname).ok_or("§3.7: unresolved lproc")?;
            let first = rows.len();
            for b in &ff.blocks {
                labels.insert((fname.clone(), b.name.clone()), rows.len());
                for i in &b.instrs {
                    rows.push((fname.clone(), i.clone()));
                }
            }
            ranges.insert(fname.clone(), (first, rows.len()));
            for (n, w) in crate::oracle::location_widths(ff) {
                widths.insert((fname.clone(), n.clone()), w);
                locations.push((fname.clone(), n, w));
            }
        }
        for (p, v) in f.lists[0].iter().zip(inputs) {
            let w = p.ty.width().unwrap();
            if v.bits() > w {
                return Err("§3.3: input width mismatch".into());
            }
            widths.insert((name.into(), p.name.clone()), w);
            pseudo.insert((name.into(), p.name.clone()), v.clone());
        }
        let mut argmap = BTreeMap::new();
        for (p, v) in f.lists[4].iter().zip(args) {
            argmap.insert(p.name.clone(), v.clone());
            match v {
                Value::Int { bits, .. } | Value::Ptr(bits) => {
                    pseudo.insert((name.into(), p.name.clone()), bits.clone());
                    widths.insert(
                        (name.into(), p.name.clone()),
                        p.ty.width().unwrap_or(obj.ptr_width),
                    );
                }
                _ => {}
            }
        }
        let mut state = Configuration::default();
        if let Some(l) = &obj.layout {
            let mut atop = 0;
            for (gname, g) in &l.globals {
                state.set(
                    Key::Tab(atop),
                    Self::entry(&g.base, &g.size, true, true, obj.ptr_width),
                );
                atop += 1;
                for (i, b) in crate::sha256::from_hex(&obj.data[gname])?
                    .iter()
                    .enumerate()
                {
                    state.set(Key::Mem(g.base.add(&u(i))), u(*b as usize));
                }
            }
            for (p, cells) in f.lists[3].iter().zip(mem) {
                let Type::Region(a, w) = p.ty else {
                    return Err("§5.1: invalid region".into());
                };
                let base = l
                    .interface_regions
                    .get(name)
                    .and_then(|m| m.get(&p.name))
                    .ok_or("§7.2: unmapped region")?;
                state.set(
                    Key::Tab(atop),
                    Self::entry(base, &u((1usize << a) * w / 8), true, true, obj.ptr_width),
                );
                atop += 1;
                for (i, v) in cells.iter().enumerate() {
                    for j in 0..w / 8 {
                        state.set(Key::Mem(base.add(&u(i * w / 8 + j))), v.shr(j * 8).mask(8));
                    }
                }
            }
            state.set(Key::Counter(1), u(atop));
        }
        let p = u(rows.len()).bits().max(1);
        let (heap, atop, stack) = obj
            .layout
            .as_ref()
            .map_or((0, 0, 0), |l| (l.h + 2, l.a_tab + 1, l.s_cells + 1));
        let depth = if obj.layout.is_some() {
            BigUint::from_u64(budget.depth).add(&u(2)).sub(&u(1)).bits()
        } else {
            0
        };
        let cell = p
            .max(stack)
            .max(u(locations.len().saturating_sub(1)).bits())
            .max(locations.iter().map(|(_, _, w)| *w).max().unwrap_or(0));
        let mut slot_widths = Vec::<usize>::new();
        for (fname, i) in &rows {
            let ws = match i.op.as_str() {
                "lift.alloc" => vec![obj.ptr_width, 2 * obj.ptr_width + 2, heap, atop],
                "lift.call" => {
                    let ff = obj
                        .function(i.operands[0].trim_start_matches('@'))
                        .ok_or("§3.7: unresolved lproc")?;
                    let mut ws = ff.lists[0]
                        .iter()
                        .map(|p| p.ty.width().unwrap())
                        .collect::<Vec<_>>();
                    ws.extend([stack, stack, depth]);
                    ws
                }
                "lift.ret" => vec![cell, stack, depth],
                op if op.starts_with("lift.store.i") => vec![
                    op.rsplit('i')
                        .next()
                        .unwrap()
                        .parse::<usize>()
                        .map_err(|_| "§3.3: invalid store width")?,
                ],
                _ => i
                    .lhs
                    .iter()
                    .map(|n| {
                        widths
                            .get(&(fname.clone(), n.clone()))
                            .copied()
                            .ok_or("§3.3: unknown destination")
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            slot_widths.resize(slot_widths.len().max(ws.len()), 0);
            for (j, w) in ws.into_iter().enumerate() {
                slot_widths[j] = slot_widths[j].max(w);
            }
        }
        Ok(Self {
            obj,
            name: name.into(),
            budget,
            args: argmap,
            rows,
            labels,
            ranges,
            locations,
            widths,
            pseudo,
            widths_derived: DerivedWidths {
                pc: p,
                heap,
                atop,
                stack,
                depth,
                cell,
                slots: slot_widths,
            },
            state,
            frames: vec![],
            steps: 0,
        })
    }
    fn entry(base: &BigUint, size: &BigUint, live: bool, pinned: bool, p: usize) -> BigUint {
        base.add(&size.shl(p))
            .add(&u(usize::from(live)).shl(2 * p))
            .add(&u(usize::from(pinned)).shl(2 * p + 1))
    }
    fn counter(&self, i: usize) -> BigUint {
        self.state.get(&Key::Counter(i))
    }
    fn count(&self, i: usize) -> usize {
        self.counter(i)
            .to_usize()
            .expect("host limit: step counter address")
    }
    fn read(&self, f: &str, n: &str) -> Result<BigUint, String> {
        let k = (f.to_string(), n.to_string());
        if let Some(v) = self.pseudo.get(&k) {
            return Ok(v.clone());
        }
        if self.widths.contains_key(&k) {
            return Ok(self.state.get(&Key::Loc(k.0, k.1)));
        }
        BigUint::parse_int(n).map_err(|_| format!("§3.7: unresolved {f}.{n}"))
    }
    fn width(&self, f: &str, n: &str) -> Result<usize, String> {
        self.widths
            .get(&(f.into(), n.into()))
            .copied()
            .ok_or_else(|| format!("§3.3: unknown width {f}.{n}"))
    }
    fn loc(f: &str, n: &str) -> Key {
        Key::Loc(f.into(), n.into())
    }
    fn label(&self, f: &str, n: &str) -> Result<usize, String> {
        self.labels
            .get(&(f.into(), n.trim_start_matches('^').into()))
            .copied()
            .ok_or("§3.6: invalid label".into())
    }
    fn inrange(&self, p: &BigUint, n: &BigUint) -> bool {
        (0..self.count(1)).any(|i| {
            let e = self.state.get(&Key::Tab(i));
            let w = self.obj.ptr_width;
            let base = e.mask(w);
            let size = e.shr(w).mask(w);
            e.bit(2 * w) && base <= *p && p.add(n) <= base.add(&size)
        })
    }
    fn load(&self, p: &BigUint, n: usize) -> BigUint {
        (0..n).fold(BigUint::zero(), |v, i| {
            v.add(&self.state.get(&Key::Mem(p.add(&u(i)))).shl(i * 8))
        })
    }
    fn row(&self, f: &str, i: &Instr, k: usize) -> Result<Row, String> {
        let mut r = Row {
            flag: false,
            code: BigUint::zero(),
            succ: k + 1,
            assign: vec![],
        };
        let x = &i.operands;
        let base = i.op.strip_prefix("lift.").unwrap_or(&i.op);
        let op = base.split('.').next().unwrap();
        let read = |n: usize| self.read(f, &x[n]);
        let width = || self.width(f, &i.lhs[0]);
        let mut value = None;
        match op {
            "const" => value = Some(crate::oracle::parse_const(&x[0], width()?)?),
            "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr" => {
                value = Some(binop(op, &read(0)?, &read(1)?, width()?))
            }
            "udiv" | "urem" | "sdiv" | "srem" => {
                let (a, b, w) = (read(0)?, read(1)?, width()?);
                if let Some(c) = div_fault(op, &a, &b, w) {
                    r.flag = true;
                    r.code = BigUint::from_u64(c);
                } else {
                    value = Some(divop(op, &a, &b, w));
                }
            }
            "icmp" => {
                value = Some(u(usize::from(icmp(
                    base.split('.').nth(1).unwrap(),
                    &read(0)?,
                    &read(1)?,
                    self.width(f, &x[0])?,
                ))))
            }
            "select" => value = Some(if read(0)?.bit(0) { read(1)? } else { read(2)? }),
            "mov" => value = Some(read(0)?),
            "zext" | "trunc" => value = Some(read(0)?.mask(width()?)),
            "sext" => value = Some(sign_extend(&read(0)?, self.width(f, &x[0])?, width()?)),
            "parmov" => {
                for (d, s) in i.lhs.iter().zip(crate::validate::list_operand(&x[0])?) {
                    r.assign.push((Self::loc(f, d), self.read(f, &s)?));
                }
            }
            "label" => value = Some(u(self.label(f, &x[0])?)),
            "addr" => {
                let n = x[0].trim_start_matches('@');
                let l = self.obj.layout.as_ref().ok_or("§7.2: no layout")?;
                value = Some(
                    l.globals
                        .get(n)
                        .map(|g| g.base.clone())
                        .or_else(|| {
                            l.interface_regions
                                .get(&self.name)
                                .and_then(|m| m.get(n))
                                .cloned()
                        })
                        .ok_or("§7.2: unknown address")?,
                );
            }
            "boundscheck" => {
                r.flag = !self.inrange(&read(0)?, &read(1)?);
                r.code = u(7);
            }
            "load" | "store" => {
                let n: usize = base
                    .rsplit('i')
                    .next()
                    .unwrap()
                    .parse()
                    .map_err(|_| "§3.3: invalid memory width")?;
                let a = read(0)?;
                r.flag = !self.inrange(&a, &u(n / 8));
                r.code = u(7);
                if !r.flag {
                    if op == "load" {
                        value = Some(self.load(&a, n / 8));
                    } else {
                        r.assign.push((Key::Mem(a), read(1)?));
                    }
                }
            }
            "alloc" => {
                let l = self.obj.layout.as_ref().ok_or("§7.2: no layout")?;
                let n = read(0)?;
                let need = if n.is_zero() { u(1) } else { n.clone() };
                let heap = self.counter(0).add(&need);
                r.flag = heap > BigUint::from_u64(self.budget.heap)
                    || self.counter(1) >= u(1).shl(l.a_tab);
                r.code = u(2);
                if !r.flag {
                    let p = l.heap_base.add(&self.counter(0));
                    r.assign = vec![
                        (Self::loc(f, &i.lhs[0]), p.clone()),
                        (
                            Key::Tab(self.count(1)),
                            Self::entry(&p, &n, true, false, self.obj.ptr_width),
                        ),
                        (Key::Counter(0), heap),
                        (Key::Counter(1), self.counter(1).add(&u(1))),
                    ];
                }
            }
            "free" => {
                let p = read(0)?;
                let w = self.obj.ptr_width;
                r.flag = !(0..self.count(1)).any(|j| {
                    let e = self.state.get(&Key::Tab(j));
                    e.mask(w) == p && e.bit(2 * w) && !e.bit(2 * w + 1)
                });
                r.code = u(7);
            }
            "faultif" => {
                r.flag = read(0)?.bit(0);
                r.code = read(1)?;
            }
            "fault" => {
                r.flag = true;
                r.code = read(0)?;
            }
            "qjump" => r.succ = self.label(f, &x[0])?,
            "qbranch" => r.succ = self.label(f, &x[if read(0)?.bit(0) { 1 } else { 2 }])?,
            "qjumpr" => {
                let a = read(0)?.to_usize();
                let (first, end) = self.ranges[f];
                r.flag = !a.is_some_and(|a| a >= first && a < end);
                r.code = u(4);
                r.succ = a.unwrap_or(0);
            }
            "call" => {
                let callee = x[0].trim_start_matches('@');
                let locs = self.function_locs(callee);
                r.flag = self.counter(4).add(&u(1)) > BigUint::from_u64(self.budget.depth)
                    || self.counter(2).add(&u(3 + locs.len()))
                        > BigUint::from_u64(self.budget.stack);
                r.code = u(3);
                r.succ = self.ranges.get(callee).ok_or("§3.7: unresolved lproc")?.0;
            }
            "ret" => {
                r.flag = self.counter(4).is_zero();
                r.code = u(4);
                if !r.flag {
                    r.succ = self
                        .state
                        .get(&Key::Stack(self.count(3)))
                        .to_usize()
                        .ok_or("host limit: return index")?;
                }
            }
            "retval" => r.succ = self.rows.len(),
            "oracle_call" | "oracle_uncall" | "perm_call" => {}
            _ => return Err(format!("unimplemented: lifted row {} (milestone 3)", i.op)),
        }
        if let Some(v) = value {
            r.assign.push((Self::loc(f, &i.lhs[0]), v));
        }
        if r.flag {
            r.succ = self.rows.len();
            r.assign.clear();
        }
        Ok(r)
    }
    fn function_locs(&self, f: &str) -> Vec<(String, usize)> {
        self.locations
            .iter()
            .filter(|(ff, _, _)| ff == f)
            .map(|(_, n, w)| (n.clone(), *w))
            .collect()
    }
    fn exchange_loc_stack(&mut self, f: &str, base: usize) {
        for (j, (n, w)) in self.function_locs(f).into_iter().enumerate() {
            let k = Self::loc(f, &n);
            let v = self.state.get(&k);
            let sk = Key::Stack(base + 3 + j);
            let cell = self.state.get(&sk);
            self.state.set(k, cell.mask(w));
            let upper = cell.shr(w).shl(w);
            self.state.set(sk, upper.add(&v.mask(w)));
        }
    }
    fn call(
        &mut self,
        f: &str,
        i: &Instr,
        k: usize,
        frame: &mut Frame,
        inverse: bool,
    ) -> Result<(), String> {
        let callee = i.operands[0].trim_start_matches('@');
        let ff = self.obj.function(callee).ok_or("§3.7: unresolved lproc")?;
        let params = ff.lists[0].clone();
        let names = crate::validate::list_operand(&i.operands[1])?;
        let m = params.len();
        let size = 3 + self.function_locs(callee).len();
        if !inverse {
            frame.slots = vec![BigUint::zero(); m + 3];
            for (j, n) in names.iter().enumerate() {
                frame.slots[j] = self.read(f, n)?;
            }
        }
        if inverse {
            for j in (0..3).rev() {
                self.state
                    .exchange(Key::Counter([3, 2, 4][j]), &mut frame.slots[m + j]);
            }
            let vals = [
                self.counter(2),
                self.counter(2).add(&u(size)),
                self.counter(4).add(&u(1)),
            ];
            for (j, v) in vals.iter().enumerate() {
                frame.slots[m + j] = frame.slots[m + j].bitxor(v);
            }
        }
        let base = self.count(2);
        if !inverse {
            self.exchange_loc_stack(callee, base);
        }
        let dest_id = self
            .locations
            .iter()
            .position(|(ff, n, _)| ff == f && n == &i.lhs[0])
            .ok_or("§3.7: destination id missing")?;
        if !inverse {
            for (j, v) in [u(k + 1), self.counter(3), u(dest_id)]
                .into_iter()
                .enumerate()
            {
                self.state.xor(Key::Stack(base + j), &v);
            }
        }
        for (j, p) in params.iter().enumerate() {
            self.state.xor(Self::loc(callee, &p.name), &frame.slots[j]);
        }
        if inverse {
            for (j, v) in [u(k + 1), self.counter(3), u(dest_id)]
                .into_iter()
                .enumerate()
                .rev()
            {
                self.state.xor(Key::Stack(base + j), &v);
            }
            self.exchange_loc_stack(callee, base);
            for (j, n) in names.iter().enumerate() {
                frame.slots[j] = frame.slots[j].bitxor(&self.read(f, n)?);
            }
        } else {
            let vals = [
                self.counter(2),
                self.counter(2).add(&u(size)),
                self.counter(4).add(&u(1)),
            ];
            for (j, v) in vals.into_iter().enumerate() {
                frame.slots[m + j] = v;
            }
            for j in 0..3 {
                self.state
                    .exchange(Key::Counter([3, 2, 4][j]), &mut frame.slots[m + j]);
            }
        }
        Ok(())
    }
    fn ret(&mut self, f: &str, i: &Instr, frame: &mut Frame, inverse: bool) -> Result<(), String> {
        if inverse {
            self.state.exchange(Key::Counter(4), &mut frame.slots[2]);
            self.state.exchange(Key::Counter(3), &mut frame.slots[1]);
            frame.slots[2] = frame.slots[2].bitxor(&self.counter(4).sub(&u(1)));
            frame.slots[1] = frame.slots[1].bitxor(&self.state.get(&Key::Stack(self.count(3) + 1)));
        }
        let base = self.count(3);
        if !inverse {
            frame.slots = vec![
                self.read(f, &i.operands[0])?,
                BigUint::zero(),
                BigUint::zero(),
            ];
            self.exchange_loc_stack(f, base);
        }
        let id = self
            .state
            .get(&Key::Stack(base + 2))
            .to_usize()
            .ok_or("host limit: location id")?;
        let (ff, n, _) = self
            .locations
            .get(id)
            .ok_or("§3.7: bad destination id")?
            .clone();
        self.state.exchange(Self::loc(&ff, &n), &mut frame.slots[0]);
        if inverse {
            self.exchange_loc_stack(f, base);
            frame.slots[0] = frame.slots[0].bitxor(&self.read(f, &i.operands[0])?);
        } else {
            frame.slots[1] = self.state.get(&Key::Stack(base + 1));
            frame.slots[2] = self.counter(4).sub(&u(1));
            self.state.exchange(Key::Counter(3), &mut frame.slots[1]);
            self.state.exchange(Key::Counter(4), &mut frame.slots[2]);
        }
        Ok(())
    }
    fn direct(&mut self, f: &str, i: &Instr, inverse: bool) -> Result<(), String> {
        match i.op.as_str() {
            "lift.retval" => {
                let v = self.read(f, &i.operands[0])?;
                self.state.xor(Key::Result, &v);
            }
            "lift.free" => {
                let p = self.read(f, &i.operands[0])?;
                let w = self.obj.ptr_width;
                for j in 0..self.count(1) {
                    let e = self.state.get(&Key::Tab(j));
                    if e.mask(w) == p {
                        self.state.xor(Key::Tab(j), &u(1).shl(2 * w));
                    }
                }
            }
            "lift.oracle_call" | "lift.oracle_uncall" | "lift.perm_call" => {
                self.region_call(f, i, inverse)?
            }
            _ => {}
        }
        Ok(())
    }
    fn region_call(&mut self, f: &str, i: &Instr, inverse: bool) -> Result<(), String> {
        let x = &i.operands;
        let name = x[0].trim_start_matches('@');
        let oracle = i.op != "lift.perm_call";
        let (ins_idx, mem_idx, args_idx) = if oracle { (2, 5, 6) } else { (2, 3, 4) };
        let inputs = crate::validate::list_operand(&x[ins_idx])?
            .iter()
            .map(|n| self.read(f, n))
            .collect::<Result<Vec<_>, _>>()?;
        let mem_names = crate::validate::list_operand(&x[mem_idx])?;
        let root = self.obj.function(&self.name).unwrap();
        let mut regions = vec![];
        let mut bases = vec![];
        for n in mem_names {
            let p = root.lists[3]
                .iter()
                .find(|p| p.name == n)
                .ok_or("§5.1: unmapped call region")?;
            let Type::Region(a, w) = p.ty else {
                return Err("§5.1: invalid region".into());
            };
            let base = self.obj.layout.as_ref().unwrap().interface_regions[&self.name][&n].clone();
            regions.push(
                (0..(1usize << a))
                    .map(|j| self.load(&base.add(&u(j * w / 8)), w / 8))
                    .collect::<Vec<_>>(),
            );
            bases.push((base, w));
        }
        let args = crate::validate::list_operand(&x[args_idx])?
            .iter()
            .map(|n| {
                self.args
                    .get(n)
                    .cloned()
                    .ok_or_else(|| format!("§3.7: unknown classical argument {n}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if oracle {
            let tk = self.args.get(&x[1]).ok_or("§5.4: unknown token")?;
            let budget = match tk {
                Value::Token(b) | Value::TokenId(_, b) => *b,
                _ => return Err("§5.4: expected token".into()),
            };
            let o =
                crate::oracle::run_oracle(self.obj, name, &inputs, &mut regions, &args, budget)?;
            self.state.xor(Self::loc(f, &x[3]), &o.payload);
            self.state.xor(Self::loc(f, &x[4]), &o.code);
        } else {
            let names = crate::validate::list_operand(&x[1])?;
            let muts = names
                .iter()
                .map(|n| self.read(f, n))
                .collect::<Result<Vec<_>, _>>()?;
            let sig = crate::validate::action_sig(self.obj, name)?;
            let mut s = crate::action::QuantumState::new();
            let mb: Vec<_> = sig.muts.iter().map(|w| s.alloc_wires(*w)).collect();
            let ib: Vec<_> = sig.ins.iter().map(|w| s.alloc_wires(*w)).collect();
            for (b, v) in mb.iter().zip(&muts).chain(ib.iter().zip(&inputs)) {
                s.prepare(b, v);
            }
            let mut rs = vec![];
            for ((a, w, _), vs) in sig.mem.iter().zip(&regions) {
                let mut cells = vec![];
                for v in vs {
                    let b = s.alloc_wires(*w);
                    s.prepare(&b, v);
                    cells.push(b);
                }
                rs.push(crate::action::RegionWires {
                    a: *a,
                    w: *w,
                    cells,
                });
            }
            let idle = s.alloc_wires(crate::qcli::borrow_capacity(self.obj, name)?);
            crate::action::execute_action_with_idle(
                self.obj,
                &mut s,
                name,
                &mb,
                &ib,
                &rs,
                &args,
                inverse,
                &[],
                &idle,
            )?;
            for (n, b) in names.iter().zip(&mb) {
                self.state.set(
                    Self::loc(f, n),
                    s.constant_value(b)
                        .ok_or("§5.3: permutation produced superposition")?,
                );
            }
            for ((base, w), r) in bases.iter().zip(&rs) {
                for (j, b) in r.cells.iter().enumerate() {
                    let v = s.constant_value(b).ok_or("§5.3: non-basis region")?;
                    for z in 0..w / 8 {
                        self.state
                            .set(Key::Mem(base.add(&u(j * w / 8 + z))), v.shr(z * 8).mask(8));
                    }
                }
            }
        }
        Ok(())
    }
    fn exchange_assignment(&mut self, i: &Instr, k: Key, v: &mut BigUint) -> Result<(), String> {
        if let Key::Mem(base) = &k
            && i.op.starts_with("lift.store.i")
        {
            let bits: usize =
                i.op.rsplit('i')
                    .next()
                    .unwrap()
                    .parse()
                    .map_err(|_| "§3.3: invalid store width")?;
            let old = self.load(base, bits / 8);
            for j in 0..bits / 8 {
                self.state
                    .set(Key::Mem(base.add(&u(j))), v.shr(j * 8).mask(8));
            }
            *v = old;
            return Ok(());
        }
        self.state.exchange(k, v);
        Ok(())
    }
    pub fn forward_step(&mut self) -> Result<(), String> {
        if self.frames.len() as u64 >= self.budget.steps {
            return Err("§5.4: more forward steps than B".into());
        }
        let k = self.state.pc;
        let mut frame = Frame::default();
        let successor = if k == self.rows.len() {
            k
        } else {
            let (f, i) = self.rows[k].clone();
            let row = self.row(&f, &i, k)?;
            frame.flag = row.flag;
            if matches!(i.op.as_str(), "qjumpr" | "lift.ret") {
                frame.succ = if i.op == "qjumpr" {
                    self.read(&f, &i.operands[0])?.to_usize().unwrap_or(0)
                } else if !row.flag {
                    row.succ
                } else {
                    0
                };
            }
            if !row.flag {
                match i.op.as_str() {
                    "lift.call" => self.call(&f, &i, k, &mut frame, false)?,
                    "lift.ret" => self.ret(&f, &i, &mut frame, false)?,
                    _ => {
                        frame.slots = row.assign.iter().map(|(_, v)| v.clone()).collect();
                        for ((key, _), slot) in row.assign.iter().zip(&mut frame.slots) {
                            self.exchange_assignment(&i, key.clone(), slot)?;
                        }
                        self.direct(&f, &i, false)?;
                    }
                }
            } else {
                self.state.xor(Key::Fault, &u(1));
                self.state.xor(Key::Code, &row.code);
            }
            self.steps += 1;
            row.succ
        };
        self.state.pc_prime ^= successor;
        std::mem::swap(&mut self.state.pc, &mut self.state.pc_prime);
        std::mem::swap(&mut self.state.pc_prime, &mut frame.old_pc);
        frame
            .slots
            .resize(self.widths_derived.slots.len(), BigUint::zero());
        for (value, width) in frame.slots.iter().zip(&self.widths_derived.slots) {
            if value.bits() > *width {
                return Err("§3.6: frame slot exceeded derived width".into());
            }
        }
        self.frames.push(frame);
        Ok(())
    }
    pub fn inverse_step(&mut self) -> Result<(), String> {
        let mut frame = self.frames.pop().ok_or("§3.6: inverse without frame")?;
        let k = frame.old_pc;
        std::mem::swap(&mut self.state.pc_prime, &mut frame.old_pc);
        std::mem::swap(&mut self.state.pc, &mut self.state.pc_prime);
        if k == self.rows.len() {
            self.state.pc_prime ^= k;
        } else {
            self.steps -= 1;
            let (f, i) = self.rows[k].clone();
            let successor = self.state.pc_prime;
            // Successor is uncomputed before row effects; flag/succ are history.
            let expected = if frame.flag {
                self.rows.len()
            } else {
                match i.op.as_str() {
                    "qjumpr" | "lift.ret" => frame.succ,
                    "qjump" => self.label(&f, &i.operands[0])?,
                    "qbranch" => self.label(
                        &f,
                        &i.operands[if self.read(&f, &i.operands[0])?.bit(0) {
                            1
                        } else {
                            2
                        }],
                    )?,
                    "lift.call" => self.ranges[i.operands[0].trim_start_matches('@')].0,
                    "lift.retval" => self.rows.len(),
                    _ => k + 1,
                }
            };
            if successor != expected {
                return Err("§3.6: successor uncompute mismatch".into());
            }
            self.state.pc_prime ^= expected;
            if frame.flag {
                let row = self.row(&f, &i, k)?;
                self.state.xor(Key::Code, &row.code);
                self.state.xor(Key::Fault, &u(1));
            } else {
                match i.op.as_str() {
                    "lift.call" => self.call(&f, &i, k, &mut frame, true)?,
                    "lift.ret" => self.ret(&f, &i, &mut frame, true)?,
                    _ => {
                        self.direct(&f, &i, true)?;
                        // Most destinations are static. alloc's indexed table destination
                        // is evaluated after inverse restoration of atop (§3.6).
                        let keys = if i.op == "lift.alloc" {
                            vec![
                                Self::loc(&f, &i.lhs[0]),
                                Key::Tab(0),
                                Key::Counter(0),
                                Key::Counter(1),
                            ]
                        } else if i.op.starts_with("lift.store.") {
                            vec![Key::Mem(self.read(&f, &i.operands[0])?)]
                        } else {
                            i.lhs.iter().map(|n| Self::loc(&f, n)).collect()
                        };
                        for j in (0..keys.len()).rev() {
                            let key = if i.op == "lift.alloc" && j == 1 {
                                Key::Tab(self.count(1))
                            } else {
                                keys[j].clone()
                            };
                            self.exchange_assignment(&i, key, &mut frame.slots[j])?;
                        }
                        let row = self.row(&f, &i, k)?;
                        for (slot, (_, v)) in frame.slots.iter_mut().zip(row.assign) {
                            *slot = slot.bitxor(&v);
                        }
                    }
                }
            }
            let old = self.row(&f, &i, k)?;
            if old.flag != frame.flag {
                return Err("§3.6: flag uncompute mismatch".into());
            }
            frame.flag ^= old.flag;
            if matches!(i.op.as_str(), "qjumpr" | "lift.ret") {
                let succ = if i.op == "qjumpr" {
                    self.read(&f, &i.operands[0])?.to_usize().unwrap_or(0)
                } else if !old.flag {
                    old.succ
                } else {
                    0
                };
                frame.succ ^= succ;
            }
        }
        if frame.old_pc != 0
            || frame.flag
            || frame.succ != 0
            || frame.slots.iter().any(|s| !s.is_zero())
            || self.state.pc_prime != 0
        {
            return Err(format!(
                "§3.6: inverse left a dirty frame at {k}: {frame:?}"
            ));
        }
        Ok(())
    }
    pub fn outcome(&self) -> Outcome {
        let halted = self.state.pc == self.rows.len();
        let fault = !self.state.get(&Key::Fault).is_zero();
        Outcome {
            payload: if halted && !fault {
                self.state.get(&Key::Result)
            } else {
                BigUint::zero()
            },
            code: if fault {
                self.state.get(&Key::Code)
            } else if !halted {
                u(1)
            } else {
                BigUint::zero()
            },
            steps: self.steps,
        }
    }
    pub fn state_json(&self) -> crate::json::Json {
        use crate::json::Json;
        let mut locs = BTreeMap::new();
        for (f, n, _) in &self.locations {
            locs.insert(
                format!("{f}.{n}"),
                Json::u(&self.state.get(&Self::loc(f, n))),
            );
        }
        for ((f, n), v) in &self.pseudo {
            locs.insert(format!("{f}.{n}"), Json::u(v));
        }
        Json::obj(vec![
            ("pc", Json::u64(self.state.pc as u64)),
            ("pc_prime", Json::u64(self.state.pc_prime as u64)),
            (
                "faulted",
                Json::Bool(!self.state.get(&Key::Fault).is_zero()),
            ),
            (
                "counters",
                Json::Arr((0..5).map(|j| Json::u(&self.counter(j))).collect()),
            ),
            ("locations", Json::Obj(locs)),
        ])
    }
}
