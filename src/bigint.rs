//! Arbitrary-width unsigned bit-vectors (§14.1: bit-vectors of any width).
//!
//! Little-endian `u64` limbs, normalized (no trailing zero limbs; zero is
//! the empty vector). Width is carried by callers; `mask` applies it.

use std::cmp::Ordering;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BigUint {
    limbs: Vec<u64>,
}

impl BigUint {
    pub fn zero() -> Self {
        BigUint { limbs: Vec::new() }
    }
    pub fn from_u64(v: u64) -> Self {
        if v == 0 { Self::zero() } else { BigUint { limbs: vec![v] } }
    }
    pub fn from_u128(v: u128) -> Self {
        let lo = v as u64;
        let hi = (v >> 64) as u64;
        let mut b = BigUint { limbs: vec![lo, hi] };
        b.norm();
        b
    }
    fn norm(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
    }
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }
    pub fn bit(&self, i: usize) -> bool {
        let l = i / 64;
        l < self.limbs.len() && (self.limbs[l] >> (i % 64)) & 1 == 1
    }
    pub fn set_bit(&mut self, i: usize, v: bool) {
        let l = i / 64;
        if l >= self.limbs.len() {
            self.limbs.resize(l + 1, 0);
        }
        if v {
            self.limbs[l] |= 1u64 << (i % 64);
        } else {
            self.limbs[l] &= !(1u64 << (i % 64));
        }
        self.norm();
    }
    /// Highest set bit index + 1; 0 for zero.
    pub fn bits(&self) -> usize {
        match self.limbs.last() {
            None => 0,
            Some(&top) => (self.limbs.len() - 1) * 64 + (64 - top.leading_zeros() as usize),
        }
    }
    pub fn mask(&self, width: usize) -> BigUint {
        if width == 0 {
            return Self::zero();
        }
        let keep = width.div_ceil(64);
        let mut limbs: Vec<u64> = self.limbs.iter().take(keep).copied().collect();
        let rem = width % 64;
        if rem != 0 && limbs.len() == keep {
            limbs[keep - 1] &= (1u64 << rem) - 1;
        }
        let mut b = BigUint { limbs };
        b.norm();
        b
    }
    pub fn cmp_big(&self, other: &BigUint) -> Ordering {
        if self.limbs.len() != other.limbs.len() {
            return self.limbs.len().cmp(&other.limbs.len());
        }
        for i in (0..self.limbs.len()).rev() {
            if self.limbs[i] != other.limbs[i] {
                return self.limbs[i].cmp(&other.limbs[i]);
            }
        }
        Ordering::Equal
    }
    pub fn add(&self, other: &BigUint) -> BigUint {
        let n = self.limbs.len().max(other.limbs.len());
        let mut limbs = Vec::with_capacity(n + 1);
        let mut carry = 0u128;
        for i in 0..n {
            let a = self.limbs.get(i).copied().unwrap_or(0) as u128;
            let b = other.limbs.get(i).copied().unwrap_or(0) as u128;
            let s = a + b + carry;
            limbs.push(s as u64);
            carry = s >> 64;
        }
        if carry != 0 {
            limbs.push(carry as u64);
        }
        let mut r = BigUint { limbs };
        r.norm();
        r
    }
    /// Requires self >= other.
    pub fn sub(&self, other: &BigUint) -> BigUint {
        debug_assert!(self.cmp_big(other) != Ordering::Less);
        let mut limbs = Vec::with_capacity(self.limbs.len());
        let mut borrow = 0i128;
        for i in 0..self.limbs.len() {
            let a = self.limbs[i] as i128;
            let b = other.limbs.get(i).copied().unwrap_or(0) as i128;
            let mut d = a - b - borrow;
            if d < 0 {
                d += 1i128 << 64;
                borrow = 1;
            } else {
                borrow = 0;
            }
            limbs.push(d as u64);
        }
        let mut r = BigUint { limbs };
        r.norm();
        r
    }
    pub fn mul(&self, other: &BigUint) -> BigUint {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let mut limbs = vec![0u64; self.limbs.len() + other.limbs.len()];
        for (i, &a) in self.limbs.iter().enumerate() {
            let mut carry = 0u128;
            for (j, &b) in other.limbs.iter().enumerate() {
                let cur = limbs[i + j] as u128 + (a as u128) * (b as u128) + carry;
                limbs[i + j] = cur as u64;
                carry = cur >> 64;
            }
            limbs[i + other.limbs.len()] = carry as u64;
        }
        let mut r = BigUint { limbs };
        r.norm();
        r
    }
    /// (quotient, remainder); divisor must be nonzero.
    pub fn divmod(&self, divisor: &BigUint) -> (BigUint, BigUint) {
        assert!(!divisor.is_zero(), "BigUint::divmod by zero");
        if self.cmp_big(divisor) == Ordering::Less {
            return (Self::zero(), self.clone());
        }
        let mut q = Self::zero();
        let mut r = Self::zero();
        for i in (0..self.bits()).rev() {
            r = r.shl(1);
            if self.bit(i) {
                r.set_bit(0, true);
            }
            if r.cmp_big(divisor) != Ordering::Less {
                r = r.sub(divisor);
                q.set_bit(i, true);
            }
        }
        (q, r)
    }
    pub fn shl(&self, n: usize) -> BigUint {
        if self.is_zero() {
            return Self::zero();
        }
        let words = n / 64;
        let rem = n % 64;
        let mut limbs = vec![0u64; words];
        if rem == 0 {
            limbs.extend_from_slice(&self.limbs);
        } else {
            let mut carry = 0u64;
            for &l in &self.limbs {
                limbs.push((l << rem) | carry);
                carry = l >> (64 - rem);
            }
            if carry != 0 {
                limbs.push(carry);
            }
        }
        let mut r = BigUint { limbs };
        r.norm();
        r
    }
    pub fn shr(&self, n: usize) -> BigUint {
        let words = n / 64;
        if words >= self.limbs.len() {
            return Self::zero();
        }
        let rem = n % 64;
        let mut limbs = Vec::new();
        for i in words..self.limbs.len() {
            let mut v = self.limbs[i] >> rem;
            if rem != 0 && i + 1 < self.limbs.len() {
                v |= self.limbs[i + 1] << (64 - rem);
            }
            limbs.push(v);
        }
        let mut r = BigUint { limbs };
        r.norm();
        r
    }
    fn binop(&self, other: &BigUint, f: fn(u64, u64) -> u64) -> BigUint {
        let n = self.limbs.len().max(other.limbs.len());
        let mut limbs = Vec::with_capacity(n);
        for i in 0..n {
            limbs.push(f(
                self.limbs.get(i).copied().unwrap_or(0),
                other.limbs.get(i).copied().unwrap_or(0),
            ));
        }
        let mut r = BigUint { limbs };
        r.norm();
        r
    }
    pub fn bitand(&self, o: &BigUint) -> BigUint {
        self.binop(o, |a, b| a & b)
    }
    pub fn bitor(&self, o: &BigUint) -> BigUint {
        self.binop(o, |a, b| a | b)
    }
    pub fn bitxor(&self, o: &BigUint) -> BigUint {
        self.binop(o, |a, b| a ^ b)
    }
    pub fn to_u64(&self) -> Option<u64> {
        match self.limbs.len() {
            0 => Some(0),
            1 => Some(self.limbs[0]),
            _ => None,
        }
    }
    pub fn to_u128(&self) -> Option<u128> {
        if self.bits() > 128 {
            return None;
        }
        let lo = self.limbs.first().copied().unwrap_or(0) as u128;
        let hi = self.limbs.get(1).copied().unwrap_or(0) as u128;
        Some(lo | (hi << 64))
    }
    pub fn to_usize(&self) -> Option<usize> {
        self.to_u64().and_then(|v| usize::try_from(v).ok())
    }
    pub fn parse_decimal(text: &str) -> Result<BigUint, String> {
        if text.is_empty() || !text.bytes().all(|c| c.is_ascii_digit()) {
            return Err(format!("invalid decimal integer {text:?}"));
        }
        let ten = BigUint::from_u64(10);
        let mut r = BigUint::zero();
        for c in text.bytes() {
            r = r.mul(&ten).add(&BigUint::from_u64((c - b'0') as u64));
        }
        Ok(r)
    }
    pub fn parse_int(text: &str) -> Result<BigUint, String> {
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            if hex.is_empty() {
                return Err(format!("invalid hex integer {text:?}"));
            }
            let mut r = BigUint::zero();
            for (i, c) in hex.chars().rev().enumerate() {
                let v = c.to_digit(16).ok_or_else(|| format!("invalid hex integer {text:?}"))?;
                if v != 0 {
                    r = r.add(&BigUint::from_u64(v as u64).shl(i * 4));
                }
            }
            Ok(r)
        } else {
            Self::parse_decimal(text)
        }
    }
    pub fn to_decimal(&self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        let mut cur = self.clone();
        let big = BigUint::from_u64(1_000_000_000_000_000_000);
        let mut chunks: Vec<u64> = Vec::new();
        while !cur.is_zero() {
            let (q, r) = cur.divmod(&big);
            chunks.push(r.to_u64().unwrap());
            cur = q;
        }
        let mut out = format!("{}", chunks[chunks.len() - 1]);
        for c in chunks.iter().rev().skip(1) {
            out.push_str(&format!("{c:018}"));
        }
        out
    }
    pub fn limbs(&self) -> &[u64] {
        &self.limbs
    }
    pub fn from_limbs(limbs: Vec<u64>) -> BigUint {
        let mut b = BigUint { limbs };
        b.norm();
        b
    }
}

