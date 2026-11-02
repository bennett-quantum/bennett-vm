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
//! Verdicts here are only rejections with a cited rule; the analysis stays out of
//! what it cannot decide. A write under a predicate, through a region call or by an
//! exchange makes its wires unknown, an opcode it does not model or a `qloop` whose
//! state never repeats within `MAX_LOOP_VISITS` ends it for the function, and the
//! run-time `qfree` cleanliness check remains in all cases.
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
    reads: Vec<Read>,
    controls: Vec<(Wire, bool)>,
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
    /// Written in a way this analysis does not follow: never a rejection.
    unknown: BTreeSet<Wire>,
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
        Ok(self
            .bundle(name)?
            .iter()
            .all(|w| self.version(w) == CLEAN || self.unknown.contains(w)))
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
            self.unknown.remove(&w);
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
        // Under predicates, only values born in this same context are followed.
        let controls = s.control_wires()?;
        let same_context = outputs
            .iter()
            .all(|w| s.created.get(&w.0).is_some_and(|c| c.controls == controls));
        if !s.predicates.is_empty() && !same_context {
            s.unknown.extend(outputs.iter().cloned());
            if other {
                for w in outputs {
                    *s.closed.entry(w.clone()).or_default() += 1;
                }
            }
            return Ok(());
        }
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
        if !s.predicates.is_empty() {
            s.unknown.extend(targets);
            return Ok(());
        }
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
        if let Some(control) = &control {
            s.predicates.push((control.clone(), true));
        }
        let cells = |r: &str| (r.to_string(), CELLS);
        let mut outputs = vec![];
        let mut other = false;
        if op == "action_call" {
            let (Ok(sig), Some(callee)) = (action_sig(self.obj, name), callee) else {
                return Err(Stop::Unmodelled);
            };
            for n in list_operand(&xs[1])? {
                outputs.extend(s.bundle(&n)?);
            }
            for (n, (_, _, role)) in list_operand(&xs[3])?.iter().zip(&sig.mem) {
                if role != "in" {
                    outputs.push(cells(n));
                }
            }
            other = callee.kind == FnKind::Action;
        } else if op == "oracle_call" {
            outputs.extend(s.bundle(&xs[3])?);
            outputs.extend(s.bundle(&xs[4])?);
        } else {
            return Err(Stop::Unmodelled);
        }
        self.saw_other |= other;
        for w in &outputs {
            s.unknown.insert(w.clone());
            if other {
                *s.closed.entry(w.clone()).or_default() += 1;
            }
            if w.1 == CELLS {
                *s.epochs.entry(w.clone()).or_default() += 1;
            }
        }
        if control.is_some() {
            s.predicates.pop();
        }
        Ok(())
    }
    fn creation(&self, s: &State, kind: &'static str, deps: &[Wire]) -> Result<Creation, Stop> {
        let mut read_wires: BTreeSet<Wire> = deps.iter().cloned().collect();
        read_wires.extend(s.control_wires()?.into_iter().map(|(w, _)| w));
        Ok(Creation {
            kind,
            reads: read_wires.iter().map(|w| s.read(w)).collect(),
            controls: s.control_wires()?,
            closures: read_wires
                .iter()
                .map(|w| (w.clone(), s.closed.get(w).copied().unwrap_or(0)))
                .collect(),
        })
    }
    /// A cleanup after its inverse event: its value clean, under the computing
    /// sequence's controls, with no window closed since.
    fn outside_window(&self, s: &State, name: &str) -> Result<bool, Stop> {
        let Some(c) = s.created.get(name) else {
            return Ok(true);
        };
        Ok(c.controls != s.control_wires()? || !s.clean(name)? || s.window_closed(c))
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
                let creation = self.creation(s, kind, &[])?;
                s.produce(&i.lhs[0], width, creation)
            }
            "qfree" => {
                match s.created.get(&a[0]) {
                    None => {
                        return Err(Stop::Reject(err("3.3", "release of non-workspace value")));
                    }
                    Some(c) if c.kind == "qborrow" => {
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
            // §4.5: a copy is the XOR of its source into a clean bundle, so a
            // matching `qxor` or `quncopy` is its inverse.
            "qcopy" => {
                let source = s.bundle(&a[0])?;
                let creation = self.creation(s, "qcopy", &source)?;
                s.produce(&i.lhs[0], source.len(), creation)?;
                let rest = format!("{} {}", a[0], i.lhs[0]);
                let target = s.bundle(&i.lhs[0])?;
                let undo = Some(("qxor", rest.as_str()));
                self.change(s, "qxor", &rest, undo, &target, &source, false)
            }
            "quncopy" => {
                let (source, target) = (s.bundle(&a[0])?, s.bundle(&a[1])?);
                let rest = a.join(" ");
                let undo = Some(("qxor", rest.as_str()));
                self.change(s, "qxor", &rest, undo, &target, &source, false)?;
                if self.outside_window(s, &a[1])? {
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
                let creation = self.creation(s, "qload", &deps)?;
                s.produce(&i.lhs[0], width, creation)?;
                let rest = format!("{} {} {}", a[0], a[1], i.lhs[0]);
                let target = s.bundle(&i.lhs[0])?;
                let undo = Some(("qload", rest.as_str()));
                self.change(s, "qload", &rest, undo, &target, &deps, false)
            }
            "qunload" => {
                let mut deps = self.memory_address(s, &a[1])?;
                deps.push((a[0].clone(), CELLS));
                let target = s.bundle(&a[2])?;
                let rest = a.join(" ");
                let undo = Some(("qload", rest.as_str()));
                self.change(s, "qload", &rest, undo, &target, &deps, false)?;
                if self.outside_window(s, &a[2])? {
                    return Err(Stop::Reject(err("6.2", "qunload outside computing window")));
                }
                s.consume(&a[2])
            }
            "qstore" => {
                let cell = (a[0].clone(), CELLS);
                let mut reads = self.memory_address(s, &a[1])?;
                reads.extend(s.bundle(&a[2])?);
                let rest = a.join(" ");
                let undo = Some(("qstore", rest.as_str()));
                self.change(
                    s,
                    "qstore",
                    &rest,
                    undo,
                    std::slice::from_ref(&cell),
                    &reads,
                    false,
                )?;
                *s.epochs.entry(cell).or_default() += 1;
                Ok(())
            }
            "qexchm" => {
                let cell = (a[0].clone(), CELLS);
                let value = s.bundle(&a[2])?;
                s.unknown.extend(value);
                s.unknown.insert(cell.clone());
                *s.epochs.entry(cell).or_default() += 1;
                Ok(())
            }
            "qxor" => {
                let (source, target) = (s.bundle(&a[0])?, s.bundle(&a[1])?);
                let rest = a.join(" ");
                let undo = Some(("qxor", rest.as_str()));
                self.change(s, "qxor", &rest, undo, &target, &source, false)
            }
            "qexch" => {
                let (left, right) = (s.bundle(&a[0])?, s.bundle(&a[1])?);
                s.unknown.extend(left);
                s.unknown.extend(right);
                Ok(())
            }
            "qinplace" => {
                let target = s.bundle(&a[3])?;
                s.unknown.extend(target);
                Ok(())
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
        unknown: BTreeSet::new(),
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
