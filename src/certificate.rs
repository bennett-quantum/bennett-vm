//! ISA 0.8 §12: independent replay, structural proofs and verification references.
//! No producer assertion is a verdict. Scheduled claims require the actual
//! bound realisation; the unpublished ref library is never substituted.
use crate::{
    action::{self, QuantumState, RegionWires},
    bigint::BigUint as U,
    json::Json,
    object::{FnKind, Object},
    value::{Budget, Value},
};
use std::collections::{BTreeMap, BTreeSet};
fn err(s: impl std::fmt::Display) -> String {
    format!("§12.1: {s}")
}
fn field<'a>(j: &'a Json, k: &str) -> Result<&'a Json, String> {
    j.get(k).ok_or_else(|| err(format!("missing {k}")))
}
fn string<'a>(j: &'a Json, k: &str) -> Result<&'a str, String> {
    field(j, k)?
        .as_str()
        .ok_or_else(|| err(format!("invalid {k}")))
}
fn array(j: &Json) -> Result<&Vec<Json>, String> {
    j.as_arr().ok_or_else(|| err("expected array"))
}
fn integer(j: &Json) -> Result<U, String> {
    j.as_big().ok_or_else(|| err("expected unsigned integer"))
}
/// Counts and fraction are exact integers; sampled replay never earns proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Proof,
    Evidence { checked: U, covered: U, domain: U },
}
#[derive(Clone)]
struct Component {
    slot: usize,
    lo: U,
    hi: U,
}
struct Domain {
    widths: Vec<usize>,
    components: Vec<Component>,
    bundles: usize,
    borrowed: usize,
    regions: Vec<(usize, usize)>,
    full: bool,
    count: U,
    total: U,
}
fn domain(obj: &Object, name: &str, j: &Json) -> Result<Domain, String> {
    let f = obj.function(name).ok_or_else(|| err("unknown region"))?;
    let oracle = f.kind == FnKind::Oracle;
    if !matches!(f.kind, FnKind::Action | FnKind::PermAction | FnKind::Oracle) {
        return Err(err("claims require an oracle or action"));
    }
    let mut names = vec![];
    let mut widths = vec![];
    let groups = if oracle { 3 } else { 2 };
    for p in f.lists[..groups].iter().flatten() {
        names.push(p.name.clone());
        widths.push(p.ty.width().ok_or_else(|| err("bundle width"))?);
    }
    let bundles = widths.len();
    let mem = if oracle {
        crate::validate::oracle_sig(obj, name)?.mem
    } else {
        crate::validate::action_sig(obj, name)?.mem
    };
    let borrowed_defs = field(j, "borrowed")?;
    let mut flat = vec![];
    f.flat_instrs(&mut flat);
    let declared: BTreeMap<_, _> = flat
        .iter()
        .filter(|i| i.op == "qborrow")
        .map(|i| (i.lhs[0].clone(), i.operands[1].parse::<usize>().unwrap()))
        .collect();
    // Wire binding follows actual qborrow allocation order; the domain is
    // free to enumerate components in any order without changing that binding.
    for i in flat.iter().filter(|i| i.op == "qborrow") {
        names.push(i.lhs[0].clone());
        widths.push(declared[&i.lhs[0]]);
    }
    for b in array(borrowed_defs)? {
        let n = string(b, "name")?;
        let w = field(b, "width")?.as_usize().ok_or_else(|| err("width"))?;
        if declared.get(n) != Some(&w) {
            return Err(err(
                "borrowed domain must name actual qborrow bundles with their widths",
            ));
        }
    }
    let borrowed = widths.len() - bundles;
    let mut regions = vec![];
    for (a, w, _) in &mem {
        let cells = 1usize
            .checked_shl(
                (*a).try_into()
                    .map_err(|_| err("host limit: region address width"))?,
            )
            .ok_or_else(|| err("host limit: region cells"))?;
        regions.push((widths.len(), cells));
        widths.extend(std::iter::repeat_n(*w, cells));
    }
    let mut components = vec![];
    let mut seen = BTreeSet::new();
    let mut add = |slot: usize, v: &Json| -> Result<(), String> {
        if !seen.insert(slot) {
            return Err(err("§12.2: duplicate domain component"));
        }
        let lo = integer(field(v, "lo")?)?;
        let hi = integer(field(v, "hi")?)?;
        if lo > hi || hi.bits() > widths[slot] {
            return Err(err("§12.2: range exceeds signature width or is reversed"));
        }
        components.push(Component { slot, lo, hi });
        Ok(())
    };
    for b in array(field(j, "bundles")?)?
        .iter()
        .chain(array(borrowed_defs)?)
    {
        let n = string(b, "name")?;
        let slot = names
            .iter()
            .position(|x| x == n)
            .ok_or_else(|| err("unknown domain bundle"))?;
        if field(b, "width")?.as_usize() != Some(widths[slot]) {
            return Err(err("domain width differs from signature"));
        }
        add(slot, b)?;
    }
    let mut region_seen = BTreeSet::new();
    for r in array(field(j, "regions")?)? {
        let n = string(r, "name")?;
        let k = f.lists[groups]
            .iter()
            .position(|p| p.name == n)
            .ok_or_else(|| err("unknown domain region"))?;
        if !region_seen.insert(k) {
            return Err(err("duplicate region"));
        }
        let (start, count) = regions[k];
        let cells = field(r, "cells")?;
        if cells.as_str() == Some("all") {
            for (slot, width) in widths.iter().enumerate().skip(start).take(count) {
                add(
                    slot,
                    &Json::obj(vec![
                        ("lo", Json::u64(0)),
                        (
                            "hi",
                            Json::u(&U::from_u64(1).shl(*width).sub(&U::from_u64(1))),
                        ),
                    ]),
                )?;
            }
        } else {
            for c in array(cells)? {
                let index = field(c, "index")?
                    .as_usize()
                    .filter(|i| *i < count)
                    .ok_or_else(|| err("cell index outside region"))?;
                add(start + index, c)?;
            }
        }
    }
    let one = U::from_u64(1);
    let mut count = one.clone();
    for c in &components {
        count = count.mul(&c.hi.sub(&c.lo).add(&one));
    }
    let missing_dirty = if oracle {
        0
    } else {
        crate::qcli::borrow_capacity(obj, name)?
            .saturating_sub(widths[bundles..bundles + borrowed].iter().sum())
    };
    let total = one.shl(widths.iter().sum::<usize>() + missing_dirty);
    // An omitted dirty pool can never turn a partial-domain replay into proof.
    let full = components.len() == widths.len()
        && count == total
        && borrowed == declared.len()
        && missing_dirty == 0;
    Ok(Domain {
        widths,
        components,
        bundles,
        borrowed,
        regions,
        full,
        count,
        total,
    })
}
impl Domain {
    fn values(&self, index: &U) -> Vec<U> {
        let mut vals = vec![U::zero(); self.widths.len()];
        let mut n = index.clone();
        for c in self.components.iter().rev() {
            let (q, r) = n.divmod(&c.hi.sub(&c.lo).add(&U::from_u64(1)));
            vals[c.slot] = c.lo.add(&r);
            n = q;
        }
        vals
    }
    fn index(&self, vals: &[U]) -> U {
        self.components.iter().fold(U::zero(), |n, c| {
            n.mul(&c.hi.sub(&c.lo).add(&U::from_u64(1)))
                .add(&vals[c.slot].sub(&c.lo))
        })
    }
}
/// Public sampler follows §12.1 component order and rejection, including R=1
/// (zero words) and arbitrary-width ranges.
pub struct Xoshiro256(pub [u64; 4]);
impl Xoshiro256 {
    pub fn word(&mut self) -> u64 {
        let s = &mut self.0;
        let out = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        out
    }
    pub fn range(&mut self, lo: &U, hi: &U) -> Result<U, String> {
        if lo > hi {
            return Err(err("reversed sample range"));
        }
        let r = hi.sub(lo).add(&U::from_u64(1));
        let bits = r.sub(&U::from_u64(1)).bits();
        loop {
            let mut x = U::zero();
            for i in 0..bits.div_ceil(64) {
                x = x.add(&U::from_u64(self.word()).shl(i * 64));
            }
            x = x.mask(bits);
            if x < r {
                return Ok(lo.add(&x));
            }
        }
    }
}
fn execute(
    obj: &Object,
    name: &str,
    d: &Domain,
    vals: &[U],
    args: &[Value],
    budget: &Budget,
) -> Result<Vec<U>, String> {
    if obj.function(name).is_some_and(|f| f.kind == FnKind::Oracle) {
        let sig = crate::validate::oracle_sig(obj, name)?;
        let mem: Vec<_> = d
            .regions
            .iter()
            .map(|(start, n)| vals[*start..start + n].to_vec())
            .collect();
        let mut machine =
            crate::step::Machine::new(obj, name, &vals[..sig.ins.len()], &mem, args, *budget)?;
        let entry = machine.state.clone();
        for _ in 0..budget.steps {
            machine.forward_step()?;
        }
        let result = machine.outcome();
        let mut out = vals.to_vec();
        out[sig.ins.len()] = out[sig.ins.len()].bitxor(&result.payload);
        out[sig.ins.len() + 1] = out[sig.ins.len() + 1].bitxor(&result.code);
        for _ in 0..budget.steps {
            machine.inverse_step()?;
        }
        if machine.state != entry || !machine.frames.is_empty() {
            return Err(err("§5.2: full tuple scratch/history not restored"));
        }
        return Ok(out);
    }
    let sig = crate::validate::action_sig(obj, name)?;
    let mut s = QuantumState::new();
    let ws: Vec<_> = d.widths.iter().map(|n| s.alloc_wires(*n)).collect();
    for (w, v) in ws.iter().zip(vals) {
        s.prepare(w, v);
    }
    let regs: Vec<_> = d
        .regions
        .iter()
        .zip(&sig.mem)
        .map(|((start, n), (a, w, _))| RegionWires {
            a: *a,
            w: *w,
            cells: ws[*start..start + n].to_vec(),
        })
        .collect();
    let mut pool = ws[d.bundles..d.bundles + d.borrowed].concat();
    let needed = crate::qcli::borrow_capacity(obj, name)?;
    if pool.len() > needed {
        return Err(err("borrowed domain exceeds designated pool"));
    }
    pool.extend(s.alloc_wires(needed - pool.len()));
    action::execute_action_with_idle(
        obj,
        &mut s,
        name,
        &ws[..sig.muts.len()],
        &ws[sig.muts.len()..d.bundles],
        &regs,
        args,
        false,
        &[],
        &pool,
    )?;
    let out: Vec<_> = ws
        .iter()
        .map(|w| {
            s.constant_value(w)
                .ok_or_else(|| err("full tuple is not basis definite"))
        })
        .collect::<Result<_, _>>()?;
    // Compare the exact shadow to a positive unit basis vector as well: basis
    // values alone would lose a relative/global phase and fail T3 under control.
    let mut expected = QuantumState::new();
    let all = expected.alloc_wires(s.wire_count());
    let mut key = U::zero();
    for (w, v) in ws.iter().zip(&out) {
        for (i, w) in w.iter().enumerate() {
            key.set_bit(*w, v.bit(i));
        }
    }
    expected.prepare(&all, &key);
    if !s.exact_equals(&expected) {
        return Err(err(
            "full tuple has phase or residual workspace/entanglement; not T3",
        ));
    }
    for k in d.bundles..d.bundles + d.borrowed {
        if vals[k] != out[k] {
            return Err(err("§5.6: dirty bundle not restored"));
        }
    }
    Ok(out)
}
fn resolve<'a>(
    obj: &'a Object,
    libs: &'a [Object],
    hash: &str,
) -> Result<(&'a Object, String), String> {
    for o in libs.iter().chain(std::iter::once(obj)) {
        let ids = crate::identity::compute(o)?;
        for (name, h) in ids.closure {
            if h.as_deref() == Some(hash) {
                return Ok((o, name));
            }
        }
    }
    Err(err(format!(
        "unresolved library/expected program closure_hash {hash}"
    )))
}
fn executable(obj: &Object, name: &str, claim: &Json) -> Result<(), String> {
    let subject = field(claim, "subject")?;
    let hash = string(subject, "schedule_hash")?;
    let closure = string(subject, "closure_hash")?;
    let ids = crate::identity::compute(obj)?;
    if ids.closure.get(name).and_then(|x| x.as_deref()) != Some(closure) {
        return Err(err("subject closure_hash differs from object"));
    }
    string(subject, "variant")?;
    field(subject, "inherited_controls")?
        .as_usize()
        .ok_or_else(|| err("inherited_controls"))?;
    let schedule = crate::sched::object_schedule(obj)?
        .ok_or_else(|| err("subject schedule_hash requires bound root schedule"))?;
    if crate::sha256::sha256_hex(schedule.to_jcs().as_bytes()) != hash {
        return Err(err("subject schedule_hash differs from root schedule"));
    }
    crate::sched::validate_entries(&schedule, obj, &ids)?;
    crate::sched::require_library(schedule.get("bindings").and_then(|b| b.get("expansion")))?;
    Err(err(
        "unimplemented: invocation-context realisation requires §11 ref@0.8 (b-isa-spec-8r4; bennett-vm-47t)",
    ))
}
/// Replay one claim. `args` and `budget` are resolved invocation parameters;
/// `libraries` are separately loaded, hash-checked objects, never trusted verdicts.
pub fn verify_claim(
    obj: &Object,
    name: &str,
    claim: &Json,
    args: &[Value],
    budget: &Budget,
    libraries: &[Object],
) -> Result<Verdict, String> {
    let _guard = ReplayGuard::new(obj, name, claim)?;
    match string(claim, "kind")? {
        "numerical" | "channel" => {
            executable(obj, name, claim)?;
            unreachable!()
        }
        "structural" => structural(obj, name, claim, args, budget, libraries),
        "exhaustive" | "sampled" => {
            let kind = string(claim, "kind")?;
            let expected = claim.get("expected").ok_or_else(||err("unimplemented: §12.1 sampled payload does not define an expected relation/default (bennett-vm-clw, isa-0.9); replay request from bennett-driver must bind the comparison"))?;
            if expected.as_str() == Some("oracle") {
                executable(obj, name, claim)?;
                unreachable!()
            }
            let d = domain(obj, name, field(claim, "domain")?)?;
            let mut rng = None;
            let count = if kind == "sampled" {
                let sampler = field(claim, "sampler")?;
                if string(sampler, "algorithm")? != "xoshiro256**" {
                    return Err(err("unknown sampler"));
                }
                let seed = array(field(sampler, "seed")?)?;
                if seed.len() != 4 || seed.iter().any(|x| x.as_str().is_none()) {
                    return Err(err("seed must be four decimal strings"));
                }
                let mut words = [0; 4];
                for (w, s) in words.iter_mut().zip(seed) {
                    *w = s.as_u64().ok_or_else(|| err("invalid seed word"))?;
                }
                if words == [0; 4] {
                    return Err(err("all-zero xoshiro256** seed"));
                }
                rng = Some(Xoshiro256(words));
                integer(field(sampler, "count")?)?
            } else {
                d.count.clone()
            };
            let program = expected
                .get("program")
                .map(|p| p.as_str().ok_or_else(|| err("program hash")))
                .transpose()?
                .map(|h| resolve(obj, libraries, h))
                .transpose()?;
            let table = expected.get("table").map(array).transpose()?;
            if program.is_some() == table.is_some() {
                return Err(err("expected must contain exactly one table or program"));
            }
            if let Some(table) = table
                && U::from_u64(table.len() as u64) != d.count
            {
                return Err(err("expected table must contain every domain element"));
            }
            let mut case = U::zero();
            let mut sampled_indices = BTreeSet::new();
            while case < count {
                let mut vals = d.values(&case);
                if let Some(r) = &mut rng {
                    for c in &d.components {
                        vals[c.slot] = r.range(&c.lo, &c.hi)?;
                    }
                }
                if kind == "sampled" {
                    sampled_indices.insert(d.index(&vals));
                }
                let actual = execute(obj, name, &d, &vals, args, budget)?;
                let wanted = if let Some((o, n)) = &program {
                    // Position-based ABI mapping, not the expected program's local names.
                    let mut a = crate::validate::signature_of(o, n)?.clone();
                    let mut b = crate::validate::signature_of(obj, name)?.clone();
                    if let Json::Obj(m) = &mut a {
                        m.remove("bounds");
                    }
                    if let Json::Obj(m) = &mut b {
                        m.remove("bounds");
                    }
                    if a != b {
                        return Err(err("expected program ABI mismatch"));
                    }
                    execute(o, n, &d, &vals, args, budget)?
                } else {
                    let index = d
                        .index(&vals)
                        .to_usize()
                        .ok_or_else(|| err("host limit: table index"))?;
                    array(&table.unwrap()[index])?
                        .iter()
                        .map(integer)
                        .collect::<Result<Vec<_>, _>>()?
                };
                if actual != wanted {
                    return Err(err(format!(
                        "full tuple mismatch at domain element {}: got {:?}, expected {:?}",
                        d.index(&vals).to_decimal(),
                        actual,
                        wanted
                    )));
                }
                case = case.add(&U::from_u64(1));
            }
            Ok(if kind == "exhaustive" && d.full {
                Verdict::Proof
            } else {
                Verdict::Evidence {
                    checked: count,
                    covered: if kind == "sampled" {
                        U::from_u64(sampled_indices.len() as u64)
                    } else {
                        d.count
                    },
                    domain: d.total,
                }
            })
        }
        other => Err(err(format!(
            "unknown claim kind {other}; inverse_pair and borrow_frames are §12.3 verification references, not claim kinds"
        ))),
    }
}
fn structural(
    obj: &Object,
    name: &str,
    c: &Json,
    args: &[Value],
    budget: &Budget,
    libs: &[Object],
) -> Result<Verdict, String> {
    match string(c, "rule")? {
        "translation_lemma" => {
            crate::qvalidate::translation_witness(obj, name, field(c, "witness")?)?
        }
        "adjoint_table" => {
            if field(c, "witness")?.as_obj().is_none_or(|m| !m.is_empty()) {
                return Err(err("adjoint_table witness must be {}"));
            }
            let f = obj
                .function(name)
                .filter(|f| matches!(f.kind, FnKind::Action | FnKind::PermAction))
                .ok_or_else(|| err("adjoint_table requires a Q action"))?;
            let mut flat = vec![];
            f.flat_instrs(&mut flat);
            // §5.3 gives an adjoint for every admitted action instruction.
            // Validation has already proved typed bindings, finite bounds,
            // matched predicate/lifetime pairs and per-frame dirty restoration.
            // This proves the table's transformation, not equality to an
            // expected oracle nor any target approximation bound.
            for i in flat {
                if !action::is_gate_name(&i.op)
                    && !matches!(
                        i.op.as_str(),
                        "qalloc"
                            | "qfree"
                            | "qborrow"
                            | "qreturn"
                            | "qregion"
                            | "qcopy"
                            | "quncopy"
                            | "qxor"
                            | "qexch"
                            | "qinplace"
                            | "qload"
                            | "qunload"
                            | "qstore"
                            | "qexchm"
                            | "oracle_call"
                            | "action_call"
                            | "adj"
                            | "ctl"
                            | "qpred"
                            | "qunpred"
                            | "qif"
                            | "qloop"
                    )
                {
                    return Err(err(format!("§5.3: no adjoint table row for {}", i.op)));
                }
            }
        }

        "inverse_pair_library" => {
            let w = field(c, "witness")?;
            let (o, n) = resolve(obj, libs, string(w, "library_closure_hash")?)?;
            let index = field(w, "claim_index")?
                .as_usize()
                .ok_or_else(|| err("claim_index"))?;
            let cert = object_document(o, "certificate")?
                .ok_or_else(|| err("library has no certificate"))?;
            let hash = crate::identity::compute(o)?.closure[&n].clone().unwrap();
            let claims = array(field(field(field(&cert, "regions")?, &hash)?, "claims")?)?;
            let claim = claims
                .get(index)
                .ok_or_else(|| err("library claim index out of range"))?;
            if verify_claim(o, &n, claim, args, budget, libs)? != Verdict::Proof {
                return Err(err("library claim is evidence, not proof"));
            }
            if crate::identity::compute(obj)?.closure[name].as_deref() != Some(&hash) {
                return Err(err(
                    "library claim does not prove this region; no equivalence subclaim",
                ));
            }
        }
        other => return Err(err(format!("unknown structural rule {other}"))),
    }
    Ok(Verdict::Proof)
}
/// Metadata is excluded from object identity but retained verbatim for checking.
pub fn object_document(obj: &Object, section: &str) -> Result<Option<Json>, String> {
    let marker = format!("\nsection {section}\n");
    obj.text
        .split_once(&marker)
        .map(|(_, s)| crate::json::parse(s.lines().next().ok_or_else(|| err("empty metadata"))?))
        .transpose()
}
/// §4.5 requires totality and *both* compositions on every n-bit input.
pub fn verify_inverse_pair(
    obj: &Object,
    g: &str,
    ginv: &str,
    budget: &Budget,
) -> Result<Verdict, String> {
    let guard_claim = Json::obj(vec![(
        "inverse_pair",
        Json::Str(format!("{g}:{ginv}:{}", budget.to_json().to_jcs())),
    )]);
    let _guard = ReplayGuard::new(obj, g, &guard_claim)?;
    let a = crate::validate::oracle_sig(obj, g)?;
    let b = crate::validate::oracle_sig(obj, ginv)?;
    if a.ins != vec![a.tgt]
        || b.ins != a.ins
        || b.tgt != a.tgt
        || !a.mem.is_empty()
        || !b.mem.is_empty()
        || !a.args.is_empty()
        || !b.args.is_empty()
    {
        return Err(err(
            "§4.5: inverse pair must be total permutation oracles on the same n-bit encoding",
        ));
    }
    let end = U::from_u64(1).shl(a.tgt);
    let mut x = U::zero();
    while x < end {
        for (first, second) in [(g, ginv), (ginv, g)] {
            let y = crate::oracle::run_oracle(
                obj,
                first,
                std::slice::from_ref(&x),
                &mut [],
                &[],
                *budget,
            )?;
            let z = crate::oracle::run_oracle(
                obj,
                second,
                std::slice::from_ref(&y.payload),
                &mut [],
                &[],
                *budget,
            )?;
            if !y.code.is_zero() || !z.code.is_zero() || z.payload != x {
                return Err(err(format!(
                    "§4.5: FAULT_NONE and g∘ginv = ginv∘g = id required on every input; {first};{second} failed at {}",
                    x.to_decimal()
                )));
            }
        }
        x = x.add(&U::from_u64(1));
    }
    Ok(Verdict::Proof)
}