impl std::fmt::Display for BigUint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_decimal())
    }
}

/// Sign-extend the low `width` bits of `v` to `to` bits.
pub fn sign_extend(v: &BigUint, width: usize, to: usize) -> BigUint {
    let v = v.mask(width);
    if width == 0 || to <= width || !v.bit(width - 1) {
        return v.mask(to);
    }
    let mut r = v;
    for i in width..to {
        r.set_bit(i, true);
    }
    r
}

/// Signed value of an unsigned `width`-bit vector, as (negative, magnitude).
pub fn to_signed(v: &BigUint, width: usize) -> (bool, BigUint) {
    let v = v.mask(width);
    if width == 0 || !v.bit(width - 1) {
        return (false, v);
    }
    let mag = BigUint::from_u64(1).shl(width).sub(&v);
    (true, mag)
}

/// Unsigned bits of signed (negative, magnitude) at `width`.
pub fn from_signed(neg: bool, mag: &BigUint, width: usize) -> BigUint {
    if !neg || mag.is_zero() {
        return mag.mask(width);
    }
    BigUint::from_u64(1).shl(width).sub(mag).mask(width)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arithmetic() {
        let x = BigUint::parse_decimal("1606938044258990275541962092341162602522202993782792835301379").unwrap();
        let y = BigUint::parse_decimal("1267650600228229401496703205381").unwrap();
        assert_eq!(x.to_decimal(), "1606938044258990275541962092341162602522202993782792835301379");
        let (q, r) = x.divmod(&y);
        assert_eq!(q.to_decimal(), "1267650600228229401496703205371");
        assert_eq!(r.to_u64(), Some(28));
        assert_eq!(q.mul(&y).add(&r), x);
        assert_eq!(BigUint::from_u64(1).shl(257).bits(), 258);
        assert_eq!(x.mask(8).to_u64(), Some(3));
        let (neg, mag) = to_signed(&BigUint::from_u64(0xff), 8);
        assert!(neg && mag == BigUint::from_u64(1));
        assert_eq!(from_signed(true, &BigUint::from_u64(1), 8), BigUint::from_u64(0xff));
    }
}

