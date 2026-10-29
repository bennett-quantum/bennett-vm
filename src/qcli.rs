//! Standalone coherent CLI, sharing the device action executor.
use crate::{
    action::{QuantumState, RegionWires, execute_action_with_idle},
    bigint::BigUint,
    cliargs,
    json::Json,
    object::{Object, Type},
    value::{Budget, Value},
};
use std::collections::BTreeMap;
pub fn bind_args(
    obj: &Object,
    types: &[Type],
    raw: &[String],
    budget: &Budget,
) -> Result<Vec<Value>, String> {
    if types.len() != raw.len() {
        return Err("§5.1: classical argument count differs from signature".into());
    }
    raw.iter()
        .zip(types)
        .map(|(s, t)| {
            Ok(match (t, cliargs::parse_raw_arg(s, budget)?) {
                (Type::I(n), cliargs::RawArg::Int(x)) => {
                    Value::int(*n, cliargs::bind_int(&x, *n, &format!("i{n}"))?)
                }
                (Type::Ptr, cliargs::RawArg::Int(x)) => {
                    Value::Ptr(cliargs::bind_int(&x, obj.ptr_width, "ptr")?)
                }
                (Type::F64, cliargs::RawArg::F64(x)) => Value::F64(x),
                (Type::Token, cliargs::RawArg::Token(x)) => {
                    cliargs::validate_budget(obj, &x)?;
                    Value::Token(x)
                }
                _ => return Err("§10: E_ARG: argument type mismatch".into()),
            })
        })
        .collect()
}
pub fn borrow_capacity(obj: &Object, name: &str) -> Result<usize, String> {
    fn list(
        obj: &Object,
        xs: &[crate::object::Instr],
        live: &mut usize,
        peak: &mut usize,
    ) -> Result<(), String> {
        for i in xs {
            if i.op == "qborrow" {
                *live += i.operands[1]
                    .parse::<usize>()
                    .map_err(|_| "§4.3: invalid borrow size")?;
                *peak = (*peak).max(*live);
            }
            // Conservative sum is safe: unused designated pool wires are untouched.
            let mut call = i;
            let synthetic;
            if i.op == "adj" {
                synthetic = crate::object::Instr {
                    op: i.operands[0].clone(),
                    operands: i.operands[1..].to_vec(),
                    ..i.clone()
                };
                call = &synthetic;
            }
            if call.op == "action_call" {
                *peak = (*peak)
                    .max(*live + borrow_capacity(obj, call.operands[0].trim_start_matches('@'))?);
            }
            for body in &i.nested {
                list(obj, body, live, peak)?;
            }
        }
        Ok(())
    }
    let f = obj.function(name).ok_or("§5.1: unresolved action")?;
    let mut live = 0;
    let mut peak = 0;
    for b in &f.blocks {
        list(obj, &b.instrs, &mut live, &mut peak)?;
    }
    Ok(peak)
}
fn bits(key: &BigUint, n: usize) -> String {
    (0..n).map(|w| if key.bit(w) { '1' } else { '0' }).collect()
}
fn state_json(s: &QuantumState, n: usize) -> Json {
    Json::Obj(
        s.amps
            .iter()
            .map(|(k, a)| {
                (
                    bits(k, n),
                    Json::Arr(vec![Json::Num(a.0.to_string()), Json::Num(a.1.to_string())]),
                )
            })
            .collect(),
    )
}
fn values(s: &QuantumState, bs: &[Vec<usize>]) -> Json {
    Json::Arr(
        bs.iter()
            .map(|b| s.constant_value(b).map_or(Json::Null, |v| Json::u(&v)))
            .collect(),
    )
}
fn parse_bits(s: &str, n: usize) -> Result<BigUint, String> {
    if s.len() != n || !s.bytes().all(|c| c == b'0' || c == b'1') {
        return Err("§14.2: basis bit string width or alphabet mismatch".into());
    }
    let mut v = BigUint::zero();
    for (i, c) in s.bytes().enumerate() {
        v.set_bit(i, c == b'1');
    }
    Ok(v)
}
pub fn command(args: &[String], reverse: bool) -> Result<(), String> {
    let o = cliargs::scan_opts(
        args,
        &["mut", "in", "mem", "basis", "state", "args", "arg"],
        &["adjoint"],
    )?;
    if o.positional.len() != 2 {
        return Err("run-action needs <object> <action>".into());
    }
    let (obj, _) = crate::load(&cliargs::read_object(&o.positional[0])?, false)?;
    let name = cliargs::entry_name(&o.positional[1]);
    let sig = crate::validate::action_sig(&obj, name)?;
    let f = obj.function(name).ok_or("§5.1: unresolved action")?;
    let mut s = QuantumState::new();
    let muts: Vec<_> = sig.muts.iter().map(|w| s.alloc_wires(*w)).collect();
    let ins: Vec<_> = sig.ins.iter().map(|w| s.alloc_wires(*w)).collect();
    let mut regions = vec![];
    for (a, w, _) in &sig.mem {
        let count = 1usize
            .checked_shl(*a as u32)
            .ok_or("host limit: region cells")?;
        regions.push(RegionWires {
            a: *a,
            w: *w,
            cells: (0..count).map(|_| s.alloc_wires(*w)).collect(),
        });
    }
    let n = s.wire_count();
    let forms = usize::from(o.one("mut")?.is_some())
        + usize::from(o.one("basis")?.is_some())
        + usize::from(o.one("state")?.is_some());
    if forms != 1 {
        return Err("§14.2: supply exactly one of --mut, --basis, --state".into());
    }
    if let Some(bs) = o.one("basis")? {
        s.prepare(&(0..n).collect::<Vec<_>>(), &parse_bits(bs, n)?);
    } else if let Some(st) = o.one("state")? {
        let mut amps = BTreeMap::new();
        for term in st.split(';') {
            let (bs, a) = term.split_once('=').ok_or("invalid --state term")?;
            let ab = a.split(',').collect::<Vec<_>>();
            if ab.len() > 2 {
                return Err("invalid complex amplitude".into());
            }
            let re = ab[0].parse::<f64>().map_err(|_| "invalid amplitude")?;
            let im = if ab.len() == 2 {
                ab[1].parse::<f64>().map_err(|_| "invalid amplitude")?
            } else {
                0.
            };
            if !re.is_finite() || !im.is_finite() {
                return Err("nonfinite amplitude".into());
            }
            if amps.insert(parse_bits(bs, n)?, (re, im)).is_some() {
                return Err("duplicate state component".into());
            }
        }
        s.set_state(amps);
    } else {
        for (key, bundles) in [("mut", &muts), ("in", &ins)] {
            let raw = o.one(key)?;
            let vs = raw.map(|v| {
                if v.is_empty() {
                    vec![]
                } else {
                    v.split(',').collect::<Vec<_>>()
                }
            });
            if let Some(vs) = vs {
                if vs.len() != bundles.len() {
                    return Err("§5.1: bundle count mismatch".into());
                }
                for (v, b) in vs.iter().zip(bundles) {
                    let v = cliargs::bind_int(v, b.len(), "wire bundle")?;
                    s.prepare(b, &v);
                }
            }
        }
        let mut given = BTreeMap::new();
        for kv in o.all("mem") {
            let (k, v) = kv.split_once('=').ok_or("invalid --mem")?;
            if given.insert(k.to_string(), v.to_string()).is_some() {
                return Err("duplicate --mem".into());
            }
        }
        for (p, r) in f.lists[2].iter().zip(&regions) {
            if let Some(hex) = given.remove(&p.name) {
                let bytes = crate::sha256::from_hex(&hex)?;
                if bytes.len() != r.cells.len() * r.w.div_ceil(8) {
                    return Err("§5.1: region byte count mismatch".into());
                }
                let mut v = BigUint::zero();
                for (i, b) in bytes.iter().enumerate() {
                    v = v.add(&BigUint::from_u64(*b as u64).shl(i * 8));
                }
                for (i, c) in r.cells.iter().enumerate() {
                    s.prepare(c, &v.shr(i * r.w.div_ceil(8) * 8).mask(r.w));
                }
            }
        }
        if !given.is_empty() {
            return Err("§5.1: unknown interface region".into());
        }
    }
    if o.one("mut")?.is_none() && (o.one("in")?.is_some() || !o.all("mem").is_empty()) {
        return Err("§14.2: --in/--mem require --mut".into());
    }
    let raw = if let Some(a) = o.one("args")? {
        if !o.all("arg").is_empty() {
            return Err("--args and --arg are exclusive".into());
        }
        if a.is_empty() {
            vec![]
        } else {
            a.split(',').map(str::to_string).collect()
        }
    } else {
        o.all("arg")
    };
    let av = bind_args(
        &obj,
        &sig.args,
        &raw,
        &cliargs::default_budget(&obj, 1_000_000),
    )?;
    let pool = s.alloc_wires(borrow_capacity(&obj, name)?);
    let entry = s.clone();
    let adj = o.flags.contains("adjoint");
    execute_action_with_idle(
        &obj,
        &mut s,
        name,
        &muts,
        &ins,
        &regions,
        &av,
        adj,
        &[],
        &pool,
    )?;
    if !s.is_clean(&pool) {
        return Err("§5.6: borrowed pool was not returned to its entry state".into());
    }
    let forward = state_json(&s, n);
    let mut fields = vec![
        ("action", Json::Str(name.into())),
        ("adjoint", Json::Bool(adj)),
        ("wires", Json::u64(n as u64)),
    ];
    if reverse {
        execute_action_with_idle(
            &obj,
            &mut s,
            name,
            &muts,
            &ins,
            &regions,
            &av,
            !adj,
            &[],
            &pool,
        )?;
        let eq = s.exact_equals(&entry);
        fields.extend([
            ("equals_entry", Json::Bool(eq)),
            (
                "first_difference",
                if eq {
                    Json::Null
                } else {
                    Json::Str("state".into())
                },
            ),
            (
                "status",
                Json::Str(if eq { "REVERSED" } else { "FAILED" }.into()),
            ),
            ("forward_state", forward),
        ]);
    }
    fields.extend([
        ("state", state_json(&s, n)),
        ("mut", values(&s, &muts)),
        ("in", values(&s, &ins)),
    ]);
    if !reverse {
        fields.push((
            "mem",
            Json::Obj(
                f.lists[2]
                    .iter()
                    .zip(&regions)
                    .map(|(p, r)| (p.name.clone(), values(&s, &r.cells)))
                    .collect(),
            ),
        ));
    }
    println!("{}", Json::obj(fields).to_jcs());
    Ok(())
}
