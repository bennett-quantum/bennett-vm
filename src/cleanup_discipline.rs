//! §6.2 at load: a workspace value is released only after the exact inverse of its
//! computing sequence, and `quncopy`/`qunload` only within their computing window.
//!
//! Each wire (and `r[*]`, standing for a region's cells) carries a version: a
//! cons of (previous version, event). An event's identity is its operation, its
//! operands, the versions it read and its effective controls, so an event cancels
//! the top of a log only when it is the same operation at the same versions.
//! A value with `other` provenance (§6.1) additionally requires that nothing live
//! read its current version (§6.1 dependency-set test).
//!
//! An opcode this analysis does not model leaves the function to the run-time
//! `qfree` cleanliness check, as does a `qloop` whose state never repeats within
//! `MAX_LOOP_VISITS`: verdicts here are only ever rejections with a cited rule.
use crate::{
    object::{FnKind, Function, Instr, Object, Type},
    qvalidate::{flip, wires},
    validate::{action_sig, err, list_operand, oracle_sig},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MAX_LOOP_VISITS: u64 = 4096;

type Wire = (String, usize);
type Version = u32;
type Event = u32;
const CLEAN: Version = 0;
/// `r[*]`: the pseudo-wire of region `r`'s cells.
const CELLS: usize = usize::MAX;

/// (wire, version, cell epoch) as read by an event.
type Read = (Wire, Version, u64);

#[derive(Clone, PartialEq)]
struct Creation {
    kind: &'static str,
    sources: Vec<String>,
    reads: Vec<Read>,
    controls: Vec<(Wire, bool, Version, u64)>,
    /// Window-closing counts of the read wires at the computation.
    closures: Vec<(Wire, u64)>,
}

enum Stop {
    /// Leave this function to the run-time checks.
    Unmodelled,
    Reject(String),
}
impl From<String> for Stop {
    fn from(e: String) -> Self {
        Stop::Reject(e)
    }
}
type Outcome = Result<(), Stop>;

/// Interned identities, shared by every visit of every loop.
#[derive(Default)]
struct Tables {
    events: HashMap<String, Event>,
    event_reads: Vec<Vec<Read>>,
    event_other: Vec<bool>,
    versions: HashMap<(Version, Event), Version>,
    /// (previous version, event, any `other` event in the log); [0] is CLEAN.
    version_nodes: Vec<(Version, Event, bool)>,
}
impl Tables {
    fn new() -> Self {
        Self {
            version_nodes: vec![(CLEAN, Event::MAX, false)],
            ..Self::default()
        }
    }
    fn event(&mut self, key: String) -> Event {
        let next = self.event_reads.len() as Event;
        let id = *self.events.entry(key).or_insert(next);
        if id == next {
            self.event_reads.push(vec![]);
            self.event_other.push(false);
        }
        id
    }
    fn push(&mut self, version: Version, event: Event) -> Version {
        let other = self.version_nodes[version as usize].2 || self.event_other[event as usize];
        let next = self.version_nodes.len() as Version;
        let id = *self.versions.entry((version, event)).or_insert(next);
        if id == next {
            self.version_nodes.push((version, event, other));
        }
        id
    }
    fn top(&self, version: Version) -> Option<Event> {
        (version != CLEAN).then(|| self.version_nodes[version as usize].1)
    }
    /// The wire is at `current` and the reader read `read`, a state at or above it.
    fn read_is_current(&self, current: Version, mut read: Version) -> bool {
        if current == CLEAN {
            return false;
        }
        loop {
            if read == current {
                return true;
            }
            if read == CLEAN {
                return false;
            }
            read = self.version_nodes[read as usize].0;
        }
    }
}

#[derive(Clone, PartialEq)]
struct State {
    env: BTreeMap<String, usize>,
    /// Region name to cell width.
    regions: BTreeMap<String, usize>,
    predicates: Vec<(String, bool)>,
    created: BTreeMap<String, Creation>,
    logs: BTreeMap<Wire, Version>,
    epochs: BTreeMap<Wire, u64>,
    closed: BTreeMap<Wire, u64>,
    phases: Vec<Event>,
}

struct Analysis<'a> {
    obj: &'a Object,
    tables: Tables,
    saw_other: bool,
}

