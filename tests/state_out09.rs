//! The numeric oracle is (|2> + i|3>)(|0> - |1>)/2, with a constant image.
use crate::json::{self, Json};
use std::{path::PathBuf, process::{Command, Output}, sync::atomic::{AtomicUsize, Ordering}};

mod source {
    use crate as bennett_vm;
    include!("support/source09.rs");
}

struct Fixture { dir: PathBuf, object: PathBuf, state: PathBuf }
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("state-out-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let object = dir.join("readout.bisa");
        std::fs::write(&object, source::source(include_str!("fixtures/state_out09/readout.functions").trim())).unwrap();
        Self { state: dir.join("state with spaces.json"), dir, object }
    }
    fn run(&self, command: &str, engine: &str, entry: &str, args: &[&str]) -> Output {
        let binary = std::env::var_os("BENNETT_VM_TEST_BIN").map(PathBuf::from).unwrap_or_else(|| {
            std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("bennett-vm")
        });
        Command::new(binary).args([command, self.object.to_str().unwrap(), entry]).args(args)
            .env("BENNETT_VM_ENGINE", engine)
            .env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY")
            .env_remove("DBUS_SESSION_BUS_ADDRESS").env_remove("BROWSER")
            .output().expect("build the VM binary before running CLI readout tests")
    }
    fn read(&self) -> Json {
        json::parse(&std::fs::read_to_string(&self.state).expect("pre-measurement state file")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { std::fs::remove_dir_all(&self.dir).unwrap(); }
}
fn success(out: Output) -> Json {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    json::parse(std::str::from_utf8(&out.stdout).unwrap()).unwrap()
}
fn number(value: &Json) -> f64 {
    match value { Json::Num(text) => text.parse().unwrap(), _ => panic!("expected JSON number: {value:?}") }
}
fn check_state(state: &Json) {
    assert_eq!(state.get("engine").and_then(Json::as_str), Some("exact"));
    let registers = state.get("registers").unwrap().as_arr().unwrap();
    assert_eq!(registers.len(), 4);
    for (index, width) in [2, 1, 68, 1].into_iter().enumerate() {
        assert_eq!(registers[index].get("reg").and_then(Json::as_usize), Some(index));
        assert_eq!(registers[index].get("width").and_then(Json::as_u64), Some(width));
    }
    // Stable wire 0 is the least significant bit; image hex is little-endian bytes.
    let constant = state.get("constant").unwrap().as_obj().unwrap();
    assert_eq!(constant.len(), 1);
    assert_eq!(constant["2"].as_str(), Some("010000000000000008"));
    let amps = state.get("amps").unwrap().as_arr().unwrap();
    assert_eq!(amps.len(), 4, "both measurement outcomes and the spectator survive");
    for (q, t, re, im) in [("2", "0", 0.5, 0.0), ("2", "1", -0.5, 0.0), ("3", "0", 0.0, 0.5), ("3", "1", 0.0, -0.5)] {
        let rows: Vec<_> = amps.iter().filter(|row| {
            let basis = row.get("basis").unwrap();
            basis.get("0").and_then(Json::as_str) == Some(q) && basis.get("1").and_then(Json::as_str) == Some(t)
        }).collect();
        assert_eq!(rows.len(), 1, "unique basis ({q},{t})");
        let basis = rows[0].get("basis").unwrap().as_obj().unwrap();
        assert_eq!(basis.len(), 3);
        assert_eq!(basis["3"].as_str(), Some("0"));
        assert!((number(rows[0].get("re").unwrap()) - re).abs() < 3e-16);
        assert!((number(rows[0].get("im").unwrap()) - im).abs() < 3e-16);
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
                    let out = success(fixture.run(command, engine, entry, &["--state-out", path, "--draws", draw]));
                    assert_eq!(out.get("status").and_then(Json::as_str), Some("COMPLETED"));
                    assert_eq!(out.get("results").unwrap().as_arr().unwrap()[0].get("value").and_then(Json::as_str), Some(expected));
                    let snapshot = fixture.read();
                    check_state(&snapshot);
                    snapshots.push(snapshot);
                }
                assert_eq!(snapshots[0], snapshots[1], "readout precedes draw-dependent collapse");
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
        assert_eq!(std::fs::read_to_string(&fixture.state).unwrap(), "keep existing output");
    }
}

#[test]
fn unwritable_state_destination_is_a_loud_command_failure() {
    let fixture = Fixture::new();
    for command in ["run-cc", "run-program"] {
        for path in [fixture.dir.clone(), fixture.dir.join("missing-parent/state.json")] {
            let out = fixture.run(command, "interpreter", "sample", &["--state-out", path.to_str().unwrap()]);
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
        let out = fixture.run(command, "interpreter", "sample", &["--state-out", path, "--state-out", path]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("option --state-out given more than once"));
        assert!(!fixture.state.exists());
        let out = fixture.run(command, "interpreter", "sample", &["--state-out"]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("missing value for --state-out"));
    }
}

#[test]
fn register_views_do_not_renumber_or_duplicate_allocations() {
    let fixture = Fixture::new();
    success(fixture.run("run-program", "interpreter", "views", &["--state-out", fixture.state.to_str().unwrap()]));
    let state = fixture.read();
    let registers = state.get("registers").unwrap().as_arr().unwrap();
    assert_eq!(registers.len(), 2);
    for (index, register) in registers.iter().enumerate() {
        assert_eq!(register.get("reg").and_then(Json::as_usize), Some(index));
        assert_eq!(register.get("width").and_then(Json::as_u64), Some(1));
    }
    assert_eq!(state.get("amps").unwrap().as_arr().unwrap().len(), 1);
}
