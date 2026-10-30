//! Run the complete published harness unchanged; retain unavailable cases.
use std::{path::Path, process::Command};
#[test]
fn published_conformance_report() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let harness = repo
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/target/debug/bennett-isa-conformance");
    for mode in ["interpreter", "jit"] {
        let output = Command::new(&harness)
            .args(["--bin", env!("CARGO_BIN_EXE_bennett-vm")])
            .env("BENNETT_VM_ENGINE", mode)
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
        std::fs::write(repo.join(format!(".tmp/conformance-{mode}.log")), &report).unwrap();
        // ISA 91aa124 adds D sampled relation/invocation and G resolved estimate
        // witnesses, unavailable pending 0.9 (clw/hsd). Keep every case;
        // exit 2 reports unavailable cases, not FAIL.
        assert_eq!(output.status.code(), Some(2), "{report}");
        for row in [
            "| A | 21 | 0 | 0 | 21 |",
            "| B | 35 | 0 | 0 | 35 |",
            "| C | 339 | 0 | 0 | 339 |",
            "| D | 7 | 0 | 6 | 13 |",
            "| E | 0 | 0 | 16 | 16 |",
            "| F | 33 | 0 | 2 | 35 |",
            "| G | 10 | 0 | 14 | 24 |",
            "| H | 0 | 0 | 10 | 10 |",
            "| all | 445 | 0 | 48 | 493 |",
        ] {
            assert!(
                report.contains(row),
                "must execute every published case: {report}"
            );
        }
        assert!(!report.lines().any(|l| l.starts_with("FAIL ")), "{report}");
        println!(
            "{mode}: 445 available cases pass; 48 unavailable; raw published report: {}",
            repo.join(format!(".tmp/conformance-{mode}.log")).display()
        );
    }
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
