//! Run the complete published harness unchanged; keep its raw red verdict.
use std::{path::Path, process::Command};
#[test]
fn published_conformance_report() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let harness = repo
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/target/debug/bennett-isa-conformance");
    let output = Command::new(&harness)
        .args(["--bin", env!("CARGO_BIN_EXE_bennett-vm")])
        .env("TMPDIR", repo.join(".tmp"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("BROWSER")
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "{}: {e}; build the published harness separately",
                harness.display()
            )
        });
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::create_dir_all(repo.join(".tmp")).unwrap();
    std::fs::write(repo.join(".tmp/conformance.log"), &report).unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "Full milestone-3+ suite is expected to remain RED; audit improved/staged cases before changing this assertion.\n{report}"
    );
    assert!(
        report.contains("| all |") && report.contains("490 |"),
        "Must run all 490 published cases, without filtering: {report}"
    );
    // The full report remains red; milestone 2's served G and E probes must pass.
    for line in report.lines().filter(|l| l.starts_with("FAIL ")) {
        assert!(
            !line.starts_with("FAIL G-")
                && !line.starts_with("FAIL E-")
                && !line.starts_with("FAIL H-"),
            "Launch/staged refusal regression: {line}"
        );
    }
    println!(
        "Full conformance verdict remains RED (exit 1); report: {}",
        repo.join(".tmp/conformance.log").display()
    );
}
#[test]
fn metadata_unknown_fields_are_load_errors() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/cases/objects");
    for name in ["f_cert_unknown_field.bisa", "f_sched_unknown_field.bisa"] {
        let text = std::fs::read_to_string(root.join(name)).unwrap();
        let e = bennett_vm::load(&text, false).unwrap_err();
        assert!(e.contains("§9.1"), "{name}: {e}");
    }
}
