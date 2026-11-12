//! Static CC/R operand schemas and SSA types (§3.3, §4.1, §4.7).
use crate::object::{BlockIndex, FnKind, Function, Object, Type};
use crate::validate::{action_sig, branch_target, cc_sig, err, list_operand, oracle_sig};
use std::collections::BTreeMap;
fn check(ok: bool, why: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(err("3.3", why)) }
}
pub(crate) fn valid_type(t: &Type) -> Result<(), String> {
    check(
        matches!(
            t,
            Type::I(1..=128)
                | Type::Ptr
                | Type::F64
                | Type::Token
                | Type::Qreg(1..)
                | Type::QregDynamic
        ),
        "invalid classical type/width",
    )
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
    check(
        env.get(name).is_none_or(|old| old == &ty),
        "definitions of a classical name differ in type",
    )?;
    valid_type(&ty)?;
    env.insert(name.into(), ty);
    Ok(())
}
fn same_list(env: &BTreeMap<String, Type>, names: &[String], types: &[Type]) -> Result<(), String> {
    check(names.len() == types.len(), "argument/result count mismatch")?;
    for (n, t) in names.iter().zip(types) {
        let actual = read(env, n)?;
        check(
            actual == *t
                || matches!(
                    (&actual, t),
                    (Type::Qreg(_), Type::QregDynamic) | (Type::QregDynamic, Type::Qreg(_))
                ),
            "classical operand types differ",
        )?;
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
            matches!(read(env, n)?, Type::QregDynamic) || read(env, n)? == Type::Qreg(w),
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
fn regions(
    env: &BTreeMap<String, Type>,
    names: &str,
    shapes: &[(usize, usize, String)],
) -> Result<(), String> {
    let names = list_operand(names)?;
    check(names.len() == shapes.len(), "region slot count mismatch")?;
    for (name, (a, w, _)) in names.iter().zip(shapes) {
        match read(env, name)? {
            Type::QregDynamic => {} // The exact shape is checked at transfer.
            Type::Qreg(n) => {
                let cells = u32::try_from(*a).ok().and_then(|a| 1usize.checked_shl(a));
                check(
                    cells.and_then(|c| c.checked_mul(*w)) == Some(n),
                    "qreg width differs from region shape",
                )?;
            }
            _ => return Err(err("3.3", "region slot requires qreg")),
        }
    }
    Ok(())
}
pub fn classical(obj: &Object, f: &Function) -> Result<(), String> {
    if !matches!(f.kind, FnKind::Cc | FnKind::R) {
        return Ok(());
    }
    let (blocks, mut env) = (BlockIndex::new(f), BTreeMap::new());
    for p in &f.lists[0] {
        bind(&mut env, &p.name, p.ty.clone())?;
    }
    for b in &f.blocks {
        for p in &b.params {
            bind(&mut env, &p.name, p.ty.clone())?;
        }
    }
    // §3.4 permits forward textual references and repeated definitions of
    // one name at one type. Infer to a fixed point; execution owns liveness.
    loop {
        let before = env.len();
        let mut unresolved = None;
        for b in &f.blocks {
            for ins in &b.instrs {
                let result = (|| -> Result<(), String> {
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
                            valid_type(&Type::I(n))?;
                            let v = crate::oracle::parse_const(&xs[0], n)?;
                            let _ = v;
                            results.push(Type::I(n));
                        }
                        "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr"
                        | "udiv" | "urem" | "sdiv" | "srem" => {
                            let a = read(&env, &xs[0])?;
                            check(integer(&a), "arithmetic needs integer operands")?;
                            check(a == read(&env, &xs[1])?, "classical operand types differ")?;
                            results.push(a);
                        }
                        "icmp" => {
                            let a = read(&env, &xs[0])?;
                            check(
                                integer(&a) || a == Type::Ptr,
                                "icmp needs integer or pointer operands",
                            )?;
                            check(a == read(&env, &xs[1])?, "classical operand types differ")?;
                            results.push(Type::I(1));
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
                            valid_type(&Type::I(m))?;
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
                                let (n, args) = branch_target(target)
                                    .ok_or_else(|| err("4.1", "malformed branch"))?;
                                let b = blocks
                                    .get(&n)
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
                            let arity = match xs[0].as_str() {
                                "clock.read" => Some(2),
                                "console.write" | "entropy.read" | "block.read" | "block.write" => {
                                    Some(3)
                                }
                                _ => None, // Unmodelled devices fault at runtime (§17.3).
                            };
                            check(
                                arity.is_none_or(|n| xs.len() == n),
                                "modelled device operand count mismatch",
                            )?;
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
                        "fault" => {}
                        "budget" => {
                            for x in &xs[1..] {
                                check(read(&env, x)? == Type::I(64), "budget limits must be i64")?;
                            }
                            results.push(Type::Token);
                        }
                        "qreg" => match op {
                            "qreg.new" => {
                                if let Ok(n) = xs[1].parse::<usize>() {
                                    check(n > 0, "literal qreg length must be positive")?;
                                    results.push(Type::Qreg(n));
                                } else {
                                    check(
                                        obj.isa == "0.9" && read(&env, &xs[1])? == Type::I(64),
                                        "dynamic qreg length must be i64",
                                    )?;
                                    results.push(Type::QregDynamic);
                                }
                            }
                            "qreg.free" | "qreg.width" | "qreg.id" => {
                                check(
                                    matches!(
                                        read(&env, &xs[0])?,
                                        Type::Qreg(_) | Type::QregDynamic
                                    ),
                                    "expected qreg handle",
                                )?;
                                if op != "qreg.free" {
                                    results.push(Type::I(64));
                                }
                            }
                            "qreg.from_id" => {
                                check(read(&env, &xs[0])? == Type::I(64), "qreg id must be i64")?;
                                results.push(Type::QregDynamic);
                            }
                            "qreg.slice" => {
                                check(
                                    matches!(
                                        read(&env, &xs[0])?,
                                        Type::Qreg(_) | Type::QregDynamic
                                    ),
                                    "slice needs qreg",
                                )?;
                                for x in &xs[1..] {
                                    if x.parse::<usize>().is_err() {
                                        check(
                                            read(&env, x)? == Type::I(64),
                                            "slice index must be i64",
                                        )?;
                                    }
                                }
                                results.push(match xs[2].parse::<usize>() {
                                    Ok(n) => Type::Qreg(n),
                                    Err(_) => Type::QregDynamic,
                                });
                            }
                            "qreg.concat" => {
                                let (a, b) = (read(&env, &xs[0])?, read(&env, &xs[1])?);
                                check(
                                    matches!(a, Type::Qreg(_) | Type::QregDynamic)
                                        && matches!(b, Type::Qreg(_) | Type::QregDynamic),
                                    "concat needs qreg",
                                )?;
                                results.push(match (a, b) {
                                    (Type::Qreg(a), Type::Qreg(b)) => Type::Qreg(
                                        a.checked_add(b).ok_or("host limit: concatenated width")?,
                                    ),
                                    _ => Type::QregDynamic,
                                });
                            }
                            _ => return Err(err("4.8", "unknown qreg row")),
                        },
                        "measure" => {
                            let ty = read(&env, &xs[0])?;
                            let n = if op == "measure" {
                                let Type::Qreg(n) = ty else {
                                    return Err(err(
                                        "3.3",
                                        "dynamic measure requires width suffix",
                                    ));
                                };
                                n
                            } else {
                                check(
                                    matches!(ty, Type::Qreg(_) | Type::QregDynamic),
                                    "measure needs qreg",
                                )?;
                                let width = suffix()?;
                                check(
                                    !matches!(ty, Type::Qreg(n) if n != width),
                                    "static measurement width differs from qreg",
                                )?;
                                width
                            };
                            check((1..=128).contains(&n), "measurement width outside 1..=128")?;
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
                                regions(&env, &xs[5], &sig.mem)?;
                                same_list(&env, &list_operand(&xs[6])?, &sig.args)?;
                            } else {
                                let sig = action_sig(obj, name)?;
                                bundles(&env, &xs[1], &sig.muts)?;
                                bundles(&env, &xs[2], &sig.ins)?;
                                regions(&env, &xs[3], &sig.mem)?;
                                same_list(&env, &list_operand(&xs[4])?, &sig.args)?;
                            }
                        }
                        _ => return Err(err("4.8", format!("unknown classical row {op}"))),
                    }
                    check(results.len() == ins.lhs.len(), "result count mismatch")?;
                    for (n, t) in ins.lhs.iter().zip(results) {
                        bind(&mut env, n, t)?;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    let error = format!(
                        "{error} at @{} ^{} line {}: {}",
                        f.name, b.name, ins.line, ins.raw
                    );
                    if error.contains("undefined classical operand") {
                        unresolved = Some(error);
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        if env.len() == before {
            if let Some(error) = unresolved {
                return Err(error);
            }
            if obj.isa == "0.9" {
                crate::qreg_typing::function(f, &env)?;
            }
            return Ok(());
        }
    }
}
