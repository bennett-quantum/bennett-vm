//! Sequential CC/R execution with dynamic instances and retained LIFO history.
//! Histories store displaced data; segment reversal never re-executes a callee.
use crate::{
    action::{self, QuantumState, RegionWires},
    bigint::{BigUint, sign_extend},
    cliargs,
    history::History,
    json::{self, Json},
    object::{FnKind, Instr, Object, Type},
    persistent::{Map, Seq},
    sha256::{from_hex, sha256_hex, to_hex as hex},
    value::{self, Value},
};
use std::collections::{BTreeMap, BTreeSet};

/// Indices belong to an installed executable, after any private expansion.
/// Keeping them separate leaves the public, mutable object representation intact.
pub(crate) struct ExecutionIndex {
    pub functions: std::collections::HashMap<String, usize>,
    pub bodies: Vec<FunctionIndex>,
}
pub(crate) struct FunctionIndex {
    pub blocks: std::collections::HashMap<String, usize>,
    pub operands: Vec<Vec<Vec<String>>>,
}

impl ExecutionIndex {
    pub fn new(object: &Object) -> Self {
        Self {
            functions: object
                .functions
                .iter()
                .enumerate()
                .map(|(index, function)| (function.name.clone(), index))
                .collect(),
            bodies: object.functions.iter().map(FunctionIndex::new).collect(),
        }
    }
}
impl FunctionIndex {
    fn new(function: &crate::object::Function) -> Self {
        Self {
            blocks: function
                .blocks
                .iter()
                .enumerate()
                .map(|(index, block)| (block.name.clone(), index))
                .collect(),
            operands: function
                .blocks
                .iter()
                .map(|block| block.instrs.iter().map(resource_operands).collect())
                .collect(),
        }
    }
}

fn resource_operands(ins: &Instr) -> Vec<String> {
    ins.operands
        .iter()
        .flat_map(|text| {
            text.split(|c: char| match c {
                '_' | '.' | '%' => false,
                _ => !c.is_ascii_alphanumeric(),
            })
        })
        .map(str::to_owned)
        .collect()
}

