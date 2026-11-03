//! ISA 0.9 coherent source operands, before abstract frame analysis or execution.
use crate::{
    object::{FnKind, Function, Instr, Object, Type},
    validate::{action_sig, err, list_operand, oracle_sig},
};
use std::collections::{BTreeMap, BTreeSet};
type Wire = (String, usize);
fn require(ok: bool, why: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(err("3.3", why)) }
}
#[derive(Clone)]
struct Schema {
    wires: BTreeMap<String, (usize, bool)>,
    regions: BTreeMap<String, (usize, usize, bool)>,
    args: BTreeMap<String, Type>,
    locals: BTreeMap<String, bool>,
}
impl Schema {
    fn allocate(&mut self, name: &str, width: usize, borrowed: bool) -> Result<(), String> {
        require(width > 0, "zero-width local wire")?;
        require(
            !self.wires.contains_key(name)
                && !self.regions.contains_key(name)
                && !self.args.contains_key(name),
            "live local name reused",
        )?;
        self.wires.insert(name.into(), (width, true));
        self.locals.insert(name.into(), borrowed);
        Ok(())
    }
    fn release(&mut self, name: &str, borrowed: bool, pred: &[Wire]) -> Result<(), String> {
        require(
            self.locals.get(name) == Some(&borrowed),
            "release requires an owned local of matching allocation kind",
        )?;
        let wires = self.bundle(name, self.width(name)?, true)?;
        self.check_aliases(&wires, &wires, pred)?;
        self.locals.remove(name);
        self.wires.remove(name);
        Ok(())
    }
    fn bundle(&self, name: &str, width: usize, modified: bool) -> Result<Vec<Wire>, String> {
        // §4.6 admits () only as the zero-bit address of region[0,W].
        // No declared wire/value type has width zero.
        if name == "()" && width == 0 {
            return Ok(vec![]);
        }
        let (name, index) = if let Some((base, bit)) = name.split_once('[') {
            (
                base,
                Some(
                    bit.strip_suffix(']')
                        .ok_or_else(|| err("3.3", "malformed wire index"))?
                        .parse::<usize>()
                        .map_err(|_| err("3.3", "invalid wire index"))?,
                ),
            )
        } else {
            (name, None)
        };
        let (n, writable) = self
            .wires
            .get(name)
            .ok_or_else(|| err("3.3", format!("unknown wire bundle {name:?}")))?;
        require(
            !modified || *writable,
            "in wire passed as a modified call operand",
        )?;
        if let Some(i) = index {
            require(i < *n && width == 1, "call wire index/width mismatch")?;
            Ok(vec![(name.into(), i)])
        } else {
            require(*n == width, "call wire width mismatch")?;
            Ok((0..*n).map(|i| (name.into(), i)).collect())
        }
    }
    fn width(&self, name: &str) -> Result<usize, String> {
        if let Some((base, index)) = name.split_once('[') {
            let n = self
                .wires
                .get(base)
                .ok_or_else(|| err("3.3", "unknown wire"))?
                .0;
            let i = index
                .trim_end_matches(']')
                .parse::<usize>()
                .map_err(|_| err("3.3", "invalid wire index"))?;
            require(i < n, "wire index out of range")?;
            Ok(1)
        } else {
            self.wires
                .get(name)
                .map(|p| p.0)
                .ok_or_else(|| err("3.3", "unknown wire bundle"))
        }
    }
    fn bundles(&self, names: &str, widths: &[usize], modified: bool) -> Result<Vec<Wire>, String> {
        let names = list_operand(names)?;
        require(names.len() == widths.len(), "call wire slot count mismatch")?;
        names
            .iter()
            .zip(widths)
            .map(|(n, w)| self.bundle(n, *w, modified))
            .collect::<Result<Vec<_>, _>>()
            .map(|b| b.concat())
    }
    fn value(&self, name: &str, ty: &Type) -> Result<(), String> {
        require(
            self.args.get(name) == Some(ty),
            "call classical/token argument type mismatch",
        )
    }
    fn values(&self, names: &str, types: &[Type]) -> Result<(), String> {
        let names = list_operand(names)?;
        require(
            names.len() == types.len(),
            "call classical argument count mismatch",
        )?;
        for (name, ty) in names.iter().zip(types) {
            self.value(name, ty)?;
        }
        Ok(())
    }
    fn memory(&self, names: &str, shapes: &[(usize, usize, String)]) -> Result<Vec<Wire>, String> {
        let names = list_operand(names)?;
        require(names.len() == shapes.len(), "call region count mismatch")?;
        let mut all = vec![];
        for (name, (a, w, role)) in names.iter().zip(shapes) {
            let (aa, ww, mutable) = self
                .regions
                .get(name)
                .ok_or_else(|| err("3.3", "call memory operand is not a region"))?;
            require((a, w) == (aa, ww), "call region shape mismatch")?;
            require(
                role != "mut" || *mutable,
                "in region passed as a modified call operand",
            )?;
            // Alias checking needs the region owner, never an exponential
            // enumeration of its cells (K75 domains stay admissible).
            all.push((name.clone(), usize::MAX));
        }
        Ok(all)
    }
    fn check_aliases(&self, all: &[Wire], modified: &[Wire], pred: &[Wire]) -> Result<(), String> {
        require(
            all.iter().collect::<BTreeSet<_>>().len() == all.len(),
            "call interface slots overlap",
        )?;
        require(
            modified.iter().all(|w| !pred.contains(w)),
            "call modifies an effective control",
        )
    }
    fn call(&self, obj: &Object, ins: &Instr, pred: &[Wire]) -> Result<(), String> {
        let (_, control, op, xs) = crate::action::normalize_call(ins)?;
        let mut pred = pred.to_vec();
        if let Some(c) = control {
            pred.extend(self.bundle(c, 1, false)?);
        }
        let (mut all, modified, regions) = if op == "oracle_call" {
            let sig = oracle_sig(obj, xs[0].trim_start_matches('@'))?;
            self.value(&xs[1], &Type::Token)?;
            self.values(&xs[6], &sig.args)?;
            let modified = [
                self.bundle(&xs[3], sig.tgt, true)?,
                self.bundle(&xs[4], sig.fault, true)?,
            ]
            .concat();
            let all = [modified.clone(), self.bundles(&xs[2], &sig.ins, false)?].concat();
            (all, modified, self.memory(&xs[5], &sig.mem)?)
        } else {
            let sig = action_sig(obj, xs[0].trim_start_matches('@'))?;
            self.values(&xs[4], &sig.args)?;
            let modified = self.bundles(&xs[1], &sig.muts, true)?;
            let all = [modified.clone(), self.bundles(&xs[2], &sig.ins, false)?].concat();
            let regions = self.memory(&xs[3], &sig.mem)?;
            // A mut region is also a target of the call for control disjointness.
            let mut targets = modified;
            for (name, (_, _, role)) in list_operand(&xs[3])?.iter().zip(&sig.mem) {
                if role == "mut" {
                    targets.extend(regions.iter().filter(|(n, _)| n == name).cloned());
                }
            }
            (all, targets, regions)
        };
        all.extend(regions);
        self.check_aliases(&all, &modified, &pred)
    }
    fn walk(&mut self, obj: &Object, xs: &[Instr], inherited: &[Wire]) -> Result<(), String> {
        let mut pred = inherited.to_vec();
        for i in xs {
            match i.op.as_str() {
                "action_call" | "oracle_call" | "adj" | "ctl" => self.call(obj, i, &pred)?,
                "qinplace" => {
                    let sig = oracle_sig(obj, i.operands[0].trim_start_matches('@'))?;
                    self.value(&i.operands[2], &Type::Token)?;
                    let modified = self.bundle(&i.operands[3], sig.tgt, true)?;
                    let mut all = modified.clone();
                    if i.operands.len() == 7 {
                        all.extend(self.bundles(&i.operands[4], &sig.ins[1..], false)?);
                        all.extend(self.memory(&i.operands[5], &sig.mem)?);
                        self.values(&i.operands[6], &sig.args)?;
                    }
                    self.check_aliases(&all, &modified, &pred)?;
                    require(
                        all.iter().all(|wire| !pred.contains(wire)),
                        "qinplace inputs overlap effective controls",
                    )?;
                }
                "rx" | "ry" | "rz" => {
                    let target = self.bundle(&i.operands[1], 1, i.op != "rz")?;
                    self.check_aliases(&target, &target, &pred)?;
                    let angle = &i.operands[0];
                    if self.args.contains_key(angle) {
                        self.value(angle, &Type::F64)?;
                    } else {
                        let value = angle
                            .parse::<f64>()
                            .map_err(|_| err("3.3", "rotation angle is not f64"))?;
                        require(value.is_finite(), "rotation literal must be finite")?;
                        let mut canonical = if value == 0. && value.is_sign_negative() {
                            "-0.0".into()
                        } else {
                            crate::json::format_f64(value)
                        };
                        if !canonical.contains(['.', 'e']) {
                            canonical.push_str(".0");
                        }
                        if *angle != canonical {
                            return Err(err("16.1", "noncanonical FLOAT gate angle"));
                        }
                    }
                }
                "qalloc" | "qborrow" => {
                    let n = i.operands[1]
                        .parse()
                        .map_err(|_| err("3.3", "invalid allocation width"))?;
                    self.allocate(&i.lhs[0], n, i.op == "qborrow")?;
                }
                "qcopy" => {
                    let n = self.width(&i.operands[0])?;
                    self.allocate(&i.lhs[0], n, false)?;
                }
                "qfree" | "qreturn" => self.release(&i.operands[0], i.op == "qreturn", &pred)?,
                "quncopy" => {
                    let n = self.width(&i.operands[0])?;
                    let source = self.bundle(&i.operands[0], n, false)?;
                    let target = self.bundle(&i.operands[1], n, true)?;
                    self.check_aliases(&[source, target.clone()].concat(), &target, &pred)?;
                    self.release(&i.operands[1], false, &pred)?;
                }
                "qload" | "qunload" | "qstore" | "qexchm" => {
                    let (a, w, _) = self
                        .regions
                        .get(&i.operands[0])
                        .copied()
                        .ok_or_else(|| err("3.3", "load operand is not a region"))?;
                    let address = self.bundle(&i.operands[1], a, false)?;
                    if i.op == "qload" {
                        self.allocate(&i.lhs[0], w, false)?;
                    } else {
                        let modified = i.op != "qstore";
                        let value = self.bundle(&i.operands[2], w, modified)?;
                        self.check_aliases(
                            &[address, value.clone()].concat(),
                            if modified { &value } else { &[] },
                            &pred,
                        )?;
                        if i.op == "qunload" {
                            self.release(&i.operands[2], false, &pred)?;
                        } else {
                            require(self.regions[&i.operands[0]].2, "memory write to in region")?;
                        }
                    }
                }
                "qregion" => {
                    let a = i.operands[1]
                        .parse()
                        .map_err(|_| err("3.3", "invalid region address width"))?;
                    let w = i.operands[2]
                        .parse()
                        .map_err(|_| err("3.3", "invalid region cell width"))?;
                    require(w > 0, "zero-width region cell")?;
                    if let Some(image) = i.operands.get(3) {
                        let sig =
                            crate::validate::signature_of(obj, image.trim_start_matches('@'))?;
                        let bits = crate::bigint::BigUint::from_u64(w as u64).shl(a);
                        let bytes = sig
                            .get("size")
                            .and_then(|v| v.as_big())
                            .ok_or_else(|| err("3.3", "region image size missing"))?;
                        require(bytes.shl(3) == bits, "region image size mismatch")?;
                    }
                    require(
                        self.regions.get(&i.operands[0]) == Some(&(a, w, true)),
                        "region declaration mismatch",
                    )?;
                }
                "qpred" => pred.extend(self.bundle(&i.operands[0], 1, false)?),
                "qunpred" => {
                    let wire = self.bundle(&i.operands[0], 1, false)?;
                    require(
                        pred.ends_with(&wire) && pred.len() > inherited.len(),
                        "predicate stack mismatch",
                    )?;
                    pred.pop();
                }
                "qif" | "qloop" => {
                    let mut ctx = pred.clone();
                    if i.op == "qif" {
                        ctx.extend(self.bundle(&i.operands[0], 1, false)?);
                    } else if i.operands.len() == 3 {
                        let n = i.operands[0]
                            .parse()
                            .map_err(|_| err("3.3", "invalid loop bound"))?;
                        ctx.extend(self.bundle(&i.operands[2], n, false)?);
                    }
                    for b in &i.nested {
                        let before = self.wires.clone();
                        let locals = self.locals.clone();
                        self.walk(obj, b, &ctx)?;
                        if i.op == "qloop" {
                            require(self.wires == before, "loop entry/exit live bindings differ")?;
                            let trips: usize = i.operands[0]
                                .parse()
                                .map_err(|_| err("3.3", "invalid loop bound"))?;
                            if trips == 0 {
                                self.locals = locals;
                            } else if trips > 1 && self.locals != locals {
                                // A one-trip loop may change allocation ownership while
                                // keeping its live types. Repeated visits must also typecheck
                                // their release kinds against the new live ownership.
                                let after = self.locals.clone();
                                self.walk(obj, b, &ctx)?;
                                require(
                                    self.wires == before && self.locals == after,
                                    "loop ownership is not stable on repeated visits",
                                )?;
                            }
                        }
                    }
                }
                op if crate::action::is_gate_name(op) || matches!(op, "qxor" | "qexch") => {
                    let diagonal = matches!(op, "s" | "sdg" | "t" | "tdg" | "rz" | "cz");
                    let (sources, targets) = match op {
                        "mcx" => (
                            self.bundles(
                                &i.operands[0],
                                &vec![1; list_operand(&i.operands[0])?.len()],
                                false,
                            )?,
                            self.bundle(&i.operands[1], 1, true)?,
                        ),
                        "cx" => (
                            self.bundle(&i.operands[0], 1, false)?,
                            self.bundle(&i.operands[1], 1, true)?,
                        ),
                        "ccx" => (
                            [
                                self.bundle(&i.operands[0], 1, false)?,
                                self.bundle(&i.operands[1], 1, false)?,
                            ]
                            .concat(),
                            self.bundle(&i.operands[2], 1, true)?,
                        ),
                        "qxor" | "qexch" => {
                            let n = self.width(&i.operands[0])?;
                            let a = self.bundle(&i.operands[0], n, op == "qexch")?;
                            let b = self.bundle(&i.operands[1], n, true)?;
                            if op == "qexch" {
                                (vec![], [a, b].concat())
                            } else {
                                (a, b)
                            }
                        }
                        "swap" | "cz" => (
                            vec![],
                            [
                                self.bundle(&i.operands[0], 1, !diagonal)?,
                                self.bundle(&i.operands[1], 1, !diagonal)?,
                            ]
                            .concat(),
                        ),
                        _ => (
                            vec![],
                            self.bundle(i.operands.last().unwrap(), 1, !diagonal)?,
                        ),
                    };
                    self.check_aliases(&[sources, targets.clone()].concat(), &targets, &pred)?;
                }
                _ => {}
            }
        }
        require(
            pred == inherited,
            "predicate stack not empty at structured boundary",
        )?;
        Ok(())
    }
}
pub(crate) fn function(obj: &Object, f: &Function) -> Result<(), String> {
    if !matches!(f.kind, FnKind::Action | FnKind::PermAction) {
        return Ok(());
    }
    let mut s = Schema {
        wires: BTreeMap::new(),
        regions: BTreeMap::new(),
        args: BTreeMap::new(),
        locals: BTreeMap::new(),
    };
    for (group, ps) in f.lists[..2].iter().enumerate() {
        for p in ps {
            if let Type::Wire(w) = p.ty {
                s.wires.insert(p.name.clone(), (w, group == 0));
            }
        }
    }
    for p in &f.lists[2] {
        if let Type::Region(a, w) = p.ty {
            s.regions
                .insert(p.name.clone(), (a, w, p.role.as_deref() == Some("mut")));
        }
    }
    for p in &f.lists[3] {
        s.args.insert(p.name.clone(), p.ty.clone());
    }
    // Local regions have function-wide scope, including in structured bodies.
    let mut flat = vec![];
    f.flat_instrs(&mut flat);
    for i in flat {
        if i.op == "qregion" {
            let a = i.operands[1]
                .parse()
                .map_err(|_| err("3.3", "invalid region address width"))?;
            let w = i.operands[2]
                .parse()
                .map_err(|_| err("3.3", "invalid region cell width"))?;
            require(
                !s.wires.contains_key(&i.operands[0])
                    && !s.args.contains_key(&i.operands[0])
                    && s.regions
                        .insert(i.operands[0].clone(), (a, w, true))
                        .is_none(),
                "duplicate local region name",
            )?;
        }
    }
    let body = f
        .blocks
        .iter()
        .flat_map(|b| b.instrs.clone())
        .collect::<Vec<_>>();
    s.walk(obj, &body, &[])?;
    require(s.locals.is_empty(), "local wire lifetime not released")?;
    Ok(())
}
