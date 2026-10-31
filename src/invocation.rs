//! ISA 0.9 §12.3 positional argument and invocation binding. Token values are
//! copied tuples, never host ids; binary64 bits survive without decimal conversion.
use crate::{
    bigint::BigUint as U,
    json::{self, Json},
    object::{FnKind, Object, Type},
    target::{arr, field, keys, string},
    value::{Budget, Value},
};
pub fn unsigned(j: &Json) -> Result<U, String> {
    let s = string(j)?;
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("§12.3: expected canonical unsigned decimal string".into());
    }
    U::parse_decimal(s)
}
pub fn decode_budget(j: &Json) -> Result<Budget, String> {
    let a = arr(j)?;
    if a.len() != 4 {
        return Err("§12.3: budget requires four explicit components".into());
    }
    let a = a
        .iter()
        .map(|x| {
            unsigned(x)?
                .to_u64()
                .ok_or("§12.3: host limit: budget exceeds u64".into())
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Budget {
        steps: a[0],
        heap: a[1],
        stack: a[2],
        depth: a[3],
    })
}
pub fn decode_args(j: &Json, types: &[Type], ptr_width: usize) -> Result<Vec<Value>, String> {
    let a = arr(j)?;
    if a.len() != types.len() {
        return Err("§12.3: resolved argument arity mismatch (no defaults)".into());
    }
    a.iter()
        .zip(types)
        .map(|(j, t)| {
            Ok(match t {
                Type::I(n) => {
                    let b = unsigned(j)?;
                    if b.bits() > *n {
                        return Err("§12.3: integer argument out of range".into());
                    }
                    Value::int(*n, b)
                }
                Type::Ptr => {
                    let b = unsigned(j)?;
                    if b.bits() > ptr_width {
                        return Err("§12.3: pointer argument out of range".into());
                    }
                    Value::Ptr(b)
                }
                Type::F64 => {
                    let h = string(j)?
                        .strip_prefix("0x")
                        .filter(|s| {
                            s.len() == 16
                                && s.bytes()
                                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        })
                        .ok_or("§12.3: invalid f64 bits")?;
                    Value::F64(f64::from_bits(
                        u64::from_str_radix(h, 16).map_err(|e| e.to_string())?,
                    ))
                }
                Type::Token => Value::Token(decode_budget(j)?),
                _ => return Err("§12.3: nonclassical invocation argument".into()),
            })
        })
        .collect()
}
pub fn params_hash(args: &Json) -> String {
    crate::sha256::sha256_hex(args.to_jcs().as_bytes())
}
pub fn resolve(
    obj: &Object,
    name: &str,
    invocation: &Json,
) -> Result<(Vec<Value>, Budget), String> {
    keys(invocation, &["args", "budget"])?;
    let f = obj
        .function(name)
        .ok_or("§12.3: unknown invocation region")?;
    let group = match f.kind {
        FnKind::Oracle => 4,
        FnKind::Action | FnKind::PermAction => 3,
        _ => return Err("§12.3: invocation requires region".into()),
    };
    let types = f.lists[group]
        .iter()
        .map(|p| p.ty.clone())
        .collect::<Vec<_>>();
    let args = decode_args(field(invocation, "args")?, &types, obj.ptr_width)?;
    let budget = decode_budget(field(invocation, "budget")?)?;
    crate::cliargs::validate_budget(obj, &budget)?;
    for v in &args {
        if let Value::Token(b) = v {
            crate::cliargs::validate_budget(obj, b)?;
        }
    }
    Ok((args, budget))
}
/// A request parser must check canonical bytes before hashing/copying them.
pub fn parse_args(raw: &str) -> Result<Json, String> {
    let j = json::parse(raw)?;
    arr(&j)?;
    if j.to_jcs() != raw {
        return Err("§10.1: params_jcs must be canonical JCS".into());
    }
    Ok(j)
}
