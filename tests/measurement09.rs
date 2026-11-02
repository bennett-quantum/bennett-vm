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
