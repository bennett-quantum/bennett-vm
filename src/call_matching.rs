//! §5.5: on every path a `lift.oracle_uncall` matches the most recent preceding
//! unmatched `lift.oracle_call` with the same operands: callee, token, `ins`, `tgt`,
//! `fr'`, `mem` and `args`. Matching no call on some path, different calls on
//! different paths, or a row between the pair writing one of its `ins` is
//! ill-formed. An intervening `lift.store` is not: its address is data.
//!
//! `tgt` and `fr'` are part of the match at 0.8 and 0.9 alike, as in the reference
//! interpreter (ruling R1, 2026-11-02): two pairs on different targets may cross.
//! 0.9's E17.5 sentence reads as LIFO on the other five operands only; that text
//! is recorded against the reference in docs/reviews/2026-11-02-vm-fixes-review.md.
//!
//! A `qjumpr` target is a run-time value, so a function with one is left to the
//! value-precise pairing of `ref_library`'s `oracle_cleanup_pairs`: the paths this
//! CFG would add are not all paths the program has.
use crate::{
    object::{Function, Instr},
    validate::{err, list_operand},
};
use std::collections::BTreeMap;

/// Callee, token, `ins`, `tgt`, `fr'`, `mem`, `args`, as written.
type Key = Vec<String>;

#[derive(Clone, PartialEq)]
struct Outstanding {
    /// Paths disagree below `calls`, so nothing under them can be matched.
    disagree_below: bool,
    /// (calling row, no row has written its `ins` since), oldest first.
    calls: Vec<(usize, bool)>,
}

type State = BTreeMap<Key, Outstanding>;

fn key(ins: &Instr) -> Key {
    ins.operands[..7].to_vec()
}

fn meet(a: &State, b: &State) -> State {
    let mut out = State::new();
    for k in a.keys().chain(b.keys()) {
        let none = Outstanding {
            disagree_below: false,
            calls: vec![],
        };
        let (x, y) = (a.get(k).unwrap_or(&none), b.get(k).unwrap_or(&none));
        let shared = x
            .calls
            .iter()
            .rev()
            .zip(y.calls.iter().rev())
            .take_while(|(p, q)| p == q)
            .count();
        let agree = shared == x.calls.len()
            && shared == y.calls.len()
            && x.disagree_below == y.disagree_below;
        out.insert(
            k.clone(),
            Outstanding {
                disagree_below: !agree || x.disagree_below,
                calls: x.calls[x.calls.len() - shared..].to_vec(),
            },
        );
    }
    out
}

fn transfer(rows: &[&Instr], row: usize, state: &State) -> Result<State, String> {
    let ins = rows[row];
    let mut next = state.clone();
    let mut writes = ins.lhs.clone();
    let delivery = matches!(ins.op.as_str(), "lift.oracle_call" | "lift.oracle_uncall");
    if delivery {
        writes.extend([ins.operands[3].clone(), ins.operands[4].clone()]);
    } else if ins.op == "lift.perm_call" {
        writes.extend(list_operand(&ins.operands[1])?);
    }
    if ins.op == "lift.oracle_uncall"
        && let Some(entry) = next.get_mut(&key(ins))
    {
        entry.calls.pop();
    }
    for (k, entry) in next.iter_mut() {
        let read = list_operand(&k[2])?;
        if read.iter().any(|n| writes.contains(n)) {
            for call in &mut entry.calls {
                call.1 = false;
            }
        }
    }
    if ins.op == "lift.oracle_call" {
        next.entry(key(ins))
            .or_insert(Outstanding {
                disagree_below: false,
                calls: vec![],
            })
            .calls
            .push((row, true));
    }
    Ok(next)
}

pub(crate) fn function(f: &Function) -> Result<(), String> {
    let mut rows = vec![];
    let mut starts = BTreeMap::new();
    for block in &f.blocks {
        starts.insert(block.name.as_str(), rows.len());
        rows.extend(&block.instrs);
    }
    if !rows.iter().any(|i| i.op == "lift.oracle_uncall") || rows.iter().any(|i| i.op == "qjumpr") {
        return Ok(());
    }
    let label = |target: &str| -> Result<usize, String> {
        let name = target.trim_start_matches('^');
        let name = name.split('(').next().unwrap_or(name);
        starts
            .get(name)
            .copied()
            .ok_or_else(|| err("3.6", format!("unknown block {target} in @{}", f.name)))
    };
    let successors = |row: usize| -> Result<Vec<usize>, String> {
        let ins = rows[row];
        Ok(match ins.op.as_str() {
            "qjump" => vec![label(&ins.operands[0])?],
            "qbranch" => vec![label(&ins.operands[1])?, label(&ins.operands[2])?],
            "lift.retval" | "lift.ret" | "fault" => vec![],
            _ if row + 1 < rows.len() => vec![row + 1],
            _ => vec![],
        })
    };
    let mut entry: Vec<Option<State>> = vec![None; rows.len()];
    entry[0] = Some(State::new());
    let mut work = vec![0];
    while let Some(row) = work.pop() {
        let out = transfer(&rows, row, entry[row].as_ref().unwrap())?;
        for next in successors(row)? {
            let merged = match &entry[next] {
                None => out.clone(),
                Some(prior) => meet(prior, &out),
            };
            if entry[next].as_ref() != Some(&merged) {
                entry[next] = Some(merged);
                work.push(next);
            }
        }
    }
    for (row, state) in entry.iter().enumerate() {
        let ins = rows[row];
        let Some(state) = state.as_ref().filter(|_| ins.op == "lift.oracle_uncall") else {
            continue;
        };
        let matched = state.get(&key(ins)).and_then(|o| o.calls.last());
        let Some((_, unwritten)) = matched else {
            return Err(err(
                "5.5",
                format!(
                    "lift.oracle_uncall at line {} in @{} matches no single lift.oracle_call on every path",
                    ins.line, f.name
                ),
            ));
        };
        if !unwritten {
            return Err(err(
                "5.5",
                format!(
                    "a row between lift.oracle_call and lift.oracle_uncall at line {} in @{} writes its ins",
                    ins.line, f.name
                ),
            ));
        }
    }
    Ok(())
}
