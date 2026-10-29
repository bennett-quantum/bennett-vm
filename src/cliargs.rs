//! CLI option parsing shared by the commands. Spellings follow the
//! reference interpreter README exactly.

use crate::bigint::BigUint;
use crate::object::Object;
use crate::value::Budget;

pub fn parse_u64(text: &str, what: &str) -> Result<u64, String> {
    let big = BigUint::parse_int(text).map_err(|_| format!("invalid {what} {text:?}"))?;
    big.to_u64()
        .ok_or_else(|| format!("host limit: {what} {text:?} exceeds 2^64-1"))
}

/// Layout-derived default limits (§5.4, reference README): heap and
/// stack from the layout, depth 64 with a layout, zero memoryless.
pub fn default_limits(obj: &Object) -> (u64, u64, u64) {
    match &obj.layout {
        None => (0, 0, 0),
        Some(l) => {
            let heap = l.heap_limit_max.to_u64().unwrap_or(u64::MAX);
            let stack = if l.s_cells >= 64 {
                u64::MAX
            } else {
                1u64 << l.s_cells
            };
            (heap, stack, 64)
        }
    }
}

/// `bennett_budget` validation (§5.4).
pub fn validate_budget(obj: &Object, b: &Budget) -> Result<(), String> {
    match &obj.layout {
        None => {
            if b.heap != 0 || b.stack != 0 || b.depth != 0 {
                return Err("§5.4: memoryless budgets require zero heap, stack and depth".into());
            }
        }
        Some(l) => {
            let heap_max = l.heap_limit_max.clone();
            let stack_max = BigUint::from_u64(1).shl(l.s_cells);
            if BigUint::from_u64(b.heap).cmp_big(&heap_max) == std::cmp::Ordering::Greater
                || BigUint::from_u64(b.stack).cmp_big(&stack_max) == std::cmp::Ordering::Greater
            {
                return Err("§5.4: budget heap/stack limit exceeds the layout".into());
            }
        }
    }
    Ok(())
}

/// Parse `B` or `B,heap,stack,depth`; missing trailing components take
/// the layout defaults.
pub fn parse_budget(text: &str, obj: &Object) -> Result<Budget, String> {
    let parts: Vec<&str> = text.split(',').collect();
    if parts.len() != 1 && parts.len() != 4 {
        return Err("budget requires B or B,heap,stack,depth (§5.4)".into());
    }
    let (dh, ds, dd) = default_limits(obj);
    let b = Budget {
        steps: parse_u64(parts[0], "budget steps")?,
        heap: if parts.len() == 4 {
            parse_u64(parts[1], "budget heap")?
        } else {
            dh
        },
        stack: if parts.len() == 4 {
            parse_u64(parts[2], "budget stack")?
        } else {
            ds
        },
        depth: if parts.len() == 4 {
            parse_u64(parts[3], "budget depth")?
        } else {
            dd
        },
    };
    validate_budget(obj, &b)?;
    Ok(b)
}

pub fn default_budget(obj: &Object, steps: u64) -> Budget {
    let (h, s, d) = default_limits(obj);
    Budget {
        steps,
        heap: h,
        stack: s,
        depth: d,
    }
}

/// A `token:B[,heap,stack,depth]` value (§5.4-validated by callers).
pub fn parse_token(text: &str, defaults: &Budget) -> Result<Budget, String> {
    let inner = text
        .strip_prefix("token:")
        .ok_or_else(|| format!("invalid token value {text:?}"))?;
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 1 && parts.len() != 4 {
        return Err(format!("invalid token value {text:?}"));
    }
    Ok(Budget {
        steps: parse_u64(parts[0], "token steps")?,
        heap: if parts.len() == 4 {
            parse_u64(parts[1], "token heap")?
        } else {
            defaults.heap
        },
        stack: if parts.len() == 4 {
            parse_u64(parts[2], "token stack")?
        } else {
            defaults.stack
        },
        depth: if parts.len() == 4 {
            parse_u64(parts[3], "token depth")?
        } else {
            defaults.depth
        },
    })
}

