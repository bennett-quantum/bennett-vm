//! Interface and borrow-frame checks before identity verification (§3.3, §5).
use crate::object::{FnKind, Function, Instr, Object, Type};
use crate::validate::{err, list_operand};
use std::collections::{BTreeMap, BTreeSet};
type Wire = (String, usize);
fn wires(env: &BTreeMap<String, usize>, s: &str) -> Result<Vec<Wire>, String> {
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
fn flip(ins: &Instr, env: &BTreeMap<String, usize>) -> Result<(Vec<Wire>, Wire), String> {
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
    )
}
pub fn validate(_obj: &Object, f: &Function) -> Result<(), String> {
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
    for b in &f.blocks {
        walk(&b.instrs, &env, &inputs, &mut vec![])?;
    }
    if !borrowed.is_empty() {
        let mut segment = Vec::new();
        let mut pred = Vec::<Wire>::new();
        let token = |w: &Wire| format!("{}[{}]", w.0, w.1);
        fn expand(xs: &[Instr], out: &mut Vec<Instr>) {
            for i in xs {
                let make = |op: &str, operand: String| Instr {
                    op: op.into(),
                    operands: vec![operand],
                    lhs: vec![],
                    nested: vec![],
                    line: i.line,
                    raw: i.raw.clone(),
                };
                if i.op == "qif" {
                    let p = i.operands[0].clone();
                    out.push(make("qpred", p.clone()));
                    expand(&i.nested[0], out);
                    out.push(make("qunpred", p.clone()));
                    out.push(make("x", p.clone()));
                    out.push(make("qpred", p.clone()));
                    expand(&i.nested[1], out);
                    out.push(make("qunpred", p.clone()));
                    out.push(make("x", p));
                } else if i.op == "qloop" {
                    // Prove each source frame before composing iterations. The
                    // while flags range over conjunctions of immutable selectors.
                    if i.operands.len() == 3 {
                        let n = i.operands[0]
                            .parse::<usize>()
                            .expect("validated loop bound");
                        for j in 0..n {
                            out.push(make("qpred", format!("{}[{j}]", i.operands[2])));
                        }
                        expand(&i.nested[0], out);
                        for j in (0..n).rev() {
                            out.push(make("qunpred", format!("{}[{j}]", i.operands[2])));
                        }
                    } else {
                        expand(&i.nested[0], out);
                    }
                } else {
                    out.push(i.clone());
                }
            }
        }
        let mut proof = vec![];
        for b in &f.blocks {
            expand(&b.instrs, &mut proof);
        }
        for i in &proof {
            match i.op.as_str() {
                "qpred" => {
                    pred.extend(wires(&env, &i.operands[0])?);
                    continue;
                }
                "qunpred" => {
                    let n = wires(&env, &i.operands[0])?.len();
                    pred.truncate(pred.len() - n);
                    continue;
                }
                "qalloc" | "qfree" | "qborrow" => continue,
                "qreturn" => {
                    borrow_frame(&segment, &env, &borrowed)?;
                    segment.clear();
                    continue;
                }
                "x" | "cx" | "ccx" | "mcx" => {
                    let (mut cs, t) = flip(i, &env)?;
                    cs.extend(pred.clone());
                    cs.sort();
                    cs.dedup();
                    let mut g = i.clone();
                    g.op = "mcx".into();
                    g.operands = vec![
                        format!("({})", cs.iter().map(token).collect::<Vec<_>>().join(", ")),
                        token(&t),
                    ];
                    segment.push(g);
                    continue;
                }
                "swap" | "qexch" if pred.is_empty() => {
                    segment.push(i.clone());
                    continue;
                }
                "qcopy" | "quncopy" | "qxor" => {
                    let a = wires(&env, &i.operands[0])?;
                    let dst = if i.op == "qcopy" {
                        &i.lhs[0]
                    } else {
                        &i.operands[1]
                    };
                    let b = wires(&env, dst)?;
                    for (a, b) in a.iter().zip(b) {
                        let mut cs = pred.clone();
                        cs.push(a.clone());
                        let mut g = i.clone();
                        g.op = "mcx".into();
                        g.lhs.clear();
                        g.operands = vec![
                            format!("({})", cs.iter().map(token).collect::<Vec<_>>().join(", ")),
                            token(&b),
                        ];
                        segment.push(g);
                    }
                    continue;
                }
                _ => {}
            }
            borrow_frame(&segment, &env, &borrowed)?;
            segment.clear();
            if i.operands
                .iter()
                .flat_map(|s| list_operand(s).unwrap_or_else(|_| vec![s.clone()]))
                .any(|s| wires(&env, &s).is_ok_and(|ws| ws.iter().any(|w| borrowed.contains(w))))
            {
                return Err(err("5.6", "borrowed operand outside a permutation frame"));
            }
        }
        borrow_frame(&segment, &env, &borrowed)?;
    }
    Ok(())
}

/// T3 over the full finite frame domain. Structural proof is the fast path;
/// there is no arbitrary fixed width ceiling on the exhaustive alternative.
fn exhaustive_frame(
    seq: &[Instr],
    env: &BTreeMap<String, usize>,
    borrowed: &BTreeSet<Wire>,
) -> Result<(), String> {
    use crate::bigint::BigUint;
    enum Gate {
        Flip(Vec<usize>, usize),
        Swap(usize, usize),
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
        if i.op == "swap" || i.op == "qexch" {
            let a = wires(env, &i.operands[0])?;
            let b = wires(env, &i.operands[1])?;
            for (a, b) in a.iter().zip(&b) {
                gates.push(Gate::Swap(id(a), id(b)));
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
        .filter(|i| !bs.contains(i))
        .collect::<Vec<_>>();
    let mut x = BigUint::zero();
    let xend = BigUint::from_u64(1).shl(xs.len());
    let bend = BigUint::from_u64(1).shl(bs.len());
    while x < xend {
        let mut b = BigUint::zero();
        let mut expected = None;
        while b < bend {
            let mut state = BigUint::zero();
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
                    Gate::Swap(a, b) => {
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
