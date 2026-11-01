//! ISA 0.9 §3.7 fixed bindings. Keys contain entry positions, never values.
use crate::{
    object::{FnKind, Function, Instr, Object, Param, Type},
    validate::{err, list_operand, signature_of},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    pub symbol: String,
    pub key: Vec<usize>,
    pub tokens: Vec<usize>,
    pub classical: Vec<usize>,
    pub regions: Vec<usize>,
}

impl Instance {
    fn runtime_name(&self) -> String {
        if self.key.is_empty() {
            self.symbol.clone()
        } else {
            format!(
                "{}${}",
                self.symbol,
                self.key
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            )
        }
    }
    fn origins(&self, f: &Function) -> BTreeMap<String, usize> {
        let mut origins = BTreeMap::new();
        if f.kind == FnKind::Oracle {
            for (i, p) in f.lists[4].iter().enumerate() {
                origins.insert(p.name.clone(), i);
            }
            for (i, p) in f.lists[3].iter().enumerate() {
                origins.insert(p.name.clone(), i);
            }
        } else {
            for (ps, indices) in [&f.lists[2], &f.lists[3], &f.lists[4]].iter().zip([
                &self.tokens,
                &self.classical,
                &self.regions,
            ]) {
                for (p, i) in ps.iter().zip(indices) {
                    origins.insert(p.name.clone(), *i);
                }
            }
        }
        origins
    }
}

/// Private execution graph. Retargeted names are never hashed or serialized as source.
#[derive(Clone)]
pub(crate) struct Plan {
    pub functions: BTreeMap<String, Function>,
    pub order: Vec<String>,
    pub arguments: BTreeMap<(String, String), usize>,
    pub regions: BTreeMap<(String, String), usize>,
}
thread_local! {
    // Bounded to one complete parsed object, with one plan per oracle.
    static PLAN_CACHE: std::cell::RefCell<Option<(String, BTreeMap<String, Plan>)>> = const { std::cell::RefCell::new(None) };
}
impl Plan {
    pub fn form(obj: &Object, root: &str) -> Result<Self, String> {
        let key = format!("{obj:?}");
        if let Some(plan) = PLAN_CACHE.with(|cache| {
            cache
                .borrow()
                .as_ref()
                .filter(|(k, _)| *k == key)
                .and_then(|(_, plans)| plans.get(root))
                .cloned()
        }) {
            return Ok(plan);
        }
        let plan = Self::uncached(obj, root)?;
        PLAN_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.as_ref().is_none_or(|(k, _)| *k != key) {
                *cache = Some((key, BTreeMap::new()));
            }
            cache.as_mut().unwrap().1.insert(root.into(), plan.clone());
        });
        Ok(plan)
    }
    fn uncached(obj: &Object, root: &str) -> Result<Self, String> {
        let instances = for_oracle(obj, root)?;
        let mut plan = Self {
            functions: BTreeMap::new(),
            order: vec![],
            arguments: BTreeMap::new(),
            regions: BTreeMap::new(),
        };
        for instance in &instances {
            let f = obj
                .function(&instance.symbol)
                .ok_or_else(|| err("3.7", "unresolved instance"))?;
            let id = instance.runtime_name();
            let origins = instance.origins(f);
            let (args, mem) = if f.kind == FnKind::Oracle {
                (f.lists[4].iter().collect::<Vec<_>>(), &f.lists[3])
            } else {
                (f.lists[2].iter().chain(&f.lists[3]).collect(), &f.lists[4])
            };
            for p in args {
                plan.arguments
                    .insert((id.clone(), p.name.clone()), origins[&p.name]);
            }
            for p in mem {
                plan.regions
                    .insert((id.clone(), p.name.clone()), origins[&p.name]);
            }
            let mut body = f.clone();
            body.name = id.clone();
            for ins in body
                .blocks
                .iter_mut()
                .flat_map(|b| &mut b.instrs)
                .filter(|i| i.op == "lift.call")
            {
                let mut key = vec![];
                if ins.operands.len() == 5 {
                    for group in &ins.operands[2..] {
                        for n in list_operand(group)? {
                            key.push(
                                *origins
                                    .get(&n)
                                    .ok_or_else(|| err("3.7", "unknown fixed origin"))?,
                            );
                        }
                    }
                }
                let target = instances
                    .iter()
                    .find(|i| i.symbol == ins.operands[0].trim_start_matches('@') && i.key == key)
                    .ok_or_else(|| err("3.7", "missing fixed callee instance"))?;
                ins.operands[0] = format!("@{}", target.runtime_name());
            }
            plan.order.push(id.clone());
            plan.functions.insert(id, body);
        }
        Ok(plan)
    }
}

fn params(f: &Function, group: usize) -> &[Param] {
    match (f.kind, group) {
        (FnKind::Oracle, 2 | 3) => &f.lists[4],
        (FnKind::Oracle, 4) => &f.lists[3],
        _ => &f.lists[group],
    }
}

