//! The numeric oracle is (|2> + i|3>)(|0> - |1>)/2, with a constant image.
use crate::json::{self, Json};
use std::{
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    dir: PathBuf,
    object: PathBuf,
    state: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "state-out-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let object = dir.join("readout.bisa");
        std::fs::write(&object, include_str!("fixtures/state_out09/readout.bisa")).unwrap();
        Self {
            state: dir.join("state with spaces.json"),
            dir,
            object,
        }
    }
    fn run(&self, command: &str, engine: &str, entry: &str, args: &[&str]) -> Output {
        let binary = std::env::var_os("BENNETT_VM_TEST_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("bennett-vm")
            });
        Command::new(binary)
            .args([command, self.object.to_str().unwrap(), entry])
            .args(args)
            .env("BENNETT_VM_ENGINE", engine)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env_remove("BROWSER")
            .output()
            .expect("build the VM binary before running CLI readout tests")
    }
    fn read(&self) -> Json {
        json::parse(&std::fs::read_to_string(&self.state).expect("pre-measurement state file"))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn success(out: Output) -> Json {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    json::parse(std::str::from_utf8(&out.stdout).unwrap()).unwrap()
}
fn number(value: &Json) -> f64 {
    match value {
        Json::Num(text) => text.parse().unwrap(),
        _ => panic!("expected JSON number: {value:?}"),
    }
}
fn check_state(state: &Json) {
    assert_eq!(state.get("engine").and_then(Json::as_str), Some("exact"));
    let registers = state.get("registers").unwrap().as_arr().unwrap();
    assert_eq!(registers.len(), 4);
    for (index, width) in [2, 1, 68, 1].into_iter().enumerate() {
        assert_eq!(
            registers[index].get("reg").and_then(Json::as_usize),
            Some(index)
        );
        assert_eq!(
            registers[index].get("width").and_then(Json::as_u64),
            Some(width)
        );
    }
    // Stable wire 0 is the least significant bit; image hex is little-endian bytes.
    let constant = state.get("constant").unwrap().as_obj().unwrap();
    assert_eq!(constant.len(), 1);
    assert_eq!(constant["2"].as_str(), Some("010000000000000008"));
    let amps = state.get("amps").unwrap().as_arr().unwrap();
    assert_eq!(
        amps.len(),
        4,
        "both measurement outcomes and the spectator survive"
    );
    for (q, t, re, im) in [
        ("2", "0", 0.5, 0.0),
        ("2", "1", -0.5, 0.0),
        ("3", "0", 0.0, 0.5),
        ("3", "1", 0.0, -0.5),
    ] {
        let rows: Vec<_> = amps
            .iter()
            .filter(|row| {
                let basis = row.get("basis").unwrap();
                basis.get("0").and_then(Json::as_str) == Some(q)
                    && basis.get("1").and_then(Json::as_str) == Some(t)
            })
            .collect();
        assert_eq!(rows.len(), 1, "unique basis ({q},{t})");
        let basis = rows[0].get("basis").unwrap().as_obj().unwrap();
        assert_eq!(basis.len(), 3);
        assert_eq!(basis["3"].as_str(), Some("0"));
        assert_eq!(
            decode_scalar(rows[0].get("re").unwrap(), &crate::exact::Exact::one()),
            crate::exact::Exact::pair(re, 0.0)
        );
        assert_eq!(
            decode_scalar(rows[0].get("im").unwrap(), &crate::exact::Exact::one()),
            crate::exact::Exact::pair(im, 0.0)
        );
    }
}

#[test]
fn first_executed_measurement_exports_the_uncollapsed_exact_state_once() {
    for command in ["run-cc", "run-program"] {
        for engine in ["interpreter", "jit"] {
            for entry in ["sample", "skipped"] {
                let fixture = Fixture::new();
                let mut snapshots = Vec::new();
                for (draw, expected) in [("0", "2"), ("0.9", "3")] {
                    let path = fixture.state.to_str().unwrap();
                    let out = success(fixture.run(
                        command,
                        engine,
                        entry,
                        &["--state-out", path, "--draws", draw],
                    ));
                    assert_eq!(out.get("status").and_then(Json::as_str), Some("COMPLETED"));
                    assert_eq!(
                        out.get("results").unwrap().as_arr().unwrap()[0]
                            .get("value")
                            .and_then(Json::as_str),
                        Some(expected)
                    );
                    let snapshot = fixture.read();
                    check_state(&snapshot);
                    snapshots.push(snapshot);
                }
                assert_eq!(
                    snapshots[0], snapshots[1],
                    "readout precedes draw-dependent collapse"
                );
            }
        }
    }
}

#[test]
fn a_launch_without_measurement_leaves_the_destination_untouched() {
    for command in ["run-cc", "run-program"] {
        let fixture = Fixture::new();
        let path = fixture.state.to_str().unwrap();
        success(fixture.run(command, "interpreter", "unmeasured", &["--state-out", path]));
        assert!(!fixture.state.exists());
        std::fs::write(&fixture.state, "keep existing output").unwrap();
        success(fixture.run(command, "interpreter", "unmeasured", &["--state-out", path]));
        assert_eq!(
            std::fs::read_to_string(&fixture.state).unwrap(),
            "keep existing output"
        );
    }
}

#[test]
fn unwritable_state_destination_is_a_loud_command_failure() {
    let fixture = Fixture::new();
    for command in ["run-cc", "run-program"] {
        for path in [
            fixture.dir.clone(),
            fixture.dir.join("missing-parent/state.json"),
        ] {
            let out = fixture.run(
                command,
                "interpreter",
                "sample",
                &["--state-out", path.to_str().unwrap()],
            );
            assert!(!out.status.success());
            assert!(out.stdout.is_empty());
            let error = String::from_utf8_lossy(&out.stderr);
            assert!(error.contains("--state-out"), "{error}");
            assert!(error.contains(path.to_str().unwrap()), "{error}");
        }
    }
}

#[test]
fn duplicate_and_missing_state_options_fail_before_writing() {
    let fixture = Fixture::new();
    for command in ["run-cc", "run-program"] {
        let path = fixture.state.to_str().unwrap();
        let out = fixture.run(
            command,
            "interpreter",
            "sample",
            &["--state-out", path, "--state-out", path],
        );
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("option --state-out given more than once")
        );
        assert!(!fixture.state.exists());
        let out = fixture.run(command, "interpreter", "sample", &["--state-out"]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("missing value for --state-out"));
    }
}