type Key = (u64, String);
// Entry arguments use row zero. Executed definitions use their retained
// history row and position within that row, independent of later cancellation.
type InstanceId = (usize, usize);
type Environment = Map<Key, Map<InstanceId, Value>>;
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
    memory: Map<BigUint, u8>,
    table: Map<usize, Allocation>,
    allocation_index: Map<BigUint, usize>,
    interval_index: Map<BigUint, usize>,
    table_sorted: bool,
    bump: BigUint,
    console: Seq<u8>,
    clock: u64,
    entropy_pos: usize,
    block: Map<u64, Vec<u8>>,
    tape: Seq<Json>,
    hosts: Seq<Json>,
    measurements: Seq<Json>,
    handles: Map<u64, Handle>,
    next_handle: u64,
    tokens: Map<u64, (String, value::Budget)>,
    writes: BTreeSet<BigUint>,
}
#[derive(Clone, Debug, PartialEq)]
struct Activation {
    id: u64,
    function: String,
    function_index: usize,
    block: usize,
    index: usize,
    dest: Vec<String>,
    call_start: Option<usize>,
    args: Vec<Value>,
}
#[derive(Clone, Default)]
struct FrameEvents {
    tape: Vec<std::rc::Rc<Json>>,
    hosts: Vec<std::rc::Rc<Json>>,
    measurements: Vec<std::rc::Rc<Json>>,
}
#[cfg_attr(not(test), derive(Clone))]
struct Frame {
    classes_before: Map<usize, String>,
    classes_after: Map<usize, String>,
    wire_boundary: usize,
    region: Option<Invocation>,
    definitions: Vec<(Key, InstanceId)>,
    next_activation: u64,
    memory_keys: BTreeSet<BigUint>,
    data: Option<Data>,
    after: Option<Data>,
    control: Seq<Activation>,
    pure: bool,
    events: FrameEvents,
    marker: bool,
    measured: bool,
    tokens_created: Vec<u64>,
}
#[cfg(test)]
mod frame_work;
#[derive(Clone)]
struct Segment {
    start: usize,
    end: usize,
    name: String,
    args: Vec<Value>,
    results: Vec<Value>,
}
mod trace;
use trace::TraceLog;
#[derive(Clone)]
struct Machine {
    program: Option<std::sync::Arc<crate::jit::Program>>,
    indices: std::sync::Arc<ExecutionIndex>,
    quantum: QuantumState,
    quantum_events: Seq<Invocation>,
    pending_region: Option<Invocation>,
    obj: std::sync::Arc<Object>,
    data: Data,
    env: Environment,
    stack: Seq<Activation>,
    history: History<Frame, Segment>,
    trace: TraceLog,
    next_activation: u64,
    steps: u64,
    fault: Option<u64>,
    results: Vec<Value>,
    entropy: std::sync::Arc<Vec<u8>>,
    effects: EffectJournal,
    draws: Seq<f64>,
    draw_pos: usize,
    replies: Seq<Json>,
    reply_pos: usize,
    definitions: Vec<(Key, InstanceId)>,
    stateful: bool,
    register_changes: BTreeMap<Key, Option<Value>>,
    nan_registers: Map<Key, ()>,
    unavailable_handle: Option<u64>,
}
/// The execution journal is deliberately outside reversible device state: uncall
/// cancels the active tape, but replay must still consume those executed effects.
#[derive(Clone, Default)]
struct EffectJournal {
    saved: Seq<Json>,
    replay: Option<std::sync::Arc<Vec<Json>>>,
}
impl EffectJournal {
    fn read(path: Option<&str>) -> Result<Self, String> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let doc = json::parse(&cliargs::read_object(path)?)
            .map_err(|e| format!("§7.3/§10: replay tape: {e}"))?;
        let entries = doc
            .as_arr()
            .ok_or("§7.3/§10: replay tape must be an array")?;
        for (index, entry) in entries.iter().enumerate() {
            let valid = || -> Option<()> {
                let fields = entry.as_obj()?;
                if fields.len() != 4 {
                    return None;
                }
                entry.get("dev")?.as_str()?;
                entry.get("op")?.as_str()?;
                from_hex(entry.get("bytes")?.as_str()?).ok()?;
                for arg in entry.get("args")?.as_arr()? {
                    let value = arg.as_u64()?;
                    if *arg != Json::u64(value) {
                        return None;
                    }
                }
                Some(())
            };
            if valid().is_none() {
                return Err(format!(
                    "§7.3/§10: invalid replay tape entry {index} (dev/op/args/bytes encoding)"
                ));
            }
        }
        Ok(Self {
            saved: Seq::new(),
            replay: Some(std::sync::Arc::new(entries.clone())),
        })
    }
    fn append(&mut self, event: Json) -> Result<(), String> {
        if let Some(expected) = &self.replay
            && expected.get(self.saved.len()) != Some(&event)
        {
            return Err(format!(
                "§7.3/§10: replay mismatch at effect {}: expected {:?}, actual {}",
                self.saved.len(),
                expected.get(self.saved.len()),
                event.to_jcs()
            ));
        }
        self.saved.push(event);
        Ok(())
    }
    fn finish(&self) -> Result<(), String> {
        if let Some(expected) = &self.replay
            && expected.len() != self.saved.len()
        {
            return Err(format!(
                "§7.3/§10: replay mismatch: consumed {} of {} effects",
                self.saved.len(),
                expected.len()
            ));
        }
        Ok(())
    }
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
fn current(env: &Environment, k: &Key) -> Option<Value> {
    #[cfg(test)]
    REGISTER_READS.with(|n| n.set(n.get() + 1));
    env.get(k)
        .and_then(|v| v.last_key_value())
        .map(|(_, v)| v.clone())
}
fn is_nan(v: &Value) -> bool {
    matches!(v, Value::F64(f) if f.is_nan())
}
fn nan_keys(env: &Environment) -> Map<Key, ()> {
    env.iter()
        .filter(|(_, vs)| vs.last_key_value().is_some_and(|(_, v)| is_nan(v)))
        .map(|(k, _)| (k.clone(), ()))
        .collect()
}
/// `measure` and its `measure.iN` form (§3.3): every test of "is this a
/// measure frame" goes through the base opcode.
fn is_measure(op: &str) -> bool {
    op.split('.').next() == Some("measure")
}
fn pure(op: &str) -> bool {
    if matches!(op, "qreg.width" | "qreg.id" | "qreg.from_id") {
        return true;
    }
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
        let block = if let Some(name) = obj.layout.as_ref().and_then(|l| l.block_image.as_ref()) {
            crate::device::block_image(
                obj.data
                    .get(name)
                    .ok_or("§7.3: block image data symbol missing")?,
            )?
        } else {
            BTreeMap::new()
        };
        let mut d = Self {
            memory: Map::new(),
            table: Map::new(),
            allocation_index: Map::new(),
            interval_index: Map::new(),
            table_sorted: false,
            bump: BigUint::zero(),
            console: Seq::new(),
            clock: 0,
            entropy_pos: 0,
            block: block.into_iter().collect(),
            tape: Seq::new(),
            hosts: Seq::new(),
            measurements: Seq::new(),
            handles: Map::new(),
            next_handle: 1,
            tokens: Map::new(),
            writes: BTreeSet::new(),
        };
        if let Some(l) = &obj.layout {
            d.bump = l.heap_base.clone();
            for (name, g) in &l.globals {
                d.add_allocation(Allocation {
                    base: g.base.clone(),
                    size: g.size.clone(),
                    live: true,
                    pinned: true,
                });
                d.write(&g.base, &from_hex(&obj.data[name])?);
            }
        }
        Ok(d)
    }
    fn add_allocation(&mut self, entry: Allocation) {
        let index = self.table.floor(&usize::MAX).map_or(0, |(i, _)| i + 1);
        // Zero-sized globals may share a base. Preserve every table entry,
        // while the interval index selects the entry with maximal extent.
        if self
            .allocation_index
            .get(&entry.base)
            .is_none_or(|i| self.table[i].size <= entry.size)
        {
            self.allocation_index.insert(entry.base.clone(), index);
        }
        if !entry.size.is_zero() {
            self.interval_index.insert(entry.base.clone(), index);
        }
        self.table.insert(index, entry);
    }
    fn allocation_at_or_before(&self, p: &BigUint) -> Option<&Allocation> {
        let (_, index) = self.interval_index.floor(p)?;
        self.table.get(index)
    }
    fn ordered_allocations(&self) -> Vec<&Allocation> {
        let mut entries: Vec<_> = self.table.iter().map(|(_, e)| e).collect();
        if self.table_sorted {
            entries.sort_by(|a, b| a.base.cmp(&b.base));
        }
        entries
    }
    fn valid(&self, p: &BigUint, n: &BigUint) -> bool {
        self.allocation_at_or_before(p)
            .is_some_and(|e| e.live && p.add(n) <= e.base.add(&e.size))
            || (n.is_zero()
                && self
                    .allocation_index
                    .get(p)
                    .is_some_and(|i| self.table[i].live))
            || (n.is_zero()
                && !p.is_zero()
                && self
                    .allocation_at_or_before(&p.sub(&u(1)))
                    .is_some_and(|e| e.live && *p <= e.base.add(&e.size)))
    }
    /// The transfer buffer is host memory: a length no host can allocate is
    /// a host-limit diagnostic, never a panic.
    fn read(&self, p: &BigUint, n: usize) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(n)
            .map_err(|e| format!("host limit: §7.1 transfer of {n} bytes: {e}"))?;
        bytes.extend((0..n).map(|i| *self.memory.get(&p.add(&u(i as u64))).unwrap_or(&0)));
        Ok(bytes)
    }
    fn write(&mut self, p: &BigUint, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            let a = p.add(&u(i as u64));
            self.writes.insert(a.clone());
            if *b == 0 {
                self.memory.remove(&a);
            } else {
                self.memory.insert(a, *b);
            }
        }
    }
    fn digest(&self) -> String {
        // The spec fixes the memory state, but no digest serialization.
        let ordered = self.ordered_allocations();
        let entries = ordered
            .into_iter()
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
        self.bump == other.bump
            && self.ordered_allocations() == other.ordered_allocations()
            && self.memory == other.memory
    }
    fn new_handle(&mut self, wires: Vec<usize>, parent: Option<u64>) -> u64 {
        let id = self.next_handle;
        self.next_handle = self
            .next_handle
            .checked_add(1)
            .expect("host limit: register identities");
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
    fn mint_token(&mut self, name: String, budget: value::Budget) -> u64 {
        let id = value::fresh_identity();
        self.tokens.insert(id, (name, budget));
        id
    }
}
impl Machine {
    fn note_register(&mut self, key: &Key) {
        if !self.register_changes.contains_key(key) {
            self.register_changes
                .insert(key.clone(), current(&self.env, key));
        }
    }
    fn read(&self, name: &str) -> Result<Value, u64> {
        current(&self.env, &(self.stack.last().unwrap().id, name.into())).ok_or(4)
    }
    fn bits(&self, name: &str) -> Result<BigUint, u64> {
        self.read(name)?.bits().cloned().ok_or(4)
    }
    fn number(&self, name: &str) -> Result<u64, u64> {
        self.bits(name)?.to_u64().ok_or(4)
    }
    fn dimension(&self, name: &str) -> Result<BigUint, u64> {
        if crate::validate::is_int_literal(name) {
            BigUint::parse_int(name).map_err(|_| 4)
        } else {
            self.bits(name)
        }
    }
    fn token_budget(&self, token: &Value) -> Result<value::Budget, u64> {
        match token {
            Value::Token(budget) if self.obj.isa == "0.8" => Ok(*budget),
            Value::TokenId(id, budget)
                if self.data.tokens.get(id).is_some_and(|(_, b)| b == budget) =>
            {
                Ok(*budget)
            }
            _ => Err(4),
        }
    }
    fn transfer(&self, values: &[Value], params: &[crate::object::Param]) -> Result<(), u64> {
        if self.obj.isa != "0.9" {
            return Ok(());
        }
        for (value, param) in values.iter().zip(params) {
            if matches!(param.ty, Type::Qreg(_) | Type::QregDynamic) {
                let Value::Qreg(id) = value else {
                    return Err(4);
                };
                let handle = self.data.handle(*id)?;
                if handle.parent.is_some()
                    || matches!(param.ty, Type::Qreg(n) if n != handle.wires.len())
                {
                    return Err(4);
                }
            }
        }
        Ok(())
    }
    fn define_at(&mut self, id: u64, name: &str, v: Value) {
        let key = (id, name.into());
        if is_nan(&v) {
            self.nan_registers.insert(key.clone(), ());
        } else {
            self.nan_registers.remove(&key);
        }
        self.note_register(&key);
        if !self.env.contains_key(&key) {
            self.env.insert(key.clone(), Map::new());
        }
        let instance = (self.history.len() + 1, self.definitions.len());
        let vs = self.env.get_mut(&key).unwrap();
        self.definitions.push((key, instance));
        vs.insert(instance, v);
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
    fn wires(&self, n: &str) -> Result<Vec<usize>, u64> {
        if let Some((base, index)) = n.split_once('[') {
            let i: usize = index.trim_end_matches(']').parse().map_err(|_| 4u64)?;
            let ws = self.wires(base)?;
            return ws.get(i).map(|w| vec![*w]).ok_or(4);
        }
        let id = self.id(n)?;
        Ok(self.data.handle(id).unwrap().wires.clone())
    }
    fn undo(&mut self, index: usize, control: bool) -> Result<(), String> {
        if self.history.reversed(index) || self.history.get(index).marker {
            if control {
                self.stack = self.history.get(index).control.clone();
                self.next_activation = self.history.get(index).next_activation;
            }
            return Ok(());
        }
        #[cfg(test)]
        crate::history_work::record(crate::history_work::Operation::UndoFrame, 1);
        let keys: Vec<_> = self
            .history
            .get(index)
            .definitions
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            self.note_register(&key);
        }
        let frame = self.history.get(index);
        self.quantum.classical_event()?;
        if let Some(invocation) = &frame.region {
            let inverse = invocation.inverse();
            apply_invocation(&self.obj, &mut self.quantum, &inverse)?;
            record_quantum(&mut self.quantum_events, inverse);
        }
        if !frame.pure && !self.stateful {
            self.quantum.rewind_clean_wires(frame.wire_boundary)?;
        }
        for (key, instance) in frame.definitions.iter().rev() {
            let vs = self
                .env
                .get_mut(key)
                .expect("active defining frame has a register");
            assert!(
                vs.contains_key(instance),
                "active defining frame owns its instance"
            );
            vs.remove(instance);
            if vs.is_empty() {
                self.env.remove(key);
            }
        }
        for (key, _) in &frame.definitions {
            if self
                .env
                .get(key)
                .and_then(|vs| vs.last_key_value().map(|(_, v)| v))
                .is_some_and(is_nan)
            {
                self.nan_registers.insert(key.clone(), ());
            } else {
                self.nan_registers.remove(key);
            }
        }
        if let Some(d) = &frame.data {
            if let Some(after) = &frame.after {
                self.data.undo_delta(d, after, &frame.memory_keys);
            } else {
                let next_handle = self.data.next_handle;
                let mut writes = std::mem::take(&mut self.data.writes);
                writes.extend(frame.memory_keys.iter().cloned());
                self.data = d.clone();
                if self.obj.isa == "0.9" {
                    self.data.next_handle = self.data.next_handle.max(next_handle);
                }
                self.data.writes = writes;
            }
        }
        for key in frame.classes_before.changed_keys(&frame.classes_after) {
            if let Some(class) = frame.classes_before.get(&key) {
                self.quantum.wire_classes.insert(key, class.clone());
            } else {
                self.quantum.wire_classes.remove(&key);
            }
        }
        if control {
            self.stack = frame.control.clone();
            self.next_activation = frame.next_activation;
        }
        self.history.mark_reversed(index);
        Ok(())
    }
    fn step(&mut self) -> Result<(), String> {
        if let Some(p) = self.program.clone() {
            let a = self.stack.last().unwrap();
            let start = p.row_at(a.function_index, a.block, a.index)?;
            p.step(start, |ins| self.step_row(Some(ins)))
        } else {
            self.step_row(None)
        }
    }
    fn drive(&mut self, limit: Option<u64>, fuel: u64) -> Result<(), String> {
        let p = self.program.clone();
        if let Some(p) = p {
            let a = self.stack.last().unwrap();
            let start = p.row_at(a.function_index, a.block, a.index)?;
            p.run(start, |_, ins| {
                if limit.is_some_and(|n| self.steps >= n) {
                    return Ok(None);
                }
                if self.steps >= fuel {
                    return Err("§3.4: execution fuel exhausted".into());
                }
                self.step_row(Some(ins))?;
                if self.fault.is_some() {
                    return Ok(None);
                }
                self.stack
                    .last()
                    .map(|a| p.row_at(a.function_index, a.block, a.index))
                    .transpose()
            })
        } else {
            while !self.stack.is_empty() && self.fault.is_none() {
                if limit.is_some_and(|n| self.steps >= n) {
                    break;
                }
                if self.steps >= fuel {
                    return Err("§3.4: execution fuel exhausted".into());
                }
                self.step_row(None)?;
            }
            Ok(())
        }
    }
    fn step_row(&mut self, effective: Option<&Instr>) -> Result<(), String> {
        // Replay rejection is a diagnostic before this effect commits, including
        // its control advance and any target classical-step accounting.
        let activation = self.stack.last().unwrap();
        let row = effective.unwrap_or_else(|| {
            &self.obj.functions[activation.function_index].blocks[activation.block].instrs
                [activation.index]
        });
        let checkpoint =
            (self.effects.replay.is_some() && row.op == "effect").then(|| self.clone());
        let result = self.step_row_inner(effective);
        if result.is_err()
            && let Some(before) = checkpoint
        {
            before.history.restore();
            *self = before;
        }
        result
    }
    fn step_row_inner(&mut self, effective: Option<&Instr>) -> Result<(), String> {
        let a = self.stack.last().unwrap().clone();
        let obj = self.obj.clone();
        let f = &obj.functions[a.function_index];
        let block = &f.blocks[a.block];
        let ins = block
            .instrs
            .get(a.index)
            .ok_or_else(|| err("instruction pointer fell off block"))?
            .clone();
        let before = self.data.clone();
        let wire_boundary = self.quantum.wire_count();
        self.pending_region = None;
        let classes_before = self.quantum.wire_classes.clone();
        self.register_changes.clear();
        self.data.writes.clear();
        let control = self.stack.clone();
        let next_activation = self.next_activation;
        self.definitions.clear();
        self.stack.last_mut().unwrap().index += 1;
        self.quantum.classical_event()?;
        let fault = self.exec(effective.unwrap_or(&ins))?;
        if let Some(code) = fault {
            self.data = before.clone();
            self.fault = Some(code);
            self.results.clear();
        }
        let addresses = self.data.writes.clone();
        if !matches!(self.trace, TraceLog::Disabled) {
            self.trace
                .record(self.trace_row(&a, &before, &addresses), &obj);
        }
        let events = if self.stateful && !pure(&ins.op) && ins.op != "uncall" {
            FrameEvents {
                tape: self.data.tape.shared_from(before.tape.len()),
                hosts: self.data.hosts.shared_from(before.hosts.len()),
                measurements: self
                    .data
                    .measurements
                    .shared_from(before.measurements.len()),
            }
        } else {
            FrameEvents::default()
        };
        let frame_pure = pure(&ins.op) && fault.is_none();
        let measured = is_measure(&ins.op) && fault.is_none();
        self.history.push(
            Frame {
                classes_before,
                classes_after: self.quantum.wire_classes.clone(),
                wire_boundary,
                region: self.pending_region.take(),
                definitions: self.definitions.clone(),
                next_activation,
                memory_keys: addresses,
                tokens_created: before
                    .tokens
                    .changed_keys(&self.data.tokens)
                    .into_iter()
                    .filter(|id| {
                        !before.tokens.contains_key(id) && self.data.tokens.contains_key(id)
                    })
                    .collect(),
                data: (!pure(&ins.op)).then_some(before),
                after: self.stateful.then(|| self.data.clone()),
                control,
                pure: frame_pure,
                events,
                marker: ins.op == "uncall" && fault.is_none(),
                measured,
            },
            frame_pure,
            measured,
        );
        self.steps += 1;
        #[cfg(test)]
        crate::execution_work::record(crate::execution_work::Operation::Step, 1);
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
        let obj = self.obj.clone();
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
                    Value::int(w, value::binop(base, bits, &b, w)?)
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
                    for (i, b) in self.data.read(&p, w / 8)?.iter().enumerate() {
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
                self.data.add_allocation(Allocation {
                    base: p.clone(),
                    size: n,
                    live: true,
                    pinned: false,
                });
                result = Some(Value::Ptr(p));
            }
            "free" => {
                let p = get!(self.bits(&xs[0]));
                let index = self.data.allocation_index.get(&p).copied();
                let Some(e) = index
                    .and_then(|i| self.data.table.get_mut(&i))
                    .filter(|e| e.live && !e.pinned)
                else {
                    return Ok(Some(7));
                };
                e.live = false;
            }
            "trap" => return Ok(Some(4)),
            "fault" if obj.isa == "0.9" => {
                let code = cliargs::parse_u64(&xs[0], "fault cause")?;
                return Ok(Some(if code == 0 { 4 } else { code }));
            }
            "budget" if obj.isa == "0.9" => {
                let budget = value::Budget {
                    steps: get!(self.number(&xs[1])),
                    heap: get!(self.number(&xs[2])),
                    stack: get!(self.number(&xs[3])),
                    depth: get!(self.number(&xs[4])),
                };
                if cliargs::validate_budget(&obj, &budget).is_err() {
                    return Ok(Some(4));
                }
                let id = self
                    .data
                    .mint_token(xs[0].trim_start_matches('@').into(), budget);
                result = Some(Value::TokenId(id, budget));
            }
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
                let f = &obj.functions[a.function_index];
                let bi = self.indices.bodies[a.function_index].blocks[label];
                get!(self.transfer(&values, &f.blocks[bi].params));
                for (p, v) in f.blocks[bi].params.iter().zip(values) {
                    self.define(&p.name, v);
                }
                let a = self.stack.last_mut().unwrap();
                a.block = bi;
                a.index = 0;
            }
            "call" => {
                let name = xs[0].trim_start_matches('@');
                let function_index = *self
                    .indices
                    .functions
                    .get(name)
                    .ok_or_else(|| format!("unimplemented: unresolved call @{name}"))?;
                let f = &obj.functions[function_index];
                let values = get!(self.operands(&xs[1..]));
                get!(self.transfer(&values, &f.lists[0]));
                let id = self.next_activation;
                self.next_activation += 1;
                for (p, v) in f.lists[0].iter().zip(&values) {
                    self.define_at(id, &p.name, v.clone());
                }
                self.stack.push(Activation {
                    id,
                    function: name.into(),
                    function_index,
                    block: 0,
                    index: 0,
                    dest: ins.lhs.clone(),
                    call_start: Some(self.history.len()),
                    args: values,
                });
            }
            "ret" => {
                let values = get!(self.operands(xs));
                let function = &obj.functions[self.stack.last().unwrap().function_index];
                get!(self.transfer(&values, &function.lists[1]));
                let a = self.stack.pop().unwrap();
                if let Some(start) = a.call_start {
                    self.history.complete(
                        start,
                        self.history.len(),
                        Segment {
                            start,
                            end: self.history.len(),
                            name: a.function,
                            args: a.args,
                            results: values.clone(),
                        },
                    );
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
                let Some(completed) = self.history.latest_segment() else {
                    return Ok(Some(4));
                };
                let seg = &completed.value;
                if seg.name != xs[0].trim_start_matches('@')
                    || seg.args != av
                    || seg.results != rv
                    || self.history.impure_from(seg.end + 1)
                    || self.history.measured_in(seg.start, seg.end + 1)
                {
                    return Ok(Some(4));
                }
                let (start, end) = (seg.start, seg.end);
                for i in (start..=end).rev() {
                    self.undo(i, false)?;
                }
                self.history.retire(start);
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
                let payload = self.data.read(&p, size)?;
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
                    let ws = if obj.isa == "0.9" {
                        let n = get!(self.dimension(&xs[1]))
                            .to_usize()
                            .ok_or("host limit: quantum register width")?;
                        if self.quantum.target.is_none() {
                            self.quantum.try_alloc_wires(n)?
                        } else {
                            match self.quantum.checked_target_alloc(n, &xs[0]) {
                                Ok(wires) => wires,
                                Err(action::AllocationError::Capacity(_)) => return Ok(Some(2)),
                                Err(action::AllocationError::Host(message)) => return Err(message),
                            }
                        }
                    } else {
                        let n: usize = xs[1].parse().map_err(err)?;
                        self.quantum.target_alloc(n, &xs[0])?
                    };
                    let id = self.data.new_handle(ws, None);
                    result = Some(Value::Qreg(id));
                }
                "qreg.free" => {
                    let id = get!(self.id(&xs[0]));
                    let h = self.data.handle(id).unwrap();
                    if h.parent.is_some() || !self.quantum.is_clean(&h.wires) {
                        return Ok(Some(4));
                    }
                    self.quantum.target_release(&h.wires.clone());
                    self.data.handles.get_mut(&id).unwrap().live = false;
                }
                "qreg.slice" => {
                    let id = get!(self.id(&xs[0]));
                    let h = self.data.handle(id).unwrap();
                    let (lo, n) = if obj.isa == "0.9" {
                        let lo = get!(self.dimension(&xs[1]));
                        let n = get!(self.dimension(&xs[2]));
                        if lo.add(&n) > u(h.wires.len() as u64) {
                            return Ok(Some(4));
                        }
                        (lo.to_usize().unwrap(), n.to_usize().unwrap())
                    } else {
                        (xs[1].parse().map_err(err)?, xs[2].parse().map_err(err)?)
                    };
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
                "qreg.width" | "qreg.id" if obj.isa == "0.9" => {
                    let id = get!(self.id(&xs[0]));
                    let handle = self.data.handle(id).unwrap();
                    if handle.parent.is_some() {
                        return Ok(Some(4));
                    }
                    result = Some(Value::int(
                        64,
                        u(if op == "qreg.id" {
                            id
                        } else {
                            handle.wires.len() as u64
                        }),
                    ));
                }
                "qreg.from_id" if obj.isa == "0.9" => {
                    let id = get!(self.number(&xs[0]));
                    if self.unavailable_handle == Some(id) {
                        return Ok(Some(4));
                    }
                    let handle = get!(self.data.handle(id));
                    if handle.parent.is_some() {
                        return Ok(Some(4));
                    }
                    result = Some(Value::Qreg(id));
                }
                _ => return Err(format!("unimplemented: {op}")),
            },
            "measure" => {
                let id = get!(self.id(&xs[0]));
                let h = self.data.handle(id).unwrap();
                if h.parent.is_some() {
                    return Ok(Some(4));
                }
                let ws = h.wires.clone();
                if ins.op != "measure" {
                    let opcode = &ins.op;
                    let width = opcode
                        .strip_prefix("measure.i")
                        .and_then(|text| text.parse::<usize>().ok())
                        .filter(|width| *width != 0)
                        .ok_or_else(|| err("invalid typed measurement width"))?;
                    if width != ws.len() {
                        return Ok(Some(4));
                    }
                }
                let draw = *self.draws.get(self.draw_pos).unwrap_or(&0.0);
                self.draw_pos += 1;
                let handles = self.data.handles.values();
                let bundles = handles.filter(|h| h.parent.is_none()).map(|h| &h.wires[..]);
                self.quantum.state_output.capture(&self.quantum, bundles)?;
                let (v, bits) = self.quantum.measure(&ws, draw);
                self.quantum.target_release(&ws);
                self.data.handles.get_mut(&id).unwrap().live = false;
                self.data.measurements.push(Json::obj(vec![
                    ("bits", Json::Str(bits)),
                    ("reg", Json::u64(id)),
                    ("value", Json::u(&v)),
                ]));
                result = Some(Value::int(ws.len(), v));
            }
            "action_call" | "oracle_call" | "adj" | "ctl" => return self.region(ins),
            _ => return Err(format!("unimplemented: {op}")),
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
        // Form and validate the event from entry state before any device or
        // memory mutation. Entropy comes from the supplied entropy, never the tape.
        let bytes = match (dev, op) {
            ("console", "write") | ("block", "write") => self.data.read(&u(p), n)?,
            ("clock", "read") => self.data.clock.to_le_bytes().to_vec(),
            ("entropy", "read") => {
                self.entropy[self.data.entropy_pos..self.data.entropy_pos + n].to_vec()
            }
            ("block", "read") => self
                .data
                .block
                .get(&args[0])
                .cloned()
                .unwrap_or_else(|| vec![0; 512]),
            _ => unreachable!(),
        };
        let event = Json::obj(vec![
            (
                "args",
                Json::Arr(args.iter().map(|v| Json::u64(*v)).collect()),
            ),
            ("bytes", Json::Str(hex(&bytes))),
            ("dev", Json::Str(dev.into())),
            ("op", Json::Str(op.into())),
        ]);
        self.effects.append(event.clone())?;
        match (dev, op) {
            ("console", "write") => self.data.console.extend(bytes.iter().copied()),
            ("clock", "read") => {
                self.data.clock = self.data.clock.wrapping_add(1);
                self.data.write(&u(p), &bytes);
            }
            ("entropy", "read") => {
                self.data.entropy_pos += n;
                self.data.write(&u(p), &bytes);
            }
            ("block", "write") => {
                self.data.block.insert(args[0], bytes);
            }
            ("block", "read") => self.data.write(&u(p), &bytes),
            _ => unreachable!(),
        }
        self.data.tape.push(event);
        Ok(None)
    }
    /// A region call. `Ok(Some(code))` is the §3.3 CC trap / R fault, taken
    /// before any effect, including any view consumption.
    fn region(&mut self, ins: &Instr) -> Result<Option<u64>, String> {
        macro_rules! binding {
            ($value:expr) => {
                match $value {
                    Ok(value) => value,
                    Err(code) => return Ok(Some(code)),
                }
            };
        }
        let mut xs = ins.operands.as_slice();
        let mut op = ins.op.as_str();
        let mut adjoint = false;
        let mut controls = vec![];
        let mut used = vec![];
        if op == "ctl" {
            controls = binding!(self.wires(&xs[0]));
            used.push(xs[0].clone());
            if controls.len() != 1 {
                return Ok(Some(4));
            }
            op = &xs[1];
            xs = &xs[2..];
        }
        if op == "adj" {
            adjoint = true;
            op = &xs[0];
            xs = &xs[1..];
        }
        let name = xs[0].trim_start_matches('@').to_string();
        let mut bundles = |names: &[String]| -> Result<Vec<Vec<usize>>, u64> {
            names
                .iter()
                .map(|n| {
                    used.push(n.clone());
                    self.wires(n)
                })
                .collect()
        };
        let invocation = if op == "action_call" {
            let muts = binding!(bundles(&list(&xs[1])?));
            let inputs = binding!(bundles(&list(&xs[2])?));
            let mem = binding!(bundles(&list(&xs[3])?));
            let sig = crate::validate::action_sig(&self.obj, &name)?;
            let regions = mem
                .into_iter()
                .zip(&sig.mem)
                .map(|(ws, (a, w, _))| RegionWires {
                    cells: ws.chunks(*w).map(|s| s.to_vec()).collect(),
                    a: *a,
                    w: *w,
                })
                .collect::<Vec<_>>();
            if !action::action_binding_valid(&sig, &muts, &inputs, &regions, &controls) {
                return Ok(Some(4));
            }
            // 0.9 E17.7, and the reference at 0.8: a classical operand without a
            // current instance traps.
            let args = binding!(self.operands(&list(&xs[4])?));
            for arg in &args {
                if matches!(arg, Value::Token(_) | Value::TokenId(..)) {
                    binding!(self.token_budget(arg));
                }
            }
            Invocation::Action {
                name,
                muts,
                inputs,
                regions,
                args,
                adjoint,
                controls,
            }
        } else if op == "oracle_call" {
            let inputs = binding!(bundles(&list(&xs[2])?));
            let target = binding!(self.wires(&xs[3]));
            let fault = binding!(self.wires(&xs[4]));
            let mem_names = list(&xs[5])?;
            let mem = binding!(
                mem_names
                    .iter()
                    .map(|n| self.wires(n))
                    .collect::<Result<Vec<_>, _>>()
            );
            let sig = crate::validate::oracle_sig(&self.obj, &name)?;
            let regions = mem
                .iter()
                .zip(&sig.mem)
                .map(|(ws, (a, w, _))| RegionWires {
                    cells: ws.chunks(*w).map(|c| c.to_vec()).collect(),
                    a: *a,
                    w: *w,
                })
                .collect::<Vec<_>>();
            if !action::oracle_binding_valid(&sig, &inputs, &target, &fault, &regions, &controls) {
                return Ok(Some(4));
            }
            used.extend([xs[3].clone(), xs[4].clone()]);
            used.extend(mem_names);
            let budget = binding!(self.token_budget(&binding!(self.read(&xs[1]))));
            let args = binding!(self.operands(&list(&xs[6])?));
            for arg in &args {
                if matches!(arg, Value::Token(_) | Value::TokenId(..)) {
                    binding!(self.token_budget(arg));
                }
            }
            Invocation::Oracle {
                name,
                inputs,
                target,
                fault,
                mem,
                args,
                budget,
                controls,
            }
        } else {
            return Err(err(format!("invalid region call {op}")));
        };
        // Resolve every ledger transition while all operands are still live.
        // A view also used as a read-only control is consumed exactly once.
        let handles = used
            .iter()
            .map(|n| self.id(&action::base_name(n)))
            .collect::<Result<BTreeSet<_>, _>>();
        let handles = binding!(handles);
        apply_invocation(&self.obj, &mut self.quantum, &invocation)?;
        record_quantum(&mut self.quantum_events, invocation.clone());
        self.pending_region = Some(invocation);
        for id in handles {
            if let Some(parent) = self.data.handles[&id].parent {
                self.data.handles.get_mut(&id).unwrap().live = false;
                self.data.handles.get_mut(&parent).unwrap().viewed = false;
            }
        }
        Ok(None)
    }
}
fn list(text: &str) -> Result<Vec<String>, String> {
    crate::validate::list_operand(text)
}

