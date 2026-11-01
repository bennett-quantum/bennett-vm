//! Source proof for K65 lifted pairs, independent of constructor descriptors.
use super::*;
use crate::object::{Function, Instr};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleCleanupPair {
    pub computing_point: crate::sched::Point,
    pub cleanup_point: crate::sched::Point,
}

fn failure(f: &Function, i: &Instr, why: &str) -> String {
    err(format!(
        "§5.5: @{} line {} {}: {why}",
        f.name, i.line, i.raw
    ))
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    callee: String,
    token: String,
    inputs: Vec<String>,
    memory: Vec<String>,
    arguments: Vec<String>,
}
fn key(i: &Instr) -> Result<Option<Identity>, String> {
    if !matches!(i.op.as_str(), "lift.oracle_call" | "lift.oracle_uncall") {
        return Ok(None);
    }
    Ok(Some(Identity {
        callee: i.operands[0].clone(),
        token: i.operands[1].clone(),
        inputs: crate::validate::list_operand(&i.operands[2])?,
        memory: crate::validate::list_operand(&i.operands[5])?,
        arguments: crate::validate::list_operand(&i.operands[6])?,
    }))
}
fn writes(i: &Instr) -> Result<Vec<String>, String> {
    let mut writes = i.lhs.clone();
    match i.op.as_str() {
        "lift.oracle_call" | "lift.oracle_uncall" => {
            writes.extend(i.operands[3..5].iter().cloned());
        }
        "lift.perm_call" => writes.extend(crate::validate::list_operand(&i.operands[1])?),
        _ => {}
    }
    Ok(writes)
}
fn reachable(edges: &[Vec<usize>], start: usize, stop: Option<usize>) -> Vec<bool> {
    let mut seen = vec![false; edges.len()];
    let mut pending = vec![start];
    while let Some(n) = pending.pop() {
        if !seen[n] {
            seen[n] = true;
            if Some(n) != stop {
                pending.extend(&edges[n]);
            }
        }
    }
    seen
}

/// Balanced-path saturation handles branches and balanced cycles without
/// unrolling a source path or bounding the number of retained invocations.
/// Each §5.5 operand identity has an independent LIFO stack. Other call
/// identities are neutral for this stack.
pub(super) fn pairs(f: &Function) -> Result<BTreeMap<usize, usize>, String> {
    let rows = f.blocks.iter().flat_map(|b| &b.instrs).collect::<Vec<_>>();
    if !rows.iter().any(|i| i.op == "lift.oracle_uncall") {
        return Ok(BTreeMap::new());
    }
    let n = rows.len();
    let mut labels = BTreeMap::new();
    let mut offset = 0;
    for b in &f.blocks {
        labels.insert(b.name.as_str(), offset);
        offset += b.instrs.len();
    }
    let target = |s: &str| -> Result<usize, String> {
        labels
            .get(s.trim_start_matches('^'))
            .copied()
            .ok_or_else(|| err("§5.5: unresolved lifted control-flow target"))
    };
    let mut edges = vec![vec![]; n + 1];
    for (row, i) in rows.iter().enumerate() {
        edges[row] = match i.op.as_str() {
            "qjump" => vec![target(&i.operands[0])?],
            "qbranch" => vec![target(&i.operands[1])?, target(&i.operands[2])?],
            // No guessed target set may stand in for a proof of pair identity.
            "qjumpr" => {
                return Err(failure(
                    f,
                    i,
                    "unavailable.reason: indirect lifted cleanup path proof remains staged in bennett-vm-47t-c2",
                ));
            }
            "lift.ret" | "lift.retval" | "fault" => vec![n],
            _ => vec![row + 1],
        };
    }
    let live = reachable(&edges, 0, None);
    let keys = rows.iter().map(|i| key(i)).collect::<Result<Vec<_>, _>>()?;
    let identities = rows
        .iter()
        .enumerate()
        .filter(|(_, i)| i.op == "lift.oracle_uncall")
        .filter_map(|(n, _)| keys[n].clone())
        .collect::<BTreeSet<_>>();
    let mut result = BTreeMap::new();
    for identity in identities {
        let opens = rows
            .iter()
            .enumerate()
            .filter(|(row, i)| i.op == "lift.oracle_call" && keys[*row].as_ref() == Some(&identity))
            .map(|(row, _)| row)
            .collect::<Vec<_>>();
        let closes = rows
            .iter()
            .enumerate()
            .filter(|(row, i)| {
                i.op == "lift.oracle_uncall" && keys[*row].as_ref() == Some(&identity)
            })
            .map(|(row, _)| row)
            .collect::<Vec<_>>();
        let mut balanced = vec![vec![false; n + 1]; n + 1];
        for row in 0..=n {
            balanced[row][row] = true;
            if !opens.contains(&row) && !closes.contains(&row) {
                for &next in &edges[row] {
                    balanced[row][next] = true;
                }
            }
        }
        loop {
            // Concatenation and c B u are precisely the Dyck grammar.
            for k in 0..=n {
                let suffixes = balanced[k].clone();
                for prefixes in &mut balanced {
                    if prefixes[k] {
                        for (path, suffix) in prefixes.iter_mut().zip(&suffixes) {
                            *path |= suffix;
                        }
                    }
                }
            }
            let mut changed = false;
            for &open in &opens {
                for &close in &closes {
                    if edges[open].iter().any(|&next| balanced[next][close]) {
                        for &next in &edges[close] {
                            if !balanced[open][next] {
                                balanced[open][next] = true;
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for close in closes.into_iter().filter(|row| live[*row]) {
            let i = rows[close];
            if balanced[0][close] {
                return Err(failure(f, i, "uncall has a path with no matching call"));
            }
            let candidates = opens
                .iter()
                .copied()
                .filter(|&open| live[open] && edges[open].iter().any(|&next| balanced[next][close]))
                .collect::<Vec<_>>();
            if candidates.len() != 1 {
                return Err(failure(
                    f,
                    i,
                    "uncall does not match one source call on every path",
                ));
            }
            let open = candidates[0];
            if rows[open].operands[3..5] != i.operands[3..5] {
                return Err(failure(
                    f,
                    i,
                    "uncall must use the matched call's successor targets",
                ));
            }
            let inputs = crate::validate::list_operand(&i.operands[2])?;
            let mut reverse = vec![vec![]; n + 1];
            for (a, successors) in edges.iter().enumerate() {
                for &b in successors {
                    reverse[b].push(a);
                }
            }
            let to_close = reachable(&reverse, close, Some(open));
            let mut between = vec![false; n + 1];
            for &next in &edges[open] {
                let from_open = reachable(&edges, next, Some(close));
                for row in 0..n {
                    between[row] |= from_open[row] && to_close[row] && row != close;
                }
            }
            for row in 0..n {
                if between[row] && writes(rows[row])?.iter().any(|w| inputs.contains(w)) {
                    return Err(failure(
                        f,
                        i,
                        "intervening row writes a matched call input location",
                    ));
                }
            }
            result.insert(close, open);
        }
    }
    Ok(result)
}
