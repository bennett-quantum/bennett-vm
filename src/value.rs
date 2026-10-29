//! Classical values, budgets and width arithmetic shared by the CC/R
//! executor and the §14.1 oracle interpreter.

use crate::bigint::{from_signed, to_signed, BigUint};
use crate::json::Json;
use std::cmp::Ordering;

pub const FAULT_NONE: u64 = 0;
pub const FAULT_BUDGET_STEPS: u64 = 1;
pub const FAULT_BUDGET_HEAP: u64 = 2;
pub const FAULT_BUDGET_DEPTH: u64 = 3;
pub const FAULT_UB_TRAP: u64 = 4;
pub const FAULT_UB_DIV0: u64 = 5;
pub const FAULT_UB_SDIV_OVERFLOW: u64 = 6;
pub const FAULT_UB_MEMFAULT: u64 = 7;
pub const FAULT_EFFECT_UNMODELLED: u64 = 8;
pub const FAULT_CALLEE: u64 = 9;
pub const FAULT_ENTROPY_EXHAUSTED: u64 = 10;

pub fn fault_name(code: u64) -> Option<&'static str> {
    Some(match code {
        0 => "FAULT_NONE",
        1 => "FAULT_BUDGET_STEPS",
        2 => "FAULT_BUDGET_HEAP",
        3 => "FAULT_BUDGET_DEPTH",
        4 => "FAULT_UB_TRAP",
        5 => "FAULT_UB_DIV0",
        6 => "FAULT_UB_SDIV_OVERFLOW",
        7 => "FAULT_UB_MEMFAULT",
        8 => "FAULT_EFFECT_UNMODELLED",
        9 => "FAULT_CALLEE",
        10 => "FAULT_ENTROPY_EXHAUSTED",
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    pub steps: u64,
    pub heap: u64,
    pub stack: u64,
    pub depth: u64,
}

impl Budget {
    pub fn to_json(&self) -> Json {
        Json::obj(vec![
            ("depth", Json::u64(self.depth)),
            ("heap", Json::u64(self.heap)),
            ("stack", Json::u64(self.stack)),
            ("steps", Json::u64(self.steps)),
        ])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int { width: usize, bits: BigUint },
    Ptr(BigUint),
    F64(f64),
    Token(Budget),
    Qreg(u64),
}

impl Value {
    pub fn int(width: usize, bits: BigUint) -> Value {
        Value::Int { width, bits: bits.mask(width) }
    }
    /// Unsigned bits of an integer/pointer value.
    pub fn bits(&self) -> Option<&BigUint> {
        match self {
            Value::Int { bits, .. } => Some(bits),
            Value::Ptr(bits) => Some(bits),
            _ => None,
        }
    }
    pub fn width(&self, ptr_width: usize) -> Option<usize> {
        match self {
            Value::Int { width, .. } => Some(*width),
            Value::Ptr(_) => Some(ptr_width),
            _ => None,
        }
    }
    pub fn type_name(&self) -> String {
        match self {
            Value::Int { width, .. } => format!("i{width}"),
            Value::Ptr(_) => "ptr".into(),
            Value::F64(_) => "f64".into(),
            Value::Token(_) => "token".into(),
            Value::Qreg(_) => "qreg".into(),
        }
    }
    pub fn to_json(&self) -> Json {
        match self {
            Value::Int { width, bits } => Json::obj(vec![
                ("type", Json::Str(format!("i{width}"))),
                ("value", Json::Str(bits.to_decimal())),
            ]),
            Value::Ptr(bits) => Json::obj(vec![
                ("type", Json::Str("ptr".into())),
                ("value", Json::Str(bits.to_decimal())),
            ]),
            Value::F64(v) => Json::obj(vec![
                ("type", Json::Str("f64".into())),
                ("value", Json::Num(crate::json::format_f64(*v))),
            ]),
            Value::Token(b) => Json::obj(vec![
                ("type", Json::Str("token".into())),
                ("value", b.to_json()),
            ]),
            Value::Qreg(id) => Json::obj(vec![
                ("type", Json::Str("qreg".into())),
                ("value", Json::u64(*id)),
            ]),
        }
    }
    /// Debug-shaped rendering used in `first_difference` strings.
    pub fn debug_shape(&self) -> String {
        match self {
            Value::Int { width, bits } => format!("Int({width}, {bits})"),
            Value::Ptr(bits) => format!("Ptr({bits})"),
            Value::F64(v) => format!("F64({v})"),
            Value::Token(b) => format!("Token({}, {}, {}, {})", b.steps, b.heap, b.stack, b.depth),
            Value::Qreg(id) => format!("Qreg({id})"),
        }
    }
}

/// `op` over `width`-bit values; wraps. Shift amounts at or above the
/// width give zero, for `ashr` too (§4.1).
pub fn binop(op: &str, a: &BigUint, b: &BigUint, width: usize) -> BigUint {
    let a = a.mask(width);
    let b = b.mask(width);
    let shift = b.to_usize().unwrap_or(usize::MAX);
    let r = match op {
        "add" => a.add(&b),
        "sub" => {
            if a.cmp_big(&b) == Ordering::Less {
                BigUint::from_u64(1).shl(width).add(&a).sub(&b)
            } else {
                a.sub(&b)
            }
        }
        "mul" => a.mul(&b),
        "and" => a.bitand(&b),
        "or" => a.bitor(&b),
        "xor" => a.bitxor(&b),
        "shl" => if shift >= width { BigUint::zero() } else { a.shl(shift) },
        "lshr" => if shift >= width { BigUint::zero() } else { a.shr(shift) },
        "ashr" => {
            if shift >= width {
                BigUint::zero()
            } else {
                let (neg, mag) = to_signed(&a, width);
                if !neg {
                    a.shr(shift)
                } else {
                    // ashr of -m by s = -(ceil(m / 2^s)) for m > 0.
                    let denom = BigUint::from_u64(1).shl(shift);
                    let (q, r) = mag.divmod(&denom);
                    let q = if r.is_zero() { q } else { q.add(&BigUint::from_u64(1)) };
                    from_signed(true, &q, width)
                }
            }
        }
        _ => panic!("binop: unknown op {op}"),
    };
    r.mask(width)
}

/// Fault code a division raises, if any (§4.2 `lift.div`).
pub fn div_fault(op: &str, a: &BigUint, b: &BigUint, width: usize) -> Option<u64> {
    let a = a.mask(width);
    let b = b.mask(width);
    if b.is_zero() {
        return Some(FAULT_UB_DIV0);
    }
    if op == "sdiv" || op == "srem" {
        let (na, ma) = to_signed(&a, width);
        let (nb, mb) = to_signed(&b, width);
        let min_mag = BigUint::from_u64(1).shl(width - 1);
        if na && ma == min_mag && nb && mb == BigUint::from_u64(1) {
            return Some(FAULT_UB_SDIV_OVERFLOW);
        }
    }
    None
}

/// Division proper; caller checked `div_fault`. Signed division
/// truncates toward zero (§3.3).
pub fn divop(op: &str, a: &BigUint, b: &BigUint, width: usize) -> BigUint {
    let a = a.mask(width);
    let b = b.mask(width);
    match op {
        "udiv" => a.divmod(&b).0,
        "urem" => a.divmod(&b).1,
        "sdiv" => {
            let (na, ma) = to_signed(&a, width);
            let (nb, mb) = to_signed(&b, width);
            let q = ma.divmod(&mb).0;
            from_signed(na != nb, &q, width)
        }
        "srem" => {
            let (na, ma) = to_signed(&a, width);
            let (_, mb) = to_signed(&b, width);
            let r = ma.divmod(&mb).1;
            from_signed(na, &r, width)
        }
        _ => panic!("divop: unknown op {op}"),
    }
}

pub fn icmp(pred: &str, a: &BigUint, b: &BigUint, width: usize) -> bool {
    let a = a.mask(width);
    let b = b.mask(width);
    match pred {
        "eq" => a == b,
        "ne" => a != b,
        "ult" => a.cmp_big(&b) == Ordering::Less,
        "ule" => a.cmp_big(&b) != Ordering::Greater,
        "slt" | "sle" => {
            let (na, ma) = to_signed(&a, width);
            let (nb, mb) = to_signed(&b, width);
            let ord = match (na, nb) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => ma.cmp_big(&mb),
                (true, true) => mb.cmp_big(&ma),
            };
            if pred == "slt" { ord == Ordering::Less } else { ord != Ordering::Greater }
        }
        _ => panic!("icmp: unknown predicate {pred}"),
    }
}