pub enum RawArg {
    Int(String),
    F64(f64),
    Token(Budget),
    QregBits(String),
}

pub fn parse_raw_arg(text: &str, token_defaults: &Budget) -> Result<RawArg, String> {
    if let Some(rest) = text.strip_prefix("f64:") {
        let v: f64 = rest
            .parse()
            .map_err(|_| format!("invalid f64 argument {text:?}"))?;
        if !v.is_finite() {
            return Err(format!("invalid f64 argument {text:?}: not finite"));
        }
        Ok(RawArg::F64(v))
    } else if text.starts_with("token:") {
        Ok(RawArg::Token(parse_token(text, token_defaults)?))
    } else if let Some(bits) = text.strip_prefix("qreg:") {
        Ok(RawArg::QregBits(bits.to_string()))
    } else {
        Ok(RawArg::Int(text.to_string()))
    }
}

/// Bind a raw integer to an `iN`/`ptr` width: unsigned range, plus
/// negatives within the signed range (§10 CLI convention).
pub fn bind_int(text: &str, width: usize, type_name: &str) -> Result<BigUint, String> {
    if let Some(neg) = text.strip_prefix('-') {
        if neg.starts_with("0x") || neg.starts_with("0X") {
            return Err(format!(
                "§10: E_ARG: integer {text} does not fit {type_name}"
            ));
        }
        let mag = BigUint::parse_decimal(neg)
            .map_err(|_| format!("§10: E_ARG: invalid integer argument {text:?}"))?;
        let min_mag = BigUint::from_u64(1).shl(width - 1);
        if width == 0 || mag.is_zero() || mag.cmp_big(&min_mag) == std::cmp::Ordering::Greater {
            return Err(format!(
                "§10: E_ARG: integer {text} does not fit {type_name}"
            ));
        }
        return Ok(BigUint::from_u64(1).shl(width).sub(&mag).mask(width));
    }
    let v = BigUint::parse_int(text)
        .map_err(|_| format!("§10: E_ARG: invalid integer argument {text:?}"))?;
    if v.bits() > width {
        return Err(format!(
            "§10: E_ARG: integer {text} does not fit {type_name}"
        ));
    }
    Ok(v)
}

/// Option scanner: positionals plus `--flag value` / boolean flags.
pub struct Opts {
    pub positional: Vec<String>,
    pub map: std::collections::BTreeMap<String, Vec<String>>,
    pub flags: std::collections::BTreeSet<String>,
}

pub fn scan_opts(
    args: &[String],
    value_flags: &[&str],
    bool_flags: &[&str],
) -> Result<Opts, String> {
    let mut out = Opts {
        positional: Vec::new(),
        map: Default::default(),
        flags: Default::default(),
    };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.strip_prefix("--") {
            if value_flags.contains(&name) {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| format!("missing value for --{name}"))?;
                out.map.entry(name.to_string()).or_default().push(v.clone());
                i += 2;
            } else if bool_flags.contains(&name) {
                out.flags.insert(name.to_string());
                i += 1;
            } else {
                return Err(format!("unknown option --{name}"));
            }
        } else {
            out.positional.push(a.clone());
            i += 1;
        }
    }
    Ok(out)
}

impl Opts {
    pub fn one(&self, name: &str) -> Result<Option<&str>, String> {
        match self.map.get(name) {
            None => Ok(None),
            Some(v) if v.len() == 1 => Ok(Some(v[0].as_str())),
            Some(_) => Err(format!("option --{name} given more than once")),
        }
    }
    pub fn all(&self, name: &str) -> Vec<String> {
        self.map.get(name).cloned().unwrap_or_default()
    }
}

/// Strip a leading `@` from an entry name, as the reference CLI allows.
pub fn entry_name(text: &str) -> &str {
    text.strip_prefix('@').unwrap_or(text)
}

pub fn read_object(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}