/// The inverse gate of a fixed gate; rotations negate their angle instead.
fn inverse_name(op: &str) -> &str {
    match op {
        "s" => "sdg",
        "sdg" => "s",
        "t" => "tdg",
        "tdg" => "t",
        other => other,
    }
}

impl State {
    fn bundle(&self, name: &str) -> Result<Vec<Wire>, Stop> {
        wires(&self.env, name)
            .map_err(|_| Stop::Reject(err("3.3", format!("unknown or consumed wire {name}"))))
    }
    fn version(&self, w: &Wire) -> Version {
        self.logs.get(w).copied().unwrap_or(CLEAN)
    }
    fn read(&self, w: &Wire) -> Read {
        (
            w.clone(),
            self.version(w),
            self.epochs.get(w).copied().unwrap_or(0),
        )
    }
    fn control_wires(&self) -> Result<Vec<(Wire, bool)>, Stop> {
        let mut out = vec![];
        for (p, positive) in &self.predicates {
            for w in self.bundle(p)? {
                out.push((w, *positive));
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
    fn controls(&self) -> Result<Vec<(Wire, bool, Version, u64)>, Stop> {
        Ok(self
            .control_wires()?
            .into_iter()
            .map(|(w, p)| {
                let (w, v, e) = self.read(&w);
                (w, p, v, e)
            })
            .collect())
    }
    fn clean(&self, name: &str) -> Result<bool, Stop> {
        Ok(self.bundle(name)?.iter().all(|w| self.version(w) == CLEAN))
    }
    fn produce(&mut self, name: &str, width: usize, creation: Creation) -> Outcome {
        if self.env.contains_key(name) {
            return Err(Stop::Reject(err(
                "3.3",
                format!("linear name {name} produced twice"),
            )));
        }
        self.env.insert(name.into(), width);
        self.created.insert(name.into(), creation);
        Ok(())
    }
    fn consume(&mut self, name: &str) -> Outcome {
        for w in self.bundle(name)? {
            self.logs.remove(&w);
            self.closed.remove(&w);
        }
        self.env.remove(name);
        self.created.remove(name);
        Ok(())
    }
    fn window_closed(&self, c: &Creation) -> bool {
        c.closures
            .iter()
            .any(|(w, n)| self.closed.get(w).copied().unwrap_or(0) != *n)
    }
}

impl Analysis<'_> {
    /// Direct reads, the effective controls, and what their versions read.
    fn reads(&self, s: &State, direct: &[Wire]) -> Result<Vec<Read>, Stop> {
        let mut wires: BTreeSet<Wire> = direct.iter().cloned().collect();
        wires.extend(s.control_wires()?.into_iter().map(|(w, _)| w));
        let mut out: Vec<Read> = wires.iter().map(|w| s.read(w)).collect();
        if self.saw_other {
            let mut seen = wires;
            let mut pending = out.clone();
            let mut versions = BTreeSet::new();
            while let Some((wire, _, _)) = pending.pop() {
                let mut version = s.version(&wire);
                while versions.insert(version) && version != CLEAN {
                    let (previous, event, _) = self.tables.version_nodes[version as usize];
                    for read in &self.tables.event_reads[event as usize] {
                        if seen.insert(read.0.clone()) {
                            out.push(read.clone());
                            pending.push(read.clone());
                        }
                    }
                    version = previous;
                }
                if let Some(c) = s.created.get(&wire.0) {
                    for read in &c.reads {
                        if seen.insert(read.0.clone()) {
                            out.push(read.clone());
                            pending.push(read.clone());
                        }
                    }
                }
            }
        }
        Ok(out)
    }
    fn key(&self, s: &State, op: &str, rest: &str, direct: &[Wire]) -> Result<String, Stop> {
        let snapshot = direct.iter().map(|w| s.read(w)).collect::<Vec<_>>();
        Ok(format!("{op}|{rest}|{snapshot:?}|{:?}", s.controls()?))
    }
    /// The event's outputs change together: it cancels whole or not at all.
    /// `inverse` is the (op, rest) of the event that this one undoes.
    #[allow(clippy::too_many_arguments)]
    fn change(
        &mut self,
        s: &mut State,
        op: &str,
        rest: &str,
        inverse: Option<(&str, &str)>,
        outputs: &[Wire],
        direct: &[Wire],
        other: bool,
    ) -> Outcome {
        self.saw_other |= other;
        let key = self.key(s, op, rest, direct)?;
        let reads = self.reads(s, direct)?;
        let event = self.tables.event(key);
        self.tables.event_reads[event as usize] = reads;
        self.tables.event_other[event as usize] = other;
        let undoes = match inverse {
            Some((inverse_op, inverse_rest)) => {
                let key = self.key(s, inverse_op, inverse_rest, direct)?;
                Some(self.tables.event(key))
            }
            None => None,
        };
        let cancels = undoes.is_some()
            && outputs.iter().all(|w| {
                let version = s.version(w);
                self.tables.top(version) == undoes && !self.blocked(s, w, version, other, outputs)
            });
        for w in outputs {
            let version = s.version(w);
            let next = if cancels {
                self.tables.version_nodes[version as usize].0
            } else {
                self.tables.push(version, event)
            };
            if next == CLEAN {
                s.logs.remove(w);
            } else {
                s.logs.insert(w.clone(), next);
            }
            if other {
                *s.closed.entry(w.clone()).or_default() += 1;
            }
        }
        Ok(())
    }
    /// §6.1 dependency-set test, for `other` values only: a live value, a phase or
    /// a created value still reads the version the event would undo.
    fn blocked(
        &self,
        s: &State,
        w: &Wire,
        version: Version,
        other: bool,
        outputs: &[Wire],
    ) -> bool {
        if !other && !self.tables.version_nodes[version as usize].2 {
            return false;
        }
        let reads_it = |reads: &[Read]| {
            reads
                .iter()
                .any(|(r, read, _)| r == w && self.tables.read_is_current(version, *read))
        };
        let event_reads_it = |event: Event| reads_it(&self.tables.event_reads[event as usize]);
        s.phases.iter().any(|e| event_reads_it(*e))
            || s.logs.iter().any(|(target, log)| {
                let mut log = *log;
                if outputs.contains(target) {
                    return false;
                }
                while log != CLEAN {
                    let (previous, event, _) = self.tables.version_nodes[log as usize];
                    if event_reads_it(event) {
                        return true;
                    }
                    log = previous;
                }
                false
            })
            || s.created.values().any(|c| reads_it(&c.reads))
    }
    fn gate(&mut self, s: &mut State, i: &Instr) -> Outcome {
        let op = i.op.as_str();
        let (controls, targets, rest) = match op {
            "x" | "cx" | "ccx" | "mcx" => {
                let (c, t) = flip(i, &s.env).map_err(|_| {
                    Stop::Reject(err("3.3", format!("unknown or consumed wire in {}", i.raw)))
                })?;
                (c, vec![t], i.operands.join(" "))
            }
            "cz" => {
                let (a, b) = (s.bundle(&i.operands[0])?, s.bundle(&i.operands[1])?);
                ([a, b].concat(), vec![], i.operands.join(" "))
            }
            "h" | "s" | "sdg" | "t" | "tdg" | "swap" | "rx" | "ry" | "rz" => {
                let start = usize::from(matches!(op, "rx" | "ry" | "rz"));
                let mut ts = vec![];
                for operand in &i.operands[start..] {
                    ts.extend(s.bundle(operand)?);
                }
                (vec![], ts, i.operands[start..].join(" "))
            }
            _ => return Err(Stop::Unmodelled),
        };
        if op == "cz" || matches!(op, "s" | "sdg" | "t" | "tdg" | "rz") {
            return self.diagonal(s, i, controls, targets, rest);
        }
        if matches!(op, "rx" | "ry") {
            let angle = &i.operands[0];
            let forward = format!("{} {rest}", normal_angle(angle));
            let inverse = negated_angle(angle).map(|n| format!("{n} {rest}"));
            let inverse = inverse.as_deref().map(|r| (op, r));
            return self.change(s, op, &forward, inverse, &targets, &[], true);
        }
        let other = op == "h";
        self.change(
            s,
            op,
            &rest,
            Some((op, rest.as_str())),
            &targets,
            &controls,
            other,
        )
    }
    /// A diagonal gate changes no perm value; an `other` operand logs it, and its
    /// phase stays outstanding, as a read of its operands, until its inverse.
    fn diagonal(
        &mut self,
        s: &mut State,
        i: &Instr,
        controls: Vec<Wire>,
        targets: Vec<Wire>,
        rest: String,
    ) -> Outcome {
        let op = i.op.as_str();
        let (forward, inverse): (String, Option<String>) = if op == "rz" {
            let angle = &i.operands[0];
            (
                format!("{} {rest}", normal_angle(angle)),
                negated_angle(angle).map(|n| format!("{n} {rest}")),
            )
        } else {
            (rest.clone(), Some(rest))
        };
        let inverse_op = inverse_name(op);
        let operands = [controls.clone(), targets.clone()].concat();
        let perm: Vec<Wire> = operands
            .iter()
            .filter(|w| !self.tables.version_nodes[s.version(w) as usize].2)
            .cloned()
            .collect();
        if !perm.is_empty() {
            let key = self.key(s, op, &forward, &operands)?;
            let reads = self.reads(s, &perm)?;
            let event = self.tables.event(key);
            self.tables.event_reads[event as usize] = reads;
            let undoes = match &inverse {
                Some(rest) => {
                    let key = self.key(s, inverse_op, rest, &operands)?;
                    Some(self.tables.event(key))
                }
                None => None,
            };
            match undoes.and_then(|u| s.phases.iter().position(|p| *p == u)) {
                Some(index) => {
                    s.phases.remove(index);
                }
                None => s.phases.push(event),
            }
        }
        let others: Vec<Wire> = targets
            .iter()
            .filter(|w| self.tables.version_nodes[s.version(w) as usize].2)
            .cloned()
            .collect();
        if others.is_empty() {
            return Ok(());
        }
        let key = self.key(s, op, &forward, &controls)?;
        let event = self.tables.event(key);
        let undoes = match &inverse {
            Some(rest) => {
                let key = self.key(s, inverse_op, rest, &controls)?;
                Some(self.tables.event(key))
            }
            None => None,
        };
        for w in &others {
            let version = s.version(w);
            let next = if self.tables.top(version) == undoes && undoes.is_some() {
                self.tables.version_nodes[version as usize].0
            } else {
                self.tables.push(version, event)
            };
            if next == CLEAN {
                s.logs.remove(w);
            } else {
                s.logs.insert(w.clone(), next);
            }
        }
        Ok(())
    }
    fn call(&mut self, s: &mut State, i: &Instr) -> Outcome {
        let mut xs = i.operands.as_slice();
        let mut op = i.op.as_str();
        let mut control = None;
        let mut adjoint = false;
        if op == "ctl" {
            control = Some(xs[0].clone());
            op = &xs[1];
            xs = &xs[2..];
        }
        if op == "adj" {
            adjoint = true;
            op = &xs[0];
            xs = &xs[1..];
        }
        let name = xs[0].trim_start_matches('@');
        let callee = self.obj.function(name);
        if control.is_some() {
            s.predicates.push((control.clone().unwrap(), true));
        }
        let cells = |r: &str| (r.to_string(), CELLS);
        let (outputs, reads, kind, inverse_kind, other) = if op == "action_call" {
            let Ok(sig) = action_sig(self.obj, name) else {
                return Err(Stop::Unmodelled);
            };
            let mut outputs = vec![];
            for n in list_operand(&xs[1])? {
                outputs.extend(s.bundle(&n)?);
            }
            let mut reads = vec![];
            for n in list_operand(&xs[2])? {
                reads.extend(s.bundle(&n)?);
            }
            for (n, (_, _, role)) in list_operand(&xs[3])?.iter().zip(&sig.mem) {
                if role == "in" {
                    reads.push(cells(n));
                } else {
                    outputs.push(cells(n));
                }
            }
            let Some(callee) = callee else {
                return Err(Stop::Unmodelled);
            };
            let other = callee.kind == FnKind::Action;
            let (forward, backward) = if adjoint {
                ("call_adj", "call_fwd")
            } else {
                ("call_fwd", "call_adj")
            };
            (outputs, reads, forward, backward, other)
        } else if op == "oracle_call" {
            let Ok(sig) = oracle_sig(self.obj, name) else {
                return Err(Stop::Unmodelled);
            };
            let mut outputs = s.bundle(&xs[3])?;
            outputs.extend(s.bundle(&xs[4])?);
            let mut reads = vec![];
            for n in list_operand(&xs[2])? {
                reads.extend(s.bundle(&n)?);
            }
            for (n, _) in list_operand(&xs[5])?.iter().zip(&sig.mem) {
                reads.push(cells(n));
            }
            (outputs, reads, "oracle_call", "oracle_call", false)
        } else {
            return Err(Stop::Unmodelled);
        };
        let rest = xs.join(" ");
        self.change(
            s,
            kind,
            &rest,
            Some((inverse_kind, rest.as_str())),
            &outputs,
            &reads,
            other,
        )?;
        for w in outputs.iter().filter(|w| w.1 == CELLS) {
            *s.epochs.entry(w.clone()).or_default() += 1;
        }
        if control.is_some() {
            s.predicates.pop();
        }
        Ok(())
    }
    fn creation(
        &self,
        s: &State,
        kind: &'static str,
        sources: Vec<String>,
        deps: &[Wire],
    ) -> Result<Creation, Stop> {
        let mut read_wires: BTreeSet<Wire> = deps.iter().cloned().collect();
        read_wires.extend(s.control_wires()?.into_iter().map(|(w, _)| w));
        Ok(Creation {
            kind,
            sources,
            reads: read_wires.iter().map(|w| s.read(w)).collect(),
            controls: s.controls()?,
            closures: read_wires
                .iter()
                .map(|w| (w.clone(), s.closed.get(w).copied().unwrap_or(0)))
                .collect(),
        })
    }
    /// A cleanup's reads, controls and window against the computing sequence.
    fn outside_window(
        &self,
        s: &State,
        name: &str,
        kind: &'static str,
        sources: &[String],
        deps: &[Wire],
    ) -> Result<bool, Stop> {
        let Some(c) = s.created.get(name) else {
            return Ok(true);
        };
        let now = self.creation(s, kind, sources.to_vec(), deps)?;
        Ok(c.kind != kind
            || c.sources != now.sources
            || c.reads != now.reads
            || c.controls != now.controls
            || !s.clean(name)?
            || s.window_closed(c))
    }
    fn sequence(&mut self, s: &mut State, body: &[Instr]) -> Outcome {
        for i in body {
            self.row(s, i)?;
        }
        Ok(())
    }
    fn memory_address(&self, s: &State, address: &str) -> Result<Vec<Wire>, Stop> {
        if address == "()" {
            Ok(vec![])
        } else {
            s.bundle(address)
        }
    }
    fn row(&mut self, s: &mut State, i: &Instr) -> Outcome {
        let a = &i.operands;
        match i.op.as_str() {
            "qif" => {
                for (arm, body) in i.nested.iter().enumerate() {
                    s.predicates.push((a[0].clone(), arm == 0));
                    self.sequence(s, body)?;
                    s.predicates.pop();
                }
                Ok(())
            }
            "qloop" => self.qloop(s, i),
            "qpred" => {
                s.predicates.push((a[0].clone(), true));
                Ok(())
            }
            "qunpred" => {
                s.predicates.pop();
                Ok(())
            }
            "qalloc" | "qborrow" => {
                let width = a[1].parse().map_err(|_| Stop::Unmodelled)?;
                let kind = if i.op == "qalloc" {
                    "qalloc"
                } else {
                    "qborrow"
                };
                let creation = self.creation(s, kind, vec![], &[])?;
                s.produce(&i.lhs[0], width, creation)
            }
            "qfree" => {
                match s.created.get(&a[0]) {
                    None => return Err(Stop::Reject(err("3.3", "release of non-workspace value"))),
                    Some(c) if c.kind != "qalloc" => {
                        return Err(Stop::Reject(err("5.1", "allocation/release kind mismatch")));
                    }
                    Some(_) => {}
                }
                if !s.clean(&a[0])? {
                    return Err(Stop::Reject(err(
                        "6.2",
                        "released value has not been uncomputed",
                    )));
                }
                s.consume(&a[0])
            }
            "qreturn" => s.consume(&a[0]),
            "qcopy" => {
                let source = s.bundle(&a[0])?;
                let creation = self.creation(s, "qcopy", vec![a[0].clone()], &source)?;
                s.produce(&i.lhs[0], source.len(), creation)
            }
            "quncopy" => {
                let source = s.bundle(&a[0])?;
                if !s.created.contains_key(&a[1])
                    || self.outside_window(s, &a[1], "qcopy", &[a[0].clone()], &source)?
                {
                    return Err(Stop::Reject(err(
                        "6.2",
                        "quncopy outside its computing window",
                    )));
                }
                s.consume(&a[1])
            }
            "qregion" => {
                s.regions
                    .insert(a[0].clone(), a[2].parse().map_err(|_| Stop::Unmodelled)?);
                Ok(())
            }
            "qload" => {
                let Some(width) = s.regions.get(&a[0]).copied() else {
                    return Err(Stop::Unmodelled);
                };
                let mut deps = self.memory_address(s, &a[1])?;
                deps.push((a[0].clone(), CELLS));
                let creation = self.creation(s, "qload", a.clone(), &deps)?;
                s.produce(&i.lhs[0], width, creation)
            }
            "qunload" => {
                let mut deps = self.memory_address(s, &a[1])?;
                deps.push((a[0].clone(), CELLS));
                if !s.created.contains_key(&a[2])
                    || self.outside_window(s, &a[2], "qload", &a[..2], &deps)?
                {
                    return Err(Stop::Reject(err("6.2", "qunload outside computing window")));
                }
                s.consume(&a[2])
            }
            "qstore" | "qexchm" => {
                let cell = (a[0].clone(), CELLS);
                let mut reads = self.memory_address(s, &a[1])?;
                let value = s.bundle(&a[2])?;
                let mut outputs = vec![cell.clone()];
                if i.op == "qstore" {
                    reads.extend(value);
                } else {
                    outputs.extend(value);
                }
                let (op, rest) = (i.op.as_str(), a.join(" "));
                self.change(
                    s,
                    op,
                    &rest,
                    Some((op, rest.as_str())),
                    &outputs,
                    &reads,
                    false,
                )?;
                *s.epochs.entry(cell).or_default() += 1;
                Ok(())
            }
            "qxor" => {
                let (source, target) = (s.bundle(&a[0])?, s.bundle(&a[1])?);
                let rest = a.join(" ");
                self.change(
                    s,
                    "qxor",
                    &rest,
                    Some(("qxor", rest.as_str())),
                    &target,
                    &source,
                    false,
                )
            }
            "qexch" => {
                let outputs = [s.bundle(&a[0])?, s.bundle(&a[1])?].concat();
                let rest = a.join(" ");
                self.change(
                    s,
                    "qexch",
                    &rest,
                    Some(("qexch", rest.as_str())),
                    &outputs,
                    &[],
                    false,
                )
            }
            "qinplace" => {
                let target = s.bundle(&a[3])?;
                let mut reads = vec![];
                if a.len() > 4 {
                    for n in list_operand(&a[4])? {
                        reads.extend(s.bundle(&n)?);
                    }
                }
                let forward = a.join(" ");
                let mut swapped = a.clone();
                swapped.swap(0, 1);
                let inverse = swapped.join(" ");
                let undo = Some(("qinplace", inverse.as_str()));
                self.change(s, "qinplace", &forward, undo, &target, &reads, false)
            }
            "action_call" | "oracle_call" | "adj" | "ctl" => self.call(s, i),
            op if crate::action::is_gate_name(op) => self.gate(s, i),
            _ => Err(Stop::Unmodelled),
        }
    }
    fn qloop(&mut self, s: &mut State, i: &Instr) -> Outcome {
        let a = &i.operands;
        let n: u64 = a[0].parse().map_err(|_| Stop::Unmodelled)?;
        let selector = a.get(2);
        if n == 0 {
            return self.sequence(&mut s.clone(), &i.nested[0]);
        }
        let outer = s.predicates.clone();
        let mut seen: Vec<(u64, State)> = vec![];
        let mut visit = 0u64;
        let mut visits = 0u64;
        while visit < n {
            if let Some(selector) = selector {
                s.predicates
                    .extend((0..=visit).map(|k| (format!("{selector}[{k}]"), true)));
            } else {
                let key = cycle_key(s);
                if let Some((prior, _)) = seen.iter().find(|(_, state)| *state == key) {
                    let period = visit - prior;
                    let skip = (n - visit) / period * period;
                    if skip != 0 {
                        visit += skip;
                        continue;
                    }
                }
                seen.push((visit, key));
            }
            visits += 1;
            if visits > MAX_LOOP_VISITS {
                return Err(Stop::Unmodelled);
            }
            self.sequence(s, &i.nested[0])?;
            s.predicates = outer.clone();
            visit += 1;
        }
        Ok(())
    }
}

/// The state up to the absolute closure counts, which are only ever compared for
/// equality with the counts a live creation saved.
fn cycle_key(s: &State) -> State {
    let mut key = s.clone();
    let mut classes: BTreeMap<Wire, BTreeSet<u64>> = BTreeMap::new();
    for (w, n) in &s.closed {
        classes.entry(w.clone()).or_default().insert(*n);
    }
    for c in s.created.values() {
        for (w, n) in &c.closures {
            classes.entry(w.clone()).or_default().insert(*n);
        }
    }
    let rank = |w: &Wire, n: u64| classes[w].iter().position(|m| *m == n).unwrap() as u64;
    for (w, n) in &mut key.closed {
        *n = rank(w, *n);
    }
    for c in key.created.values_mut() {
        for (w, n) in &mut c.closures {
            *n = rank(w, *n);
        }
    }
    key
}

/// A literal angle by its bits, so spellings of one value are one identity. A
/// classical-argument angle has no spelled negation and undoes nothing here.
fn normal_angle(angle: &str) -> String {
    match angle.parse::<f64>() {
        Ok(value) => format!("#{:016x}", value.to_bits()),
        Err(_) => angle.into(),
    }
}
fn negated_angle(angle: &str) -> Option<String> {
    let value: f64 = angle.parse().ok()?;
    Some(format!("#{:016x}", (-value).to_bits()))
}

pub(crate) fn function(obj: &Object, f: &Function) -> Result<(), String> {
    if !matches!(f.kind, FnKind::Action | FnKind::PermAction) {
        return Ok(());
    }
    let mut env = BTreeMap::new();
    let mut regions = BTreeMap::new();
    for p in f.lists.iter().flatten() {
        match p.ty {
            Type::Wire(n) => {
                env.insert(p.name.clone(), n);
            }
            Type::Region(_, w) => {
                regions.insert(p.name.clone(), w);
            }
            _ => {}
        }
    }
    let mut state = State {
        env,
        regions,
        predicates: vec![],
        created: BTreeMap::new(),
        logs: BTreeMap::new(),
        epochs: BTreeMap::new(),
        closed: BTreeMap::new(),
        phases: vec![],
    };
    let mut analysis = Analysis {
        obj,
        tables: Tables::new(),
        saw_other: false,
    };
    for block in &f.blocks {
        match analysis.sequence(&mut state, &block.instrs) {
            Ok(()) => {}
            Err(Stop::Unmodelled) => return Ok(()),
            Err(Stop::Reject(e)) => return Err(format!("{e} in @{}", f.name)),
        }
    }
    Ok(())
}
