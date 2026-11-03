//! Typed measurement has the same irreversible boundary as plain measurement.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::json::{self, Json};
use std::{
    path::Path,
    process::{Command, Output},
};

fn run(path: &Path, engine: &str, command: &str, entry: &str, draw: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
        .args([command, path.to_str().unwrap(), entry, "--draws", draw])
        .env("BENNETT_VM_ENGINE", engine)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("BROWSER")
        .output()
        .unwrap()
}

fn check(engine: &str, uncall: bool) {
    for width in [1, 2] {
        check_width(engine, uncall, width);
    }
}

fn check_width(engine: &str, uncall: bool, width: usize) {
    let gates = if width == 1 {
        "h q[0]"
    } else {
        "h q[0]\ncx q[0] q[1]"
    };
    let text = support::source(&format!(
        "action @mix (q:wire[{width}] mut) () () () ()\n^entry():\n{gates}\nfn cc @sample () (r:i{width}) () () ()\n^entry():\nq = qreg.new any {width}\naction_call @mix (q) () () ()\nr = measure.i{width} q\nret r\nfn cc @bad_uncall () () () () ()\n^entry():\nr = call @sample\nuncall @sample () (r)\nret",
    ));
    let path = std::env::temp_dir().join(format!(
        "measure09-{engine}-{uncall}-{width}-{}.bisa",
        std::process::id()
    ));
    std::fs::write(&path, text).unwrap();
    for (draw, expected) in [("0.1", "0"), ("0.9", if width == 1 { "1" } else { "3" })] {
        let out = run(&path, engine, "run-cc", "sample", draw);
        assert!(
            out.status.success(),
            "{engine}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let result = json::parse(std::str::from_utf8(&out.stdout).unwrap().trim()).unwrap();
        assert_eq!(
            result.get("results").unwrap().as_arr().unwrap()[0].get("value"),
            Some(&Json::Str(expected.into()))
        );
    }
    if !uncall {
        let out = run(&path, engine, "reverse", "sample", "0.1");
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("E_NOT_REVERSIBLE"),
            "{engine}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    } else {
        let out = run(&path, engine, "run-cc", "bad_uncall", "0.1");
        assert!(
            out.status.success(),
            "{engine}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let result = json::parse(std::str::from_utf8(&out.stdout).unwrap().trim()).unwrap();
        assert_eq!(result.get("status").and_then(Json::as_str), Some("FAILED"));
        assert_eq!(result.get("fault").and_then(Json::as_u64), Some(4));
        assert_eq!(
            result.get("measurements").unwrap().as_arr().unwrap().len(),
            1
        );
        assert_eq!(result.get("faults").unwrap().as_arr().unwrap().len(), 1);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn typed_measurement_is_irreversible_interpreter() {
    check("interpreter", false);
}
#[test]
fn typed_measurement_is_irreversible_jit() {
    check("jit", false);
}
#[test]
fn typed_measurement_uncall_traps_interpreter() {
    check("interpreter", true);
}
#[test]
fn typed_measurement_uncall_traps_jit() {
    check("jit", true);
}

#[test]
fn finite_pi_rotation_keeps_a_positive_first_outcome_on_both_engines() {
    for version in ["0.8", "0.9"] {
        for angle in [
            std::f64::consts::PI,
            std::f64::consts::PI.next_up(),
            std::f64::consts::PI.next_down(),
        ] {
            let text = support::source_at(
                version,
                &format!(
                    "action @rotate (q:wire mut) () () () ()\n^entry():\nry {angle} q\nfn cc @sample () (r:i1) () () ()\n^entry():\nq = qreg.new any 1\naction_call @rotate (q) () () ()\nr = measure q\nret r"
                ),
            );
            let path = std::env::temp_dir().join(format!(
                "measure-phase-{version}-{}-{}.bisa",
                angle.to_bits(),
                std::process::id()
            ));
            std::fs::write(&path, text).unwrap();
            for engine in ["interpreter", "jit"] {
                for command in ["run-cc", "run-program"] {
                    // Binary64 PI is dyadic, so cos(PI/2)^2 is positive.
                    // Its neighbours also have weights between 1e-40 and 1e-30.
                    for (draw, expected) in [("0", "0"), ("1e-40", "0"), ("1e-30", "1")] {
                        let out = run(&path, engine, command, "sample", draw);
                        assert!(
                            out.status.success(),
                            "{engine} {command}: {}",
                            String::from_utf8_lossy(&out.stderr)
                        );
                        let result =
                            json::parse(std::str::from_utf8(&out.stdout).unwrap()).unwrap();
                        assert_eq!(
                            result.get("status").and_then(Json::as_str),
                            Some("COMPLETED")
                        );
                        assert_eq!(
                            result.get("results").unwrap().as_arr().unwrap()[0]
                                .get("value")
                                .and_then(Json::as_str),
                            Some(expected),
                            "{version} {engine} {command} {angle} draw={draw}"
                        );
                    }
                }
            }
            std::fs::remove_file(path).unwrap();
        }
    }
}