#[test]
fn register_views_do_not_renumber_or_duplicate_allocations() {
    let fixture = Fixture::new();
    success(fixture.run(
        "run-program",
        "interpreter",
        "views",
        &["--state-out", fixture.state.to_str().unwrap()],
    ));
    let state = fixture.read();
    let registers = state.get("registers").unwrap().as_arr().unwrap();
    assert_eq!(registers.len(), 2);
    for (index, register) in registers.iter().enumerate() {
        assert_eq!(register.get("reg").and_then(Json::as_usize), Some(index));
        assert_eq!(register.get("width").and_then(Json::as_u64), Some(1));
    }
    assert_eq!(state.get("amps").unwrap().as_arr().unwrap().len(), 1);
}

struct AlternateEngine(Vec<(crate::bigint::BigUint, crate::action::Amp)>);
impl crate::action::QuantumReadout for AlternateEngine {
    type Amplitude = crate::action::Amp;
    fn visit_amplitudes(&self, visit: &mut dyn FnMut(&crate::bigint::BigUint, crate::action::Amp)) {
        for (basis, amplitude) in &self.0 {
            visit(basis, *amplitude);
        }
    }
}
fn output(fixture: &Fixture) -> crate::action::StateOutput {
    let opts = crate::cliargs::scan_opts(
        &["--state-out".into(), fixture.state.to_str().unwrap().into()],
        &["state-out"],
        &[],
    )
    .unwrap();
    crate::cliargs::state_output(&opts).unwrap()
}

#[test]
fn the_same_hook_accepts_another_engine_and_captures_once_across_clones() {
    let fixture = Fixture::new();
    let state_output = output(&fixture);
    let checkpoint = state_output.clone();
    let engine = AlternateEngine(vec![(crate::bigint::BigUint::from_u64(1), (0.0, -1.0))]);
    let wires = [0];
    state_output
        .capture(&engine, std::iter::once(wires.as_slice()))
        .unwrap();
    let state = fixture.read();
    assert_eq!(state.get("engine").and_then(Json::as_str), Some("dense"));
    let row = &state.get("amps").unwrap().as_arr().unwrap()[0];
    assert_eq!(
        row.get("basis").unwrap().get("0").and_then(Json::as_str),
        Some("1")
    );
    assert_eq!(number(row.get("re").unwrap()), 0.0);
    assert_eq!(number(row.get("im").unwrap()), -1.0);
    std::fs::write(&fixture.state, "already captured").unwrap();
    checkpoint
        .capture(&engine, std::iter::once(wires.as_slice()))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&fixture.state).unwrap(),
        "already captured"
    );
}

