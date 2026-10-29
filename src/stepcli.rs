//! Domain enumeration and prefix reversal for the literal machine.
use crate::{
    bigint::BigUint,
    cliargs,
    json::Json,
    object::Object,
    step::Machine,
    value::{Budget, Value},
};
use std::collections::BTreeMap;
struct Inputs {
    mem: Vec<Vec<BigUint>>,
    args: Vec<Value>,
    fixed: Vec<String>,
}
fn inputs(obj: &Object, name: &str, o: &cliargs::Opts, budget: &Budget) -> Result<Inputs, String> {
    let f = obj.function(name).ok_or("§5.2: unknown oracle")?;
    let sig = crate::validate::oracle_sig(obj, name)?;
    let mut given = BTreeMap::new();
    for kv in o.all("mem") {
        let (n, hex) = kv.split_once('=').ok_or("invalid --mem")?;
        let bytes = crate::sha256::from_hex(hex)?;
        if given.insert(n.to_string(), bytes).is_some() {
            return Err("duplicate --mem".into());
        }
    }
    let mut mem = vec![];
    let mut fixed = vec![];
    for (p, (a, w, _)) in f.lists[3].iter().zip(&sig.mem) {
        let count = 1usize
            .checked_shl(*a as u32)
            .ok_or("host limit: region cells")?;
        let cells = if let Some(bytes) = given.remove(&p.name) {
            fixed.push(p.name.clone());
            let stride = w.div_ceil(8);
            if bytes.len() != count * stride {
                return Err("§5.1: region byte count mismatch".into());
            }
            bytes
                .chunks(stride)
                .map(|bs| {
                    bs.iter()
                        .enumerate()
                        .fold(BigUint::zero(), |v, (j, b)| {
                            v.add(&BigUint::from_u64(*b as u64).shl(j * 8))
                        })
                        .mask(*w)
                })
                .collect()
        } else {
            vec![BigUint::zero(); count]
        };
        mem.push(cells);
    }
    if !given.is_empty() {
        return Err("§5.1: unknown interface region".into());
    }
    let args = crate::qcli::bind_args(obj, &sig.args, &o.all("arg"), budget)?;
    Ok(Inputs { mem, args, fixed })
}
fn budget(obj: &Object, o: &cliargs::Opts, n: u64) -> Result<Budget, String> {
    o.one("budget")?.map_or_else(
        || Ok(cliargs::default_budget(obj, n)),
        |s| cliargs::parse_budget(s, obj),
    )
}
pub fn reverse(args: &[String]) -> Result<(), String> {
    let o = cliargs::scan_opts(args, &["input", "budget", "mem", "arg", "steps"], &[])?;
    if o.positional.len() != 2 {
        return Err("reverse needs <object> <oracle>".into());
    }
    let (obj, _) = crate::load(&cliargs::read_object(&o.positional[0])?, false)?;
    let name = cliargs::entry_name(&o.positional[1]);
    let budget = budget(&obj, &o, 1_000_000)?;
    let inp = inputs(&obj, name, &o, &budget)?;
    let raw = o.one("input")?.ok_or("oracle reverse requires --input")?;
    let vs = if raw.is_empty() {
        vec![]
    } else {
        raw.split(',')
            .map(BigUint::parse_int)
            .collect::<Result<Vec<_>, _>>()?
    };
    let n = o
        .one("steps")?
        .map_or(Ok(budget.steps), |n| cliargs::parse_u64(n, "prefix steps"))?;
    if n > budget.steps {
        return Err("§5.4: prefix exceeds B".into());
    }
    let mut m = Machine::new(&obj, name, &vs, &inp.mem, &inp.args, budget.clone())?;
    let entry = m.state.clone();
    for _ in 0..n {
        m.forward_step()?;
    }
    let outcome = m.outcome();
    for _ in 0..n {
        m.inverse_step()?;
    }
    let eq = m.state == entry;
    let state = m.state_json();
    println!(
        "{}",
        Json::obj(vec![
            ("oracle", Json::Str(name.into())),
            ("budget", budget.to_json()),
            ("payload", Json::u(&outcome.payload)),
            ("code", Json::u(&outcome.code)),
            ("steps", Json::u64(outcome.steps)),
            ("forward_steps", Json::u64(n)),
            ("reverse_steps", Json::u64(n)),
            ("frames_clean", Json::Bool(m.frames.is_empty())),
            ("equals_entry", Json::Bool(eq)),
            (
                "first_difference",
                if eq {
                    Json::Null
                } else {
                    Json::Str("step state".into())
                }
            ),
            (
                "status",
                Json::Str(if eq { "REVERSED" } else { "FAILED" }.into())
            ),
            ("state_after_reversal", state)
        ])
        .to_jcs()
    );
    Ok(())
}
struct Rng([u64; 4]);
impl Rng {
    fn next(&mut self) -> u64 {
        let s = &mut self.0;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }
    fn bits(&mut self, n: usize) -> BigUint {
        let mut v = BigUint::zero();
        for j in 0..n.div_ceil(64) {
            v = v.add(&BigUint::from_u64(self.next()).shl(64 * j));
        }
        v.mask(n)
    }
}
pub fn check(args: &[String]) -> Result<(), String> {
    let o = cliargs::scan_opts(
        args,
        &["budget", "mem", "arg", "max-bits", "samples", "seed"],
        &["exhaustive", "suite"],
    )?;
    if o.positional.len() != 2 {
        return Err("step-check needs <object> <oracle> MODE".into());
    }
    if usize::from(o.flags.contains("exhaustive"))
        + usize::from(o.flags.contains("suite"))
        + usize::from(o.one("samples")?.is_some())
        != 1
    {
        return Err("step-check needs exactly one MODE".into());
    }
    let (obj, _) = crate::load(&cliargs::read_object(&o.positional[0])?, false)?;
    let name = cliargs::entry_name(&o.positional[1]);
    let f = obj.function(name).ok_or("§5.2: unknown oracle")?;
    let sig = crate::validate::oracle_sig(&obj, name)?;
    let budget = budget(&obj, &o, 256)?;
    let inp = inputs(&obj, name, &o, &budget)?;
    let mut widths = sig.ins.clone();
    for (p, (a, w, _)) in f.lists[3].iter().zip(&sig.mem) {
        if !inp.fixed.contains(&p.name) {
            let count = 1usize
                .checked_shl(*a as u32)
                .ok_or("host limit: region domain")?;
            widths.extend(std::iter::repeat_n(*w, count));
        }
    }
    let bits = widths.iter().try_fold(0usize, |n, w| {
        n.checked_add(*w).ok_or("host limit: domain bits")
    })?;
    let exhaustive = o.flags.contains("exhaustive") || (o.flags.contains("suite") && bits <= 16);
    let count = if exhaustive {
        let guard = o
            .one("max-bits")?
            .map_or(Ok(16), |v| cliargs::parse_u64(v, "max-bits"))?;
        if bits as u64 > guard {
            return Err(format!(
                "step-check domain {bits} exceeds --max-bits {guard}"
            ));
        }
        1u64.checked_shl(bits as u32)
            .ok_or("host limit: exhaustive cases")?
    } else {
        o.one("samples")?
            .map_or(Ok(1 << 16), |v| cliargs::parse_u64(v, "samples"))?
    };
    let mut seed = [1, 2, 3, 4];
    if let Some(s) = o.one("seed")? {
        let parts = s
            .split(',')
            .map(|v| cliargs::parse_u64(v, "seed"))
            .collect::<Result<Vec<_>, _>>()?;
        seed = parts.try_into().map_err(|_| "seed needs four words")?;
        if seed == [0; 4] {
            return Err("seed must not be all zero".into());
        }
    }
    let mut rng = Rng(seed);
    for case in 0..count {
        let mut vals = vec![BigUint::zero(); widths.len()];
        let mut packed = BigUint::from_u64(case);
        for j in (0..widths.len()).rev() {
            vals[j] = if exhaustive {
                let v = packed.mask(widths[j]);
                packed = packed.shr(widths[j]);
                v
            } else {
                rng.bits(widths[j])
            };
        }
        let ins = &vals[..sig.ins.len()];
        let mut mem = inp.mem.clone();
        let mut at = sig.ins.len();
        for (j, p) in f.lists[3].iter().enumerate() {
            if !inp.fixed.contains(&p.name) {
                for cell in &mut mem[j] {
                    *cell = vals[at].clone();
                    at += 1;
                }
            }
        }
        let seq = crate::oracle::run_oracle(
            &obj,
            name,
            ins,
            &mut mem.clone(),
            &inp.args,
            budget.clone(),
        )?;
        let mut m = Machine::new(&obj, name, ins, &mem, &inp.args, budget.clone())?;
        let entry = m.state.clone();
        for _ in 0..budget.steps {
            m.forward_step()?;
        }
        let step = m.outcome();
        if seq.payload != step.payload || seq.code != step.code || seq.steps != step.steps {
            return Err(format!(
                "T6 divergence case {case}: sequential {seq:?}, literal {step:?}"
            ));
        }
        for _ in 0..budget.steps {
            m.inverse_step()?;
        }
        if m.state != entry || !m.frames.is_empty() {
            return Err(format!("T2 divergence case {case}"));
        }
    }
    println!(
        "{}",
        Json::obj(vec![
            (
                "target_fault",
                Json::Str("XOR delivery is independent of their entry values".into())
            ),
            ("cases", Json::u64(count)),
            ("input_and_memory_bits", Json::u64(bits as u64)),
            (
                "mode",
                Json::Str(if exhaustive { "exhaustive" } else { "sampled" }.into())
            ),
            (
                "fixed_regions",
                Json::Arr(inp.fixed.into_iter().map(Json::Str).collect())
            ),
            ("status", Json::Str("ok".into()))
        ])
        .to_jcs()
    );
    Ok(())
}
