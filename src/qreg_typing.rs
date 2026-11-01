//! Exactly the name-based qreg/view admission rules of §3.3. Alias/width
//! preconditions dependent on device state remain atomic runtime checks.
use crate::{
    object::{Function, Instr, Type},
    validate::{branch_target, err, list_operand},
};
use std::collections::{BTreeMap, BTreeSet};
fn uses(i: &Instr, name: &str) -> bool {
    if matches!(i.op.as_str(), "adj" | "ctl") {
        let (_, control, op, operands) =
            crate::action::normalize_call(i).expect("validated region call");
        if control == Some(name) {
            return true;
        }
        let call = Instr {
            op: op.into(),
            operands: operands.to_vec(),
            ..i.clone()
        };
        return uses(&call, name);
    }
    // Device/service, class and mnemonic tokens are not value references,
    // even when a legal classical name has exactly the same spelling.
    let skip = usize::from(matches!(i.op.as_str(), "effect" | "hostcall" | "qreg.new"));
    i.operands.iter().skip(skip).any(|s| {
        let bytes = s.as_bytes();
        let mut start = 0;
        while start < bytes.len() {
            if bytes[start].is_ascii_alphabetic() || bytes[start] == b'_' {
                let mut end = start + 1;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'.'))
                {
                    end += 1;
                }
                if &s[start..end] == name
                    && (start == 0 || !matches!(bytes[start - 1], b'@' | b'^'))
                {
                    return true;
                }
                start = end;
            } else {
                start += 1;
            }
        }
        false
    })
}
fn region(i: &Instr) -> bool {
    matches!(i.op.as_str(), "action_call" | "oracle_call" | "adj" | "ctl")
}
fn slots(i: &Instr) -> Result<Vec<String>, String> {
    let (_, _, op, xs) = crate::action::normalize_call(i)?;
    if op == "oracle_call" {
        Ok([
            list_operand(&xs[2])?,
            vec![xs[3].clone(), xs[4].clone()],
            list_operand(&xs[5])?,
        ]
        .concat())
    } else {
        Ok([
            list_operand(&xs[1])?,
            list_operand(&xs[2])?,
            list_operand(&xs[3])?,
        ]
        .concat())
    }
}
pub(crate) fn function(f: &Function, env: &BTreeMap<String, Type>) -> Result<(), String> {
    for b in &f.blocks {
        for (index, i) in b.instrs.iter().enumerate() {
            let args = if region(i) {
                slots(i)?
            } else if i.op == "qreg.concat" {
                i.operands.clone()
            } else {
                vec![]
            };
            let mut names = BTreeSet::new();
            for n in &args {
                if matches!(env.get(n), Some(Type::Qreg(_) | Type::QregDynamic)) && !names.insert(n)
                {
                    return Err(err("3.3", "same qreg name fills two operands"));
                }
            }
            if i.op != "qreg.slice" {
                continue;
            }
            let (view, parent) = (&i.lhs[0], &i.operands[0]);
            let mut pending = vec![(b.name.clone(), index + 1, false)];
            let mut seen = BTreeSet::new();
            while let Some((block, pos, consumed)) = pending.pop() {
                if !seen.insert((block.clone(), pos, consumed)) {
                    continue;
                }
                let b = f.block(&block).unwrap();
                let Some(next) = b.instrs.get(pos) else {
                    if !consumed {
                        return Err(err("3.3", "view reaches function end unconsumed"));
                    }
                    continue;
                };
                let call = region(next);
                let used = uses(next, view);
                if used && (!call || consumed) {
                    return Err(err(
                        "3.3",
                        "view must be consumed by exactly one region call",
                    ));
                }
                if !consumed && uses(next, parent) {
                    return Err(err("3.3", "slice parent used before view consumption"));
                }
                let consumed = consumed || used;
                if next.lhs.contains(view) {
                    if !consumed {
                        return Err(err("3.3", "live view overwritten"));
                    }
                    continue; // A new dynamic binding is checked at its own producer.
                }
                match next.op.as_str() {
                    "trap" | "fault" => {}
                    "ret" => {
                        if !consumed {
                            return Err(err("3.3", "view reaches exit unconsumed"));
                        }
                    }
                    "br" => {
                        for target in &next.operands[usize::from(next.operands.len() == 3)..] {
                            let (name, _) = branch_target(target).unwrap();
                            pending.push((name, 0, consumed));
                        }
                    }
                    _ => pending.push((block, pos + 1, consumed)),
                }
            }
        }
    }
    Ok(())
}
