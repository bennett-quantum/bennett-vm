//! Sequential CC/R execution with dynamic instances and retained LIFO history.
//! Histories store displaced data; segment reversal never re-executes a callee.
use crate::{
    action::{self, QuantumState, RegionWires},
    bigint::{BigUint, sign_extend},
    cliargs,
    json::{self, Json},
    object::{FnKind, Instr, Object, Type},
    sha256::{from_hex, sha256_hex, to_hex as hex},
    value::{self, Value},
};
use std::collections::{BTreeMap, BTreeSet};

type Key = (u64, String);
#[derive(Clone, Debug, PartialEq)]
struct Allocation {
    base: BigUint,
    size: BigUint,
    live: bool,
    pinned: bool,
}
#[derive(Clone, Debug, PartialEq)]
struct Handle {
    wires: Vec<usize>,
    live: bool,
    parent: Option<u64>,
    viewed: bool,
}
#[derive(Clone, Debug)]
struct Data {
    memory: BTreeMap<BigUint, u8>,
    table: Vec<Allocation>,
    bump: BigUint,
    console: Vec<u8>,
    clock: u64,
    entropy_pos: usize,
    block: BTreeMap<u64, Vec<u8>>,
    tape: Vec<Json>,
    hosts: Vec<Json>,
    measurements: Vec<Json>,
    quantum: QuantumState,
    handles: BTreeMap<u64, Handle>,
    next_handle: u64,
}
#[derive(Clone, Debug, PartialEq)]
struct Activation {
    id: u64,
    function: String,
    block: usize,
    index: usize,
    dest: Vec<String>,
    call_start: Option<usize>,
    args: Vec<Value>,
}
#[derive(Clone)]
struct Frame {
    definitions: Vec<(Key, usize)>,
    next_activation: u64,
    data: Option<Data>,
    control: Vec<Activation>,
    pure: bool,
    reversed: bool,
    marker: bool,
    measured: bool,
}
struct Segment {
    start: usize,
    end: usize,
    name: String,
    args: Vec<Value>,
    results: Vec<Value>,
}
struct Machine<'a> {
    obj: &'a Object,
    data: Data,
    env: BTreeMap<Key, Vec<Value>>,
    stack: Vec<Activation>,
    history: Vec<Frame>,
    segments: Vec<Segment>,
    trace: Vec<Json>,
    next_activation: u64,
    steps: u64,
    fault: Option<u64>,
    results: Vec<Value>,
    entropy: Vec<u8>,
    draws: Vec<f64>,
    draw_pos: usize,
    replies: Vec<Json>,
    reply_pos: usize,
    definitions: Vec<(Key, usize)>,
}
fn err(s: impl std::fmt::Display) -> String {
    format!("§3.4: {s}")
}
fn u(n: u64) -> BigUint {
    BigUint::from_u64(n)
}
fn strs(xs: &[String]) -> Json {
    Json::Arr(xs.iter().cloned().map(Json::Str).collect())
}
fn vals(xs: &[Value]) -> Json {
    Json::Arr(xs.iter().map(Value::to_json).collect())
}
fn current(env: &BTreeMap<Key, Vec<Value>>, k: &Key) -> Option<Value> {
    env.get(k).and_then(|v| v.last()).cloned()
}
fn pure(op: &str) -> bool {
    matches!(
        op.split('.').next().unwrap(),
        "const"
            | "add"
            | "sub"
            | "mul"
            | "and"
            | "or"
            | "xor"
            | "shl"
            | "lshr"
            | "ashr"
            | "udiv"
            | "urem"
            | "sdiv"
            | "srem"
            | "icmp"
            | "select"
            | "zext"
            | "sext"
            | "trunc"
            | "ptrtoint"
            | "inttoptr"
            | "load"
            | "br"
    )
}
impl Data {
    fn new(obj: &Object) -> Result<Self, String> {
        let mut d = Self {
            memory: BTreeMap::new(),
            table: vec![],
            bump: BigUint::zero(),
            console: vec![],
            clock: 0,
            entropy_pos: 0,
            block: BTreeMap::new(),
            tape: vec![],
            hosts: vec![],
            measurements: vec![],
            quantum: QuantumState::new(),
            handles: BTreeMap::new(),
            next_handle: 1,
        };
        if let Some(l) = &obj.layout {
            d.bump = l.heap_base.clone();
            for (name, g) in &l.globals {
                d.table.push(Allocation {
                    base: g.base.clone(),
                    size: g.size.clone(),
                    live: true,
                    pinned: true,
                });
                d.write(&g.base, &from_hex(&obj.data[name])?);
            }
            if let Some(name) = &l.block_image {
                for (i, bytes) in from_hex(&obj.data[name])?.chunks(512).enumerate() {
                    d.block.insert(i as u64, bytes.to_vec());
                }
            }
        }
        Ok(d)
    }
    fn valid(&self, p: &BigUint, n: &BigUint) -> bool {
        self.table
            .iter()
            .any(|e| e.live && e.base <= *p && p.add(n) <= e.base.add(&e.size))
    }
    fn read(&self, p: &BigUint, n: usize) -> Vec<u8> {
        (0..n)
            .map(|i| *self.memory.get(&p.add(&u(i as u64))).unwrap_or(&0))
            .collect()
    }
    fn write(&mut self, p: &BigUint, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            let a = p.add(&u(i as u64));
            if *b == 0 {
                self.memory.remove(&a);
            } else {
                self.memory.insert(a, *b);
            }
        }
    }
    fn digest(&self) -> String {
        // The spec fixes the memory state, but no digest serialization.
        let entries = self
            .table
            .iter()
            .map(|e| {
                Json::obj(vec![
                    ("base", Json::u(&e.base)),
                    ("size", Json::u(&e.size)),
                    ("live", Json::Bool(e.live)),
                    ("pinned", Json::Bool(e.pinned)),
                ])
            })
            .collect();
        let bytes = self
            .memory
            .iter()
            .map(|(a, b)| Json::Arr(vec![Json::u(a), Json::u64(*b as u64)]))
            .collect();
        sha256_hex(
            Json::obj(vec![
                ("bump", Json::u(&self.bump)),
                ("table", Json::Arr(entries)),
                ("bytes", Json::Arr(bytes)),
            ])
            .to_jcs()
            .as_bytes(),
        )
    }
    fn memory_eq(&self, other: &Self) -> bool {
        self.bump == other.bump && self.table == other.table && self.memory == other.memory
    }
    fn new_handle(&mut self, wires: Vec<usize>, parent: Option<u64>) -> u64 {
        let id = self.next_handle;
        self.next_handle += 1;
        self.handles.insert(
            id,
            Handle {
                wires,
                live: true,
                parent,
                viewed: false,
            },
        );
        id
    }
    fn handle(&self, id: u64) -> Result<&Handle, u64> {
        self.handles
            .get(&id)
            .filter(|h| h.live && !h.viewed)
            .ok_or(4)
    }
}
impl<'a> Machine<'a> {
    fn read(&self, name: &str) -> Result<Value, u64> {
        current(&self.env, &(self.stack.last().unwrap().id, name.into())).ok_or(4)
    }
    fn bits(&self, name: &str) -> Result<BigUint, u64> {
        self.read(name)?.bits().cloned().ok_or(4)
    }
    fn number(&self, name: &str) -> Result<u64, u64> {
        self.bits(name)?.to_u64().ok_or(4)
    }
    fn define_at(&mut self, id: u64, name: &str, v: Value) {
        let key = (id, name.into());
        let vs = self.env.entry(key.clone()).or_default();
        self.definitions.push((key, vs.len()));
        vs.push(v);
    }
    fn define(&mut self, name: &str, v: Value) {
        self.define_at(self.stack.last().unwrap().id, name, v);
    }
    fn operands(&self, xs: &[String]) -> Result<Vec<Value>, u64> {
        xs.iter().map(|n| self.read(n)).collect()
    }
    fn id(&self, n: &str) -> Result<u64, u64> {
        if let Value::Qreg(i) = self.read(n)? {
            self.data.handle(i)?;
            Ok(i)
        } else {
            Err(4)
        }
    }
    fn wires(&self, n: &str) -> Result<Vec<usize>, String> {
        if let Some((base, index)) = n.split_once('[') {
            let i: usize = index.trim_end_matches(']').parse().map_err(err)?;
            let ws = self.wires(base)?;
            return ws
                .get(i)
                .map(|w| vec![*w])
                .ok_or_else(|| err("wire index out of bounds"));
        }
        let id = self.id(n).map_err(|_| err(format!("invalid qreg {n}")))?;
        Ok(self.data.handle(id).unwrap().wires.clone())
    }
    fn undo(&mut self, index: usize, control: bool) {
        let frame = &mut self.history[index];
        if frame.reversed || frame.marker {
            return;
        }
        for (key, len) in frame.definitions.iter().rev() {
            if let Some(vs) = self.env.get_mut(key) {
                vs.truncate(*len);
                if vs.is_empty() {
                    self.env.remove(key);
                }
            }
        }
        if let Some(d) = &frame.data {
            self.data = d.clone();
        }
        if control {
            self.stack = frame.control.clone();
            self.next_activation = frame.next_activation;
        }
        frame.reversed = true;
    }
    fn step(&mut self) -> Result<(), String> {
        let a = self.stack.last().unwrap().clone();
        let f = self.obj.function(&a.function).unwrap();
        let block = &f.blocks[a.block];
        let ins = block
            .instrs
            .get(a.index)
            .ok_or_else(|| err("instruction pointer fell off block"))?
            .clone();
        let before = self.data.clone();
        let env_before = self.env.clone();
        let control = self.stack.clone();
        let next_activation = self.next_activation;
        self.definitions.clear();
        self.stack.last_mut().unwrap().index += 1;
        let fault = self.exec(&ins)?;
        if let Some(code) = fault {
            self.data = before.clone();
            self.fault = Some(code);
            self.results.clear();
        }
        let mut keys: BTreeSet<Key> = self.definitions.iter().map(|(k, _)| k.clone()).collect();
        keys.extend(
            env_before
                .keys()
                .chain(self.env.keys())
                .filter(|k| current(&env_before, k) != current(&self.env, k))
                .cloned(),
        );
        let regs = keys
            .into_iter()
            .map(|k| {
                Json::obj(vec![
                    ("activation", Json::u64(k.0)),
                    ("name", Json::Str(k.1.clone())),
                    (
                        "pre",
                        current(&env_before, &k)
                            .map(|v| v.to_json())
                            .unwrap_or(Json::Null),
                    ),
                    (
                        "post",
                        current(&self.env, &k)
                            .map(|v| v.to_json())
                            .unwrap_or(Json::Null),
                    ),
                ])
            })
            .collect();
        let addresses: BTreeSet<_> = before
            .memory
            .keys()
            .chain(self.data.memory.keys())
            .cloned()
            .collect();
        let mem = addresses
            .into_iter()
            .filter_map(|a| {
                let p = *before.memory.get(&a).unwrap_or(&0);
                let q = *self.data.memory.get(&a).unwrap_or(&0);
                (p != q).then(|| {
                    Json::obj(vec![
                        ("address", Json::u(&a)),
                        ("pre", Json::u64(p as u64)),
                        ("post", Json::u64(q as u64)),
                    ])
                })
            })
            .collect();
        self.trace.push(Json::obj(vec![
            ("step", Json::u64(self.steps)),
            ("function", Json::Str(a.function)),
            ("block", Json::Str(block.name.clone())),
            ("index", Json::u64(a.index as u64)),
            ("line", Json::u64(ins.line as u64)),
            ("op", Json::Str(ins.op.clone())),
            ("lhs", strs(&ins.lhs)),
            ("operands", strs(&ins.operands)),
            ("registers", Json::Arr(regs)),
            ("memory", Json::Arr(mem)),
        ]));
        self.history.push(Frame {
            definitions: self.definitions.clone(),
            next_activation,
            data: (!pure(&ins.op)).then_some(before),
            control,
            pure: pure(&ins.op) && fault.is_none(),
            reversed: false,
            marker: ins.op == "uncall" && fault.is_none(),
            measured: ins.op == "measure" && fault.is_none(),
        });
        self.steps += 1;
        Ok(())
    }
    fn exec(&mut self, ins: &Instr) -> Result<Option<u64>, String> {
        // Dynamic precondition failures are run faults; malformed/unsupported
        // execution remains a diagnostic. All fallible rows validate first.
        macro_rules! get {
            ($x:expr) => {
                match $x {
                    Ok(v) => v,
                    Err(c) => return Ok(Some(c)),
                }
            };
        }
        let xs = &ins.operands;
        let op = ins.op.as_str();
        let base = op.split('.').next().unwrap();
        let width = || -> Result<usize, String> {
            op.rsplit_once(".i")
                .ok_or_else(|| err("missing width"))?
                .1
                .parse()
                .map_err(err)
        };
        let mut result = None;
        match base {
            "const" => {
                let w = width()?;
                result = Some(Value::int(w, crate::oracle::parse_const(&xs[0], w)?));
            }
            "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr" | "udiv"
            | "urem" | "sdiv" | "srem" | "icmp" => {
                let a = get!(self.read(&xs[0]));
                let b = get!(self.bits(&xs[1]));
                let w = a.width(self.obj.ptr_width).unwrap();
                let bits = a.bits().unwrap();
                result = Some(if base == "icmp" {
                    Value::int(
                        1,
                        u(value::icmp(op.split('.').nth(1).unwrap(), bits, &b, w) as u64),
                    )
                } else if matches!(base, "udiv" | "urem" | "sdiv" | "srem") {
                    if let Some(c) = value::div_fault(base, bits, &b, w) {
                        return Ok(Some(c));
                    }
                    Value::int(w, value::divop(base, bits, &b, w))
                } else {
                    Value::int(w, value::binop(base, bits, &b, w))
                });
            }
            "select" => {
                let p = get!(self.bits(&xs[0]));
                result = Some(get!(self.read(&xs[if p.is_zero() { 2 } else { 1 }])));
            }
            "zext" | "sext" | "trunc" | "ptrtoint" | "inttoptr" => {
                let a = get!(self.read(&xs[0]));
                let bits = a.bits().unwrap();
                let w = if base == "inttoptr" {
                    self.obj.ptr_width
                } else {
                    width()?
                };
                let out = if base == "sext" {
                    sign_extend(bits, a.width(self.obj.ptr_width).unwrap(), w)
                } else {
                    bits.mask(w)
                };
                result = Some(if base == "inttoptr" {
                    Value::Ptr(out)
                } else {
                    Value::int(w, out)
                });
            }
            "load" | "store" => {
                let w = width()?;
                let p = get!(self.bits(&xs[0]));
                if !self.data.valid(&p, &u((w / 8) as u64)) {
                    return Ok(Some(7));
                }
                if base == "load" {
                    let mut v = BigUint::zero();
                    for (i, b) in self.data.read(&p, w / 8).iter().enumerate() {
                        v = v.bitor(&u(*b as u64).shl(i * 8));
                    }
                    result = Some(Value::int(w, v));
                } else {
                    let v = get!(self.bits(&xs[1]));
                    let bytes: Vec<_> = (0..w / 8)
                        .map(|i| v.shr(i * 8).mask(8).to_u64().unwrap() as u8)
                        .collect();
                    self.data.write(&p, &bytes);
                }
            }
            "alloc" => {
                let n = get!(self.bits(&xs[0]));
                let l = self
                    .obj
                    .layout
                    .as_ref()
                    .ok_or_else(|| err("alloc requires layout"))?;
                let advance = if n.is_zero() { u(1) } else { n.clone() };
                let end = self.data.bump.add(&advance);
                if end > l.heap_base.add(&l.heap_limit_max)
                    || u(self.data.table.len() as u64) >= u(1).shl(l.a_tab)
                {
                    return Ok(Some(2));
                }
                let p = self.data.bump.clone();
                self.data.bump = end;
                self.data.table.push(Allocation {
                    base: p.clone(),
                    size: n,
                    live: true,
                    pinned: false,
                });
                result = Some(Value::Ptr(p));
            }
            "free" => {
                let p = get!(self.bits(&xs[0]));
                let Some(e) = self
                    .data
                    .table
                    .iter_mut()
                    .find(|e| e.base == p && e.live && !e.pinned)
                else {
                    return Ok(Some(7));
                };
                e.live = false;
            }
            "trap" => return Ok(Some(4)),
            "br" => {
                let target = if xs.len() == 1 {
                    &xs[0]
                } else {
                    let p = get!(self.bits(&xs[0]));
                    &xs[if p.is_zero() { 2 } else { 1 }]
                };
                let (label, rest) = target
                    .trim_start_matches('^')
                    .split_once('(')
                    .ok_or_else(|| err("bad branch"))?;
                let args = crate::validate::list_operand(&format!("({rest}"))?;
                let values = get!(self.operands(&args));
                let a = self.stack.last().unwrap();
                let f = self.obj.function(&a.function).unwrap();
                let bi = f.blocks.iter().position(|b| b.name == label).unwrap();
                for (p, v) in f.blocks[bi].params.iter().zip(values) {
                    self.define(&p.name, v);
                }
                let a = self.stack.last_mut().unwrap();
                a.block = bi;
                a.index = 0;
            }
            "call" => {
                let name = xs[0].trim_start_matches('@');
                let f = self.obj.function(name).ok_or_else(|| {
                    format!("unimplemented: unresolved call @{name} (milestone 2 linking)")
                })?;
                let values = get!(self.operands(&xs[1..]));
                let id = self.next_activation;
                self.next_activation += 1;
                for (p, v) in f.lists[0].iter().zip(&values) {
                    self.define_at(id, &p.name, v.clone());
                }
                self.stack.push(Activation {
                    id,
                    function: name.into(),
                    block: 0,
                    index: 0,
                    dest: ins.lhs.clone(),
                    call_start: Some(self.history.len()),
                    args: values,
                });
            }
            "ret" => {
                let values = get!(self.operands(xs));
                let a = self.stack.pop().unwrap();
                if let Some(start) = a.call_start {
                    self.segments.push(Segment {
                        start,
                        end: self.history.len(),
                        name: a.function,
                        args: a.args,
                        results: values.clone(),
                    });
                    let caller = self.stack.last().unwrap().id;
                    for (n, v) in a.dest.iter().zip(values) {
                        self.define_at(caller, n, v);
                    }
                } else {
                    self.results = values;
                }
            }
            "uncall" => {
                let av = get!(self.operands(&crate::validate::list_operand(&xs[1])?));
                let rv = get!(self.operands(&crate::validate::list_operand(&xs[2])?));
                let Some(seg) = self
                    .segments
                    .iter()
                    .rev()
                    .find(|s| !self.history[s.start].reversed)
                else {
                    return Ok(Some(4));
                };
                if seg.name != xs[0].trim_start_matches('@')
                    || seg.args != av
                    || seg.results != rv
                    || self.history[seg.end + 1..].iter().any(|f| !f.pure)
                    || self.history[seg.start..=seg.end].iter().any(|f| f.measured)
                {
                    return Ok(Some(4));
                }
                let (start, end) = (seg.start, seg.end);
                for i in (start..=end).rev() {
                    self.undo(i, false);
                }
            }
            "effect" => {
                return self.effect(xs);
            }
            "hostcall" => {
                let p = get!(self.bits(&xs[1]));
                let n = get!(self.number(&xs[2]));
                if !self.data.valid(&p, &u(n)) {
                    return Ok(Some(7));
                }
                let size = usize::try_from(n).map_err(|_| "host limit: request size")?;
                let payload = self.data.read(&p, size);
                let reply = self
                    .replies
                    .get(self.reply_pos)
                    .ok_or("§10: no reply for HOST_REQUEST")?;
                if reply.get("service").and_then(Json::as_str) != Some(xs[0].as_str())
                    || reply.get("payload").and_then(Json::as_str) != Some(hex(&payload).as_str())
                {
                    return Err("§10: reply does not match service and payload".into());
                }
                let bytes = from_hex(
                    reply
                        .get("reply")
                        .and_then(Json::as_str)
                        .ok_or("§10: reply must be hex")?,
                )?;
                self.reply_pos += 1;
                self.data.write(&p, &bytes[..bytes.len().min(size)]);
                self.data.hosts.push(Json::obj(vec![
                    ("address", Json::u(&p)),
                    ("capacity", Json::u64(n)),
                    ("payload", Json::Str(hex(&payload))),
                    ("reply", Json::Str(hex(&bytes))),
                    ("service", Json::Str(xs[0].clone())),
                ]));
                result = Some(Value::int(self.obj.ptr_width, u(bytes.len() as u64)));
            }
            "qreg" => match op {
                "qreg.new" => {
                    let n: usize = xs[1].parse().map_err(err)?;
                    let ws = self.data.quantum.alloc_wires(n);
                    let id = self.data.new_handle(ws, None);
                    result = Some(Value::Qreg(id));
                }
                "qreg.free" => {
                    let id = get!(self.id(&xs[0]));
                    let h = self.data.handle(id).unwrap();
                    if h.parent.is_some() || !self.data.quantum.is_clean(&h.wires) {
                        return Ok(Some(4));
                    }
                    self.data.handles.get_mut(&id).unwrap().live = false;
                }
                "qreg.slice" => {
                    let id = get!(self.id(&xs[0]));
                    let h = self.data.handle(id).unwrap();
                    let lo: usize = xs[1].parse().map_err(err)?;
                    let n: usize = xs[2].parse().map_err(err)?;
                    if h.parent.is_some() || lo.checked_add(n).is_none_or(|end| end > h.wires.len())
                    {
                        return Ok(Some(4));
                    }
                    let ws = h.wires[lo..lo + n].to_vec();
                    self.data.handles.get_mut(&id).unwrap().viewed = true;
                    let v = self.data.new_handle(ws, Some(id));
                    result = Some(Value::Qreg(v));
                }
                "qreg.concat" => {
                    let a = get!(self.id(&xs[0]));
                    let b = get!(self.id(&xs[1]));
                    let ha = self.data.handle(a).unwrap();
                    let hb = self.data.handle(b).unwrap();
                    if a == b || ha.parent.is_some() || hb.parent.is_some() {
                        return Ok(Some(4));
                    }
                    let ws = ha.wires.iter().chain(&hb.wires).copied().collect();
                    self.data.handles.get_mut(&a).unwrap().live = false;
                    self.data.handles.get_mut(&b).unwrap().live = false;
                    let id = self.data.new_handle(ws, None);
                    result = Some(Value::Qreg(id));
                }
                _ => return Err(format!("unimplemented: {op} (milestone 2)")),
            },
            "measure" => {
                let id = get!(self.id(&xs[0]));
                let h = self.data.handle(id).unwrap();
                if h.parent.is_some() {
                    return Ok(Some(4));
                }
                let ws = h.wires.clone();
                let draw = *self.draws.get(self.draw_pos).unwrap_or(&0.0);
                self.draw_pos += 1;
                let (v, bits) = self.data.quantum.measure(&ws, draw);
                self.data.handles.get_mut(&id).unwrap().live = false;
                self.data.measurements.push(Json::obj(vec![
                    ("bits", Json::Str(bits)),
                    ("reg", Json::u64(id)),
                    ("value", Json::u(&v)),
                ]));
                result = Some(Value::int(ws.len(), v));
            }
            "action_call" | "oracle_call" | "adj" | "ctl" => self.region(ins)?,
            _ => return Err(format!("unimplemented: {op} (milestone 2)")),
        }
        if let Some(v) = result {
            self.define(&ins.lhs[0], v);
        }
        Ok(None)
    }
    fn effect(&mut self, xs: &[String]) -> Result<Option<u64>, String> {
        let (dev, op) = xs[0]
            .split_once('.')
            .ok_or_else(|| err("effect missing device"))?;
        let args = match xs[1..]
            .iter()
            .map(|n| self.number(n))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(v) => v,
            Err(c) => return Ok(Some(c)),
        };
        let (p, n) = match (dev, op, args.as_slice()) {
            ("console", "write", [p, n]) | ("entropy", "read", [p, n]) => (*p, *n),
            ("clock", "read", [p]) => (*p, 8),
            ("block", "read" | "write", [_, p]) => (*p, 512),
            _ => return Ok(Some(8)),
        };
        if !self.data.valid(&u(p), &u(n)) {
            return Ok(Some(7));
        }
        let n = usize::try_from(n).map_err(|_| "host limit: effect length")?;
        if dev == "entropy"
            && self
                .data
                .entropy_pos
                .checked_add(n)
                .is_none_or(|end| end > self.entropy.len())
        {
            return Ok(Some(10));
        }
        let bytes = match (dev, op) {
            ("console", "write") => {
                let bs = self.data.read(&u(p), n);
                self.data.console.extend(&bs);
                bs
            }
            ("clock", "read") => {
                let bs = self.data.clock.to_le_bytes().to_vec();
                self.data.clock = self.data.clock.wrapping_add(1);
                self.data.write(&u(p), &bs);
                bs
            }
            ("entropy", "read") => {
                let bs = self.entropy[self.data.entropy_pos..self.data.entropy_pos + n].to_vec();
                self.data.entropy_pos += n;
                self.data.write(&u(p), &bs);
                bs
            }
            ("block", "write") => {
                let bs = self.data.read(&u(p), n);
                self.data.block.insert(args[0], bs.clone());
                bs
            }
            ("block", "read") => {
                let bs = self
                    .data
                    .block
                    .get(&args[0])
                    .cloned()
                    .unwrap_or_else(|| vec![0; 512]);
                self.data.write(&u(p), &bs);
                bs
            }
            _ => unreachable!(),
        };
        self.data.tape.push(Json::obj(vec![
            (
                "args",
                Json::Arr(args.iter().map(|v| Json::u64(*v)).collect()),
            ),
            ("bytes", Json::Str(hex(&bytes))),
            ("dev", Json::Str(dev.into())),
            ("op", Json::Str(op.into())),
        ]));
        Ok(None)
    }
    fn region(&mut self, ins: &Instr) -> Result<(), String> {
        let mut xs = ins.operands.as_slice();
        let mut op = ins.op.as_str();
        let mut adj = false;
        let mut controls = vec![];
        if op == "ctl" {
            controls = self.wires(&xs[0])?;
            op = &xs[1];
            xs = &xs[2..];
        }
        if op == "adj" {
            adj = true;
            op = &xs[0];
            xs = &xs[1..];
        }
        let name = xs[0].trim_start_matches('@');
        let mut used = Vec::new();
        let mut bundles = |text: &str| -> Result<Vec<Vec<usize>>, String> {
            crate::validate::list_operand(text)?
                .iter()
                .map(|n| {
                    used.push(n.clone());
                    self.wires(n)
                })
                .collect()
        };
        if op == "action_call" {
            let muts = bundles(&xs[1])?;
            let inputs = bundles(&xs[2])?;
            let mem = bundles(&xs[3])?;
            let sig = crate::validate::action_sig(self.obj, name)?;
            let regions = mem
                .into_iter()
                .zip(&sig.mem)
                .map(|(ws, (a, w, _))| RegionWires {
                    cells: ws.chunks(*w).map(|s| s.to_vec()).collect(),
                    a: *a,
                    w: *w,
                })
                .collect::<Vec<_>>();
            let args = self
                .operands(&crate::validate::list_operand(&xs[4])?)
                .map_err(|_| err("invalid region argument"))?;
            // §9.4: borrowing needs the caller's *designated* idle set.
            // The 0.8 one-shot CLI has no designation operand; other live
            // handles are not an implicit permission to borrow their wires.
            let idle = Vec::new();
            action::execute_action_with_idle(
                self.obj,
                &mut self.data.quantum,
                name,
                &muts,
                &inputs,
                &regions,
                &args,
                adj,
                &controls,
                &idle,
            )?;
        } else if op == "oracle_call" {
            let inputs = bundles(&xs[2])?;
            let target = self.wires(&xs[3])?;
            let fault = self.wires(&xs[4])?;
            let mem = crate::validate::list_operand(&xs[5])?
                .iter()
                .map(|n| self.wires(n))
                .collect::<Result<Vec<_>, _>>()?;
            used.extend([xs[3].clone(), xs[4].clone()]);
            used.extend(crate::validate::list_operand(&xs[5])?);
            let budget = match self.read(&xs[1]).map_err(|_| err("missing token"))? {
                Value::Token(b) => b,
                _ => return Err(err("oracle call needs token")),
            };
            let args = self
                .operands(&crate::validate::list_operand(&xs[6])?)
                .map_err(|_| err("invalid region argument"))?;
            let sig = crate::validate::oracle_sig(self.obj, name)?;
            let mut next = BTreeMap::new();
            for (key, amp) in &self.data.quantum.amps {
                let mut nk = key.clone();
                if controls.iter().all(|w| key.bit(*w)) {
                    let values = inputs
                        .iter()
                        .map(|ws| QuantumState::extract(key, ws))
                        .collect::<Vec<_>>();
                    let mut cells = mem
                        .iter()
                        .zip(&sig.mem)
                        .map(|(ws, (_, w, _))| {
                            ws.chunks(*w)
                                .map(|cell| QuantumState::extract(key, cell))
                                .collect()
                        })
                        .collect::<Vec<Vec<_>>>();
                    let out = crate::oracle::run_oracle(
                        self.obj, name, &values, &mut cells, &args, budget,
                    )?;
                    action::xor_into(&mut nk, &target, &out.payload);
                    action::xor_into(&mut nk, &fault, &out.code);
                }
                let e = next.entry(nk).or_insert((0.0, 0.0));
                e.0 += amp.0;
                e.1 += amp.1;
            }
            self.data.quantum.amps = next;
        } else {
            return Err(err(format!("invalid region call {op}")));
        }
        for n in used {
            let id = self
                .id(&action::base_name(&n))
                .map_err(|_| err("invalid region handle"))?;
            if let Some(parent) = self.data.handles[&id].parent {
                self.data.handles.get_mut(&id).unwrap().live = false;
                self.data.handles.get_mut(&parent).unwrap().viewed = false;
            }
        }
        Ok(())
    }
}

