//! §12.1 exact hand derivations; these test the interval algebra, not schedule
//! context construction (which must reject unpublished ref@0.8).
use bennett_vm::{
    interval::{self, Complex, Interval},
    json,
    target::Rational as R,
};
fn z(re: &str, im: &str) -> Complex {
    Complex::parse(
        &json::parse(&format!(r#"["{re}","{re}","{im}","{im}"]"#)).unwrap(),
        64,
    )
    .unwrap()
}
#[test]
fn numerical_zero_and_phase_sensitive_rejection() {
    // D-numerical-operator-norm: U=V=X, X†X-I=0, Frobenius bound exactly 0.
    let x = vec![z("0", "0"), z("1", "0"), z("1", "0"), z("0", "0")];
    interval::numerical(&x, &x, 2, 64, &R::zero()).unwrap();
    // -X differs by phase π; all basis values agree but norm distance is 2.
    let minus = vec![z("0", "0"), z("-1", "0"), z("-1", "0"), z("0", "0")];
    assert!(interval::numerical(&minus, &x, 2, 64, &R::zero()).is_err());
}
#[test]
fn channel_strict_feasibility_and_partial_trace() {
    // D-channel: Phi=0, Y=I4/16, Tr_out Y=I2/8, mu=3/16;
    // every pivot of Y±J and mu I-Tr_out Y is 1/16 >0.
    let choi = vec![z("0", "0"); 16];
    let mut y = choi.clone();
    for i in 0..4 {
        y[i * 4 + i] = z("0.0625", "0");
    }
    interval::channel(&choi, &y, 2, 64, &"0.1875".parse().unwrap(), &R::from(1)).unwrap();
    // mu=1/8 leaves zero pivots: even exactly PSD must reject (§12.1).
    assert!(interval::channel(&choi, &y, 2, 64, &"0.125".parse().unwrap(), &R::from(1)).is_err());
    assert!(
        interval::channel(
            &choi,
            &y,
            2,
            64,
            &"0.1875".parse().unwrap(),
            &"0.1".parse().unwrap()
        )
        .is_err()
    );
}
#[test]
fn leading_minor_shortcut_and_inconclusive_pivots_reject() {
    // §12.1 explicitly forbids the diag(0,-1) leading-principal-minor shortcut.
    let m = vec![z("0", "0"), z("0", "0"), z("0", "0"), z("-1", "0")];
    assert!(interval::positive_definite(&m, 2, 64).is_err());
    let m = vec![Complex::parse(&json::parse(r#"["0","1","0","0"]"#).unwrap(), 64).unwrap()];
    assert!(interval::positive_definite(&m, 1, 64).is_err());
    // Hermitian positive matrix with off-diagonal i/4: pivots 1,15/16.
    let m = vec![z("1", "0"), z("0", "0.25"), z("0", "-0.25"), z("1", "0")];
    interval::positive_definite(&m, 2, 64).unwrap();
}
#[test]
fn every_endpoint_rounds_outward_at_declared_precision() {
    // Four significant bits enclose 1/3 between 0.3125 and 0.34375.
    let third = R::from(1).div(&R::from(3)).unwrap();
    let x = Interval::new(third.clone(), third.clone(), 4).unwrap();
    assert_eq!(x.lo, "0.3125".parse().unwrap());
    assert_eq!(x.hi, "0.34375".parse().unwrap());
    let minus = R::zero().sub(&third);
    let x = Interval::new(minus.clone(), minus, 4).unwrap();
    assert_eq!(x.lo, "-0.34375".parse().unwrap());
    assert_eq!(x.hi, "-0.3125".parse().unwrap());
    assert!(Interval::new(R::from(1), R::zero(), 64).is_err());
}
