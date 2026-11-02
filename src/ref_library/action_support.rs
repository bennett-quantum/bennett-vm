//! K83 annotates existing device support; native host entry is unaffected.
use super::*;
use crate::{
    object::{FnKind, Object},
    sched::{OperationKind as K, Point},
};

#[derive(Default)]
pub(crate) struct Plan {
    // Declaration point -> (construct offset, release offset, creation size).
    origins: BTreeMap<String, (usize, usize, usize)>,
    pub(crate) construct: Vec<K>,
    pub(crate) release: Vec<K>,
}
pub(crate) fn plans(
    object: &Object,
    ids: &crate::identity::Identities,
) -> Result<BTreeMap<String, Plan>, String> {
    let mut plans = BTreeMap::new();
    if object.isa != "0.9" {
        return Ok(plans);
    }
    for f in &object.functions {
        if !matches!(f.kind, FnKind::Action | FnKind::PermAction) {
            continue;
        }
        let mut plan = Plan::default();
        let mut regions = vec![];
        for (point, source) in crate::sched::points(f) {
            let i = source.instruction;
            if i.op != "qregion" {
                continue;
            }
            let gates = if let Some(image) = i.operands.get(3) {
                let image = object
                    .data
                    .get(image.trim_start_matches('@'))
                    .ok_or_else(|| err("unbound region image"))?;
                image
                    .chars()
                    .map(|c| {
                        c.to_digit(16)
                            .ok_or_else(|| err("invalid region image"))
                            .map(|n| n.count_ones() as usize)
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .iter()
                    .sum()
            } else {
                0
            };
            let count = 1 + gates;
            let offset = plan.construct.len();
            plan.construct.push(K::Alloc);
            plan.construct.extend(std::iter::repeat_n(K::Gate, gates));
            regions.push((point, offset, count));
        }
        for (point, construct, count) in regions.into_iter().rev() {
            let release = plan.release.len();
            plan.release.extend(std::iter::repeat_n(K::Gate, count - 1));
            plan.release.push(K::Release);
            plan.origins.insert(point, (construct, release, count));
        }
        plans.insert(ids.interface[&f.name].clone(), plan);
    }
    Ok(plans)
}

fn origin<'a>(
    plans: &'a BTreeMap<String, Plan>,
    path: &[Point],
) -> Option<&'a (usize, usize, usize)> {
    let p = path.last()?;
    plans.get(&p.symbol)?.origins.get(&p.point)
}
fn support_path(path: &mut [Point], point: &str) {
    let p = path.last_mut().expect("support has an owning declaration");
    p.point = point.into();
    p.iteration.clear();
    p.instance.clear();
    p.call_index = None;
}
pub(super) fn annotate(
    registry: &Registry,
    leaves: &mut [Leaf],
    calls: &mut BoundCalls,
) -> Result<(), String> {
    let plans = plans(&registry.object, &registry.identities)?;
    for leaf in leaves {
        let Some(a) = leaf.address.as_mut() else {
            continue;
        };
        let Some(&(construct, release, count)) = origin(&plans, &a.path) else {
            continue;
        };
        if a.generated >= count * 2 {
            return Err(err("region support ordinal outside declaration expansion"));
        }
        if a.generated < count {
            support_path(&mut a.path, "$construct");
            a.generated += construct;
        } else {
            support_path(&mut a.path, "$release");
            // The recipe's inverse-image subtree retains its preparation
            // ordinals. K83 numbers restoration in emitted forward order.
            a.generated = release
                + if a.generated == count * 2 - 1 {
                    count - 1
                } else {
                    count * 2 - 2 - a.generated
                };
        }
    }
    let mut support = BTreeSet::new();
    let mut domains = Vec::with_capacity(calls.domains.len());
    for domain in std::mem::take(&mut calls.domains) {
        if origin(&plans, &domain.path).is_none() {
            domains.push(domain);
            continue;
        }
        if !domain.targets.is_empty() {
            return Err(err("region declaration opened a semantic callee"));
        }
        for point in ["$construct", "$release"] {
            let mut path = domain.path.clone();
            support_path(&mut path, point);
            if support.insert(path.clone()) {
                domains.push(CallDomain {
                    path,
                    targets: vec![],
                });
            }
        }
    }
    calls.domains = domains;
    Ok(())
}
