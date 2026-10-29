//! Exact signed arbitrary-precision rational arithmetic for §8 decimal costs.
use crate::bigint::BigUint as U;
use std::{cmp::Ordering, fmt, str::FromStr};
#[derive(Clone, Debug, Eq)]
pub struct Rational {
    neg: bool,
    n: U,
    d: U,
}
impl Rational {
    fn new(neg: bool, n: U, d: U) -> Self {
        assert!(!d.is_zero());
        let mut a = n.clone();
        let mut b = d.clone();
        while !b.is_zero() {
            let r = a.divmod(&b).1;
            a = b;
            b = r;
        }
        let zero = n.is_zero();
        Self {
            neg: neg && !zero,
            n: n.divmod(&a).0,
            d: d.divmod(&a).0,
        }
    }
    pub fn zero() -> Self {
        Self::from(0)
    }
    pub fn is_zero(&self) -> bool {
        self.n.is_zero()
    }
    pub fn negative(&self) -> bool {
        self.neg
    }
    pub fn add(&self, b: &Self) -> Self {
        let a = self.n.mul(&b.d);
        let n = b.n.mul(&self.d);
        let d = self.d.mul(&b.d);
        if self.neg == b.neg {
            Self::new(self.neg, a.add(&n), d)
        } else if a >= n {
            Self::new(self.neg, a.sub(&n), d)
        } else {
            Self::new(b.neg, n.sub(&a), d)
        }
    }
    pub fn sub(&self, b: &Self) -> Self {
        let mut b = b.clone();
        b.neg = !b.neg;
        self.add(&b)
    }
    pub fn mul(&self, b: &Self) -> Self {
        Self::new(self.neg != b.neg, self.n.mul(&b.n), self.d.mul(&b.d))
    }
    pub fn div(&self, b: &Self) -> Result<Self, String> {
        if b.is_zero() {
            return Err("§8.1: division by zero".into());
        }
        Ok(Self::new(
            self.neg != b.neg,
            self.n.mul(&b.d),
            self.d.mul(&b.n),
        ))
    }
    pub fn ceil(&self) -> Self {
        let (q, r) = self.n.divmod(&self.d);
        Self::new(
            self.neg,
            if !self.neg && !r.is_zero() {
                q.add(&U::from_u64(1))
            } else {
                q
            },
            U::from_u64(1),
        )
    }
    pub fn usize(&self) -> Result<usize, String> {
        if self.neg || self.d != U::from_u64(1) {
            return Err("§8.1: index/count must be a nonnegative integer".into());
        }
        self.n
            .to_usize()
            .ok_or("§8.1: index/count exceeds host address space".into())
    }
    /// Exact decimal when finite; otherwise an explicit rational diagnostic.
    pub fn decimal(&self) -> String {
        let (q, mut rem) = self.n.divmod(&self.d);
        let mut out = format!("{}{}", if self.neg { "-" } else { "" }, q.to_decimal());
        if rem.is_zero() {
            return out;
        }
        let mut d = self.d.clone();
        for p in [2, 5] {
            let p = U::from_u64(p);
            while d.divmod(&p).1.is_zero() {
                d = d.divmod(&p).0;
            }
        }
        if d != U::from_u64(1) {
            return format!(
                "{}{}/{}",
                if self.neg { "-" } else { "" },
                self.n.to_decimal(),
                self.d.to_decimal()
            );
        }
        out.push('.');
        while !rem.is_zero() {
            let (q, r) = rem.mul(&U::from_u64(10)).divmod(&self.d);
            out.push_str(&q.to_decimal());
            rem = r;
        }
        out
    }
    pub fn f64(&self) -> Result<f64, String> {
        let n = self
            .n
            .to_decimal()
            .parse::<f64>()
            .map_err(|e| e.to_string())?;
        let d = self
            .d
            .to_decimal()
            .parse::<f64>()
            .map_err(|e| e.to_string())?;
        let v = if self.neg { -n / d } else { n / d };
        if !v.is_finite() {
            return Err("§8: rational cannot be represented as a finite simulator angle".into());
        }
        Ok(v)
    }
}
impl From<u64> for Rational {
    fn from(n: u64) -> Self {
        Self {
            neg: false,
            n: U::from_u64(n),
            d: U::from_u64(1),
        }
    }
}
impl FromStr for Rational {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (neg, s) = match s.strip_prefix('-') {
            Some(s) => (true, s),
            None => (false, s),
        };
        let (s, e) = match s.split_once(['e', 'E']) {
            Some((s, e)) => (
                s,
                e.parse::<i64>()
                    .map_err(|_| "§8.1: invalid decimal exponent")?,
            ),
            None => (s, 0),
        };
        let (a, b) = s.split_once('.').unwrap_or((s, ""));
        if a.is_empty() || !a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit()) {
            return Err("§8.1: invalid decimal".into());
        }
        let n = U::parse_decimal(&format!("{a}{b}"))?;
        let scale = (b.len() as i64)
            .checked_sub(e)
            .ok_or("§8.1: decimal exponent overflow")?;
        let mut ten = U::from_u64(1);
        for _ in 0..scale.unsigned_abs() {
            ten = ten.mul(&U::from_u64(10));
        }
        Ok(if scale >= 0 {
            Self::new(neg, n, ten)
        } else {
            Self::new(neg, n.mul(&ten), U::from_u64(1))
        })
    }
}
impl Ord for Rational {
    fn cmp(&self, b: &Self) -> Ordering {
        match (self.neg, b.neg) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => {
                let c = self.n.mul(&b.d).cmp(&b.n.mul(&self.d));
                if self.neg { c.reverse() } else { c }
            }
        }
    }
}
impl PartialOrd for Rational {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl PartialEq for Rational {
    fn eq(&self, b: &Self) -> bool {
        self.cmp(b) == Ordering::Equal
    }
}
impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.decimal())
    }
}