#[test]
fn exact_readout_uses_the_exact_map_without_mutating_it() {
    let fixture = Fixture::new();
    let mut engine = crate::action::QuantumState::new();
    let wires = engine.alloc_wires(1);
    engine.prepare(&wires, &crate::bigint::BigUint::from_u64(1));
    let before = engine.clone();
    // A deliberately stale binary64 cache cannot become the readout oracle.
    engine.amps.clear();
    output(&fixture)
        .capture(&engine, std::iter::once(wires.as_slice()))
        .unwrap();
    let state = fixture.read();
    assert_eq!(state.get("engine").and_then(Json::as_str), Some("exact"));
    let rows = state.get("amps").unwrap().as_arr().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]
            .get("basis")
            .unwrap()
            .get("0")
            .and_then(Json::as_str),
        Some("1")
    );
    assert_eq!(
        decode_scalar(rows[0].get("re").unwrap(), &crate::exact::Exact::one()),
        crate::exact::Exact::one()
    );
    assert!(engine.exact_equals(&before));
    assert!(engine.amps.is_empty());
}

#[test]
fn malformed_engine_readout_fails_before_writing_or_consuming_the_hook() {
    let fixture = Fixture::new();
    let state_output = output(&fixture);
    let wires = [0];
    let empty = AlternateEngine(vec![]);
    assert!(
        state_output
            .capture(&empty, std::iter::once(wires.as_slice()))
            .unwrap_err()
            .contains("empty quantum state")
    );
    for component in [f64::NAN, f64::INFINITY] {
        let invalid = AlternateEngine(vec![(crate::bigint::BigUint::zero(), (component, 0.0))]);
        assert!(
            state_output
                .capture(&invalid, std::iter::once(wires.as_slice()))
                .unwrap_err()
                .contains("non-finite amplitude")
        );
        assert!(!fixture.state.exists());
    }
    let valid = AlternateEngine(vec![(crate::bigint::BigUint::zero(), (1.0, 0.0))]);
    state_output
        .capture(&valid, std::iter::once(wires.as_slice()))
        .unwrap();
    assert_eq!(
        fixture.read().get("amps").unwrap().as_arr().unwrap().len(),
        1
    );
}

