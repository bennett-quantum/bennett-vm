//! Collapse preserves exact relative amplitudes and the global phase.
use bennett_vm::{action::QuantumState, bigint::BigUint, exact::Exact};
use std::collections::BTreeMap;

fn hadamard(state: &mut QuantumState, wire: usize) {
    let h = Exact::sqrt_half();
    state.apply_exact(wire, &[], [[h.clone(), h.clone()], [h.clone(), h.neg()]]);
}

#[test]
fn measuring_a_spectator_preserves_the_exact_state_and_rotation_inverse() {
    for axis in ["rx", "ry", "rz"] {
        for angle in [0.123, -0.123, 1e-15] {
            let mut state = QuantumState::new();
            let q = state.alloc_wire();
            let spectator = state.alloc_wire();
            if axis == "rz" {
                hadamard(&mut state, q);
            }
            state.apply_exact(q, &[], Exact::rotation(axis, angle));
            let before = state.clone();
            assert_eq!(state.measure(&[spectator], 0.5).0, BigUint::zero());
            assert!(
                state.exact_equals(&before),
                "{axis}({angle}) changed on an unrelated measurement"
            );
            state.apply_exact(q, &[], Exact::rotation(axis, -angle));
            if axis == "rz" {
                hadamard(&mut state, q);
            }
            assert!(
                state.is_clean(&[q, spectator]),
                "{axis}({angle}) did not cancel"
            );
            assert!(state.exact_equals(&QuantumState::new()));
        }
    }
}

#[test]
fn probabilistic_collapse_preserves_exact_cancellation_and_global_phase() {
    for negative in [false, true] {
        for draw in [0.1, 0.9] {
            let mut state = QuantumState::new();
            let q = state.alloc_wire();
            let measured = state.alloc_wire();
            state.apply_exact(q, &[], Exact::rotation("ry", 0.123));
            hadamard(&mut state, measured);
            if negative {
                state.phase_if_all(&[measured], (-1.0, 0.0));
            }
            let outcome = state.measure(&[measured], draw).0;
            state.apply_exact(q, &[], Exact::rotation("ry", -0.123));
            assert!(state.is_clean(&[q, measured]));
            let mut expected = QuantumState::new();
            if negative && outcome == BigUint::from_u64(1) {
                expected.phase_if_all(&[], (-1.0, 0.0));
                assert!(
                    !state.exact_equals(&QuantumState::new()),
                    "global phase was discarded"
                );
            }
            assert!(state.exact_equals(&expected));
        }
    }
}

#[test]
fn postselection_matches_the_same_coherent_state_prepared_directly() {
    let mut measured = QuantumState::new();
    measured.alloc_wires(2);
    hadamard(&mut measured, 0);
    hadamard(&mut measured, 1);
    assert_eq!(measured.measure(&[0], 0.0).0, BigUint::zero());
    let mut direct = QuantumState::new();
    direct.alloc_wires(2);
    hadamard(&mut direct, 1);
    assert!(measured.exact_equals(&direct));
    assert!(direct.exact_equals(&measured));
}

#[test]
fn postselection_normalizes_tiny_and_cancelling_exact_weights() {
    let tiny = f64::from_bits(1);
    let mut state = QuantumState::new();
    state.alloc_wire();
    state.set_state(BTreeMap::from([
        (BigUint::zero(), (tiny, 0.0)),
        (BigUint::from_u64(1), (1.0, 0.0)),
    ]));
    assert_eq!(state.measure(&[0], 0.0).0, BigUint::zero());
    assert!(state.exact_equals(&QuantumState::new()));
    assert_eq!(state.amps[&BigUint::zero()], (1.0, 0.0));

    // This positive Pell residual is exact but rounds to zero when evaluated
    // as binary64. Selecting it still leaves the correct normalized phase.
    let residual =
        Exact::pair(131836323.0, 0.0).add(&Exact::sqrt_half().mul(&Exact::pair(-186444716.0, 0.0)));
    assert_eq!(residual.value(), (0.0, 0.0));
    for negative in [false, true] {
        let mut state = QuantumState::new();
        state.alloc_wire();
        let amplitude = if negative {
            residual.neg()
        } else {
            residual.clone()
        };
        let complex = amplitude.mul(&Exact::pair(1.0, 1.0));
        state.apply_exact(
            0,
            &[],
            [[complex, Exact::zero()], [Exact::one(), Exact::zero()]],
        );
        assert_eq!(state.measure(&[0], 0.0).0, BigUint::zero());
        let mut expected = QuantumState::new();
        let phase = Exact::sqrt_half().mul(&Exact::pair(
            if negative { -1.0 } else { 1.0 },
            if negative { -1.0 } else { 1.0 },
        ));
        expected.apply_exact(
            0,
            &[],
            [[phase, Exact::zero()], [Exact::zero(), Exact::one()]],
        );
        assert!(state.exact_equals(&expected));
        let (re, im) = state.amps[&BigUint::zero()];
        assert!((re.abs() - std::f64::consts::FRAC_1_SQRT_2).abs() < 2e-15);
        assert!((im.abs() - std::f64::consts::FRAC_1_SQRT_2).abs() < 2e-15);
        assert_eq!(re.is_sign_negative(), negative);
        assert_eq!(im.is_sign_negative(), negative);
    }
}

#[test]
fn repeated_postselection_keeps_a_finite_normalized_display() {
    let mut state = QuantumState::new();
    let wire = state.alloc_wire();
    for step in 0..2500 {
        hadamard(&mut state, wire);
        state.measure(&[wire], if step % 2 == 0 { 0.1 } else { 0.9 });
        assert!(state.exact_equals(&QuantumState::new()));
        assert_eq!(state.amps[&BigUint::zero()], (1.0, 0.0));
    }
}
