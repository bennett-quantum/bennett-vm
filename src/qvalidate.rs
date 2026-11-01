//! Interface and borrow-frame checks before identity verification (§3.3, §5).
use crate::object::{FnKind, Function, Instr, Object, Type};
use crate::validate::{err, list_operand};
use std::collections::{BTreeMap, BTreeSet};
type Wire = (String, usize);
pub(crate) fn wires(env: &BTreeMap<String, usize>, s: &str) -> Result<Vec<Wire>, String> {
    let (name, index) = if let Some((n, i)) = s.split_once('[') {
        (
            n,
            Some(
                i.trim_end_matches(']')
                    .parse::<usize>()
                    .map_err(|_| err("3.3", "invalid index"))?,
            ),
        )
    } else {
        (s, None)
    };
    let n = *env
        .get(name)
        .ok_or_else(|| err("3.3", format!("unknown wire {name}")))?;
    if let Some(i) = index {
        if i >= n {
            return Err(err("3.3", "wire index out of range"));
        }
        Ok(vec![(name.into(), i)])
    } else {
        Ok((0..n).map(|i| (name.into(), i)).collect())
    }
}
pub(crate) fn flip(
    ins: &Instr,
    env: &BTreeMap<String, usize>,
) -> Result<(Vec<Wire>, Wire), String> {
    let xs = &ins.operands;
    let (controls, target) = match ins.op.as_str() {
        "x" => (vec![], &xs[0]),
        "cx" => (vec![xs[0].clone()], &xs[1]),
        "ccx" => (xs[..2].to_vec(), &xs[2]),
        "mcx" => (list_operand(&xs[0])?, &xs[1]),
        _ => {
            return Err(err(
                "5.6",
                "borrow frame must have a T3 claim or satisfy the translation lemma (X-family flips only)",
            ));
        }
    };
    let mut cs = vec![];
    for c in controls {
        cs.extend(wires(env, &c)?);
    }
    let t = wires(env, target)?;
    if t.len() != 1 {
        return Err(err("3.3", "gate needs one target wire"));
    }
    Ok((cs, t[0].clone()))
}
fn borrow_frame(
    seq: &[Instr],
    env: &BTreeMap<String, usize>,
    borrowed: &BTreeSet<Wire>,
    entries: &[BTreeMap<Wire, bool>],
) -> Result<(), String> {
    let gates: Vec<_> = seq
        .iter()
        .filter(|i| !matches!(i.op.as_str(), "qalloc" | "qfree" | "qborrow" | "qreturn"))
        .collect();
    let touches = |i: &&Instr| {
        i.operands.iter().any(|s| {
            let names = list_operand(s).unwrap_or_else(|_| vec![s.clone()]);
            names
                .iter()
                .any(|s| wires(env, s).is_ok_and(|ws| ws.iter().any(|w| borrowed.contains(w))))
        })
    };
    let Some(first) = gates.iter().position(touches) else {
        return Ok(());
    };
    let last = gates.iter().rposition(touches).unwrap();
    let seq = &gates[first..=last];
    for vlen in 1..=seq.len() / 2 {
        if seq.len() % 2 != 0 {
            continue;
        }
        let wlen = seq.len() / 2 - vlen;
        if wlen == 0 {
            continue;
        }
        let (v, rest) = seq.split_at(vlen);
        let (w, rest) = rest.split_at(wlen);
        let (vi, w2) = rest.split_at(vlen);
        if !v
            .iter()
            .rev()
            .zip(vi)
            .all(|(a, b)| a.op == b.op && a.operands == b.operands)
            || !w
                .iter()
                .zip(w2)
                .all(|(a, b)| a.op == b.op && a.operands == b.operands)
        {
            continue;
        }
        let mut x = BTreeSet::new();
        let mut ok = true;
        for g in v {
            let Ok((cs, t)) = flip(g, env) else {
                ok = false;
                break;
            };
            ok &= borrowed.contains(&t) && cs.iter().all(|c| !borrowed.contains(c));
            x.extend(cs);
        }
        let mut wc = BTreeSet::new();
        let mut wt = BTreeSet::new();
        for g in w {
            let Ok((cs, t)) = flip(g, env) else {
                ok = false;
                break;
            };
            ok &= !borrowed.contains(&t)
                && !x.contains(&t)
                && cs.iter().filter(|c| borrowed.contains(*c)).count() <= 1;
            wc.extend(cs);
            wt.insert(t);
        }
        ok &= wt.is_disjoint(&wc);
        if ok {
            return Ok(());
        }
    }
    let _ = last;
    exhaustive_frame(
        &seq.iter().map(|i| (*i).clone()).collect::<Vec<_>>(),
        env,
        borrowed,
        &entries[first],
    )
}
/// K76 deliberately has no symbolic Boolean expressions. Causal histories include
/// readers as well as writers: an interleaved pair is not an adjoint pop.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EntryEvent {
    inverse: String,
    before: BTreeMap<Wire, Option<bool>>,
}
#[derive(Default)]
struct EntryState {
    values: BTreeMap<Wire, Option<bool>>,
    locals: BTreeSet<Wire>,
    stacks: BTreeMap<Wire, Vec<usize>>,
    events: BTreeMap<usize, EntryEvent>,
    next: usize,
}
impl EntryState {
    fn invalidate(&mut self, operands: &[Wire]) {
        let mut pending = operands.to_vec();
        let mut seen = BTreeSet::new();
        while let Some(w) = pending.pop() {
            if !seen.insert(w.clone()) {
                continue;
            }
            for id in self.stacks.remove(&w).unwrap_or_default() {
                if let Some(event) = self.events.remove(&id) {
                    pending.extend(event.before.into_keys());
                }
            }
        }
    }
    fn allocate(&mut self, ws: &[Wire], value: Option<bool>) {
        self.invalidate(ws);
        for w in ws {
            self.values.insert(w.clone(), value);
            if value.is_some() {
                self.locals.insert(w.clone());
            }
        }
    }
    fn release(&mut self, ws: &[Wire]) {
        self.invalidate(ws);
        for w in ws {
            self.values.remove(w);
            self.locals.remove(w);
        }
    }
    fn constants(&self) -> BTreeMap<Wire, bool> {
        self.locals
            .iter()
            .filter_map(|w| {
                self.values
                    .get(w)
                    .copied()
                    .flatten()
                    .map(|v| (w.clone(), v))
            })
            .collect()
    }
    fn value(&self, w: &Wire) -> Option<bool> {
        self.values.get(w).copied().flatten()
    }
    fn apply(
        &mut self,
        key: String,
        inverse: String,
        operands: &[Wire],
        controls: &[Wire],
        targets: &[Wire],
        transfer: &str,
    ) {
        if controls.iter().any(|w| self.value(w) == Some(false)) {
            return;
        }
        let operands = operands
            .iter()
            .chain(controls)
            .cloned()
            .collect::<BTreeSet<_>>();
        if let Some(id) = operands
            .iter()
            .next()
            .and_then(|w| self.stacks.get(w)?.last())
            .copied()
            && operands
                .iter()
                .all(|w| self.stacks.get(w).and_then(|s| s.last()) == Some(&id))
            && self
                .events
                .get(&id)
                .is_some_and(|e| e.inverse == key && e.before.keys().eq(operands.iter()))
        {
            let event = self.events.remove(&id).unwrap();
            for (w, v) in event.before {
                self.stacks.get_mut(&w).unwrap().pop();
                self.values.insert(w, v);
            }
            return;
        }
        let before = operands
            .iter()
            .map(|w| (w.clone(), self.value(w)))
            .collect::<BTreeMap<_, _>>();
        let known = before.values().all(Option::is_some);
        if transfer == "swap" && known {
            let a = self.value(&targets[0]);
            let b = self.value(&targets[1]);
            self.values.insert(targets[0].clone(), b);
            self.values.insert(targets[1].clone(), a);
        } else if transfer != "diagonal" {
            for t in targets {
                let v = if transfer == "flip" && known {
                    self.value(t).map(|v| !v)
                } else {
                    None
                };
                self.values.insert(t.clone(), v);
            }
        }
        let id = self.next;
        self.next += 1;
        for w in operands {
            self.stacks.entry(w).or_default().push(id);
        }
        self.events.insert(id, EntryEvent { inverse, before });
    }
}
struct LifetimeFrame {
    borrowed: BTreeSet<Wire>,
    gates: Vec<Instr>,
    entries: Vec<BTreeMap<Wire, bool>>,
}
impl LifetimeFrame {
    fn flush(&mut self, env: &BTreeMap<String, usize>) -> Result<(), String> {
        borrow_frame(&self.gates, env, &self.borrowed, &self.entries)?;
        self.gates.clear();
        self.entries.clear();
        Ok(())
    }
}
struct FrameAnalysis<'a> {
    obj: &'a Object,
    env: BTreeMap<String, usize>,
    entry: EntryState,
    borrowed: BTreeSet<Wire>,
    frames: BTreeMap<String, LifetimeFrame>,
    namespace: usize,
}
impl<'a> FrameAnalysis<'a> {
    fn new(
        obj: &'a Object,
        f: &Function,
        mut env: BTreeMap<String, usize>,
    ) -> Result<Self, String> {
        for p in &f.lists[2] {
            if let Type::Region(a, w) = p.ty {
                env.insert(p.name.clone(), region_bits(a, w)?);
            }
        }
        let mut entry = EntryState::default();
        for p in f.lists.iter().flatten() {
            if env.contains_key(&p.name) {
                for w in wires(&env, &p.name)? {
                    entry.values.insert(w, None);
                }
            }
        }
        Ok(Self {
            obj,
            env,
            entry,
            borrowed: BTreeSet::new(),
            frames: BTreeMap::new(),
            namespace: 0,
        })
    }
    fn flush(&mut self) -> Result<(), String> {
        for frame in self.frames.values_mut() {
            frame.flush(&self.env)?;
        }
        Ok(())
    }
    fn gate(
        &mut self,
        i: Instr,
        controls: Vec<Wire>,
        targets: Vec<Wire>,
        transfer: &str,
        identity: &str,
    ) {
        let all = controls.iter().chain(&targets).cloned().collect::<Vec<_>>();
        let key = format!("{identity}:{:?}:{controls:?}", i.operands);
        let constants = self.entry.constants();
        for frame in self.frames.values_mut() {
            frame.entries.push(constants.clone());
            frame.gates.push(i.clone());
        }
        self.entry
            .apply(key.clone(), key, &all, &controls, &targets, transfer);
    }
    fn flip(&mut self, controls: Vec<Wire>, target: Wire, line: usize) {
        let mut controls = controls;
        controls.sort();
        controls.dedup();
        let i = Instr {
            op: "mcx".into(),
            operands: vec![
                format!(
                    "({})",
                    controls
                        .iter()
                        .map(wire_token)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                wire_token(&target),
            ],
            lhs: vec![],
            nested: vec![],
            line,
            raw: String::new(),
        };
        self.gate(i, controls, vec![target], "flip", "x-family");
    }
    fn sequence(
        &mut self,
        xs: &[Instr],
        inherited: &[Wire],
        calls: &mut Vec<String>,
    ) -> Result<(), String> {
        let mut pred = inherited.to_vec();
        for i in xs {
            match i.op.as_str() {
                "qpred" => pred.extend(wires(&self.env, &i.operands[0])?),
                "qunpred" => {
                    let n = wires(&self.env, &i.operands[0])?.len();
                    pred.truncate(pred.len() - n);
                }
                "qif" => {
                    self.flush()?;
                    let p = wires(&self.env, &i.operands[0])?;
                    let mut ctx = pred.clone();
                    ctx.extend(p.clone());
                    self.sequence(&i.nested[0], &ctx, calls)?;
                    self.flush()?;
                    self.flip(vec![], p[0].clone(), i.line);
                    self.flush()?;
                    self.sequence(&i.nested[1], &ctx, calls)?;
                    self.flush()?;
                    self.flip(vec![], p[0].clone(), i.line);
                    self.flush()?;
                }
                "qloop" | "qloop_adj" => {
                    self.flush()?;
                    let n = i.operands[0]
                        .parse::<usize>()
                        .map_err(|_| err("5.6", "unresolved loop count"))?;
                    let selectors = if i.operands.len() == 3 {
                        wires(&self.env, &i.operands[2])?
                    } else {
                        vec![]
                    };
                    let mut flags = vec![];
                    if !selectors.is_empty() {
                        self.namespace += 1;
                        let name = format!("$flags{}", self.namespace);
                        self.env.insert(name.clone(), n);
                        flags = wires(&self.env, &name)?;
                        self.entry.allocate(&flags, Some(false));
                        for j in 0..n {
                            let mut cs = vec![selectors[j].clone()];
                            if j > 0 {
                                cs.push(flags[j - 1].clone());
                            }
                            self.flip(cs, flags[j].clone(), i.line);
                        }
                        self.flush()?;
                    }
                    let visits: Vec<_> = if i.op == "qloop_adj" {
                        (0..n).rev().collect()
                    } else {
                        (0..n).collect()
                    };
                    for j in visits {
                        let mut ctx = pred.clone();
                        if !flags.is_empty() {
                            ctx.push(flags[j].clone());
                        }
                        self.sequence(&i.nested[0], &ctx, calls)?;
                        self.flush()?;
                    }
                    for j in (0..flags.len()).rev() {
                        let mut cs = vec![selectors[j].clone()];
                        if j > 0 {
                            cs.push(flags[j - 1].clone());
                        }
                        self.flip(cs, flags[j].clone(), i.line);
                    }
                    self.flush()?;
                    self.entry.release(&flags);
                }
                "qalloc" | "qborrow" => {
                    let n = i.operands[1]
                        .parse()
                        .map_err(|_| err("5.6", "unresolved allocation width"))?;
                    self.env.insert(i.lhs[0].clone(), n);
                    let ws = wires(&self.env, &i.lhs[0])?;
                    self.entry
                        .allocate(&ws, if i.op == "qalloc" { Some(false) } else { None });
                    if i.op == "qborrow" {
                        self.borrowed.extend(ws.clone());
                        self.frames.insert(
                            i.lhs[0].clone(),
                            LifetimeFrame {
                                borrowed: ws.into_iter().collect(),
                                gates: vec![],
                                entries: vec![],
                            },
                        );
                    }
                }
                "qfree" | "qreturn" => {
                    if i.op == "qreturn" {
                        self.frames
                            .remove(&i.operands[0])
                            .ok_or_else(|| err("5.6", "missing borrowed lifetime"))?
                            .flush(&self.env)?;
                    }
                    let ws = wires(&self.env, &i.operands[0])?;
                    self.entry.release(&ws);
                    for w in ws {
                        self.borrowed.remove(&w);
                    }
                }
                "x" | "cx" | "ccx" | "mcx" => {
                    let (mut cs, t) = flip(i, &self.env)?;
                    cs.extend(pred.clone());
                    self.flip(cs, t, i.line);
                }
                "swap" | "qexch" => {
                    let a = wires(&self.env, &i.operands[0])?;
                    let b = wires(&self.env, &i.operands[1])?;
                    for (a, b) in a.into_iter().zip(b) {
                        if pred.is_empty() {
                            let mut g = i.clone();
                            g.op = "swap".into();
                            g.operands = vec![wire_token(&a), wire_token(&b)];
                            self.gate(g, vec![], vec![a, b], "swap", "swap");
                        } else {
                            let mut g = i.clone();
                            g.op = "cswap".into();
                            g.operands = vec![
                                format!(
                                    "({})",
                                    pred.iter().map(wire_token).collect::<Vec<_>>().join(", ")
                                ),
                                wire_token(&a),
                                wire_token(&b),
                            ];
                            self.gate(g, pred.clone(), vec![a, b], "swap", "swap");
                        }
                    }
                }
                "qcopy" | "quncopy" | "qxor" => {
                    let a = wires(&self.env, &i.operands[0])?;
                    let dest = if i.op == "qcopy" {
                        &i.lhs[0]
                    } else {
                        &i.operands[1]
                    };
                    if i.op == "qcopy" {
                        self.env.insert(dest.clone(), a.len());
                        let ws = wires(&self.env, dest)?;
                        self.entry.allocate(&ws, Some(false));
                    }
                    let b = wires(&self.env, dest)?;
                    for (a, b) in a.into_iter().zip(&b) {
                        let mut cs = pred.clone();
                        cs.push(a);
                        self.flip(cs, b.clone(), i.line);
                    }
                    if i.op == "quncopy" {
                        self.entry.release(&b);
                    }
                }
                "action_call" | "oracle_call" | "adj" | "ctl" => {
                    let (adj, ctrl, op, args) = crate::action::normalize_call(i)?;
                    let mut ctx = pred.clone();
                    if let Some(ctrl) = ctrl {
                        ctx.extend(wires(&self.env, ctrl)?);
                    }
                    if op == "action_call" {
                        self.local_call(&args, adj, &ctx, calls)?;
                    } else {
                        self.flush()?;
                        let mut normalized = i.clone();
                        normalized.op = op.into();
                        normalized.operands = args.clone();
                        let targets =
                            [wires(&self.env, &args[3])?, wires(&self.env, &args[4])?].concat();
                        self.opaque(&normalized, &ctx, targets, None)?;
                    }
                }
                "qinplace" => {
                    self.flush()?;
                    let targets = wires(&self.env, &i.operands[3])?;
                    let mut inv = i.clone();
                    inv.operands.swap(0, 1);
                    self.opaque(i, &pred, targets, Some(inv))?;
                }
                op if crate::action::is_gate_name(op) => {
                    self.flush()?;
                    let start = usize::from(matches!(op, "rx" | "ry" | "rz"));
                    let targets = i.operands[start..]
                        .iter()
                        .map(|s| wires(&self.env, s))
                        .collect::<Result<Vec<_>, _>>()?
                        .concat();
                    let inverse = match op {
                        "s" => "sdg",
                        "sdg" => "s",
                        "t" => "tdg",
                        "tdg" => "t",
                        _ => op,
                    };
                    let mut inv = i.clone();
                    inv.op = inverse.into();
                    if start == 1 {
                        inv.operands[0] = if let Some(v) = i.operands[0].strip_prefix('-') {
                            v.into()
                        } else {
                            format!("-{}", i.operands[0])
                        };
                    }
                    self.check_borrowed(&targets)?;
                    let key = event_key(i, &pred);
                    let all = [targets.clone(), pred.clone()].concat();
                    self.entry.apply(
                        key,
                        event_key(&inv, &pred),
                        &all,
                        &pred,
                        &targets,
                        if matches!(op, "z" | "s" | "sdg" | "t" | "tdg" | "cz" | "rz") {
                            "diagonal"
                        } else {
                            "unknown"
                        },
                    );
                }
                "qload" | "qunload" => {
                    self.flush()?;
                    let region = wires(&self.env, &i.operands[0])?;
                    let width = self
                        .obj
                        .functions
                        .iter()
                        .flat_map(|f| &f.lists[2])
                        .find_map(|p| {
                            if p.name == i.operands[0] {
                                if let Type::Region(_, w) = p.ty {
                                    Some(w)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        });
                    let target = if i.op == "qload" {
                        &i.lhs[0]
                    } else {
                        &i.operands[2]
                    };
                    if i.op == "qload" {
                        let n = self
                            .env
                            .get(target)
                            .copied()
                            .or(width)
                            .ok_or_else(|| err("5.6", "unknown load width"))?;
                        self.env.insert(target.clone(), n);
                        self.entry.allocate(&wires(&self.env, target)?, Some(false));
                    }
                    let targets = wires(&self.env, target)?;
                    let all = [
                        region,
                        wires(&self.env, &i.operands[1])?,
                        targets.clone(),
                        pred.clone(),
                    ]
                    .concat();
                    self.check_borrowed(&all)?;
                    self.entry.apply(
                        event_key(i, &pred),
                        event_key(i, &pred),
                        &all,
                        &pred,
                        &targets,
                        "unknown",
                    );
                    if i.op == "qunload" {
                        self.entry.release(&targets);
                    }
                }
                "qregion" => {
                    self.flush()?;
                    let a = i.operands[1]
                        .parse()
                        .map_err(|_| err("5.6", "unresolved region address width"))?;
                    let w = i.operands[2]
                        .parse()
                        .map_err(|_| err("5.6", "unresolved region cell width"))?;
                    let n = region_bits(a, w)?;
                    self.env.insert(i.operands[0].clone(), n);
                    let ws = wires(&self.env, &i.operands[0])?;
                    self.entry.allocate(&ws, Some(false));
                    if let Some(image) = i.operands.get(3) {
                        let data = crate::sha256::from_hex(
                            self.obj
                                .data
                                .get(image.trim_start_matches('@'))
                                .ok_or_else(|| err("5.6", "unknown image"))?,
                        )?;
                        for (j, bit) in ws.iter().enumerate() {
                            if data[j / 8] & (1 << (j % 8)) != 0 {
                                self.flip(vec![], bit.clone(), i.line);
                            }
                        }
                        self.flush()?;
                    }
                }
                "qregion_release" => {
                    self.flush()?;
                    self.entry.release(&wires(&self.env, &i.operands[0])?);
                }
                _ => {
                    self.flush()?;
                }
            }
        }
        Ok(())
    }
    fn check_borrowed(&self, ws: &[Wire]) -> Result<(), String> {
        if ws.iter().any(|w| self.borrowed.contains(w)) {
            Err(err("5.6", "borrowed operand outside a permutation frame"))
        } else {
            Ok(())
        }
    }
    fn opaque(
        &mut self,
        i: &Instr,
        pred: &[Wire],
        targets: Vec<Wire>,
        inverse: Option<Instr>,
    ) -> Result<(), String> {
        let mut all = pred.to_vec();
        for operand in &i.operands {
            for name in list_operand(operand).unwrap_or_else(|_| vec![operand.clone()]) {
                if let Ok(ws) = wires(&self.env, &name) {
                    all.extend(ws);
                }
            }
        }
        self.check_borrowed(&all)?;
        self.entry.apply(
            event_key(i, pred),
            event_key(inverse.as_ref().unwrap_or(i), pred),
            &all,
            pred,
            &targets,
            "unknown",
        );
        Ok(())
    }
    fn local_call(
        &mut self,
        args: &[String],
        adj: bool,
        pred: &[Wire],
        calls: &mut Vec<String>,
    ) -> Result<(), String> {
        let name = args[0].trim_start_matches('@');
        if calls.iter().any(|n| n == name) {
            return Err(err("5.6", "recursive action expansion"));
        }
        let Some(f) = self.obj.function(name) else {
            let mut touched = vec![];
            let mut modified = vec![];
            for (group, arg) in args[1..].iter().enumerate() {
                for s in list_operand(arg)? {
                    if let Ok(ws) = wires(&self.env, &s) {
                        touched.extend(ws.clone());
                        if group == 0 || group == 2 {
                            modified.extend(ws);
                        }
                    }
                }
            }
            touched.extend_from_slice(pred);
            self.check_borrowed(&touched)?;
            if !pred.iter().any(|w| self.entry.value(w) == Some(false)) {
                self.entry.invalidate(&touched);
                for w in modified {
                    self.entry.values.insert(w, None);
                }
            }
            return Ok(());
        };
        self.namespace += 1;
        let prefix = format!("$call{}:", self.namespace);
        let mut names = BTreeMap::new();
        for (formal, actual) in f.lists[..4].iter().zip(&args[1..]) {
            for (p, a) in formal.iter().zip(list_operand(actual)?) {
                if matches!(p.ty, Type::Wire(_)) {
                    for (j, w) in wires(&self.env, &a)?.iter().enumerate() {
                        names.insert(format!("{}[{j}]", p.name), wire_token(w));
                    }
                }
                names.insert(p.name.clone(), a);
            }
        }
        let mut flat = vec![];
        f.flat_instrs(&mut flat);
        for i in flat {
            for n in &i.lhs {
                names.insert(n.clone(), format!("{prefix}{n}"));
            }
            if i.op == "qregion" {
                names.insert(i.operands[0].clone(), format!("{prefix}{}", i.operands[0]));
            }
        }
        fn rename(i: &Instr, names: &BTreeMap<String, String>) -> Instr {
            let mut out = i.clone();
            out.lhs = i.lhs.iter().map(|s| substitute(s, names)).collect();
            out.operands = i.operands.iter().map(|s| substitute(s, names)).collect();
            out.nested = i
                .nested
                .iter()
                .map(|b| b.iter().map(|i| rename(i, names)).collect())
                .collect();
            out
        }
        let body = f
            .blocks
            .iter()
            .flat_map(|b| b.instrs.clone())
            .collect::<Vec<_>>();
        let body = if adj {
            crate::action::adjoint_list(&body, &crate::action::bundle_widths(self.obj, f))
        } else {
            body
        };
        let body = body.iter().map(|i| rename(i, &names)).collect::<Vec<_>>();
        calls.push(name.into());
        self.sequence(&body, pred, calls)?;
        calls.pop();
        Ok(())
    }
}
fn region_bits(a: usize, w: usize) -> Result<usize, String> {
    u32::try_from(a)
        .ok()
        .and_then(|a| 1usize.checked_shl(a))
        .and_then(|n| n.checked_mul(w))
        .ok_or_else(|| err("5.6", "host limit: region entry analysis width"))
}
fn wire_token(w: &Wire) -> String {
    format!("{}[{}]", w.0, w.1)
}
fn event_key(i: &Instr, pred: &[Wire]) -> String {
    format!("{}:{:?}:{pred:?}", i.op, i.operands)
}
fn substitute(s: &str, names: &BTreeMap<String, String>) -> String {
    if s.starts_with('@') {
        return s.into();
    }
    let mut out = String::new();
    let mut token = String::new();
    for c in s.chars().chain(std::iter::once(' ')) {
        if c.is_alphanumeric() || matches!(c, '_' | '$' | '.' | '[' | ']') {
            token.push(c);
        } else {
            out.push_str(
                names
                    .get(&token)
                    .cloned()
                    .or_else(|| {
                        token
                            .split_once('[')
                            .and_then(|(n, index)| names.get(n).map(|v| format!("{v}[{index}")))
                    })
                    .unwrap_or_else(|| token.clone())
                    .as_str(),
            );
            token.clear();
            out.push(c);
        }
    }
    out.pop();
    out
}

pub fn validate(obj: &Object, f: &Function) -> Result<(), String> {
    if matches!(f.kind, FnKind::Oracle | FnKind::Lproc) {
        let mut widths: BTreeMap<String, usize> =
            crate::oracle::location_widths(f).into_iter().collect();
        for p in f.lists.iter().flatten() {
            if let Some(n) = p.ty.width() {
                widths.insert(p.name.clone(), n);
            }
        }
        let mut flat = vec![];
        f.flat_instrs(&mut flat);
        for i in flat {
            let op =
                i.op.strip_prefix("lift.")
                    .unwrap_or(&i.op)
                    .split('.')
                    .next()
                    .unwrap();
            if matches!(
                op,
                "add"
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
            ) {
                let a = widths.get(&i.operands[0]);
                let b = widths.get(&i.operands[1]);
                if a.is_none() || a != b {
                    return Err(err(
                        "3.3",
                        "lifted binop/icmp operands must have equal width",
                    ));
                }
            }
        }
        return Ok(());
    }
    if !matches!(f.kind, FnKind::Action | FnKind::PermAction) {
        return Ok(());
    }
    let mut env = BTreeMap::new();
    for p in f.lists.iter().flatten() {
        if let Type::Wire(n) = p.ty {
            env.insert(p.name.clone(), n);
        }
    }
    let inputs: BTreeSet<_> = f.lists[1].iter().map(|p| p.name.clone()).collect();
    let mut borrowed = BTreeSet::new();
    let mut flat = vec![];
    f.flat_instrs(&mut flat);
    for i in &flat {
        if matches!(i.op.as_str(), "qalloc" | "qborrow") {
            let n = i.operands[1]
                .parse()
                .map_err(|_| err("3.3", "invalid allocation width"))?;
            env.insert(i.lhs[0].clone(), n);
            if i.op == "qborrow" {
                borrowed.extend(wires(&env, &i.lhs[0])?);
            }
        } else if i.op == "qload" {
            let mut region = None;
            for p in f.lists[2].iter() {
                if p.name == i.operands[0]
                    && let Type::Region(_, w) = p.ty
                {
                    region = Some(w);
                }
            }
            for q in &flat {
                if q.op == "qregion" && q.operands[0] == i.operands[0] {
                    region = q.operands[2].parse().ok();
                }
            }
            env.insert(
                i.lhs[0].clone(),
                region.ok_or_else(|| err("3.3", "unknown region"))?,
            );
        } else if i.op == "qcopy" {
            let n = wires(&env, &i.operands[0])?.len();
            env.insert(i.lhs[0].clone(), n);
        }
    }
    fn walk(
        xs: &[Instr],
        env: &BTreeMap<String, usize>,
        inputs: &BTreeSet<String>,
        pred: &mut Vec<Wire>,
    ) -> Result<(), String> {
        for i in xs {
            let op = i.op.as_str();
            if op == "qpred" {
                pred.extend(wires(env, &i.operands[0])?);
                continue;
            }
            if op == "qunpred" {
                let w = wires(env, &i.operands[0])?;
                if !pred.ends_with(&w) {
                    return Err(err("3.3", "predicate stack mismatch"));
                }
                pred.truncate(pred.len() - w.len());
                continue;
            }
            if matches!(op, "qif" | "qloop") {
                let mut p = pred.clone();
                if op == "qif" {
                    p.extend(wires(env, &i.operands[0])?);
                } else if i.operands.len() == 3 {
                    p.extend(wires(env, &i.operands[2])?);
                }
                for body in &i.nested {
                    walk(body, env, inputs, &mut p.clone())?;
                }
                continue;
            }
            let (all, targets, diagonal) = if crate::action::is_gate_name(op) {
                if matches!(op, "x" | "cx" | "ccx" | "mcx") {
                    let (cs, t) = flip(i, env)?;
                    let mut all = cs;
                    all.push(t.clone());
                    (all, vec![t], false)
                } else {
                    let start = usize::from(matches!(op, "rx" | "ry" | "rz"));
                    let mut ws = vec![];
                    for s in &i.operands[start..] {
                        ws.extend(wires(env, s)?);
                    }
                    (
                        ws.clone(),
                        ws,
                        matches!(op, "s" | "sdg" | "t" | "tdg" | "cz" | "rz"),
                    )
                }
            } else if matches!(op, "qxor" | "qexch" | "quncopy") {
                let a = wires(env, &i.operands[0])?;
                let b = wires(env, &i.operands[1])?;
                if a.len() != b.len() {
                    return Err(err("3.3", "bundle widths differ"));
                }
                let targets = if op == "qexch" {
                    [a.clone(), b.clone()].concat()
                } else {
                    b.clone()
                };
                ([a, b].concat(), targets, false)
            } else {
                continue;
            };
            if all.iter().collect::<BTreeSet<_>>().len() != all.len()
                || targets.iter().any(|w| pred.contains(w))
            {
                return Err(err(
                    "3.3",
                    "wire operands and effective controls must be disjoint from targets",
                ));
            }
            if !diagonal && targets.iter().any(|(n, _)| inputs.contains(n)) {
                return Err(err("5.1", "in wires may not be non-diagonal targets"));
            }
        }
        Ok(())
    }
    let body = f
        .blocks
        .iter()
        .flat_map(|b| b.instrs.clone())
        .collect::<Vec<_>>();
    walk(&body, &env, &inputs, &mut vec![])?;
    if !borrowed.is_empty() {
        let mut analysis = FrameAnalysis::new(obj, f, env)?;
        analysis.sequence(&body, &[], &mut vec![f.name.clone()])?;
        analysis.flush()?;
    }
    Ok(())
}

/// T3 over the normative Cartesian entry domain D_F. Structural proof is the fast path;
/// there is no arbitrary fixed width ceiling on the exhaustive alternative.
fn exhaustive_frame(
    seq: &[Instr],
    env: &BTreeMap<String, usize>,
    borrowed: &BTreeSet<Wire>,
    constants: &BTreeMap<Wire, bool>,
) -> Result<(), String> {
    use crate::bigint::BigUint;
    enum Gate {
        Flip(Vec<usize>, usize),
        Swap(Vec<usize>, usize, usize),
    }
    let mut touched = BTreeSet::new();
    for i in seq {
        for x in &i.operands {
            for name in list_operand(x).unwrap_or_else(|_| vec![x.clone()]) {
                if let Ok(ws) = wires(env, &name) {
                    touched.extend(ws);
                }
            }
        }
    }
    let ws = touched.into_iter().collect::<Vec<_>>();
    let id = |w: &Wire| ws.iter().position(|x| x == w).unwrap();
    let mut gates = vec![];
    for i in seq {
        if matches!(i.op.as_str(), "swap" | "qexch" | "cswap") {
            let offset = usize::from(i.op == "cswap");
            let controls = if offset == 1 {
                list_operand(&i.operands[0])?
                    .iter()
                    .map(|s| wires(env, s))
                    .collect::<Result<Vec<_>, _>>()?
                    .concat()
            } else {
                vec![]
            };
            let a = wires(env, &i.operands[offset])?;
            let b = wires(env, &i.operands[offset + 1])?;
            for (a, b) in a.iter().zip(&b) {
                gates.push(Gate::Swap(controls.iter().map(id).collect(), id(a), id(b)));
            }
        } else if i.op == "qxor" {
            let a = wires(env, &i.operands[0])?;
            let b = wires(env, &i.operands[1])?;
            for (a, b) in a.iter().zip(&b) {
                gates.push(Gate::Flip(vec![id(a)], id(b)));
            }
        } else {
            let (cs, t) = flip(i, env)?;
            gates.push(Gate::Flip(cs.iter().map(id).collect(), id(&t)));
        }
    }
    let bs = ws
        .iter()
        .enumerate()
        .filter(|(_, w)| borrowed.contains(*w))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let xs = (0..ws.len())
        .filter(|i| !bs.contains(i) && !constants.contains_key(&ws[*i]))
        .collect::<Vec<_>>();
    let mut x = BigUint::zero();
    let xend = BigUint::from_u64(1).shl(xs.len());
    let bend = BigUint::from_u64(1).shl(bs.len());
    while x < xend {
        let mut b = BigUint::zero();
        let mut expected = None;
        while b < bend {
            let mut state = BigUint::zero();
            for (j, w) in ws.iter().enumerate() {
                if let Some(value) = constants.get(w) {
                    state.set_bit(j, *value);
                }
            }
            for (j, w) in xs.iter().enumerate() {
                state.set_bit(*w, x.bit(j));
            }
            for (j, w) in bs.iter().enumerate() {
                state.set_bit(*w, b.bit(j));
            }
            for g in &gates {
                match g {
                    Gate::Flip(cs, t) => {
                        if cs.iter().all(|c| state.bit(*c)) {
                            let v = !state.bit(*t);
                            state.set_bit(*t, v);
                        }
                    }
                    Gate::Swap(cs, a, b) => {
                        if !cs.iter().all(|c| state.bit(*c)) {
                            continue;
                        }
                        let (aa, bb) = (state.bit(*a), state.bit(*b));
                        state.set_bit(*a, bb);
                        state.set_bit(*b, aa);
                    }
                }
            }
            for (j, w) in bs.iter().enumerate() {
                if state.bit(*w) != b.bit(j) {
                    return Err(err("5.6", "T3 frame proof: borrowed bit not restored"));
                }
                state.set_bit(*w, false);
            }
            if let Some(e) = &expected {
                if e != &state {
                    return Err(err(
                        "5.6",
                        "T3 frame proof: declared output depends on borrowed entry",
                    ));
                }
            } else {
                expected = Some(state);
            }
            b = b.add(&BigUint::from_u64(1));
        }
        x = x.add(&BigUint::from_u64(1));
    }
    Ok(())
}

/// §12.1 translation witness: check the named object frame, never only the
/// producer's V/W lists. Point ordinals follow nested textual order (§9.2).
pub(crate) fn translation_witness(
    obj: &Object,
    name: &str,
    w: &crate::json::Json,
) -> Result<(), String> {
    use crate::json::Json;
    let fail = || {
        err(
            "12.1",
            "translation_lemma witness must name the complete V; W; V^-1; W frame and satisfy §5.6 (i)-(iv)",
        )
    };
    let f = obj.function(name).ok_or_else(fail)?;
    let mut env = BTreeMap::new();
    let mut bs = BTreeSet::new();
    let mut points = vec![];
    for p in f.lists.iter().flatten() {
        if let Some(n) = p.ty.width() {
            env.insert(p.name.clone(), n);
        }
    }
    for b in &f.blocks {
        fn flatten<'a>(xs: &'a [Instr], out: &mut Vec<&'a Instr>) {
            for i in xs {
                out.push(i);
                for body in &i.nested {
                    flatten(body, out);
                }
            }
        }
        let mut flat = vec![];
        flatten(&b.instrs, &mut flat);
        for (ordinal, i) in flat.into_iter().enumerate() {
            if matches!(i.op.as_str(), "qalloc" | "qborrow") {
                env.insert(i.lhs[0].clone(), i.operands[1].parse().map_err(|_| fail())?);
                if i.op == "qborrow" {
                    bs.extend(wires(&env, &i.lhs[0])?);
                }
            }
            points.push((format!("{}:{ordinal}", b.name), i));
        }
    }
    fn contexts(
        xs: &[Instr],
        env: &BTreeMap<String, usize>,
        pred: &mut Vec<Wire>,
        out: &mut BTreeMap<usize, Vec<Wire>>,
    ) -> Result<(), String> {
        for i in xs {
            out.insert(i.line, pred.clone());
            if i.op == "qpred" {
                pred.extend(wires(env, &i.operands[0])?);
            } else if i.op == "qunpred" {
                let n = wires(env, &i.operands[0])?.len();
                pred.truncate(pred.len() - n);
            } else if i.op == "qif" {
                for body in &i.nested {
                    let mut p = pred.clone();
                    p.extend(wires(env, &i.operands[0])?);
                    contexts(body, env, &mut p, out)?;
                }
            } else {
                for body in &i.nested {
                    let mut p = pred.clone();
                    if i.op == "qloop" && i.operands.len() == 3 {
                        p.extend(wires(env, &i.operands[2])?);
                    }
                    contexts(body, env, &mut p, out)?;
                }
            }
        }
        Ok(())
    }
    let mut predicates = BTreeMap::new();
    for b in &f.blocks {
        contexts(&b.instrs, &env, &mut vec![], &mut predicates)?;
    }
    let effective_flip = |i: &Instr| -> Result<(Vec<Wire>, Wire), String> {
        let (mut cs, t) = flip(i, &env)?;
        cs.extend(predicates.get(&i.line).cloned().unwrap_or_default());
        Ok((cs, t))
    };
    let interface = crate::identity::compute(obj)?.interface[name].clone();
    let point_list = |key: &str| -> Result<Vec<String>, String> {
        w.get(key)
            .and_then(Json::as_arr)
            .ok_or_else(fail)?
            .iter()
            .map(|p| {
                let (hash, block, ordinal) = crate::certificate::program_point(p)?;
                if hash != interface {
                    return Err(fail());
                }
                Ok(format!("{block}:{ordinal}"))
            })
            .collect()
    };
    let frame = point_list("frame")?;
    if frame.len() != 2 {
        return Err(fail());
    }
    let first = points
        .iter()
        .position(|(p, _)| p == &frame[0])
        .ok_or_else(fail)?;
    let last = points
        .iter()
        .position(|(p, _)| p == &frame[1])
        .ok_or_else(fail)?;
    if first > last {
        return Err(fail());
    }
    let v = point_list("V")?;
    let ww = point_list("W")?;
    if v.is_empty() || ww.is_empty() {
        return Err(fail());
    }
    let frame_seq = &points[first..=last];
    if frame_seq.len() != 2 * (v.len() + ww.len()) {
        return Err(fail());
    }
    let refs = |ps: &[String]| -> Result<Vec<&Instr>, String> {
        ps.iter()
            .map(|p| {
                points
                    .iter()
                    .find(|(q, _)| p == q)
                    .map(|(_, i)| *i)
                    .ok_or_else(fail)
            })
            .collect()
    };
    let vg = refs(&v)?;
    let wg = refs(&ww)?;
    if frame_seq[..v.len()].iter().map(|(p, _)| p).ne(v.iter())
        || frame_seq[v.len()..v.len() + ww.len()]
            .iter()
            .map(|(p, _)| p)
            .ne(ww.iter())
    {
        return Err(fail());
    }
    let want = vg.iter().chain(&wg).chain(vg.iter().rev()).chain(&wg);
    for ((_, actual), wanted) in frame_seq.iter().zip(want) {
        if actual.op != wanted.op
            || actual.operands != wanted.operands
            || !actual.nested.is_empty()
            || effective_flip(actual)? != effective_flip(wanted)?
        {
            return Err(fail());
        }
    }
    let wire_set = |key: &str| -> Result<BTreeSet<Wire>, String> {
        let mut out = BTreeSet::new();
        for p in w.get(key).and_then(Json::as_arr).ok_or_else(fail)? {
            out.extend(wires(&env, p.as_str().ok_or_else(fail)?)?);
        }
        Ok(out)
    };
    let b = wire_set("B")?;
    let x = wire_set("X")?;
    if b.is_empty() || !b.is_subset(&bs) || !x.is_disjoint(&bs) {
        return Err(fail());
    }
    let mut targets = BTreeSet::new();
    let mut controls = BTreeSet::new();
    for g in vg {
        let (cs, t) = effective_flip(g)?;
        if !b.contains(&t) || !cs.iter().all(|c| x.contains(c)) {
            return Err(fail());
        }
    }
    for g in wg {
        let (cs, t) = effective_flip(g)?;
        if b.contains(&t) || x.contains(&t) || cs.iter().filter(|c| b.contains(*c)).count() > 1 {
            return Err(fail());
        }
        targets.insert(t);
        controls.extend(cs);
    }
    if !targets.is_disjoint(&controls) {
        return Err(fail());
    }
    // §5.6: first through last touch of B within a contiguous permutation
    // segment. Adjacent gates that never touch B do not extend that span.
    let permutation = |i: &Instr| {
        matches!(
            i.op.as_str(),
            "x" | "cx" | "ccx" | "mcx" | "swap" | "qxor" | "qexch"
        )
    };
    let touches = |i: &Instr| {
        i.operands
            .iter()
            .any(|s| wires(&env, s).is_ok_and(|ws| ws.iter().any(|w| b.contains(w))))
    };
    if !touches(points[first].1) || !touches(points[last].1) {
        return Err(fail());
    }
    for (_, i) in points[..first]
        .iter()
        .rev()
        .take_while(|(_, i)| permutation(i))
        .chain(
            points[last + 1..]
                .iter()
                .take_while(|(_, i)| permutation(i)),
        )
    {
        if touches(i) {
            return Err(fail());
        }
    }
    Ok(())
}