pub fn cmd_run_cc(args: &[String], reverse: bool) -> Result<(), String> {
    command(args, if reverse { Mode::Reverse } else { Mode::Cc })
}
pub fn cmd_reverse(args: &[String]) -> Result<(), String> {
    command(args, Mode::Reverse)
}
pub fn cmd_program(args: &[String]) -> Result<(), String> {
    command(args, Mode::Program)
}
#[derive(PartialEq)]
enum Mode {
    Cc,
    Reverse,
    Program,
}
fn command(args: &[String], mode: Mode) -> Result<(), String> {
    let opts = cliargs::scan_opts(
        args,
        &[
            "args", "input", "arg", "entropy", "replies", "draws", "budget", "mem", "steps",
            "target",
        ],
        &["reverse"],
    )?;
    if opts.positional.len() != 2 {
        return Err("command requires <object> <entry>".into());
    }
    if opts.one("target")?.is_some() {
        return Err("unimplemented: target (§8 launch lowering) (milestone 4)".into());
    }
    let (obj, _) = crate::load(&cliargs::read_object(&opts.positional[0])?, false)?;
    if obj.has_schedule && mode == Mode::Program {
        return Err("unimplemented: schedule (§11 verbatim consumption) (milestone 4)".into());
    }
    let name = cliargs::entry_name(&opts.positional[1]);
    let f = obj
        .function(name)
        .ok_or_else(|| err(format!("unknown entry @{name}")))?;
    if !matches!(f.kind, FnKind::Cc | FnKind::R) {
        return Err(format!(
            "unimplemented: reverse {} (§3.6 / exact action reversal) (milestone 3)",
            f.kind.kind_str()
        ));
    }
    if mode == Mode::Program && f.kind != FnKind::Cc {
        return Err("§10: E_ARG: run-program requires a CC entry".into());
    }
    if mode != Mode::Program && opts.flags.contains("reverse") {
        return Err(err("--reverse is a run-program option"));
    }
    if mode != Mode::Reverse && opts.one("steps")?.is_some() {
        return Err(err("--steps is a reverse option"));
    }
    let budget = match opts.one("budget")? {
        Some(t) => cliargs::parse_budget(t, &obj)?,
        None => cliargs::default_budget(&obj, 1_000_000),
    };
    let mut data = Data::new(&obj)?;
    for raw in opts.all("mem") {
        let (n, h) = raw.split_once('=').ok_or("--mem requires global=hex")?;
        let g = obj
            .layout
            .as_ref()
            .and_then(|l| l.globals.get(n))
            .ok_or("§7.1: --mem names an unknown global")?;
        let bs = from_hex(h)?;
        if g.size != u(bs.len() as u64) {
            return Err("§7.1: --mem byte count differs from global size".into());
        }
        data.write(&g.base, &bs);
    }
    let forms = usize::from(opts.one("args")?.is_some())
        + usize::from(opts.one("input")?.is_some())
        + usize::from(!opts.all("arg").is_empty());
    if forms > 1 {
        return Err("§10: E_ARG: use one argument form".into());
    }
    let raw = if let Some(t) = opts.one("args")?.or(opts.one("input")?) {
        if t.is_empty() {
            vec![]
        } else {
            t.split(',').map(String::from).collect()
        }
    } else {
        opts.all("arg")
    };
    if raw.len() != f.lists[0].len() {
        return Err("§10: E_ARG: argument count differs from signature".into());
    }
    let mut values = Vec::new();
    let mut budgets = Vec::new();
    for (raw, p) in raw.iter().zip(&f.lists[0]) {
        let r = cliargs::parse_raw_arg(raw, &budget)?;
        let v = match (&p.ty, r) {
            (Type::I(n), cliargs::RawArg::Int(t)) => {
                Value::int(*n, cliargs::bind_int(&t, *n, &p.ty.signature_text())?)
            }
            (Type::Ptr, cliargs::RawArg::Int(t)) => {
                Value::Ptr(cliargs::bind_int(&t, obj.ptr_width, "ptr")?)
            }
            (Type::F64, cliargs::RawArg::F64(v)) => Value::F64(v),
            (Type::Token, cliargs::RawArg::Token(b)) => {
                cliargs::validate_budget(&obj, &b)?;
                budgets.push(b.to_json());
                Value::Token(b)
            }
            (Type::Qreg(n), cliargs::RawArg::QregBits(bits)) => {
                if bits.len() != *n || !bits.bytes().all(|b| b == b'0' || b == b'1') {
                    return Err("§10: E_ARG: qreg bits differ from signature".into());
                }
                let ws = data.quantum.alloc_wires(*n);
                let mut v = BigUint::zero();
                for (i, b) in bits.bytes().enumerate() {
                    v.set_bit(i, b == b'1');
                }
                data.quantum.prepare(&ws, &v);
                let id = data.new_handle(ws, None);
                Value::Qreg(id)
            }
            _ => return Err("§10: E_ARG: argument type differs from signature".into()),
        };
        values.push(v);
    }
    let entropy = from_hex(opts.one("entropy")?.unwrap_or(""))?;
    let draws = match opts.one("draws")? {
        None => vec![],
        Some(t) => t
            .split(',')
            .map(|d| d.parse::<f64>().map_err(|_| err("invalid draw")))
            .collect::<Result<Vec<_>, _>>()?,
    };
    if draws
        .iter()
        .any(|d| !d.is_finite() || *d < 0.0 || *d >= 1.0)
    {
        return Err("§14.2: measurement draws must be in [0,1)".into());
    }
    let replies = match opts.one("replies")? {
        None => vec![],
        Some(p) => match json::parse(&cliargs::read_object(p)?)? {
            Json::Arr(v) => v,
            _ => return Err("§10: replies must be a JSON array".into()),
        },
    };
    let env: BTreeMap<_, _> = f.lists[0]
        .iter()
        .zip(&values)
        .map(|(p, v)| ((0, p.name.clone()), vec![v.clone()]))
        .collect();
    let entry_data = data.clone();
    let entry_env = env.clone();
    let mut m = Machine {
        obj: &obj,
        data,
        env,
        stack: vec![Activation {
            id: 0,
            function: name.into(),
            block: 0,
            index: 0,
            dest: vec![],
            call_start: None,
            args: values,
        }],
        history: vec![],
        segments: vec![],
        trace: vec![],
        next_activation: 1,
        steps: 0,
        fault: None,
        results: vec![],
        entropy,
        draws,
        draw_pos: 0,
        replies,
        reply_pos: 0,
        definitions: vec![],
    };
    let entry_stack = m.stack.clone();
    let limit = opts
        .one("steps")?
        .map(|s| cliargs::parse_u64(s, "forward steps"))
        .transpose()?;
    while !m.stack.is_empty() && m.fault.is_none() {
        if limit.is_some_and(|n| m.steps >= n) {
            break;
        }
        if m.steps >= budget.steps {
            return Err("§3.4: execution fuel exhausted".into());
        }
        m.step()?;
    }
    if m.reply_pos != m.replies.len() {
        return Err("§10: unused host replies".into());
    }
    let mut out = vec![
        ("entry", Json::Str(name.into())),
        ("steps", Json::u64(m.steps)),
        ("budgets", Json::Arr(budgets)),
        ("results", vals(&m.results)),
        ("fault", m.fault.map(Json::u64).unwrap_or(Json::Null)),
        (
            "faults",
            Json::Arr(
                m.fault
                    .map(|c| {
                        Json::obj(vec![
                            ("code", Json::u64(c)),
                            (
                                "name",
                                value::fault_name(c)
                                    .map(|s| Json::Str(s.into()))
                                    .unwrap_or(Json::Null),
                            ),
                        ])
                    })
                    .into_iter()
                    .collect(),
            ),
        ),
        ("tape", Json::Arr(m.data.tape.clone())),
        ("host_requests", Json::Arr(m.data.hosts.clone())),
    ];
    let reversing = mode == Mode::Reverse || opts.flags.contains("reverse");
    if reversing {
        if mode == Mode::Program && m.fault.is_some() {
            return Err("§10: E_INVALID_STATE: cannot reverse a FAILED run".into());
        }
        if m.history.iter().any(|f| f.measured) {
            return Err("§10: E_NOT_REVERSIBLE: run contains measure".into());
        }
        for i in (0..m.history.len()).rev() {
            m.undo(i, true);
        }
    }
    let diff = first_difference(
        &entry_env,
        &m.env,
        &entry_data,
        &m.data,
        &entry_stack,
        &m.stack,
        m.next_activation,
    );
    out.extend(vec![
        (
            "console",
            String::from_utf8(m.data.console.clone())
                .ok()
                .map(Json::Str)
                .unwrap_or(Json::Null),
        ),
        ("console_hex", Json::Str(hex(&m.data.console))),
        ("clock", Json::u64(m.data.clock)),
        ("effect_tape_length", Json::u64(m.data.tape.len() as u64)),
        ("memory_digest", Json::Str(m.data.digest())),
    ]);
    if mode == Mode::Cc {
        out.extend(vec![
            (
                "status",
                Json::Str(
                    if m.fault.is_some() {
                        "FAILED"
                    } else {
                        "COMPLETED"
                    }
                    .into(),
                ),
            ),
            ("measurements", Json::Arr(m.data.measurements)),
            ("trace", Json::Arr(m.trace)),
        ]);
    } else {
        out.extend(vec![
            ("entry_memory_digest", Json::Str(entry_data.digest())),
            (
                "memory_equals_entry",
                Json::Bool(entry_data.memory_eq(&m.data)),
            ),
            ("equals_entry", Json::Bool(diff.is_none())),
            (
                "first_difference",
                diff.map(Json::Str).unwrap_or(Json::Null),
            ),
            (
                "status",
                Json::Str(
                    if reversing {
                        "REVERSED"
                    } else if m.fault.is_some() {
                        "FAILED"
                    } else {
                        "COMPLETED"
                    }
                    .into(),
                ),
            ),
        ]);
        if mode == Mode::Program {
            out.extend(vec![
                ("run_state", Json::Str("CLOSED".into())),
                ("reversed", Json::Bool(reversing)),
                ("measurements", Json::Arr(m.data.measurements)),
            ]);
        }
    }
    println!("{}", Json::obj(out).to_jcs());
    Ok(())
}
fn first_difference(
    a: &BTreeMap<Key, Vec<Value>>,
    b: &BTreeMap<Key, Vec<Value>>,
    da: &Data,
    db: &Data,
    entry_stack: &[Activation],
    stack: &[Activation],
    next_activation: u64,
) -> Option<String> {
    let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
    for k in keys {
        let (av, bv) = (current(a, k), current(b, k));
        if av != bv {
            let fmt = |v: Option<Value>| {
                v.map(|x| format!("Some({})", x.debug_shape()))
                    .unwrap_or("None".into())
            };
            return Some(format!(
                "instances of ({}, {:?}): {} differs from {}",
                k.0,
                k.1,
                fmt(av),
                fmt(bv)
            ));
        }
    }
    if entry_stack != stack || next_activation != 1 {
        return Some(format!(
            "control: {} activations / next_id 1 differs from {} activations / next_id {}",
            entry_stack.len(),
            stack.len(),
            next_activation
        ));
    }
    if !da.memory_eq(db) {
        return Some("classical memory differs".into());
    }
    if da.console != db.console
        || da.clock != db.clock
        || da.entropy_pos != db.entropy_pos
        || da.block != db.block
        || da.tape != db.tape
    {
        return Some("device state differs".into());
    }
    if da.handles != db.handles || da.quantum.amps != db.quantum.amps {
        return Some("quantum registers differ".into());
    }
    None
}
