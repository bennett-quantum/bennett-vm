//! Exact finite sums c sqrt(2)^s i^j exp(i a), with dyadic c and a.
//! All binary64 inputs are embedded as their exact dyadic values. No epsilon
//! enters support, cleanliness or equality. Numerical evaluation is display only.
use crate::bigint::BigUint;
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dyad {
    neg: bool,
    mag: BigUint,
    scale: usize,
}
impl Dyad {
    fn new(neg: bool, mut mag: BigUint, mut scale: usize) -> Self {
        while scale > 0 && !mag.is_zero() && !mag.bit(0) {
            mag = mag.shr(1);
            scale -= 1;
        }
        if mag.is_zero() {
            Self {
                neg: false,
                mag,
                scale: 0,
            }
        } else {
            Self { neg, mag, scale }
        }
    }
    pub fn int(n: i64) -> Self {
        Self::new(n < 0, BigUint::from_u64(n.unsigned_abs()), 0)
    }
    pub fn float(x: f64) -> Self {
        assert!(x.is_finite());
        let bits = x.to_bits();
        let e = ((bits >> 52) & 2047) as i32;
        let m = (bits & ((1 << 52) - 1)) | if e == 0 { 0 } else { 1 << 52 };
        let exponent = if e == 0 { -1074 } else { e - 1023 - 52 };
        Self::new(
            bits >> 63 != 0,
            BigUint::from_u64(m).shl(exponent.max(0) as usize),
            (-exponent).max(0) as usize,
        )
    }
    pub fn neg(&self) -> Self {
        Self::new(!self.neg, self.mag.clone(), self.scale)
    }
    pub fn add(&self, b: &Self) -> Self {
        let s = self.scale.max(b.scale);
        let a = self.mag.shl(s - self.scale);
        let bmag = b.mag.shl(s - b.scale);
        if self.neg == b.neg {
            Self::new(self.neg, a.add(&bmag), s)
        } else if a >= bmag {
            Self::new(self.neg, a.sub(&bmag), s)
        } else {
            Self::new(b.neg, bmag.sub(&a), s)
        }
    }
    fn mul(&self, b: &Self) -> Self {
        Self::new(
            self.neg != b.neg,
            self.mag.mul(&b.mag),
            self.scale + b.scale,
        )
    }
    pub fn half(&self) -> Self {
        Self::new(self.neg, self.mag.clone(), self.scale + 1)
    }
    fn twice(&self) -> Self {
        Self::new(self.neg, self.mag.shl(1), self.scale)
    }
    fn unit_phase(&self) -> (f64, f64) {
        // Evaluate a bounded factor, then square; the exact angle stays in
        // the dyadic group algebra and is never converted through infinity.
        let mut a = self.clone();
        let mut squares = 0;
        while !a.value().is_finite() {
            a = a.half();
            squares += 1;
        }
        let (mut im, mut re) = a.value().sin_cos();
        for _ in 0..squares {
            let rr = re * re - im * im;
            let ii = 2. * re * im;
            let norm = rr.hypot(ii);
            re = rr / norm;
            im = ii / norm;
        }
        (re, im)
    }
    fn value(&self) -> f64 {
        let bits = self.mag.bits();
        let shift = bits.saturating_sub(53);
        let m = self.mag.shr(shift).to_u64().unwrap_or(0) as f64;
        let e = shift as i64 - self.scale as i64;
        let v = if e > 1023 {
            f64::INFINITY
        } else if e < -1074 {
            0.0
        } else {
            m * 2f64.powi(e as i32)
        };
        if self.neg { -v } else { v }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Exact(BTreeMap<(Dyad, bool, bool), Dyad>);
impl Exact {
    pub fn zero() -> Self {
        Self::default()
    }
    pub fn one() -> Self {
        Self::term(Dyad::int(1), false, false, Dyad::int(0))
    }
    fn term(c: Dyad, s: bool, i: bool, a: Dyad) -> Self {
        let mut x = Self::zero();
        x.insert((a, s, i), c);
        x
    }
    fn insert(&mut self, k: (Dyad, bool, bool), c: Dyad) {
        let c = self.0.get(&k).map_or(c.clone(), |v| v.add(&c));
        if c.mag.is_zero() {
            self.0.remove(&k);
        } else {
            self.0.insert(k, c);
        }
    }
    pub fn pair(re: f64, im: f64) -> Self {
        Self::term(Dyad::float(re), false, false, Dyad::int(0)).add(&Self::term(
            Dyad::float(im),
            false,
            true,
            Dyad::int(0),
        ))
    }
    pub fn sqrt_half() -> Self {
        Self::term(Dyad::int(1).half(), true, false, Dyad::int(0))
    }
    pub fn phase(a: Dyad) -> Self {
        Self::term(Dyad::int(1), false, false, a)
    }
    pub fn add(&self, b: &Self) -> Self {
        let mut x = self.clone();
        for (k, c) in &b.0 {
            x.insert(k.clone(), c.clone());
        }
        x
    }
    pub fn neg(&self) -> Self {
        Self(self.0.iter().map(|(k, c)| (k.clone(), c.neg())).collect())
    }
    pub fn mul(&self, b: &Self) -> Self {
        let mut x = Self::zero();
        for ((a, s, i), c) in &self.0 {
            for ((aa, ss, ii), cc) in &b.0 {
                let mut coeff = c.mul(cc);
                if *s && *ss {
                    coeff = coeff.twice();
                }
                if *i && *ii {
                    coeff = coeff.neg();
                }
                x.insert((a.add(aa), s != ss, i != ii), coeff);
            }
        }
        x
    }
    pub fn is_zero(&self) -> bool {
        self.0.is_empty()
    }
    pub fn value(&self) -> (f64, f64) {
        let mut re = 0.;
        let mut im = 0.;
        for ((a, s, i), c) in &self.0 {
            let factor = c.value() * if *s { 2f64.sqrt() } else { 1. };
            let (cos, sin) = a.unit_phase();
            if *i {
                re -= factor * sin;
                im += factor * cos;
            } else {
                re += factor * cos;
                im += factor * sin;
            }
        }
        (re, im)
    }
    pub fn rotation(axis: &str, theta: f64) -> [[Self; 2]; 2] {
        let a = Dyad::float(theta).half();
        let plus = Self::phase(a.clone());
        let minus = Self::phase(a.neg());
        let half = Self::pair(0.5, 0.);
        let c = plus.add(&minus).mul(&half);
        let sin = plus.add(&minus.neg()).mul(&Self::pair(0., -0.5));
        match axis {
            "rx" => {
                let s = sin.mul(&Self::pair(0., -1.));
                [[c.clone(), s.clone()], [s, c]]
            }
            "ry" => [[c.clone(), sin.neg()], [sin, c]],
            _ => [[minus, Self::zero()], [Self::zero(), plus]],
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combined_finite_rotation_angles_have_finite_display() {
        // §3.1 allows every finite f64 gate angle, even if a symbolic sum
        // exceeds binary64. Approximation must not manufacture NaN support.
        let p = Exact::phase(Dyad::float(f64::MAX));
        let (re, im) = p.mul(&p).value();
        assert!(re.is_finite() && im.is_finite());
        assert!((re * re + im * im - 1.).abs() < 2e-12);
    }
    #[test]
    fn exact_inverse_cancellation() {
        let h = Exact::sqrt_half();
        assert_eq!(h.mul(&h).add(&h.mul(&h)), Exact::one());
        for axis in ["rx", "ry", "rz"] {
            let m = Exact::rotation(axis, 0.123);
            let inv = Exact::rotation(axis, -0.123);
            for (i, row) in m.iter().enumerate() {
                for (j, _) in inv[0].iter().enumerate() {
                    let x = row[0].mul(&inv[0][j]).add(&row[1].mul(&inv[1][j]));
                    assert_eq!(x, if i == j { Exact::one() } else { Exact::zero() });
                }
            }
        }
    }
}