/// §13 is a producer report, not an execution profile. Preserve every entry.
pub fn validate_coverage(j: &Json) -> Result<(), String> {
    let e = |s: &str| format!("§13: {s}");
    let entries = j
        .get("entries")
        .and_then(Json::as_arr)
        .ok_or_else(|| e("entries must be an array"))?;
    for entry in entries {
        let m = entry.as_obj().ok_or_else(|| e("entry must be an object"))?;
        if m.keys().any(|k| {
            ![
                "point",
                "source",
                "classification",
                "reason",
                "fault_code",
                "shim_hash",
                "count",
            ]
            .contains(&k.as_str())
        }) {
            return Err(e("unknown coverage entry field"));
        }
        program_point(field(entry, "point")?)?;
        for key in ["source", "reason"] {
            if entry.get(key).and_then(Json::as_str).is_none() {
                return Err(e("source and reason must be strings"));
            }
        }
        if !matches!(
            entry.get("classification").and_then(Json::as_str),
            Some("lowered" | "trapped" | "shimmed")
        ) {
            return Err(e("classification must be lowered, trapped or shimmed"));
        }
        if entry.get("count").and_then(Json::as_big).is_none() {
            return Err(e("count must be an unsigned integer"));
        }
        if let Some(code) = entry.get("fault_code")
            && code.as_big().is_none()
        {
            return Err(e("fault_code must be an unsigned integer"));
        }
        if let Some(hash) = entry.get("shim_hash")
            && hash.as_str().is_none_or(|s| {
                s.len() != 64
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err(e("shim_hash must be a lowercase SHA-256 hash"));
        }
    }
    Ok(())
}

thread_local! {static REPLAY:std::cell::RefCell<Vec<String>>=const {std::cell::RefCell::new(Vec::new())};}
struct ReplayGuard;
impl ReplayGuard {
    fn new(obj: &Object, name: &str, claim: &Json) -> Result<Self, String> {
        let key = format!(
            "{}:{name}:{}",
            crate::identity::object_hash(&obj.text),
            claim.to_jcs()
        );
        REPLAY.with(|r| {
            let mut r = r.borrow_mut();
            if r.contains(&key) {
                return Err(err("cyclic library certificate/subclaim graph"));
            }
            r.push(key);
            Ok(Self)
        })
    }
}
impl Drop for ReplayGuard {
    fn drop(&mut self) {
        REPLAY.with(|r| {
            r.borrow_mut().pop();
        });
    }
}
/// Verify every claim and every §12.3 index of one resolved invocation region.
/// Library references are replayed with cycle detection, never trusted labels.
pub fn verify_region_claims(
    obj: &Object,
    name: &str,
    region: &Json,
    args: &[Value],
    budget: &Budget,
    libraries: &[Object],
) -> Result<Vec<Verdict>, String> {
    let claims = array(field(region, "claims")?)?;
    let mut verdicts = vec![];
    for c in claims {
        verdicts.push(verify_claim(obj, name, c, args, budget, libraries)?);
    }
    let proof = |i: &Json| -> Result<usize, String> {
        let n = i
            .as_usize()
            .filter(|n| *n < verdicts.len())
            .ok_or_else(|| err("§12.3: claim index out of range"))?;
        if verdicts[n] != Verdict::Proof {
            return Err(err("§12.3: evidence cannot discharge a proof obligation"));
        }
        Ok(n)
    };
    // Structural rules may only depend on successful proof subclaims, never
    // circular index graphs, even when their own side conditions also hold.
    fn visit(
        n: usize,
        claims: &[Json],
        visiting: &mut BTreeSet<usize>,
        done: &mut BTreeSet<usize>,
        verdicts: &[Verdict],
    ) -> Result<(), String> {
        if done.contains(&n) {
            return Ok(());
        }
        if !visiting.insert(n) {
            return Err(err("cyclic subclaims"));
        }
        if let Some(xs) = claims[n].get("subclaims") {
            for i in array(xs)? {
                let k = i
                    .as_usize()
                    .filter(|k| *k < claims.len())
                    .ok_or_else(|| err("subclaim index out of range"))?;
                if verdicts[k] != Verdict::Proof {
                    return Err(err("structural subclaim is evidence"));
                }
                visit(k, claims, visiting, done, verdicts)?;
            }
        }
        visiting.remove(&n);
        done.insert(n);
        Ok(())
    }
    let mut done = BTreeSet::new();
    for n in 0..claims.len() {
        visit(n, claims, &mut BTreeSet::new(), &mut done, &verdicts)?;
    }
    let v = field(region, "verification")?;
    for (key, value) in v
        .as_obj()
        .ok_or_else(|| err("verification must be object"))?
    {
        match key.as_str() {
            "oracle" => {
                let n = proof(value)?;
                if !t3_claim(obj, &claims[n], libraries, &mut BTreeSet::new())? {
                    return Err(err(
                        "§12.3: oracle verification requires a full-tuple T3 claim; adjoint/translation proves a different obligation",
                    ));
                }
            }
            "borrow_frames" => {
                for i in array(value)? {
                    let n = proof(i)?;
                    let translation = string(&claims[n], "kind")? == "structural"
                        && string(&claims[n], "rule")? == "translation_lemma";
                    if !translation && !t3_claim(obj, &claims[n], libraries, &mut BTreeSet::new())?
                    {
                        return Err(err(
                            "§5.6: borrow_frames requires T3 or translation lemma proof",
                        ));
                    }
                }
            }
            "inverse_pairs" => {
                for pair in array(value)? {
                    proof(field(pair, "claim")?)?;
                    verify_inverse_pair(obj, string(pair, "g")?, string(pair, "ginv")?, budget)?;
                }
            }
            "unitary" | "channel" => {
                for i in array(value)? {
                    let n = proof(i)?;
                    let kind = string(&claims[n], "kind")?;
                    if kind
                        != if key == "unitary" {
                            "numerical"
                        } else {
                            "channel"
                        }
                    {
                        return Err(err(
                            "§12.3: error bound requires the corresponding numerical/channel claim",
                        ));
                    }
                }
            }
            "fault_rate" => {
                return Err(err(
                    "unimplemented: native/undefined/layout-dependent fault-rate comparison belongs to bennett-driver and bennett-estimate",
                ));
            }
            _ => return Err(err(format!("§12.3: unknown verification field {key}"))),
        }
    }
    Ok(verdicts)
}
/// Executor gate: validate embedded bindings before changing the machine. The
/// replay engine deliberately bypasses this gate while independently executing
/// claims, so a certificate cannot recursively validate itself by execution.
pub(crate) fn verify_embedded(
    obj: &Object,
    name: &str,
    args: &[Value],
    budget: &Budget,
    target: Option<&crate::target::Target>,
) -> Result<(), String> {
    if !obj.has_certificate || REPLAY.with(|r| !r.borrow().is_empty()) {
        return Ok(());
    }
    let doc = object_document(obj, "certificate")?.ok_or_else(|| err("certificate missing"))?;
    let bindings = field(&doc, "bindings")?;
    let ids = crate::identity::compute(obj)?;
    if string(bindings, "object_hash")? != ids.object_hash {
        return Err(err("§12.3: certificate object_hash differs from object"));
    }
    let hash = string(bindings, "closure_hash")?;
    let root = ids
        .closure
        .iter()
        .find(|(_, h)| h.as_deref() == Some(hash))
        .map(|(n, _)| n)
        .ok_or_else(|| err("certificate closure_hash is not in object"))?;
    if string(bindings, "interface_hash")? != ids.interface[root] {
        return Err(err(
            "certificate interface_hash differs from root signature",
        ));
    }
    let expected_target = string(bindings, "target_hash")?;
    if target.is_none_or(|t| t.hash != expected_target) {
        return Err(err(
            "certificate target_hash requires matching loaded target",
        ));
    }
    if let Some(schedule) = bindings.get("schedule_hash") {
        let actual = crate::sched::object_schedule(obj)?
            .ok_or_else(|| err("certificate schedule_hash requires root schedule"))?;
        if schedule.as_str() != Some(&crate::sha256::sha256_hex(actual.to_jcs().as_bytes())) {
            return Err(err("certificate schedule_hash mismatch"));
        }
    }
    let regions = field(&doc, "regions")?;
    let region_hash = ids
        .closure
        .get(name)
        .and_then(|h| h.as_deref())
        .ok_or_else(|| err("unresolved invocation closure"))?;
    if let Some(region) = regions.get(region_hash) {
        verify_region_claims(obj, name, region, args, budget, &[])?;
    }
    // Other certificate sections require cross-repository request context;
    // claiming they were verified from this single invocation would be false.
    if doc.get("composition").is_some() {
        return Err(err(
            "unimplemented: §12.3 multiplicities/error composition requires bound schedule ref@0.8 (b-isa-spec-8r4); bennett-estimate request",
        ));
    }
    Ok(())
}

/// §9.2 program-point tuple: (symbol interface hash, block, nested-line ordinal).
pub fn program_point(j: &Json) -> Result<(&str, &str, usize), String> {
    let fail = || "§9.2/§13: program-point id must be [interface_hash, block, ordinal]".to_string();
    let a = j.as_arr().filter(|a| a.len() == 3).ok_or_else(fail)?;
    let hash = a[0]
        .as_str()
        .filter(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or_else(fail)?;
    let block = a[1].as_str().filter(|s| !s.is_empty()).ok_or_else(fail)?;
    let ordinal = a[2].as_usize().ok_or_else(fail)?;
    Ok((hash, block, ordinal))
}

/// Validate coverage point identities against the actual object (§9.2, §13).
pub fn validate_coverage_object(obj: &Object, j: &Json) -> Result<(), String> {
    validate_coverage(j)?;
    let ids = crate::identity::compute(obj)?;
    for entry in array(field(j, "entries")?)? {
        let (hash, block, ordinal) = program_point(field(entry, "point")?)?;
        let name = ids
            .interface
            .iter()
            .find(|(_, h)| h.as_str() == hash)
            .map(|(n, _)| n)
            .ok_or("§13: coverage point symbol is not in object")?;
        let f = obj
            .function(name)
            .ok_or("§13: coverage point is not an instruction symbol")?;
        let b = f
            .blocks
            .iter()
            .find(|b| b.name == block)
            .ok_or("§13: coverage point block is not in object")?;
        fn count(xs: &[crate::object::Instr]) -> usize {
            xs.iter()
                .map(|i| 1 + i.nested.iter().map(|b| count(b)).sum::<usize>())
                .sum()
        }
        if ordinal >= count(&b.instrs) {
            return Err("§13: coverage point ordinal is not in object".into());
        }
    }
    Ok(())
}

fn t3_claim(
    obj: &Object,
    c: &Json,
    libs: &[Object],
    seen: &mut BTreeSet<String>,
) -> Result<bool, String> {
    if string(c, "kind")? == "exhaustive" {
        return Ok(true);
    }
    if string(c, "kind")? != "structural" || string(c, "rule")? != "inverse_pair_library" {
        return Ok(false);
    }
    let w = field(c, "witness")?;
    let hash = string(w, "library_closure_hash")?;
    let index = field(w, "claim_index")?
        .as_usize()
        .ok_or_else(|| err("claim_index"))?;
    if !seen.insert(format!("{hash}:{index}")) {
        return Err(err("cyclic library claim"));
    }
    let (o, n) = resolve(obj, libs, hash)?;
    let cert =
        object_document(o, "certificate")?.ok_or_else(|| err("library has no certificate"))?;
    let hash = crate::identity::compute(o)?.closure[&n]
        .clone()
        .ok_or_else(|| err("unresolved library"))?;
    let claims = array(field(field(field(&cert, "regions")?, &hash)?, "claims")?)?;
    t3_claim(
        o,
        claims
            .get(index)
            .ok_or_else(|| err("library claim index"))?,
        libs,
        seen,
    )
}
