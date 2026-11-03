//! Resolve source-local allocation names before forming an action adjoint.
//! Loop backedges identify the entry/exit slots; physical lifetime occurrences
//! are still assigned after literal traversal, including each loop visit.
use crate::object::{Function, Instr, Type};
use std::collections::{BTreeMap, BTreeSet};

fn substitute(s: &str, names: &BTreeMap<String, String>) -> String {
    let mut result = String::new();
    let mut start = 0;
    for (end, c) in s.char_indices().chain(std::iter::once((s.len(), ' '))) {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$') {
            continue;
        }
        let token = &s[start..end];
        let symbol = start > 0 && s.as_bytes()[start - 1] == b'@';
        result.push_str(if symbol { token } else { names.get(token).map_or(token, String::as_str) });
        if end < s.len() { result.push(c); }
        start = end + c.len_utf8();
    }
    result
}

struct Bindings {
    live: BTreeMap<String, String>,
    used: BTreeSet<String>,
    reserved: BTreeSet<String>,
    parent: BTreeMap<String, String>,
    next: usize,
}
impl Bindings {
    fn root(&self, mut name: String) -> String {
        while let Some(parent) = self.parent.get(&name) {
            name = parent.clone();
        }
        name
    }
    fn walk(&mut self, rows: &mut [Instr]) {
        for i in rows {
            let original_operands = i.operands.clone();
            if !matches!(i.op.as_str(), "qalloc" | "qborrow") {
                for operand in &mut i.operands { *operand = substitute(operand, &self.live); }
            }
            if matches!(i.op.as_str(), "qalloc" | "qborrow" | "qcopy" | "qload") {
                let original = i.lhs[0].clone();
                let name = if self.used.insert(original.clone()) {
                    original.clone()
                } else {
                    loop {
                        let name = format!("$allocation.{}.{}", i.line, self.next);
                        self.next += 1;
                        if !self.reserved.contains(&name) && self.used.insert(name.clone()) { break name; }
                    }
                };
                i.lhs[0] = name.clone();
                self.live.insert(original, name);
            }
            let released = match i.op.as_str() {
                "qfree" | "qreturn" => original_operands.first(),
                "quncopy" => original_operands.get(1),
                "qunload" => original_operands.get(2),
                _ => None,
            };
            if let Some(name) = released { self.live.remove(name); }
            let before = self.live.clone();
            for arm in &mut i.nested { self.walk(arm); }
            if i.op == "qloop" {
                let count: usize = i.operands[0].parse().expect("validated loop bound");
                if count == 0 {
                    self.live = before;
                } else if count > 1 {
                    for (name, entry) in before {
                        let exit = self.live.get(&name).expect("validated loop live names");
                        let entry = self.root(entry);
                        let exit = self.root(exit.clone());
                        if entry != exit { self.parent.insert(exit, entry); }
                    }
                }
            }
        }
    }
}

pub(crate) fn bind_allocations(f: &Function) -> Function {
    let interface = f.lists.iter().flatten().map(|p| (p.name.clone(), p.name.clone())).collect::<BTreeMap<_, _>>();
    let mut flat = vec![];
    f.flat_instrs(&mut flat);
    let mut occupied = interface.values().cloned().collect::<BTreeSet<_>>();
    for i in flat {
        for value in i.lhs.iter().chain(&i.operands) {
            occupied.extend(value.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$'))).map(String::from));
        }
    }
    let mut bindings = Bindings { live: interface.clone(), used: interface.into_keys().collect(), reserved: occupied, parent: BTreeMap::new(), next: 0 };
    let mut result = f.clone();
    for block in &mut result.blocks { bindings.walk(&mut block.instrs); }
    let aliases = bindings.parent.keys().map(|n| (n.clone(), bindings.root(n.clone()))).collect();
    fn finish(rows: &mut [Instr], aliases: &BTreeMap<String, String>) {
        for i in rows {
            for name in &mut i.lhs { *name = substitute(name, aliases); }
            if !matches!(i.op.as_str(), "qalloc" | "qborrow") {
                for name in &mut i.operands { *name = substitute(name, aliases); }
            }
            for arm in &mut i.nested { finish(arm, aliases); }
        }
    }
    for block in &mut result.blocks { finish(&mut block.instrs, &aliases); }
    result
}

pub(super) type ReleasePlan = BTreeMap<(usize, Vec<usize>), (usize, String)>;

/// Allocation/release rows are unconditional. Resolve their metadata over the
/// literal forward visits before reversing gates; predicates need no sampling.
pub(super) fn release_metadata(f: &Function) -> ReleasePlan {
    let mut regions = f.lists[2].iter().filter_map(|p| match p.ty {
        Type::Region(_, width) => Some((p.name.clone(), width)),
        _ => None,
    }).collect::<BTreeMap<_, _>>();
    let mut flat = vec![];
    f.flat_instrs(&mut flat);
    for i in flat {
        if i.op == "qregion" {
            regions.insert(i.operands[0].clone(), i.operands[2].parse().expect("validated region width"));
        }
    }
    let mut live = f.lists[..2].iter().flatten().filter_map(|p| match p.ty {
        Type::Wire(width) => Some((p.name.clone(), (width, "any".to_string()))),
        _ => None,
    }).collect::<BTreeMap<_, _>>();
    fn walk(rows: &[Instr], live: &mut BTreeMap<String, (usize, String)>, regions: &BTreeMap<String, usize>, iterations: &mut Vec<usize>, plan: &mut ReleasePlan) {
        for i in rows {
            match i.op.as_str() {
                "qalloc" | "qborrow" => {
                    live.insert(i.lhs[0].clone(), (i.operands[1].parse().expect("validated allocation width"), i.operands[0].clone()));
                }
                "qcopy" => {
                    let source = &i.operands[0];
                    let width = if source.contains('[') { 1 } else { live.get(source).expect("live source copy").0 };
                    live.insert(i.lhs[0].clone(), (width, "any".into()));
                }
                "qload" => { live.insert(i.lhs[0].clone(), (regions[&i.operands[0]], "any".into())); }
                "qfree" | "qreturn" => {
                    let metadata = live.remove(&i.operands[0]).expect("validated live release");
                    assert!(plan.insert((i.line, iterations.clone()), metadata).is_none(), "unique literal source visit");
                }
                "quncopy" => { live.remove(&i.operands[1]).expect("validated live uncopy"); }
                "qunload" => { live.remove(&i.operands[2]).expect("validated live unload"); }
                "qloop" => {
                    for visit in 0..i.operands[0].parse().expect("validated loop bound") {
                        iterations.push(visit);
                        walk(&i.nested[0], live, regions, iterations, plan);
                        iterations.pop();
                    }
                }
                _ => {
                    for arm in &i.nested { walk(arm, live, regions, iterations, plan); }
                }
            }
        }
    }
    let mut plan = ReleasePlan::new();
    for block in &f.blocks { walk(&block.instrs, &mut live, &regions, &mut vec![], &mut plan); }
    plan
}