pub(crate) fn validate_groups(f: &Function) -> Result<(), String> {
    if !matches!(f.kind, FnKind::Oracle | FnKind::Lproc) && !f.regs.is_empty() {
        return Err(err("3.7", "only oracles/lprocs declare locations"));
    }
    if f.blocks
        .iter()
        .flat_map(|b| &b.params)
        .any(|p| matches!(p.role.as_deref(), Some("token" | "classical")))
    {
        return Err(err("16.2", "token/classical suffix on block parameter"));
    }
    if f.regs.iter().any(|p| !matches!(p.ty, Type::Reg(1..))) {
        return Err(err("3.3", "location width must be positive"));
    }
    if matches!(f.kind, FnKind::Action | FnKind::PermAction) && !f.lists[4].is_empty() {
        return Err(err("3.3", "action has four parameter groups"));
    }
    if !matches!(f.kind, FnKind::Cc | FnKind::R) && f.blocks.iter().any(|b| !b.params.is_empty()) {
        return Err(err("3.7", "only CC/R blocks have parameters"));
    }
    for (group, ps) in f.lists.iter().enumerate() {
        for p in ps {
            let valid = match f.kind {
                FnKind::Oracle => match group {
                    0 => {
                        matches!(p.ty, Type::Wire(1..))
                            && p.role.as_deref().is_none_or(|r| r == "in")
                    }
                    1 | 2 => matches!(p.ty, Type::Wire(1..)) && p.role.is_none(),
                    3 => matches!(p.ty, Type::Region(_, 1..)) && p.role.as_deref() == Some("in"),
                    4 => {
                        matches!(p.ty, Type::I(1..=128) | Type::Ptr | Type::F64 | Type::Token)
                            && p.role.is_none()
                    }
                    _ => false,
                },
                FnKind::Action | FnKind::PermAction => match group {
                    0 | 1 => {
                        matches!(p.ty, Type::Wire(1..))
                            && p.role
                                .as_deref()
                                .is_none_or(|r| r == if group == 0 { "mut" } else { "in" })
                    }
                    2 => {
                        matches!(p.ty, Type::Region(_, 1..))
                            && matches!(p.role.as_deref(), Some("in" | "mut"))
                    }
                    3 => {
                        matches!(p.ty, Type::I(1..=128) | Type::Ptr | Type::F64 | Type::Token)
                            && p.role.is_none()
                    }
                    _ => false,
                },
                _ => true,
            };
            if !valid {
                return Err(err(
                    "3.3",
                    "header parameter type/role disagrees with group",
                ));
            }
        }
    }
    for (group, ps) in f.lists.iter().enumerate() {
        for p in ps {
            if matches!(p.role.as_deref(), Some("token" | "classical"))
                && !(f.kind == FnKind::Lproc
                    && ((group == 2 && p.role.as_deref() == Some("token"))
                        || (group == 3 && p.role.as_deref() == Some("classical"))))
            {
                return Err(err("3.7", "parameter suffix disagrees with its group"));
            }
            if f.kind == FnKind::Lproc {
                let valid = match group {
                    0 | 1 => matches!(p.ty, Type::Reg(1..)) && p.role.is_none(),
                    2 => p.ty == Type::Token && p.role.as_deref().is_none_or(|r| r == "token"),
                    3 => {
                        matches!(p.ty, Type::I(1..=128) | Type::Ptr | Type::F64)
                            && p.role.as_deref().is_none_or(|r| r == "classical")
                    }
                    4 => matches!(p.ty, Type::Region(_, 1..)) && p.role.as_deref() == Some("in"),
                    _ => unreachable!(),
                };
                if !valid {
                    return Err(err(
                        "3.7",
                        format!(
                            "invalid group {} parameter {} in @{}",
                            group + 1,
                            p.name,
                            f.name
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Validate every explicit operand group even for imports; no run-time values enter here.
pub(crate) fn validate_call(obj: &Object, f: &Function, ins: &Instr) -> Result<(), String> {
    let name = ins
        .operands
        .first()
        .and_then(|s| s.strip_prefix('@'))
        .ok_or_else(|| err("3.7", "lift.call requires a named lproc"))?;
    let sig = signature_of(obj, name)?;
    let types = ["tokens", "args", "mem"].map(|k| {
        sig.get(k)
            .and_then(|v| v.as_arr())
            .ok_or_else(|| err("3.7", "lproc signature missing binding group"))
    });
    let [tokens, classical, regions] = types;
    let groups = [tokens?, classical?, regions?];
    let empty = groups.iter().all(|g| g.is_empty());
    if !(ins.operands.len() == 5 || (ins.operands.len() == 2 && empty)) {
        return Err(err(
            "3.7",
            "lift.call requires all four lists for nonempty bindings",
        ));
    }
    let result = ins
        .lhs
        .first()
        .and_then(|n| {
            crate::oracle::location_widths(f)
                .into_iter()
                .find(|(p, _)| p == n)
        })
        .map(|(_, w)| w);
    if result != sig.get("result").and_then(|v| v.as_usize()) {
        return Err(err("3.7", "lift.call destination width mismatch"));
    }
    let locations = list_operand(&ins.operands[1])?;
    let widths = sig
        .get("params")
        .and_then(|v| v.as_arr())
        .ok_or_else(|| err("3.7", "missing location types"))?;
    if locations.len() != widths.len() {
        return Err(err("3.7", "location argument count mismatch"));
    }
    for (n, w) in locations.iter().zip(widths) {
        let p = f
            .lists
            .iter()
            .flatten()
            .chain(&f.regs)
            .find(|p| p.name == *n)
            .ok_or_else(|| err("3.7", format!("unknown location argument {n}")))?;
        if !matches!(p.ty, Type::Reg(_) | Type::Wire(_) | Type::I(_))
            || p.ty.width() != w.as_usize()
        {
            return Err(err("3.7", "location argument width mismatch"));
        }
    }
    if ins.operands.len() == 2 {
        return Ok(());
    }
    for (j, want) in groups.iter().enumerate() {
        let actual = list_operand(&ins.operands[j + 2])?;
        if actual.len() != want.len() {
            return Err(err("3.7", "fixed argument count mismatch"));
        }
        for (name, ty) in actual.iter().zip(*want) {
            let p = params(f, j + 2)
                .iter()
                .find(|p| p.name == *name)
                .ok_or_else(|| err("3.7", format!("{name} is not an immutable entry binding")))?;
            let valid = if j == 2 {
                matches!(p.ty, Type::Region(a,w) if ty.get("A").and_then(|v|v.as_usize()) == Some(a) && ty.get("W").and_then(|v|v.as_usize()) == Some(w))
                    && p.role.as_deref() == Some("in")
            } else {
                Some(p.ty.signature_text().as_str()) == ty.as_str()
            };
            if !valid {
                return Err(err("3.7", "fixed argument type/role mismatch"));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_writes(f: &Function, ins: &Instr) -> Result<(), String> {
    for dst in &ins.lhs {
        if !crate::oracle::location_widths(f)
            .iter()
            .any(|(n, _)| n == dst)
        {
            return Err(err(
                "3.7",
                format!("write to non-location {dst} in @{}", f.name),
            ));
        }
    }
    Ok(())
}

/// Oracle first; lproc SCC/member ordering, then lexicographic fixed keys.
/// Repeated and recursive calls with one key share one row instance.
pub fn for_oracle(obj: &Object, oracle: &str) -> Result<Vec<Instance>, String> {
    let root = obj
        .function(oracle)
        .filter(|f| f.kind == FnKind::Oracle)
        .ok_or_else(|| err("3.7", "fixed bindings require an oracle"))?;
    let mut out = vec![Instance {
        symbol: oracle.into(),
        key: vec![],
        tokens: vec![],
        classical: vec![],
        regions: vec![],
    }];
    let mut seen = BTreeSet::new();
    let mut cursor = 0;
    while cursor < out.len() {
        let instance = out[cursor].clone();
        cursor += 1;
        let f = obj
            .function(&instance.symbol)
            .ok_or_else(|| err("3.7", format!("unresolved instance @{}", instance.symbol)))?;
        let mut mapping = BTreeMap::new();
        if f.kind == FnKind::Oracle {
            for (i, p) in f.lists[4].iter().enumerate() {
                mapping.insert(p.name.clone(), i);
            }
            for (i, p) in f.lists[3].iter().enumerate() {
                mapping.insert(p.name.clone(), i);
            }
        } else {
            for (ps, indices) in [&f.lists[2], &f.lists[3], &f.lists[4]].iter().zip([
                &instance.tokens,
                &instance.classical,
                &instance.regions,
            ]) {
                for (p, i) in ps.iter().zip(indices) {
                    mapping.insert(p.name.clone(), *i);
                }
            }
        }
        let mut flat = vec![];
        f.flat_instrs(&mut flat);
        for ins in flat.into_iter().filter(|i| i.op == "lift.call") {
            if obj.isa == "0.9" {
                validate_call(obj, f, ins)?;
            }
            let symbol = ins.operands[0].trim_start_matches('@').to_string();
            let mut groups = [vec![], vec![], vec![]];
            if ins.operands.len() == 5 {
                for (j, g) in groups.iter_mut().enumerate() {
                    for n in list_operand(&ins.operands[j + 2])? {
                        g.push(
                            *mapping
                                .get(&n)
                                .ok_or_else(|| err("3.7", "unresolved fixed argument origin"))?,
                        );
                    }
                }
            }
            let key = groups.concat();
            if seen.insert((symbol.clone(), key.clone())) {
                let [tokens, classical, regions] = groups;
                out.push(Instance {
                    symbol,
                    key,
                    tokens,
                    classical,
                    regions,
                });
            }
        }
    }
    let order = crate::oracle::index_order(obj, root)?;
    out[1..].sort_by(|a, b| {
        order
            .iter()
            .position(|n| *n == a.symbol)
            .cmp(&order.iter().position(|n| *n == b.symbol))
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(out)
}