/// A resolved invocation boundary: inverse execution never resolves names in
/// an activation which has since returned or reads changed classical inputs.
#[derive(Clone, Debug, PartialEq)]
enum Invocation {
    Action {
        name: String,
        muts: Vec<Vec<usize>>,
        inputs: Vec<Vec<usize>>,
        regions: Vec<RegionWires>,
        args: Vec<Value>,
        adjoint: bool,
        controls: Vec<usize>,
    },
    Oracle {
        name: String,
        inputs: Vec<Vec<usize>>,
        target: Vec<usize>,
        fault: Vec<usize>,
        mem: Vec<Vec<usize>>,
        args: Vec<Value>,
        budget: value::Budget,
        controls: Vec<usize>,
    },
}
impl Invocation {
    fn inverse(&self) -> Self {
        let mut inverse = self.clone();
        if let Self::Action { adjoint, .. } = &mut inverse {
            *adjoint = !*adjoint;
        }
        inverse
    }
}
fn record_quantum(events: &mut Seq<Invocation>, invocation: Invocation) {
    // An exact symbolic identity witness for snapshot comparison, independent
    // of roundoff in the Born-probability amplitude cache. No amplitudes are
    // stored or restored by a run-history frame.
    if events
        .last()
        .is_some_and(|last| *last == invocation.inverse())
    {
        events.truncate(events.len() - 1);
    } else {
        events.push(invocation);
    }
}
fn apply_invocation(
    obj: &Object,
    state: &mut QuantumState,
    invocation: &Invocation,
) -> Result<(), String> {
    match invocation {
        Invocation::Action {
            name,
            muts,
            inputs,
            regions,
            args,
            adjoint,
            controls,
        } => action::execute_action_inner(
            obj, state, name, muts, inputs, regions, args, *adjoint, controls,
        ),
        Invocation::Oracle {
            name,
            inputs,
            target,
            fault,
            mem,
            args,
            budget,
            controls,
        } => {
            let sig = crate::validate::oracle_sig(obj, name)?;
            let regions = mem
                .iter()
                .zip(&sig.mem)
                .map(|(ws, (a, w, _))| RegionWires {
                    cells: ws.chunks(*w).map(|cell| cell.to_vec()).collect(),
                    a: *a,
                    w: *w,
                })
                .collect::<Vec<_>>();
            action::execute_oracle_inner(
                obj, state, name, inputs, target, fault, &regions, args, *budget, controls,
            )
        }
    }
}

