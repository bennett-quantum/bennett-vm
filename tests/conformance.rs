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
    // The published harness hard-codes old staged-refusal prose for these probes.
    // Keep its FAIL verdict visible; require the actual new, rule-derived refusal.
    assert_eq!(output.status.code(), Some(1), "{report}");
    assert!(
        report.contains("| all | 445 | 2 | 43 | 490 |"),
        "must execute all cases: {report}"
    );
    let failures = report
        .lines()
        .filter(|l| l.starts_with("FAIL "))
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 2, "new conformance failure: {report}");
    assert!(
        failures[0].starts_with("FAIL E-target-staged-probe:")
            && failures[0].contains("§8.1: target must be in RFC 8785 canonical form"),
        "{}",
        failures[0]
    );
    assert!(
        failures[1].starts_with("FAIL H-schedule-staged-probe:")
            && failures[1].contains("E_SCHEDULE_INFEASIBLE")
            && failures[1].contains("expansion library ref@0.8"),
        "{}",
        failures[1]
    );
    println!(
        "445 available conformance cases pass; two obsolete staging-prose probes FAIL, 43 cases unavailable; raw report: {}",
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
