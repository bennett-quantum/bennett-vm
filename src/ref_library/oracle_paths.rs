//! Sound finite-set value flow for lifted cleanup paths. Top means every bit
//! pattern of the declared width, never a guessed set of source labels.
use super::*;
use crate::{bigint::BigUint, object::Function, value};

// Missing map entries are Top. Widening bounds the ascending chains even for
// arithmetic cycles; it can add paths, but cannot authorize an unsafe pair.
type Values = BTreeSet<BigUint>;
type State = BTreeMap<String, Values>;

fn bound(v: Values, limit: usize) -> Option<Values> {
    (v.len() <= limit).then_some(v)
}
fn join(dst: &mut State, src: &State, limit: usize) {
    dst.retain(|name, values| {
        let Some(other) = src.get(name) else {
            return false;
        };
        values.extend(other.iter().cloned());
        values.len() <= limit
    });
}
fn values(s: &State, name: &str, widths: &BTreeMap<String, usize>, limit: usize) -> Option<Values> {
    s.get(name).cloned().or_else(|| {
        let w = *widths.get(name)?;
        let count = 1usize.checked_shl(u32::try_from(w).ok()?)?;
        (count <= limit).then(|| (0..count).map(|v| BigUint::from_u64(v as u64)).collect())
    })
}

