//! Static CC/R operand schemas and SSA types (§3.3, §4.1, §4.7).
use crate::object::{FnKind, Function, Object, Type};
use crate::validate::{action_sig, branch_target, cc_sig, err, list_operand, oracle_sig};
use std::collections::BTreeMap;
fn check(ok: bool, why: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(err("3.3", why)) }
}
fn integer(t: &Type) -> bool {
    matches!(t, Type::I(_))
}
fn read(env: &BTreeMap<String, Type>, name: &str) -> Result<Type, String> {
    env.get(name)
        .cloned()
        .ok_or_else(|| err("3.3", format!("undefined classical operand {name:?}")))
}
fn bind(env: &mut BTreeMap<String, Type>, name: &str, ty: Type) -> Result<(), String> {
    check(!env.contains_key(name), "SSA definition reused")?;
    env.insert(name.into(), ty);
    Ok(())
}
fn same_list(env: &BTreeMap<String, Type>, names: &[String], types: &[Type]) -> Result<(), String> {
    check(names.len() == types.len(), "argument/result count mismatch")?;
    for (n, t) in names.iter().zip(types) {
        check(read(env, n)? == *t, "classical operand types differ")?;
    }
    Ok(())
}
fn bundle(env: &BTreeMap<String, Type>, n: &str, w: usize) -> Result<(), String> {
    if let Some((base, index)) = n.split_once('[') {
        let i: usize = index
            .trim_end_matches(']')
            .parse()
            .map_err(|_| err("3.3", "invalid wire index"))?;
        match read(env, base)? {
            Type::Qreg(n) => check(w == 1 && i < n, "qreg index out of bounds"),
            _ => Err(err("3.3", "wire slot requires qreg")),
        }
    } else {
        check(
            read(env, n)? == Type::Qreg(w),
            "qreg width differs from region slot",
        )
    }
}
fn bundles(env: &BTreeMap<String, Type>, names: &str, widths: &[usize]) -> Result<(), String> {
    let names = list_operand(names)?;
    check(names.len() == widths.len(), "region slot count mismatch")?;
    for (n, w) in names.iter().zip(widths) {
        bundle(env, n, *w)?;
    }
    Ok(())
}
pub fn classical(obj: &Object, f: &Function) -> Result<(), String> {
    if !matches!(f.kind, FnKind::Cc | FnKind::R) {
        return Ok(());
    }
    let mut env = BTreeMap::new();
    for p in &f.lists[0] {
        bind(&mut env, &p.name, p.ty.clone())?;
    }
    for b in &f.blocks {
        for p in &b.params {
            bind(&mut env, &p.name, p.ty.clone())?;
        }
    }
    for b in &f.blocks {
        for ins in &b.instrs {
            let op = ins.op.as_str();
            let base = op.split('.').next().unwrap();
            let xs = &ins.operands;
            let suffix = || -> Result<usize, String> {
                op.rsplit_once(".i")
                    .ok_or_else(|| err("4.8", "missing width suffix"))?
                    .1
                    .parse()
                    .map_err(|_| err("4.8", "invalid width suffix"))
            };
            let mut results = vec![];
            match base {
                "const" => {
                    let n = suffix()?;
                    let v = crate::oracle::parse_const(&xs[0], n)?;
                    let _ = v;
                    results.push(Type::I(n));
                }
                "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr" | "udiv"
                | "urem" | "sdiv" | "srem" | "icmp" => {
                    let a = read(&env, &xs[0])?;
                    check(integer(&a), "arithmetic needs integer operands")?;
                    check(a == read(&env, &xs[1])?, "classical operand types differ")?;
                    results.push(if base == "icmp" { Type::I(1) } else { a });
                }
                "select" => {
                    check(
                        read(&env, &xs[0])? == Type::I(1),
                        "select predicate must be i1",
                    )?;
                    let a = read(&env, &xs[1])?;
                    check(a == read(&env, &xs[2])?, "select arms differ")?;
                    results.push(a);
                }
                "zext" | "sext" | "trunc" => {
                    let Type::I(n) = read(&env, &xs[0])? else {
                        return Err(err("3.3", "cast needs integer"));
                    };
                    let m = suffix()?;
                    check(
                        if base == "trunc" { m < n } else { m > n },
                        "cast must strictly widen/narrow",
                    )?;
                    results.push(Type::I(m));
                }
                "ptrtoint" => {
                    check(read(&env, &xs[0])? == Type::Ptr, "ptrtoint requires ptr")?;
                    results.push(Type::I(suffix()?));
                }
                "inttoptr" => {
                    check(integer(&read(&env, &xs[0])?), "inttoptr requires integer")?;
                    results.push(Type::Ptr);
                }
                "alloc" => {
                    check(
                        integer(&read(&env, &xs[0])?),
                        "alloc length must be integer",
                    )?;
                    results.push(Type::Ptr);
                }
                "free" => check(read(&env, &xs[0])? == Type::Ptr, "free needs ptr")?,
                "load" | "store" => {
                    check(
                        read(&env, &xs[0])? == Type::Ptr,
                        "memory address must be ptr",
                    )?;
                    let ty = Type::I(suffix()?);
                    if base == "store" {
                        check(read(&env, &xs[1])? == ty, "store width differs")?;
                    } else {
                        results.push(ty);
                    }
                }
                "br" => {
                    let targets = if xs.len() == 1 {
                        xs.as_slice()
                    } else {
                        check(
                            read(&env, &xs[0])? == Type::I(1),
                            "branch condition must be i1",
                        )?;
                        &xs[1..]
                    };
                    for target in targets {
                        let (n, args) =
                            branch_target(target).ok_or_else(|| err("4.1", "malformed branch"))?;
                        let b = f
                            .block(&n)
                            .ok_or_else(|| err("4.1", "unknown branch block"))?;
                        same_list(
                            &env,
                            &args,
                            &b.params.iter().map(|p| p.ty.clone()).collect::<Vec<_>>(),
                        )?;
                    }
                }
                "call" | "uncall" => {
                    let name = xs[0].trim_start_matches('@');
                    let (params, returns) = cc_sig(obj, name)?;
                    if base == "call" {
                        same_list(&env, &xs[1..], &params)?;
                        results = returns;
                    } else {
                        same_list(&env, &list_operand(&xs[1])?, &params)?;
                        same_list(&env, &list_operand(&xs[2])?, &returns)?;
                    }
                }
                "ret" => same_list(
                    &env,
                    xs,
                    &f.lists[1].iter().map(|p| p.ty.clone()).collect::<Vec<_>>(),
                )?,
                "trap" => {}
                "effect" => {
                    let ptr_positions: &[usize] = match xs[0].as_str() {
                        "console.write" | "entropy.read" => &[1],
                        "clock.read" => &[1],
                        "block.read" | "block.write" => &[2],
                        _ => &[],
                    };
                    for i in ptr_positions {
                        if let Some(n) = xs.get(*i) {
                            check(
                                read(&env, n)? == Type::Ptr,
                                "device memory operand must be ptr",
                            )?;
                        }
                    }
                    for n in &xs[1..] {
                        let ty = read(&env, n)?;
                        check(
                            integer(&ty) || ty == Type::Ptr,
                            "device args must be integers/pointers",
                        )?;
                    }
                }
                "hostcall" => {
                    check(
                        read(&env, &xs[1])? == Type::Ptr,
                        "hostcall address needs ptr",
                    )?;
                    check(
                        integer(&read(&env, &xs[2])?),
                        "hostcall length needs integer",
                    )?;
                    results.push(Type::I(obj.ptr_width));
                }
                "qreg" => match op {
                    "qreg.new" => results.push(Type::Qreg(
                        xs[1]
                            .parse()
                            .map_err(|_| err("3.3", "invalid qreg length"))?,
                    )),
                    "qreg.free" => {
                        check(
                            matches!(read(&env, &xs[0])?, Type::Qreg(_)),
                            "qreg.free needs handle",
                        )?;
                    }
                    "qreg.slice" => {
                        let Type::Qreg(n) = read(&env, &xs[0])? else {
                            return Err(err("3.3", "slice needs qreg"));
                        };
                        let lo: usize = xs[1].parse().map_err(|_| err("3.3", "invalid slice"))?;
                        let len: usize = xs[2].parse().map_err(|_| err("3.3", "invalid slice"))?;
                        check(
                            lo.checked_add(len).is_some_and(|e| e <= n),
                            "slice outside register",
                        )?;
                        results.push(Type::Qreg(len));
                    }
                    "qreg.concat" => {
                        let (Type::Qreg(a), Type::Qreg(b)) =
                            (read(&env, &xs[0])?, read(&env, &xs[1])?)
                        else {
                            return Err(err("3.3", "concat needs qreg"));
                        };
                        results.push(Type::Qreg(
                            a.checked_add(b).ok_or("host limit: concatenated width")?,
                        ));
                    }
                    _ => return Err(err("4.8", "unknown qreg row")),
                },
                "measure" => {
                    let Type::Qreg(n) = read(&env, &xs[0])? else {
                        return Err(err("3.3", "measure needs qreg"));
                    };
                    check(n <= 128, "measurement width exceeds i128")?;
                    results.push(Type::I(n));
                }
                "action_call" | "oracle_call" | "adj" | "ctl" => {
                    let mut op = op;
                    let mut xs = xs.as_slice();
                    if op == "ctl" {
                        bundle(&env, &xs[0], 1)?;
                        op = &xs[1];
                        xs = &xs[2..];
                    }
                    if op == "adj" {
                        op = &xs[0];
                        xs = &xs[1..];
                    }
                    let name = xs[0].trim_start_matches('@');
                    if op == "oracle_call" {
                        let sig = oracle_sig(obj, name)?;
                        check(
                            read(&env, &xs[1])? == Type::Token,
                            "oracle call needs token",
                        )?;
                        bundles(&env, &xs[2], &sig.ins)?;
                        bundle(&env, &xs[3], sig.tgt)?;
                        bundle(&env, &xs[4], sig.fault)?;
                        let widths = sig
                            .mem
                            .iter()
                            .map(|(a, w, _)| {
                                (1usize.checked_shl(*a as u32).unwrap_or(0))
                                    .checked_mul(*w)
                                    .ok_or("host limit: region width")
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        bundles(&env, &xs[5], &widths)?;
                        same_list(&env, &list_operand(&xs[6])?, &sig.args)?;
                    } else {
                        let sig = action_sig(obj, name)?;
                        bundles(&env, &xs[1], &sig.muts)?;
                        bundles(&env, &xs[2], &sig.ins)?;
                        let widths = sig
                            .mem
                            .iter()
                            .map(|(a, w, _)| {
                                (1usize.checked_shl(*a as u32).unwrap_or(0))
                                    .checked_mul(*w)
                                    .ok_or("host limit: region width")
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        bundles(&env, &xs[3], &widths)?;
                        same_list(&env, &list_operand(&xs[4])?, &sig.args)?;
                    }
                }
                _ => return Err(err("4.8", format!("unknown classical row {op}"))),
            }
            check(results.len() == ins.lhs.len(), "result count mismatch")?;
            for (n, t) in ins.lhs.iter().zip(results) {
                bind(&mut env, n, t)?;
            }
        }
    }
    Ok(())
}