#[test]
fn irrational_readout_retains_the_exact_radical_and_its_phase() {
    let zero = json::parse(r#"{"numerator":[],"normalization":[{"coefficient":"1","sqrt2":false,"imaginary":false,"phase":"0"}]}"#).unwrap();
    let half_root = json::parse(r#"{"numerator":[{"coefficient":"1/2","sqrt2":true,"imaginary":false,"phase":"0"}],"normalization":[{"coefficient":"1","sqrt2":false,"imaginary":false,"phase":"0"}]}"#).unwrap();
    for (source, phase) in [
        (include_str!("fixtures/state_out09/radical.bisa"), false),
        (
            include_str!("fixtures/state_out09/radical-phase.bisa"),
            true,
        ),
    ] {
        for command in ["run-cc", "run-program"] {
            for engine in ["interpreter", "jit"] {
                let fixture = Fixture::new();
                std::fs::write(&fixture.object, source).unwrap();
                success(fixture.run(
                    command,
                    engine,
                    "sample",
                    &["--state-out", fixture.state.to_str().unwrap()],
                ));
                let state = fixture.read();
                assert_eq!(state.get("engine").and_then(Json::as_str), Some("exact"));
                let rows = state.get("amps").unwrap().as_arr().unwrap();
                assert_eq!(rows.len(), 2);
                for (basis, row) in rows.iter().enumerate() {
                    assert_eq!(
                        row.get("basis").unwrap().get("0").and_then(Json::as_str),
                        Some(basis.to_string().as_str())
                    );
                    assert_eq!(
                        row.get("re"),
                        Some(if phase && basis == 1 {
                            &zero
                        } else {
                            &half_root
                        })
                    );
                    assert_eq!(
                        row.get("im"),
                        Some(if phase && basis == 1 {
                            &half_root
                        } else {
                            &zero
                        })
                    );
                }
            }
        }
    }
}

#[test]
fn zero_width_allocations_retain_their_indices_before_nonempty_registers() {
    for command in ["run-cc", "run-program"] {
        for engine in ["interpreter", "jit"] {
            let fixture = Fixture::new();
            std::fs::write(
                &fixture.object,
                include_str!("fixtures/state_out09/zero-width.bisa"),
            )
            .unwrap();
            let result = success(fixture.run(
                command,
                engine,
                "sample",
                &["--state-out", fixture.state.to_str().unwrap()],
            ));
            assert_eq!(
                result.get("status").and_then(Json::as_str),
                Some("COMPLETED")
            );
            let state = fixture.read();
            let registers = state.get("registers").unwrap().as_arr().unwrap();
            assert_eq!(registers.len(), 3);
            for (reg, width) in [0, 0, 1].into_iter().enumerate() {
                assert_eq!(
                    registers[reg].get("reg").and_then(Json::as_usize),
                    Some(reg)
                );
                assert_eq!(
                    registers[reg].get("width").and_then(Json::as_u64),
                    Some(width)
                );
            }
            let row = &state.get("amps").unwrap().as_arr().unwrap()[0];
            for reg in ["0", "1", "2"] {
                assert_eq!(
                    row.get("basis").unwrap().get(reg).and_then(Json::as_str),
                    Some("0")
                );
            }
        }
    }
}

// Decode the proposed rational/radical format independently through Exact's
// arithmetic API. No f64 evaluation or decimal approximation enters this oracle.
fn decode_dyadic(text: &str) -> (bool, crate::bigint::BigUint, usize) {
    let (numerator, denominator) = text.split_once('/').unwrap_or((text, "1"));
    let (negative, numerator) = numerator
        .strip_prefix('-')
        .map_or((false, numerator), |n| (true, n));
    let magnitude = crate::bigint::BigUint::parse_decimal(numerator).unwrap();
    let denominator = crate::bigint::BigUint::parse_decimal(denominator).unwrap();
    assert!(!denominator.is_zero());
    let scale = denominator.bits() - 1;
    assert_eq!(denominator, crate::bigint::BigUint::from_u64(1).shl(scale));
    (negative, magnitude, scale)
}
fn decode_terms(value: &Json) -> crate::exact::Exact {
    use crate::exact::{Dyad, Exact};
    value
        .as_arr()
        .expect("exact term array")
        .iter()
        .fold(Exact::zero(), |sum, term| {
            let (negative, magnitude, scale) =
                decode_dyadic(term.get("coefficient").unwrap().as_str().unwrap());
            let mut coefficient = (0..magnitude.bits())
                .filter(|bit| magnitude.bit(*bit))
                .fold(Exact::zero(), |sum, bit| {
                    sum.add(&Exact::one().scale_pow2(bit as i128))
                });
            coefficient = coefficient.scale_pow2(-(scale as i128));
            if negative {
                coefficient = coefficient.neg();
            }
            if term.get("sqrt2") == Some(&Json::Bool(true)) {
                coefficient = coefficient.mul(&Exact::sqrt_half().scale_pow2(1));
            }
            if term.get("imaginary") == Some(&Json::Bool(true)) {
                coefficient = coefficient.mul(&Exact::pair(0.0, 1.0));
            }
            let (negative, magnitude, scale) =
                decode_dyadic(term.get("phase").unwrap().as_str().unwrap());
            let mut angle = (0..magnitude.bits())
                .rev()
                .fold(Dyad::int(0), |angle, bit| {
                    let doubled = angle.add(&angle);
                    if magnitude.bit(bit) {
                        doubled.add(&Dyad::int(1))
                    } else {
                        doubled
                    }
                });
            for _ in 0..scale {
                angle = angle.half();
            }
            if negative {
                angle = angle.neg();
            }
            sum.add(&coefficient.mul(&Exact::phase(angle)))
        })
}
fn decode_scalar(value: &Json, normalization: &crate::exact::Exact) -> crate::exact::Exact {
    assert!(
        value.as_obj().is_some(),
        "exact scalar object required: {value:?}"
    );
    assert_eq!(
        &decode_terms(value.get("normalization").unwrap()),
        normalization
    );
    decode_terms(value.get("numerator").unwrap())
}
fn assert_roundtrip(state: &Json, engine: &crate::action::QuantumState) {
    use crate::{bigint::BigUint, exact::Exact};
    let constants = state.get("constant").unwrap().as_obj().unwrap();
    let mut decoded = std::collections::BTreeMap::new();
    for row in state.get("amps").unwrap().as_arr().unwrap() {
        let mut basis = BigUint::zero();
        let mut offset = 0;
        for register in state.get("registers").unwrap().as_arr().unwrap() {
            let reg = register.get("reg").unwrap().as_usize().unwrap().to_string();
            let width = register.get("width").unwrap().as_usize().unwrap();
            let value = match constants.get(&reg) {
                Some(hex) => {
                    let bytes = crate::sha256::from_hex(hex.as_str().unwrap()).unwrap();
                    bytes
                        .iter()
                        .enumerate()
                        .fold(BigUint::zero(), |mut value, (byte, v)| {
                            for bit in 0..8 {
                                value.set_bit(byte * 8 + bit, v & (1 << bit) != 0);
                            }
                            value
                        })
                }
                None => BigUint::parse_decimal(
                    row.get("basis")
                        .unwrap()
                        .get(&reg)
                        .unwrap()
                        .as_str()
                        .unwrap(),
                )
                .unwrap(),
            };
            for bit in 0..width {
                basis.set_bit(offset + bit, value.bit(bit));
            }
            offset += width;
        }
        let re = decode_scalar(row.get("re").unwrap(), &engine.normalization);
        let im = decode_scalar(row.get("im").unwrap(), &engine.normalization);
        assert!(
            decoded
                .insert(basis, re.add(&im.mul(&Exact::pair(0.0, 1.0))))
                .is_none()
        );
    }
    assert_eq!(
        decoded, engine.exact,
        "decoded exact numerators equal the engine state"
    );
}
fn hadamard(engine: &mut crate::action::QuantumState, wire: usize) {
    let h = crate::exact::Exact::sqrt_half();
    engine.apply_exact(wire, &[], [[h.clone(), h.clone()], [h.clone(), h.neg()]]);
}

#[test]
fn radical_phase_and_multiple_registers_roundtrip_to_the_engine_exact_state() {
    use crate::{action::QuantumState, bigint::BigUint};
    for (source, phase) in [
        (include_str!("fixtures/state_out09/radical.bisa"), false),
        (
            include_str!("fixtures/state_out09/radical-phase.bisa"),
            true,
        ),
    ] {
        let fixture = Fixture::new();
        std::fs::write(&fixture.object, source).unwrap();
        success(fixture.run(
            "run-program",
            "interpreter",
            "sample",
            &["--state-out", fixture.state.to_str().unwrap()],
        ));
        let mut engine = QuantumState::new();
        engine.alloc_wires(1);
        hadamard(&mut engine, 0);
        if phase {
            engine.phase_if_all(&[0], (0.0, 1.0));
        }
        assert_roundtrip(&fixture.read(), &engine);
    }
    let fixture = Fixture::new();
    success(fixture.run(
        "run-program",
        "interpreter",
        "sample",
        &["--state-out", fixture.state.to_str().unwrap()],
    ));
    let mut engine = QuantumState::new();
    let q = engine.alloc_wires(2);
    let target = engine.alloc_wire();
    let image = engine.alloc_wires(68);
    engine.alloc_wire();
    engine.flip(q[1], &[]);
    hadamard(&mut engine, q[0]);
    engine.phase_if_all(&[q[0]], (0.0, 1.0));
    engine.flip(target, &[]);
    hadamard(&mut engine, target);
    let mut value = BigUint::from_u64(1);
    value.set_bit(67, true);
    engine.prepare(&image, &value);
    assert_roundtrip(&fixture.read(), &engine);
}

#[test]
fn zero_width_concatenations_do_not_shift_later_allocations() {
    let fixture = Fixture::new();
    std::fs::write(
        &fixture.object,
        include_str!("fixtures/state_out09/zero-width-concat.bisa"),
    )
    .unwrap();
    success(fixture.run(
        "run-program",
        "interpreter",
        "sample",
        &["--state-out", fixture.state.to_str().unwrap()],
    ));
    let state = fixture.read();
    let registers = state.get("registers").unwrap().as_arr().unwrap();
    assert_eq!(
        registers.len(),
        3,
        "concat is a view of two empty allocations"
    );
    assert_eq!(registers[2].get("width").and_then(Json::as_u64), Some(1));
}