/// Every real path projects to an edge path here. Values are width-exact;
/// joins and widening only overapproximate. Instance-local faults terminate.
pub(super) fn edges(
    f: &Function,
    first: usize,
    functions: &BTreeMap<String, Function>,
) -> Result<Vec<Vec<usize>>, String> {
    let rows = f.blocks.iter().flat_map(|b| &b.instrs).collect::<Vec<_>>();
    let n = rows.len();
    let limit = n + 1;
    let mut widths = crate::oracle::location_widths(f)
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    for p in f.lists.iter().flatten() {
        if let Some(w) = p.ty.width() {
            widths.insert(p.name.clone(), w);
        }
    }
    let mut labels = BTreeMap::new();
    let mut offset = 0;
    for b in &f.blocks {
        labels.insert(format!("^{}", b.name), offset);
        offset += b.instrs.len();
    }
    let target = |s: &str| {
        labels
            .get(s)
            .copied()
            .ok_or_else(|| err("§5.5: unresolved lifted target"))
    };
    let mut edges = vec![BTreeSet::new(); n + 1];
    let mut states = vec![None; n + 1];
    states[0] = Some(if f.kind == crate::object::FnKind::Oracle {
        crate::oracle::location_widths(f)
            .into_iter()
            .map(|(name, _)| (name, BTreeSet::from([BigUint::zero()])))
            .collect()
    } else {
        State::new()
    });
    let mut pending = BTreeSet::from([0]);
    while let Some(row) = pending.pop_first() {
        if row == n {
            continue;
        }
        let before = states[row].as_ref().unwrap();
        let i = rows[row];
        let read = |name: &str| values(before, name, &widths, limit);
        let mut after = before.clone();
        // All outputs, including oracle XOR targets, lose their old value.
        for name in super::oracle_cleanup::call_writes(f, i, functions)? {
            after.remove(&name);
        }
        if let Some(dst) = i.lhs.first() {
            let w = widths[dst];
            let base = i.op.strip_prefix("lift.").unwrap_or(&i.op);
            let result = if base.starts_with("const.") {
                Some(BTreeSet::from([crate::oracle::parse_const(
                    &i.operands[0],
                    w,
                )?]))
            } else if base == "label" {
                Some(BTreeSet::from([BigUint::from_u64(
                    (first + target(&i.operands[0])?) as u64,
                )]))
            } else if base == "mov" {
                read(&i.operands[0])
            } else if base.starts_with("zext.")
                || base.starts_with("trunc.")
                || base.starts_with("sext.")
            {
                read(&i.operands[0]).map(|vs| {
                    vs.into_iter()
                        .map(|v| {
                            if base.starts_with("sext.") {
                                crate::bigint::sign_extend(&v, widths[&i.operands[0]], w)
                            } else {
                                v.mask(w)
                            }
                        })
                        .collect()
                })
            } else if base == "select" {
                let p = read(&i.operands[0]);
                let mut out = Some(Values::new());
                for (bit, operand) in [(true, &i.operands[1]), (false, &i.operands[2])] {
                    if p.as_ref()
                        .is_none_or(|vs| vs.iter().any(|v| v.bit(0) == bit))
                    {
                        out = out.zip(read(operand)).and_then(|(mut a, b)| {
                            a.extend(b);
                            bound(a, limit)
                        });
                    }
                }
                out
            } else if matches!(
                base,
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
            ) || base.starts_with("icmp.")
            {
                read(&i.operands[0])
                    .zip(read(&i.operands[1]))
                    .and_then(|(a, b)| {
                        let aw = widths[&i.operands[0]];
                        let mut out = Values::new();
                        for x in &a {
                            for y in &b {
                                let v = if let Some(pred) = base.strip_prefix("icmp.") {
                                    BigUint::from_u64(value::icmp(pred, x, y, aw) as u64)
                                } else if matches!(base, "udiv" | "urem" | "sdiv" | "srem") {
                                    if value::div_fault(base, x, y, aw).is_some() {
                                        continue;
                                    }
                                    value::divop(base, x, y, aw)
                                } else {
                                    value::binop_reference(base, x, y, aw)
                                };
                                out.insert(v);
                                if out.len() > limit {
                                    return None;
                                }
                            }
                        }
                        Some(out)
                    })
            } else {
                None
            };
            if let Some(vs) = result {
                after.insert(dst.clone(), vs);
            }
        }
        if i.op == "lift.parmov" {
            for (dst, src) in i
                .lhs
                .iter()
                .zip(crate::validate::list_operand(&i.operands[0])?)
            {
                if let Some(vs) = read(&src) {
                    after.insert(dst.clone(), vs);
                }
            }
        }
        let mut successors = vec![];
        match i.op.as_str() {
            "qjump" => successors.push((target(&i.operands[0])?, None)),
            "qbranch" => {
                let p = read(&i.operands[0]);
                for (bit, label) in [(true, &i.operands[1]), (false, &i.operands[2])] {
                    if p.as_ref()
                        .is_none_or(|vs| vs.iter().any(|v| v.bit(0) == bit))
                    {
                        successors.push((
                            target(label)?,
                            Some((i.operands[0].clone(), BigUint::from_u64(bit as u64))),
                        ));
                    }
                }
            }
            "qjumpr" => {
                if let Some(vs) = read(&i.operands[0]) {
                    for v in vs {
                        let next = v
                            .to_usize()
                            .filter(|v| (first..first + n).contains(v))
                            .map_or(n, |v| v - first);
                        successors.push((next, Some((i.operands[0].clone(), v))));
                    }
                } else {
                    // Top includes every local row and the out-of-range fault.
                    successors.extend((0..=n).map(|next| (next, None)));
                }
            }
            "lift.ret" | "lift.retval" | "fault" => successors.push((n, None)),
            _ => successors.push((row + 1, None)),
        }
        // Budget, memory and arithmetic fault exits have no continuation into
        // an uncall. Keep them in the graph without assuming a concrete token
        // or memory image at this source-proof boundary.
        if matches!(
            i.op.as_str(),
            "lift.call"
                | "lift.alloc"
                | "lift.free"
                | "lift.boundscheck"
                | "lift.faultif"
                | "lift.udiv"
                | "lift.urem"
                | "lift.sdiv"
                | "lift.srem"
        ) || i.op.starts_with("lift.load.")
            || i.op.starts_with("lift.store.")
        {
            successors.push((n, None));
        }
        for (next, refine) in successors {
            edges[row].insert(next);
            let mut state = after.clone();
            if let Some((name, v)) = refine {
                state.insert(name, BTreeSet::from([v]));
            }
            let changed = if let Some(old) = &mut states[next] {
                let prior = old.clone();
                join(old, &state, limit);
                *old != prior
            } else {
                states[next] = Some(state);
                true
            };
            if changed {
                pending.insert(next);
            }
        }
    }
    Ok(edges.into_iter().map(|v| v.into_iter().collect()).collect())
}
