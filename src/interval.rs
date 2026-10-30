//! Rigorous §12.1 matrix checks. Inputs to these algebraic checks must be
//! checker-built enclosures, not producer-supplied realised/reference matrices.
//! Executable claim acceptance additionally requires all schedule contexts.
use crate::{json::Json, rational::Rational as R};
#[derive(Clone, Debug, PartialEq)]
pub struct Interval {
    pub lo: R,
    pub hi: R,
    p: usize,
}
impl Interval {
    pub fn new(lo: R, hi: R, p: usize) -> Result<Self, String> {
        if lo > hi {
            return Err("§12.1: reversed interval".into());
        }
        Ok(Self {
            lo: lo.binary_round(p, false)?,
            hi: hi.binary_round(p, true)?,
            p,
        })
    }
    pub fn exact(n: u64, p: usize) -> Self {
        Self::new(R::from(n), R::from(n), p).unwrap()
    }
    fn add(&self, b: &Self) -> Self {
        Self::new(self.lo.add(&b.lo), self.hi.add(&b.hi), self.p).unwrap()
    }
    fn neg(&self) -> Self {
        Self::new(R::zero().sub(&self.hi), R::zero().sub(&self.lo), self.p).unwrap()
    }
    fn sub(&self, b: &Self) -> Self {
        self.add(&b.neg())
    }
    fn mul(&self, b: &Self) -> Self {
        let v = [
            self.lo.mul(&b.lo),
            self.lo.mul(&b.hi),
            self.hi.mul(&b.lo),
            self.hi.mul(&b.hi),
        ];
        Self::new(
            v.iter().min().unwrap().clone(),
            v.iter().max().unwrap().clone(),
            self.p,
        )
        .unwrap()
    }
    fn div(&self, b: &Self) -> Result<Self, String> {
        if b.lo <= R::zero() && b.hi >= R::zero() {
            return Err("§12.1: interval division contains zero".into());
        }
        let reciprocal = Self::new(R::from(1).div(&b.hi)?, R::from(1).div(&b.lo)?, self.p)?;
        Ok(self.mul(&reciprocal))
    }
    fn square(&self) -> Self {
        let low = if self.lo <= R::zero() && self.hi >= R::zero() {
            R::zero()
        } else {
            self.lo.mul(&self.lo).min(self.hi.mul(&self.hi))
        };
        Self::new(
            low,
            self.lo.mul(&self.lo).max(self.hi.mul(&self.hi)),
            self.p,
        )
        .unwrap()
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Complex {
    pub re: Interval,
    pub im: Interval,
}
impl Complex {
    pub fn integer(n: u64, p: usize) -> Self {
        Self {
            re: Interval::exact(n, p),
            im: Interval::exact(0, p),
        }
    }
    pub fn parse(j: &Json, p: usize) -> Result<Self, String> {
        let a = j
            .as_arr()
            .filter(|a| a.len() == 4)
            .ok_or("§12.1: entry must be [re_lo,re_hi,im_lo,im_hi]")?;
        let mut v = vec![];
        for n in a {
            v.push(
                n.as_str()
                    .ok_or("§12.1: matrix bounds must be decimal strings")?
                    .parse::<R>()?,
            );
        }
        Ok(Self {
            re: Interval::new(v[0].clone(), v[1].clone(), p)?,
            im: Interval::new(v[2].clone(), v[3].clone(), p)?,
        })
    }
    fn conj(&self) -> Self {
        Self {
            re: self.re.clone(),
            im: self.im.neg(),
        }
    }
    fn add(&self, b: &Self) -> Self {
        Self {
            re: self.re.add(&b.re),
            im: self.im.add(&b.im),
        }
    }
    fn sub(&self, b: &Self) -> Self {
        Self {
            re: self.re.sub(&b.re),
            im: self.im.sub(&b.im),
        }
    }
    fn mul(&self, b: &Self) -> Self {
        Self {
            re: self.re.mul(&b.re).sub(&self.im.mul(&b.im)),
            im: self.re.mul(&b.im).add(&self.im.mul(&b.re)),
        }
    }
    fn scale(&self, b: &Interval) -> Self {
        Self {
            re: self.re.mul(b),
            im: self.im.mul(b),
        }
    }
    fn divide(&self, b: &Interval) -> Result<Self, String> {
        Ok(Self {
            re: self.re.div(b)?,
            im: self.im.div(b)?,
        })
    }
}
fn dimensions(m: &[Complex], d: usize, p: usize) -> Result<(), String> {
    if d == 0 || d.checked_mul(d) != Some(m.len()) || m.iter().any(|z| z.re.p != p || z.im.p != p) {
        return Err("§12.1: matrix dimension/precision mismatch".into());
    }
    Ok(())
}
/// Interval Hermitian LDL†, strictly positive pivot lower bounds only. No
/// semidefinite/minor shortcut, numerical tolerance or binary64 arithmetic.
pub fn positive_definite(m: &[Complex], d: usize, p: usize) -> Result<(), String> {
    dimensions(m, d, p)?;
    let zero = R::zero();
    for i in 0..d {
        if !m[i * d + i].im.lo.is_zero() || !m[i * d + i].im.hi.is_zero() {
            return Err("§12.1: Hermitian diagonal must be real".into());
        }
        for j in 0..d {
            if m[i * d + j] != m[j * d + i].conj() {
                return Err("§12.1: witness is not interval Hermitian".into());
            }
        }
    }
    let mut l = vec![Complex::integer(0, p); d * d];
    let mut diag = vec![];
    for k in 0..d {
        let mut pivot = m[k * d + k].re.clone();
        for j in 0..k {
            let z = &l[k * d + j];
            pivot = pivot.sub(&z.re.square().add(&z.im.square()).mul(&diag[j]));
        }
        if pivot.lo <= zero {
            return Err(format!(
                "§12.1: interval LDL† pivot {k} lower bound is not strictly positive (inconclusive rejects)"
            ));
        }
        l[k * d + k] = Complex::integer(1, p);
        for i in k + 1..d {
            let mut a = m[i * d + k].clone();
            for j in 0..k {
                a = a.sub(&l[i * d + j].mul(&l[k * d + j].conj()).scale(&diag[j]));
            }
            l[i * d + k] = a.divide(&pivot)?;
        }
        diag.push(pivot);
    }
    Ok(())
}
/// Verify the numerical Frobenius enclosure against a nonnegative bound.
/// Squaring avoids an unnecessary square-root enclosure without weakening ≤.
pub fn numerical(
    u: &[Complex],
    v: &[Complex],
    d: usize,
    p: usize,
    bound: &R,
) -> Result<(), String> {
    dimensions(u, d, p)?;
    dimensions(v, d, p)?;
    if bound.negative() {
        return Err("§12.1: negative operator_norm bound".into());
    }
    let mut norm = Interval::exact(0, p);
    for i in 0..d {
        for j in 0..d {
            let mut z = Complex::integer(0, p);
            for k in 0..d {
                z = z.add(&v[k * d + i].conj().mul(&u[k * d + j]));
            }
            if i == j {
                z = z.sub(&Complex::integer(1, p));
            }
            norm = norm.add(&z.re.square().add(&z.im.square()));
        }
    }
    if norm.hi > bound.mul(bound) {
        return Err("§12.1: checker-computed Frobenius upper bound exceeds bound".into());
    }
    Ok(())
}
/// SDP dual check for a checker-computed, unnormalised Choi difference.
/// Input factor first; partial trace is over output. Every test is rigorous.
pub fn channel(
    choi: &[Complex],
    y: &[Complex],
    d: usize,
    p: usize,
    mu: &R,
    bound: &R,
) -> Result<(), String> {
    let n = d.checked_mul(d).ok_or("host limit: Choi dimension")?;
    dimensions(choi, n, p)?;
    dimensions(y, n, p)?;
    if mu.negative() || bound.negative() || mu > bound {
        return Err("§12.1: require 0 <= mu <= bound".into());
    }
    for sign in [false, true] {
        let m = y
            .iter()
            .zip(choi)
            .map(|(a, b)| if sign { a.add(b) } else { a.sub(b) })
            .collect::<Vec<_>>();
        positive_definite(&m, n, p)?;
    }
    let mut trace = vec![Complex::integer(0, p); d * d];
    let mu = Interval::new(mu.clone(), mu.clone(), p)?;
    for i in 0..d {
        for j in 0..d {
            let mut z = Complex::integer(0, p);
            for k in 0..d {
                z = z.add(&y[(i * d + k) * n + j * d + k]);
            }
            let mut base = Complex::integer(0, p);
            if i == j {
                base.re = mu.clone();
            }
            trace[i * d + j] = base.sub(&z);
        }
    }
    positive_definite(&trace, d, p)
}

fn threshold(p: usize) -> Result<R, String> {
    let bits = p.checked_add(2).ok_or("host limit: interval precision")?;
    // Exact binary rational, never a floating-point epsilon.
    let mut d = R::from(1);
    for _ in 0..bits {
        d = d.mul(&R::from(2));
    }
    R::from(1).div(&d)
}
/// Outward π enclosure from Machin's formula. The alternating arctan tail is
/// bounded by its next term; each arithmetic result rounds outward at p bits.
pub fn pi(p: usize) -> Result<Interval, String> {
    Interval::new(R::zero(), R::zero(), p)?;
    fn atan_inverse(q: u64, p: usize) -> Result<Interval, String> {
        let x = R::from(1).div(&R::from(q))?;
        let xx = x.mul(&x);
        let mut power = x;
        let eps = threshold(p)?.div(&R::from(32))?;
        let mut sum = Interval::exact(0, p);
        let mut n = 0u64;
        loop {
            let term = power.div(&R::from(2 * n + 1))?;
            let term = Interval::new(term.clone(), term, p)?;
            sum = if n.is_multiple_of(2) {
                sum.add(&term)
            } else {
                sum.sub(&term)
            };
            power = power.mul(&xx);
            n = n.checked_add(1).ok_or("host limit: pi series")?;
            let next = power.div(&R::from(2 * n + 1))?;
            if next < eps {
                return Ok(sum.add(&Interval::new(R::zero().sub(&next), next, p)?));
            }
        }
    }
    Ok(atan_inverse(5, p)?
        .mul(&Interval::exact(16, p))
        .sub(&atan_inverse(239, p)?.mul(&Interval::exact(4, p))))
}
/// Enclose sin and cos of an exact rational angle by reduced-angle Taylor
/// series. The first omitted nonzero term's Lagrange bound must be <2^-(p+2).
/// Reduction uses a rigorously enclosed π; any chosen integral turn is valid.
pub fn sin_cos(theta: &R, p: usize) -> Result<(Interval, Interval), String> {
    let pi = pi(p)?;
    let tau = pi.mul(&Interval::exact(2, p));
    let midpoint = tau.lo.add(&tau.hi).div(&R::from(2))?;
    let turns = theta
        .div(&midpoint)?
        .sub(&R::from(1).div(&R::from(2))?)
        .ceil();
    let x = Interval::new(theta.clone(), theta.clone(), p)?.sub(&tau.mul(&Interval::new(
        turns.clone(),
        turns,
        p,
    )?));
    let x2 = x.square();
    let radius =
        x.lo.clone()
            .max(R::zero().sub(&x.lo))
            .max(x.hi.clone())
            .max(R::zero().sub(&x.hi));
    let radius2 = radius.mul(&radius);
    let eps = threshold(p)?;
    fn series(
        x: &Interval,
        x2: &Interval,
        radius: &R,
        radius2: &R,
        eps: &R,
        sine: bool,
        p: usize,
    ) -> Result<Interval, String> {
        let mut term = if sine {
            x.clone()
        } else {
            Interval::exact(1, p)
        };
        let mut sum = term.clone();
        let mut remainder = if sine { radius.clone() } else { R::from(1) };
        let mut degree = if sine { 1u64 } else { 0 };
        loop {
            let a = degree.checked_add(1).ok_or("host limit: Taylor degree")?;
            let b = degree.checked_add(2).ok_or("host limit: Taylor degree")?;
            let divisor = R::from(a).mul(&R::from(b));
            remainder = remainder.mul(radius2).div(&divisor)?;
            // The bound uses the first nonzero omitted term: intervening zero
            // coefficients are included in the Taylor polynomial (§17 item 8).
            if remainder < *eps {
                return Ok(sum.add(&Interval::new(R::zero().sub(&remainder), remainder, p)?));
            }
            term = term
                .mul(x2)
                .div(&Interval::new(divisor.clone(), divisor, p)?)?
                .neg();
            sum = sum.add(&term);
            degree = b;
        }
    }
    Ok((
        series(&x, &x2, &radius, &radius2, &eps, true, p)?,
        series(&x, &x2, &radius, &radius2, &eps, false, p)?,
    ))
}
/// §4.4 rx/ry/rz entries from checker-computed sin/cos enclosures of θ/2.
pub fn rotation(axis: &str, theta: &R, p: usize) -> Result<Vec<Complex>, String> {
    let (s, c) = sin_cos(&theta.div(&R::from(2))?, p)?;
    let zero = Interval::exact(0, p);
    let real = |v: Interval| Complex {
        re: v,
        im: zero.clone(),
    };
    let imaginary = |v: Interval| Complex {
        re: zero.clone(),
        im: v,
    };
    let cplx = |a: Interval, b: Interval| Complex { re: a, im: b };
    Ok(match axis {
        "rx" => vec![
            real(c.clone()),
            imaginary(s.neg()),
            imaginary(s.neg()),
            real(c),
        ],
        "ry" => vec![real(c.clone()), real(s.neg()), real(s), real(c)],
        "rz" => vec![
            cplx(c.clone(), s.neg()),
            Complex::integer(0, p),
            Complex::integer(0, p),
            cplx(c, s),
        ],
        _ => return Err("§4.4: rotation must be rx, ry or rz".into()),
    })
}