pub fn cmd_run_cc(args: &[String], reverse: bool) -> Result<(), String> {
    command(args, if reverse { Mode::Reverse } else { Mode::Cc })
}
pub fn cmd_reverse(args: &[String]) -> Result<(), String> {
    // Dispatch by entry kind before scanning tier-specific options.
    if args.len() >= 2 {
        let (obj, _) = crate::load(&cliargs::read_object(&args[0])?, false)?;
        if let Some(f) = obj.function(cliargs::entry_name(&args[1]))
            && !matches!(f.kind, FnKind::Cc | FnKind::R)
        {
            if matches!(f.kind, FnKind::Action | FnKind::PermAction) {
                return crate::qcli::command(args, true);
            }
            return crate::stepcli::reverse(args);
        }
    }
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
            "args",
            "input",
            "arg",
            "entropy",
            "replies",
            "draws",
            "budget",
            "token-aliases",
            "mem",
            "steps",
            "target",
            "tape-out",
            "state-out",
            "replay-tape",
        ],
        &["reverse"],
    )?;
    if opts.positional.len() != 2 {
        return Err("command requires <object> <entry>".into());
    }
    let state_output = cliargs::state_output(&opts)?;
    let tape_out = opts.one("tape-out")?;
    let effects = EffectJournal::read(opts.one("replay-tape")?)?;
    let (obj, _) = crate::load(&cliargs::read_object(&opts.positional[0])?, false)?;
    if obj.isa != "0.9" && opts.one("token-aliases")?.is_some() {
        return Err("unknown option --token-aliases".into());
    }
    if obj.has_schedule && mode == Mode::Program {
        let doc = crate::sched::object_schedule(&obj)?.unwrap();
        let ids = crate::identity::compute(&obj)?;
        crate::sched::validate_entries(&doc, &obj, &ids)?;
        return crate::sched::require_library(doc.get("bindings").and_then(|b| b.get("expansion")));
    }
    let target = opts
        .one("target")?
        .map(|path| crate::target::Target::parse(&cliargs::read_object(path)?))
        .transpose()?;
    if let Some(t) = &target {
        t.validate_object(&obj)?;
    }
    let name = cliargs::entry_name(&opts.positional[1]);
    let f = obj
        .function(name)
        .ok_or_else(|| err(format!("unknown entry @{name}")))?;
    if mode == Mode::Program && f.kind != FnKind::Cc {
        return Err("§10: E_ARG: run-program requires a CC entry".into());
    }
    if !matches!(f.kind, FnKind::Cc | FnKind::R) {
        return Err(format!(
            "unimplemented: reverse {} (§3.6 / exact action reversal)",
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
    let mut quantum = QuantumState::new();
    quantum.state_output = state_output;
    quantum.target = target.map(std::sync::Arc::new);
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
    let token_count = f.lists[0].iter().filter(|p| p.ty == Type::Token).count();
    let aliases = if obj.isa == "0.9" {
        cliargs::parse_token_aliases(opts.one("token-aliases")?, token_count)?
    } else {
        vec![]
    };
    let token_region = if obj.isa == "0.9" && token_count != 0 {
        Some(
            obj.functions
                .iter()
                .find(|f| f.kind == FnKind::Oracle)
                .ok_or("§10: E_ARG: tuple-only token requires a program oracle")?
                .name
                .clone(),
        )
    } else {
        None
    };
    let mut tokens = std::collections::BTreeMap::new();
    let mut token_index = 0;
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
                if let Some(region) = &token_region {
                    let label = aliases[token_index];
                    token_index += 1;
                    let id = if let Some((budget, id)) = tokens.get(&label) {
                        if *budget != b {
                            return Err("§10: E_ARG: token_aliases equates unequal tuples".into());
                        }
                        *id
                    } else {
                        let id = data.mint_token(region.clone(), b);
                        tokens.insert(label, (b, id));
                        id
                    };
                    Value::TokenId(id, b)
                } else {
                    Value::Token(b)
                }
            }
            (ty @ (Type::Qreg(_) | Type::QregDynamic), cliargs::RawArg::QregBits(bits))
                if obj.isa == "0.9" || matches!(ty, Type::Qreg(_)) =>
            {
                if matches!(ty, Type::Qreg(n) if bits.len() != *n)
                    || !bits.bytes().all(|b| b == b'0' || b == b'1')
                {
                    return Err("§10: E_ARG: qreg bits differ from signature".into());
                }
                let ws = quantum.target_alloc(bits.len(), "any")?;
                let mut v = BigUint::zero();
                for (i, b) in bits.bytes().enumerate() {
                    v.set_bit(i, b == b'1');
                }
                quantum.prepare(&ws, &v);
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
    let env: Map<_, _> = f.lists[0]
        .iter()
        .zip(&values)
        .map(|(p, v)| {
            (
                (0, p.name.clone()),
                std::iter::once(((0, 0), v.clone())).collect(),
            )
        })
        .collect();
    let entry_data = data.clone();
    let entry_env = env.clone();
    let indices = std::sync::Arc::new(ExecutionIndex::new(&obj));
    let function_index = indices.functions[name];
    let mut m = Machine {
        indices,
        program: crate::jit::for_object(&obj)?,
        quantum,
        quantum_events: Seq::new(),
        pending_region: None,
        obj: std::sync::Arc::new(obj.clone()),
        data,
        nan_registers: nan_keys(&env),
        env,
        stack: vec![Activation {
            id: 0,
            function: name.into(),
            function_index,
            block: 0,
            index: 0,
            dest: vec![],
            call_start: None,
            args: values,
        }]
        .into_iter()
        .collect(),
        history: History::new(),
        trace: TraceLog::cli(&mode)?,
        next_activation: 1,
        steps: 0,
        fault: None,
        results: vec![],
        entropy: std::sync::Arc::new(entropy),
        effects,
        draws: draws.into_iter().collect(),
        draw_pos: 0,
        replies: replies.into_iter().collect(),
        reply_pos: 0,
        definitions: vec![],
        stateful: false,
        register_changes: BTreeMap::new(),
        unavailable_handle: None,
    };
    let entry_stack = m.stack.clone();
    let limit = opts
        .one("steps")?
        .map(|s| cliargs::parse_u64(s, "forward steps"))
        .transpose()?;
    m.drive(limit, budget.steps)?;
    m.effects.finish()?;
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
        ("tape", Json::Arr(m.data.tape.to_vec())),
        ("host_requests", Json::Arr(m.data.hosts.to_vec())),
    ];
    let reversing = mode == Mode::Reverse || opts.flags.contains("reverse");
    if reversing {
        // 0.9 §10 K6: FAILED -> REVERSED. 0.8's eligibility is COMPLETED only.
        if mode == Mode::Program && m.fault.is_some() && obj.isa == "0.8" {
            return Err("§10: E_INVALID_STATE: cannot reverse a FAILED run".into());
        }
        if m.history.measured_in(0, m.history.len()) {
            return Err("§10: E_NOT_REVERSIBLE: run contains measure".into());
        }
        for i in (0..m.history.len()).rev() {
            m.undo(i, true)?;
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
        m.quantum_events.is_empty(),
        if obj.isa == "0.9" && !reversing {
            m.fault
        } else {
            None
        },
    );
    out.extend(vec![
        (
            "console",
            String::from_utf8(m.data.console.to_vec())
                .ok()
                .map(Json::Str)
                .unwrap_or(Json::Null),
        ),
        ("console_hex", Json::Str(hex(&m.data.console.to_vec()))),
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
            ("measurements", Json::Arr(m.data.measurements.to_vec())),
            ("trace", Json::Arr(m.trace.project(&m.obj))),
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
                ("measurements", Json::Arr(m.data.measurements.to_vec())),
            ]);
        }
    }
    if m.quantum.target.is_some() {
        out.push(("target_resources", m.quantum.target_accounting()?));
        out.push((
            "target_error",
            Json::Str(m.quantum.target_error().decimal()),
        ));
    }
    let output = Json::obj(out).to_jcs();
    if let Some(path) = tape_out {
        std::fs::write(
            path,
            format!("{}\n", Json::Arr(m.effects.saved.to_vec()).to_jcs()),
        )
        .map_err(|e| format!("§7.3/§10: tape output {path:?}: {e}"))?;
    }
    println!("{output}");
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn first_difference(
    a: &Environment,
    b: &Environment,
    da: &Data,
    db: &Data,
    entry_stack: &Seq<Activation>,
    stack: &Seq<Activation>,
    next_activation: u64,
    quantum_identity: bool,
    fault: Option<u64>,
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
    if let Some(code) = fault {
        return Some(format!("fault: None differs from Some({code})"));
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
    if da.handles != db.handles || !quantum_identity {
        return Some("quantum registers differ".into());
    }
    if da.tokens != db.tokens {
        return Some("token validity differs".into());
    }
    None
}

// Stateful launch adapter. Resources are exchanged only at instruction boundaries;
// history contains classical deltas and resolved inverse invocations, never Q snapshots.
impl Data {
    fn undo_delta(&mut self, before: &Self, after: &Self, memory_keys: &BTreeSet<BigUint>) {
        fn restore<K: Ord + Clone, V: PartialEq + Clone>(
            now: &mut Map<K, V>,
            before: &Map<K, V>,
            after: &Map<K, V>,
        ) {
            for k in before.changed_keys(after) {
                if let Some(v) = before.get(&k) {
                    now.insert(k, v.clone());
                } else {
                    now.remove(&k);
                }
            }
        }
        for k in memory_keys {
            if before.memory.get(k) != after.memory.get(k) {
                self.writes.insert(k.clone());
                if let Some(v) = before.memory.get(k) {
                    self.memory.insert(k.clone(), *v);
                } else {
                    self.memory.remove(k);
                }
            }
        }
        restore(&mut self.block, &before.block, &after.block);
        restore(&mut self.handles, &before.handles, &after.handles);
        restore(&mut self.tokens, &before.tokens, &after.tokens);
        if before.bump != after.bump {
            self.bump = before.bump.clone();
        }
        restore(&mut self.table, &before.table, &after.table);
        restore(
            &mut self.allocation_index,
            &before.allocation_index,
            &after.allocation_index,
        );
        restore(
            &mut self.interval_index,
            &before.interval_index,
            &after.interval_index,
        );
        // The launch adapter historically canonicalises table order on undo.
        self.table_sorted = true;
        if !before.console.same_root(&after.console) && before.console != after.console {
            self.console = before.console.clone();
        }
        if before.clock != after.clock {
            self.clock = before.clock;
        }
        if before.entropy_pos != after.entropy_pos {
            self.entropy_pos = before.entropy_pos;
        }
        if !before.tape.same_root(&after.tape) && before.tape != after.tape {
            self.tape = before.tape.clone();
        }
        if !before.hosts.same_root(&after.hosts) && before.hosts != after.hosts {
            self.hosts = before.hosts.clone();
        }
        if !before.measurements.same_root(&after.measurements)
            && before.measurements != after.measurements
        {
            self.measurements = before.measurements.clone();
        }
        // Handle IDs are never recycled across host-visible calls.
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ResourceKey {
    Allocator,
    Entry(BigUint),
    Memory(BigUint, BigUint),
    Device(String),
}
impl ResourceKey {
    #[cfg(test)]
    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Memory(a, b), Self::Memory(c, d)) => a < d && c < b,
            _ => self == other,
        }
    }
}
#[derive(Clone)]
pub(crate) struct Resources {
    indices: std::sync::Arc<ExecutionIndex>,
    data: Data,
    quantum: QuantumState,
}
impl Resources {
    pub(crate) fn token(&self, id: u64) -> Option<&(String, value::Budget)> {
        self.data.tokens.get(&id)
    }
    pub(crate) fn token_for_budget(&self, budget: &value::Budget) -> Option<u64> {
        self.data
            .tokens
            .iter()
            .find(|(_, (_, b))| b == budget)
            .map(|(id, _)| *id)
    }
    pub(crate) fn mint_token(&mut self, name: String, budget: value::Budget) -> u64 {
        self.data.mint_token(name, budget)
    }
    pub(crate) fn revoke_tokens(&mut self, tokens: &[u64]) {
        for id in tokens {
            self.data.tokens.remove(id);
        }
    }
    pub(crate) fn has_target(&self) -> bool {
        self.quantum.target.is_some()
    }
    pub(crate) fn new(obj: &Object) -> Result<Self, String> {
        Ok(Self {
            indices: std::sync::Arc::new(ExecutionIndex::new(obj)),
            data: Data::new(obj)?,
            quantum: QuantumState::new(),
        })
    }
    pub(crate) fn set_target(&mut self, target: crate::target::Target) {
        self.quantum.target = Some(std::sync::Arc::new(target));
    }
    pub(crate) fn new_reg(&mut self, n: usize, cls: &str) -> Result<u64, String> {
        let ws = self.quantum.target_alloc(n, cls)?;
        Ok(self.data.new_handle(ws, None))
    }
    pub(crate) fn reg_width(&self, id: u64) -> Option<usize> {
        self.data
            .handle(id)
            .ok()
            .filter(|h| h.parent.is_none())
            .map(|h| h.wires.len())
    }
    pub(crate) fn clean(&self, id: u64) -> bool {
        self.data
            .handle(id)
            .is_ok_and(|h| self.quantum.is_clean(&h.wires))
    }
    pub(crate) fn prepare(&mut self, id: u64, bits: &[u8]) {
        let mut v = BigUint::zero();
        for (i, b) in bits.iter().enumerate() {
            v.set_bit(i, *b != 0);
        }
        self.quantum.prepare(&self.data.handles[&id].wires, &v);
    }
    pub(crate) fn free(&mut self, id: u64) {
        self.quantum.target_release(&self.data.handles[&id].wires);
        self.data.handles.get_mut(&id).unwrap().live = false;
    }
}
#[derive(Clone)]
pub(crate) struct RuntimeRun {
    machine: Machine,
}
impl RuntimeRun {
    pub(crate) fn new(
        obj: std::sync::Arc<Object>,
        resources: &Resources,
        name: &str,
        args: Vec<Value>,
        entropy: Vec<u8>,
    ) -> Result<Self, String> {
        let program = crate::jit::for_object(&obj)?;
        let function_index = resources.indices.functions[name];
        let f = &obj.functions[function_index];
        let env = f.lists[0]
            .iter()
            .zip(&args)
            .map(|(p, v)| {
                (
                    (0, p.name.clone()),
                    std::iter::once(((0, 0), v.clone())).collect(),
                )
            })
            .collect();
        Ok(Self {
            machine: Machine {
                indices: resources.indices.clone(),
                program,
                quantum: resources.quantum.clone(),
                quantum_events: Seq::new(),
                pending_region: None,
                obj,
                data: resources.data.clone(),
                nan_registers: nan_keys(&env),
                env,
                stack: vec![Activation {
                    id: 0,
                    function: name.into(),
                    function_index,
                    block: 0,
                    index: 0,
                    dest: vec![],
                    call_start: None,
                    args,
                }]
                .into_iter()
                .collect(),
                history: History::new(),
                trace: TraceLog::new(false),
                next_activation: 1,
                steps: 0,
                fault: None,
                results: vec![],
                entropy: std::sync::Arc::new(entropy),
                effects: EffectJournal::default(),
                draws: Seq::new(),
                draw_pos: 0,
                replies: Seq::new(),
                reply_pos: 0,
                definitions: vec![],
                stateful: true,
                register_changes: BTreeMap::new(),
                unavailable_handle: None,
            },
        })
    }
    pub(crate) fn collect_trace(mut self, enabled: bool) -> Self {
        assert_eq!(
            self.machine.steps, 0,
            "trace configuration requires an unstarted run"
        );
        self.machine.trace = TraceLog::new(enabled);
        self
    }
    pub(crate) fn sync(&mut self, r: &Resources) {
        let local = &self.machine.data;
        let mut shared = r.data.clone();
        shared.entropy_pos = local.entropy_pos;
        shared.tape = local.tape.clone();
        shared.hosts = local.hosts.clone();
        shared.measurements = local.measurements.clone();
        self.machine.data = shared;
        self.machine.quantum = r.quantum.clone();
    }
    pub(crate) fn export(&self, r: &mut Resources) {
        r.data = self.machine.data.clone();
        r.data.entropy_pos = 0;
        r.data.tape.clear();
        r.data.hosts.clear();
        r.data.measurements.clear();
        r.quantum = self.machine.quantum.clone();
    }
    fn records(&self, field: fn(&FrameEvents) -> &[std::rc::Rc<Json>]) -> Vec<Json> {
        self.machine
            .history
            .iter()
            .enumerate()
            .filter(|(row, frame)| !self.machine.history.reversed(*row) && !frame.marker)
            .flat_map(|(_, frame)| {
                (0..field(&frame.events).len()).map(move |index| {
                    #[cfg(test)]
                    crate::history_work::record(crate::history_work::Operation::EventRef, 1);
                    field(&frame.events)[index].as_ref().clone()
                })
            })
            .collect()
    }
    pub(crate) fn created_regs(&self) -> Result<Vec<u64>, String> {
        let Some(frame) = self.machine.history.last() else {
            return Ok(vec![]);
        };
        match (&frame.data, &frame.after) {
            // Pure rows deliberately omit the before snapshot, including
            // stateful runs that still retain an after snapshot.
            (None, _) => Ok(vec![]),
            (Some(before), Some(after)) => Ok(before
                .handles
                .changed_keys(&after.handles)
                .into_iter()
                .filter(|q| !before.handles.contains_key(q) && after.handles.contains_key(q))
                .collect()),
            _ => Err("resource tracking requires both snapshots of the last history frame".into()),
        }
    }
    pub(crate) fn terminal(&self) -> bool {
        self.machine.fault.is_some() || self.machine.stack.is_empty()
    }
    pub(crate) fn fault(&self) -> Option<u64> {
        self.machine.fault
    }
    pub(crate) fn values(&self) -> &[Value] {
        &self.machine.results
    }
    pub(crate) fn minted_tokens(&self) -> Vec<u64> {
        self.machine
            .history
            .iter()
            .flat_map(|f| f.tokens_created.clone())
            .collect()
    }
    pub(crate) fn unavailable_handle(&mut self, id: Option<u64>) {
        self.machine.unavailable_handle = id;
    }
    pub(crate) fn pending_register_lookup(&self) -> Option<u64> {
        let m = &self.machine;
        let a = m.stack.last()?;
        let instruction = &m.obj.functions[a.function_index].blocks[a.block].instrs[a.index];
        (instruction.op == "qreg.from_id")
            .then(|| m.number(&instruction.operands[0]).ok())
            .flatten()
    }
    pub(crate) fn measured(&self) -> bool {
        self.machine
            .history
            .measured_in(0, self.machine.history.len())
    }
    pub(crate) fn tape(&self) -> Vec<Json> {
        self.records(|events| &events.tape)
    }
    pub(crate) fn measurements(&self) -> Vec<Json> {
        self.records(|events| &events.measurements)
    }
    pub(crate) fn history_len(&self) -> usize {
        self.machine.history.len()
    }
    // Locks are acquired before the instruction. Classical memory/devices share
    // an ordering domain; qreg operands lock only their actual handles.
    pub(crate) fn touches(&self) -> Result<(Vec<ResourceKey>, Vec<u64>), String> {
        let m = &self.machine;
        let Some(a) = m.stack.last() else {
            return Ok((vec![], vec![]));
        };
        let i = &m.obj.functions[a.function_index].blocks[a.block].instrs[a.index];
        let mut memory = vec![];
        let bits = |operand: &str| {
            m.bits(operand).map_err(|code| {
                format!("resource operand {operand:?} is unreadable (fault {code})")
            })
        };
        let mut range = |p: &str, n: BigUint| -> Result<(), String> {
            let p = bits(p)?;
            memory.push(ResourceKey::Memory(p.clone(), p.add(&n)));
            Ok(())
        };
        match i.op.split('.').next().unwrap() {
            "alloc" => {
                let n = bits(&i.operands[0])?;
                memory.push(ResourceKey::Allocator);
                memory.push(ResourceKey::Memory(
                    m.data.bump.clone(),
                    m.data.bump.add(&if n.is_zero() { u(1) } else { n }),
                ));
            }
            "free" => {
                let p = bits(&i.operands[0])?;
                if let Some(e) = m
                    .data
                    .allocation_index
                    .get(&p)
                    .and_then(|i| m.data.table.get(i))
                {
                    memory.push(ResourceKey::Memory(p.clone(), p.add(&e.size)));
                    memory.push(ResourceKey::Entry(p));
                }
            }
            "load" | "store" => {
                let w: usize = i.op.rsplit_once(".i").unwrap().1.parse().unwrap();
                range(&i.operands[0], u((w / 8) as u64))?;
            }
            "hostcall" => range(&i.operands[1], bits(&i.operands[2])?)?,
            "effect" => {
                let (dev, op) = i.operands[0].split_once('.').unwrap();
                let (p, n) = match (dev, op) {
                    ("clock", _) => (&i.operands[1], u(8)),
                    ("block", _) => (&i.operands[2], u(512)),
                    _ => (
                        &i.operands[1],
                        bits(
                            i.operands
                                .get(2)
                                .ok_or("resource transfer is missing its length operand")?,
                        )?,
                    ),
                };
                range(p, n)?;
                memory.push(ResourceKey::Device(dev.into()));
            }
            _ => {}
        }
        let mut regs = vec![];
        if i.op == "qreg.from_id"
            && let Ok(id) = m.number(&i.operands[0])
            && m.unavailable_handle != Some(id)
            && m.data.handle(id).is_ok_and(|h| h.parent.is_none())
        {
            regs.push(id);
        }
        for name in &m.indices.bodies[a.function_index].operands[a.block][a.index] {
            if let Ok(Value::Qreg(id)) = m.read(name) {
                regs.push(id);
                if let Some(parent) = m.data.handles.get(&id).and_then(|h| h.parent) {
                    regs.push(parent);
                }
            }
        }
        let entries = memory
            .iter()
            .filter_map(|key| {
                if let ResourceKey::Memory(p, end) = key {
                    m.data
                        .allocation_at_or_before(p)
                        .filter(|e| e.live && *end <= e.base.add(&e.size))
                        .or_else(|| {
                            if p == end {
                                m.data
                                    .allocation_index
                                    .get(p)
                                    .and_then(|i| m.data.table.get(i))
                                    .filter(|e| e.live)
                            } else {
                                None
                            }
                        })
                        .map(|e| ResourceKey::Entry(e.base.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        memory.extend(entries);
        Ok((memory, regs))
    }
    pub(crate) fn request(&self) -> Option<(String, Vec<u8>)> {
        let m = &self.machine;
        let a = m.stack.last()?;
        let i = m.obj.functions[a.function_index].blocks[a.block]
            .instrs
            .get(a.index)?;
        if i.op != "hostcall" || m.reply_pos < m.replies.len() {
            return None;
        }
        let p = m.bits(&i.operands[1]).ok()?;
        let n = m.number(&i.operands[2]).ok()?;
        if !m.data.valid(&p, &u(n)) {
            return None;
        }
        // An unallocatable payload is not a request: executing the row reports it.
        Some((
            i.operands[0].clone(),
            m.data.read(&p, usize::try_from(n).ok()?).ok()?,
        ))
    }
    pub(crate) fn reply(&mut self, bytes: &[u8]) -> Result<(), String> {
        let (service, payload) = self.request().ok_or("§10: no HOST_REQUEST")?;
        self.machine.replies.push(Json::obj(vec![
            ("service", Json::Str(service)),
            ("payload", Json::Str(hex(&payload))),
            ("reply", Json::Str(hex(bytes))),
        ]));
        Ok(())
    }
    pub(crate) fn step(&mut self) -> Result<(), String> {
        let m = &mut self.machine;
        if let Some(a) = m.stack.last()
            && is_measure(&m.obj.functions[a.function_index].blocks[a.block].instrs[a.index].op)
        {
            use std::io::Read;
            let mut bs = [0u8; 8];
            std::fs::File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut bs))
                .map_err(|e| format!("§14.2: measurement randomness unavailable: {e}"))?;
            m.draws
                .push((u64::from_le_bytes(bs) >> 11) as f64 / ((1u64 << 53) as f64));
        }
        let checkpoint = self.machine.clone();
        if let Err(e) = self.machine.step() {
            checkpoint.history.restore();
            self.machine = checkpoint;
            return Err(e);
        }
        Ok(())
    }
    pub(crate) fn reverse_to(&mut self, n: usize) -> Result<(), String> {
        if n > self.machine.history.len() {
            return Err("§7.4: history index beyond current prefix".into());
        }
        if self
            .machine
            .history
            .measured_in(n, self.machine.history.len())
        {
            return Err("§10: E_NOT_REVERSIBLE: measure".into());
        }
        // Reject a cut through a retired segment: its control checkpoint refers
        // to instances already removed by uncall. Whole-prefix reversal is valid.
        if n != 0 && self.machine.history.cuts_cancelled_segment(n) {
            return Err("§7.4: snapshot lies inside cancelled segment".into());
        }
        for i in (n..self.machine.history.len()).rev() {
            self.machine.undo(i, true)?;
        }
        self.machine.history.truncate(n);
        self.machine.trace.truncate(n);
        self.machine.steps = n as u64;
        self.machine.fault = None;
        self.machine.results.clear();
        Ok(())
    }
    pub(crate) fn snapshot(&self, state: &str) -> Result<Json, String> {
        let mut out = vec![
            ("state", Json::Str(state.into())),
            (
                "fault",
                self.machine.fault.map(Json::u64).unwrap_or(Json::Null),
            ),
            (
                "instances",
                Json::Arr(
                    self.machine
                        .env
                        .iter()
                        .map(|((id, name), values)| {
                            Json::obj(vec![
                                ("activation", Json::u64(*id)),
                                ("name", Json::Str(name.clone())),
                                (
                                    "values",
                                    Json::Arr(values.iter().map(|(_, v)| v.to_json()).collect()),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "frames",
                Json::Arr(
                    self.machine
                        .history
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            Json::obj(vec![
                                ("index", Json::u64(i as u64)),
                                ("reversed", Json::Bool(self.machine.history.reversed(i))),
                                ("uncall", Json::Bool(f.marker)),
                                ("pure", Json::Bool(f.pure)),
                                ("measured", Json::Bool(f.measured)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "console_hex",
                Json::Str(hex(&self.machine.data.console.to_vec())),
            ),
            ("clock", Json::u64(self.machine.data.clock)),
            (
                "entropy_position",
                Json::u64(self.machine.data.entropy_pos as u64),
            ),
            ("history_length", Json::u64(self.history_len() as u64)),
            ("memory_digest", Json::Str(self.machine.data.digest())),
            (
                "trace",
                Json::Arr(self.machine.trace.project(&self.machine.obj)),
            ),
            ("tape", Json::Arr(self.tape())),
            (
                "host_requests",
                Json::Arr(self.records(|events| &events.hosts)),
            ),
        ];
        if self.machine.quantum.target.is_some() {
            out.push((
                "target_resources",
                self.machine.quantum.target_accounting()?,
            ));
            out.push((
                "target_error",
                Json::Str(self.machine.quantum.target_error().decimal()),
            ));
        }
        Ok(Json::obj(out))
    }
}

#[cfg(test)]
thread_local! {
    static REGISTER_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod scaling_tests {
    use super::*;

    fn region_object(functions: &str) -> std::sync::Arc<Object> {
        let source = format!(
            "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=64\n\nsection symbols\n{{}}\n\nsection functions\n{functions}\n"
        );
        let mut obj = crate::object::parse(&source).unwrap();
        for (index, f) in obj.functions.clone().iter().enumerate() {
            obj.symbols.insert(
                f.name.clone(),
                crate::object::Symbol {
                    index,
                    name: f.name.clone(),
                    kind: f.kind.kind_str().into(),
                    signature: Json::Null,
                    stored_hash: "0".repeat(64),
                },
            );
        }
        for f in obj.functions.clone() {
            let signature = crate::validate::derived_signature(&obj, &f.name).unwrap();
            obj.symbols.get_mut(&f.name).unwrap().signature = signature;
        }
        std::sync::Arc::new(obj)
    }

    #[test]
    fn region_alias_fault_preserves_quantum_and_view_ledger_then_reverses() {
        for tier in ["cc", "r"] {
            for call in [
                "action_call @a (x) (y) () ()",
                "ctl y action_call @a (x) () () ()",
            ] {
                let action = if call.starts_with("ctl") {
                    "action @a (t:wire[1] mut) () () () ()\n^entry():\nx t"
                } else {
                    "action @a (t:wire[1] mut) (p:wire[1] in) () () ()\n^entry():\nx t"
                };
                let object = region_object(&format!(
                    "{action}\nfn {tier} @main (x:qreg[*], y:qreg[*]) () () () ()\n^entry():\n{call}\nret"
                ));
                let mut resources = Resources::new(&object).unwrap();
                let parent = resources.new_reg(2, "any").unwrap();
                resources.prepare(parent, &[1, 0]);
                let ws = resources.data.handles[&parent].wires[..1].to_vec();
                let view = resources.data.new_handle(ws, Some(parent));
                resources.data.handles.get_mut(&parent).unwrap().viewed = true;
                let mut run = RuntimeRun::new(
                    object,
                    &resources,
                    "main",
                    vec![Value::Qreg(view), Value::Qreg(view)],
                    vec![],
                )
                .unwrap();
                let quantum = run.machine.quantum.clone();
                let handles = run.machine.data.handles.clone();
                run.step().unwrap();
                assert_eq!(run.fault(), Some(4), "{tier}/{call}");
                assert!(run.machine.quantum.exact_equals(&quantum));
                assert_eq!(run.machine.data.handles, handles);
                assert!(run.machine.quantum_events.is_empty());
                assert!(run.machine.history.get(0).region.is_none());
                run.reverse_to(0).unwrap();
                assert_eq!(run.fault(), None);
                assert_eq!(run.machine.data.handles, handles);
                assert!(run.machine.quantum.exact_equals(&quantum));
            }
        }
    }

    #[test]
    fn oracle_entry_rejects_aliases_and_widths_before_mapping() {
        let object = region_object(
            "oracle @echo (v:wire[4] in) (t:wire[4]) (fr:wire[4]) () ()\n^entry():\nlift.retval v",
        );
        for (inputs, target, fault, pred) in [
            (
                vec![vec![0, 1, 2, 3]],
                vec![0, 1, 2, 3],
                vec![8, 9, 10, 11],
                vec![],
            ),
            (
                vec![vec![0, 1, 2, 3]],
                vec![4, 5, 6, 7],
                vec![4, 5, 6, 7],
                vec![],
            ),
            (
                vec![vec![0, 1, 2, 3]],
                vec![4, 5, 6, 7],
                vec![8, 9, 10, 11],
                vec![4],
            ),
            (
                vec![vec![0, 1, 2, 3]],
                vec![4, 5, 6],
                vec![8, 9, 10, 11],
                vec![],
            ),
            (
                vec![vec![0, 1, 2, 2]],
                vec![4, 5, 6, 7],
                vec![8, 9, 10, 11],
                vec![],
            ),
        ] {
            let mut state = QuantumState::new();
            state.alloc_wires(12);
            state.prepare(&[0, 1, 2, 3], &u(1));
            let before = state.clone();
            assert!(
                action::execute_oracle_inner(
                    &object,
                    &mut state,
                    "echo",
                    &inputs,
                    &target,
                    &fault,
                    &[],
                    &[],
                    value::Budget {
                        steps: 3,
                        ..Default::default()
                    },
                    &pred
                )
                .is_err()
            );
            assert!(state.exact_equals(&before));
        }
    }

    #[test]
    fn region_width_and_viewed_alias_preconditions_are_atomic_faults() {
        for viewed in [false, true] {
            let object = region_object(
                "action @a (t:wire[1] mut) () () () ()\n^entry():\nx t\nfn cc @main (x:qreg[*]) () () () ()\n^entry():\naction_call @a (x) () () ()\nret",
            );
            let mut resources = Resources::new(&object).unwrap();
            let id = resources.new_reg(2, "any").unwrap();
            if viewed {
                resources.data.handles.get_mut(&id).unwrap().viewed = true;
            }
            let mut run =
                RuntimeRun::new(object, &resources, "main", vec![Value::Qreg(id)], vec![]).unwrap();
            let before = run.machine.quantum.clone();
            let handles = run.machine.data.handles.clone();
            run.step().unwrap();
            assert_eq!(run.fault(), Some(4));
            assert!(run.machine.quantum.exact_equals(&before));
            assert_eq!(run.machine.data.handles, handles);
            run.reverse_to(0).unwrap();
            assert_eq!(run.fault(), None);
        }
    }

    #[test]
    fn region_readonly_control_view_is_consumed_once_and_restored_by_reverse() {
        let object = region_object(
            "action @a (t:wire[1] mut) (p:wire[1] in) () () ()\n^entry():\ncx p t\nfn cc @main (x:qreg[*], y:qreg[*]) () () () ()\n^entry():\nctl y action_call @a (x) (y) () ()\nret",
        );
        let mut resources = Resources::new(&object).unwrap();
        let parent = resources.new_reg(2, "any").unwrap();
        let target = resources.new_reg(1, "any").unwrap();
        resources.prepare(parent, &[1, 0]);
        let ws = resources.data.handles[&parent].wires[..1].to_vec();
        let view = resources.data.new_handle(ws, Some(parent));
        resources.data.handles.get_mut(&parent).unwrap().viewed = true;
        let mut run = RuntimeRun::new(
            object,
            &resources,
            "main",
            vec![Value::Qreg(target), Value::Qreg(view)],
            vec![],
        )
        .unwrap();
        let before = run.machine.quantum.clone();
        let handles = run.machine.data.handles.clone();
        run.step().unwrap();
        assert_eq!(run.fault(), None);
        assert!(!run.machine.quantum.exact_equals(&before));
        assert!(!run.machine.data.handles[&view].live);
        assert!(!run.machine.data.handles[&parent].viewed);
        run.reverse_to(0).unwrap();
        assert_eq!(run.machine.data.handles, handles);
        assert!(run.machine.quantum.exact_equals(&before));
    }

    fn run_for(body: &str) -> RuntimeRun {
        run_for_version(body, "0.8")
    }

    fn run_for_version(body: &str, version: &str) -> RuntimeRun {
        let source = format!(
            "bennett-object isa={version} object={version} launch={version} sched={version} cert={version} coverage={version} target={version} ptr_width=64\n\nsection symbols\n{{}}\n\nsection functions\nfn cc @main () () () () ()\n^entry():\n{body}\n"
        );
        let obj = std::sync::Arc::new(crate::object::parse(&source).unwrap());
        let resources = Resources::new(&obj).unwrap();
        RuntimeRun::new(obj, &resources, "main", vec![], vec![]).unwrap()
    }

    #[test]
    fn launch_measure_with_width_suffix_draws_like_measure() {
        // Runtime::step supplies the draw for both spellings.
        for op in ["measure", "measure.i1"] {
            let mut run = run_for(&format!("q = qreg.new any 1\nr = {op} q\nret r"));
            run.step().unwrap();
            run.step().unwrap();
            assert_eq!(run.machine.draws.len(), 1, "{op}");
            assert!(run.measured(), "{op}");
        }
    }

    #[test]
    fn typed_measurement_launch_consumes_draw_and_refuses_reverse_atomically() {
        let mut run = run_for_version("q = qreg.new any 1\na = measure.i1 q\nret", "0.9");
        run.step().unwrap();
        run.step().unwrap();
        assert_eq!(
            run.machine.draws.len(),
            1,
            "launch must provide a draw for measure.iN"
        );
        assert_eq!(run.machine.draw_pos, 1);
        let draw = *run.machine.draws.get(0).unwrap();
        assert!((0.0..1.0).contains(&draw));
        assert!(run.measured());
        assert_eq!(run.measurements().len(), 1);
        let snapshot = run.snapshot("RUNNING").unwrap();
        assert!(run.reverse_to(0).unwrap_err().contains("E_NOT_REVERSIBLE"));
        assert_eq!(run.snapshot("RUNNING").unwrap(), snapshot);
    }

    #[test]
    fn failed_typed_measurement_does_not_collapse_or_block_reverse() {
        let source = "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=64\n\nsection symbols\n{}\n\nsection functions\nfn cc @main (q:qreg[*]) () () () ()\n^entry():\na = measure.i2 q\nret\n";
        let object = std::sync::Arc::new(crate::object::parse(source).unwrap());
        let mut resources = Resources::new(&object).unwrap();
        let id = resources.new_reg(1, "any").unwrap();
        resources.prepare(id, &[1]);
        let mut run =
            RuntimeRun::new(object, &resources, "main", vec![Value::Qreg(id)], vec![]).unwrap();
        let quantum = run.machine.quantum.clone();
        run.step().unwrap();
        assert_eq!(run.fault(), Some(4));
        assert!(!run.measured());
        assert!(run.measurements().is_empty());
        assert_eq!(run.machine.draw_pos, 0);
        assert!(run.machine.quantum.exact_equals(&quantum));
        assert!(run.machine.data.handles[&id].live);
        run.reverse_to(0).unwrap();
        assert_eq!(run.fault(), None);
        assert!(run.machine.quantum.exact_equals(&quantum));
    }

    #[test]
    fn replay_mismatch_keeps_effect_entry_state_and_cursor_exactly() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../bennett-isa-spec/reference-interpreter/tests/fixtures/classical.bisa"
        ))
        .unwrap();
        let (obj, _) = crate::load(&raw, false).unwrap();
        let obj = std::sync::Arc::new(obj);
        let resources = Resources::new(&obj).unwrap();
        let mut run =
            RuntimeRun::new(obj, &resources, "effects", vec![], vec![11, 22, 33]).unwrap();
        run.machine.drive(Some(3), 100).unwrap();
        let snapshot = run.snapshot("RUNNING").unwrap();
        let stack = run.machine.stack.clone();
        let history = run.machine.history.len();
        run.machine.effects.replay = Some(std::sync::Arc::new(vec![]));
        for _ in 0..2 {
            // Call the direct row path used by both CLI engines, not the
            // RuntimeRun wrapper (which independently checkpoints diagnostics).
            assert!(
                run.machine
                    .step_row(None)
                    .unwrap_err()
                    .contains("replay mismatch")
            );
            assert_eq!(run.snapshot("RUNNING").unwrap(), snapshot);
            assert_eq!(run.machine.stack, stack);
            assert_eq!(run.machine.history.len(), history);
            assert!(run.machine.effects.saved.is_empty());
        }
        run.machine.effects.replay = None;
        run.machine.step_row(None).unwrap();
        assert_eq!(run.machine.effects.saved.len(), 1);
        run.reverse_to(0).unwrap();
        assert!(run.machine.data.tape.is_empty());
        assert_eq!(run.machine.effects.saved.len(), 1);
    }

    #[test]
    fn diagnostic_rolls_back_control_registers_trace_and_history_exactly() {
        let mut run = run_for("n = const.i64 1\nbr malformed");
        run.step().unwrap();
        let snapshot = run.snapshot("RUNNING").unwrap();
        let control = run.machine.stack.clone();
        for _ in 0..2 {
            assert!(run.step().unwrap_err().contains("bad branch"));
            assert_eq!(run.snapshot("RUNNING").unwrap(), snapshot);
            assert_eq!(run.machine.stack, control);
        }
        run.reverse_to(0).unwrap();
        assert!(run.machine.env.is_empty());
    }

    #[test]
    fn repeated_definitions_share_tails_and_reverse_all_forty_thousand_rows() {
        let mut run = run_for("n = const.i64 1\nbr ^entry()");
        run.machine.trace = TraceLog::Disabled;
        for end in [20000, 40000] {
            REGISTER_READS.with(|n| n.set(0));
            run.machine.drive(Some(end), end).unwrap();
            let reads = REGISTER_READS.with(|n| n.get());
            assert!(reads <= 2 * 20000, "{reads} reads in a 20k prefix");
        }
        assert_eq!(run.machine.env[&(0, "n".into())].len(), 20000);
        run.reverse_to(0).unwrap();
        assert!(run.machine.env.is_empty());
        assert_eq!(run.machine.stack.last().unwrap().index, 0);
    }

    #[test]
    fn zero_length_memory_access_keeps_live_predecessor_boundary() {
        let mut run = run_for("ret");
        for (base, size, live) in [(16, 4, true), (20, 0, false)] {
            run.machine.data.add_allocation(Allocation {
                base: u(base),
                size: u(size),
                live,
                pinned: false,
            });
        }
        assert!(run.machine.data.valid(&u(20), &u(0)));
        assert!(!run.machine.data.valid(&u(20), &u(1)));
    }

    #[test]
    fn allocation_index_preserves_insertion_order_and_duplicate_zero_globals() {
        let mut run = run_for("ret");
        for (base, size) in [(24, 0), (16, 4), (24, 0), (24, 3), (17, 0)] {
            run.machine.data.add_allocation(Allocation {
                base: u(base),
                size: u(size),
                live: true,
                pinned: true,
            });
        }
        assert_eq!(run.machine.data.table.len(), 5);
        assert_eq!(
            run.machine
                .data
                .ordered_allocations()
                .iter()
                .map(|a| a.base.to_u64().unwrap())
                .collect::<Vec<_>>(),
            [24, 16, 24, 24, 17]
        );
        assert!(run.machine.data.valid(&u(26), &u(1)));
        assert!(run.machine.data.valid(&u(18), &u(1)));
        assert!(run.machine.data.valid(&u(24), &u(0)));
        assert!(run.machine.data.valid(&u(20), &u(0)));
        let before = run.machine.data.clone();
        let after = run.machine.data.clone();
        run.machine
            .data
            .undo_delta(&before, &after, &BTreeSet::new());
        assert_eq!(
            run.machine
                .data
                .ordered_allocations()
                .iter()
                .map(|a| a.base.to_u64().unwrap())
                .collect::<Vec<_>>(),
            [16, 17, 24, 24, 24]
        );
    }

    #[test]
    fn uncall_trace_omits_displaced_versions_with_identical_current_values() {
        let mut run = run_for(
            "r = const.i64 1\nr = call @value\nuncall @value () (r)\nret\nfn cc @value () (x:i64) () () ()\n^entry():\nq = const.i64 1\nret q",
        );
        while !run.terminal() {
            run.step().unwrap();
        }
        let trace = run.machine.trace.project(&run.machine.obj);
        let registers = trace[4].get("registers").unwrap();
        let Json::Arr(entries) = registers else {
            panic!("expected register array")
        };
        assert_eq!(entries.len(), 1, "{registers:?}");
        assert_eq!(entries[0].get("name").unwrap().as_str(), Some("q"));
    }

    #[test]
    fn segment_cancellation_removes_only_its_owned_definitions() {
        for version in ["0.8", "0.9"] {
            let mut run = run_for_version(
                "x = const.i8 1\nx = call @value\nsaved = const.i8 2\nx = const.i8 9\nuncall @value () (saved)\nx = add.i8 x saved\nret x\nfn r @value () (r:i8) () () ()\n^entry():\nr = const.i8 2\nret r",
                version,
            );
            while !run.terminal() {
                run.step().unwrap();
            }
            assert_eq!(run.values(), &[Value::int(8, u(11))], "ISA {version}");
            let snapshot = run.snapshot("COMPLETED").unwrap();
            let instances = snapshot.get("instances").unwrap().as_arr().unwrap();
            let x = instances
                .iter()
                .find(|v| v.get("name").and_then(Json::as_str) == Some("x"))
                .unwrap();
            assert_eq!(
                x.get("values"),
                Some(&vals(&[
                    Value::int(8, u(1)),
                    Value::int(8, u(9)),
                    Value::int(8, u(11))
                ]))
            );
            assert!(
                instances
                    .iter()
                    .all(|v| v.get("activation").and_then(Json::as_u64) == Some(0))
            );
            run.reverse_to(0).unwrap();
            assert!(run.machine.env.is_empty());
            assert_eq!(run.machine.stack.last().unwrap().index, 0);
            assert_eq!(run.machine.next_activation, 1);
        }
    }

    #[test]
    fn nested_segment_cancellation_preserves_all_later_versions() {
        let mut run = run_for_version(
            "x = const.i8 1\nx = call @outer\nsaved = const.i8 3\nx = const.i8 9\nx = const.i8 10\nuncall @outer () (saved)\nret x\nfn r @outer () (r:i8) () () ()\n^entry():\nr = const.i8 2\nr = call @inner\nret r\nfn r @inner () (r:i8) () () ()\n^entry():\nr = const.i8 1\nr = const.i8 3\nret r",
            "0.9",
        );
        while !run.terminal() {
            run.step().unwrap();
        }
        assert_eq!(run.values(), &[Value::int(8, u(10))]);
        let snapshot = run.snapshot("COMPLETED").unwrap();
        let instances = snapshot.get("instances").unwrap().as_arr().unwrap();
        let x = instances
            .iter()
            .find(|v| v.get("name").and_then(Json::as_str) == Some("x"))
            .unwrap();
        assert_eq!(
            x.get("values"),
            Some(&vals(&[
                Value::int(8, u(1)),
                Value::int(8, u(9)),
                Value::int(8, u(10))
            ]))
        );
        assert!(
            instances
                .iter()
                .all(|v| v.get("activation").and_then(Json::as_u64) == Some(0))
        );
        run.reverse_to(0).unwrap();
        assert!(run.machine.env.is_empty());
    }

    #[test]
    #[should_panic(expected = "RFC 8785: nonfinite JSON number")]
    fn nan_registers_keep_the_existing_per_row_trace_diagnostic() {
        let mut run = run_for("n = const.i64 1\nret");
        run.machine.trace = TraceLog::new(true);
        run.machine.define_at(0, "f", Value::F64(f64::NAN));
        // The former full-map comparison sees NaN != NaN and attempts to
        // serialize it even on an unrelated row. Preserve that diagnostic.
        run.step().unwrap();
    }

    #[test]
    fn zero_length_launch_access_locks_its_zero_sized_allocation() {
        let mut run = run_for("p = const.i64 16\nn = const.i64 0\neffect console.write p n\nret");
        run.step().unwrap();
        run.step().unwrap();
        run.machine.data.add_allocation(Allocation {
            base: u(16),
            size: u(0),
            live: true,
            pinned: false,
        });
        let (keys, _) = run.touches().unwrap();
        assert!(keys.contains(&ResourceKey::Entry(u(16))), "{keys:?}");
    }

    #[test]
    fn scheduler_keys_reject_unreadable_operands() {
        for row in [
            "p = alloc absent",
            "x = load.i8 absent",
            "free absent",
            "hostcall service p absent",
            "effect console.write p absent",
        ] {
            let run = run_for(row);
            let error = run.touches().unwrap_err();
            assert!(
                error.contains("resource") && error.contains("absent"),
                "{row}: {error}"
            );
        }
    }

    #[test]
    fn created_register_tracking_rejects_a_partial_snapshot() {
        let mut run = run_for("q = qreg.new any 1\nret");
        assert!(run.created_regs().unwrap().is_empty());
        run.step().unwrap();
        assert_eq!(run.created_regs().unwrap().len(), 1);
        run.machine.history.edit_last(|frame| frame.after = None);
        assert!(run.created_regs().unwrap_err().contains("both snapshots"));
    }

    fn assert_nested_reversal(run: &mut RuntimeRun, initial: &Data, snapshot: &Json) {
        let frames = snapshot.get("frames").unwrap().as_arr().unwrap();
        assert_eq!(
            frames
                .iter()
                .filter(|f| f.get("uncall") == Some(&Json::Bool(true)))
                .count(),
            2
        );
        run.reverse_to(0).unwrap();
        assert_eq!(run.machine.data.memory, initial.memory);
        assert_eq!(run.machine.data.table, initial.table);
        assert_eq!(run.machine.data.bump, initial.bump);
        assert_eq!(run.machine.data.block, initial.block);
        assert_eq!(run.machine.data.console, initial.console);
        assert_eq!(run.machine.data.clock, initial.clock);
        assert_eq!(run.machine.data.handles, initial.handles);
        assert_eq!(run.machine.data.tokens, initial.tokens);
        assert!(run.machine.env.is_empty());
    }

    #[test]
    fn repeated_nested_marker_rewind_keeps_cancelled_saved_events_and_exact_bytes() {
        let raw = std::fs::read_to_string(
            crate::pins::isa().join("reference-interpreter/tests/fixtures/classical.bisa"),
        )
        .unwrap();
        let obj = std::sync::Arc::new(crate::load(&raw, false).unwrap().0);
        let resources = Resources::new(&obj).unwrap();
        let initial = resources.data.clone();
        let mut run = RuntimeRun::new(obj, &resources, "nested", vec![], vec![]).unwrap();
        let mut expected = None;
        for executed in 1..=3 {
            while !run.terminal() {
                run.step().unwrap();
            }
            assert_eq!(run.fault(), None);
            assert_eq!(run.values(), &[Value::int(64, u(0))]);
            assert!(run.tape().is_empty());
            assert_eq!(run.machine.effects.saved.len(), executed);
            let snapshot = run.snapshot("COMPLETED").unwrap();
            assert_eq!(expected.get_or_insert_with(|| snapshot.clone()), &snapshot);
            assert_nested_reversal(&mut run, &initial, &snapshot);
        }
    }

    #[test]
    fn straight_line_checkpoint_and_trace_work_is_bounded_per_row() {
        let mut body = String::new();
        for n in 0..2000 {
            body.push_str(&format!("v{n} = const.i64 {n}\n"));
        }
        let source = format!(
            "bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=64\n\nsection symbols\n{{}}\n\nsection functions\nfn cc @main () (result:i64) () () ()\n^entry():\n{body}ret v1999\n"
        );
        let obj = std::sync::Arc::new(crate::object::parse(&source).unwrap());
        let resources = Resources::new(&obj).unwrap();
        let mut run = RuntimeRun::new(obj, &resources, "main", vec![], vec![]).unwrap();
        REGISTER_READS.with(|n| n.set(0));
        while !run.terminal() {
            run.step().unwrap();
        }
        let reads = REGISTER_READS.with(|n| n.get());
        assert!(
            reads <= 4 * 2001,
            "{reads} register reads for 2001 rows: checkpoint/trace scanned retained state"
        );
        assert_eq!(run.values(), &[Value::int(64, u(1999))]);
        run.reverse_to(0).unwrap();
        assert!(run.machine.env.is_empty());
        assert_eq!(run.machine.next_activation, 1);
    }
}
