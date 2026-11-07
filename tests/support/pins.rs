//! Published sibling inputs must match the full SHA in gate.toml.
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

pub fn isa() -> &'static Path {
    static PIN: OnceLock<PathBuf> = OnceLock::new();
    PIN.get_or_init(|| {
        let config = include_str!("../../gate.toml");
        let section = config.split("[siblings.\"bennett-isa-spec\"]").nth(1).expect("ISA pin section");
        let sha = section.lines().find_map(|line| line.strip_prefix("sha = \"")?.strip_suffix('"')).expect("ISA pin SHA");
        assert_eq!(sha.len(), 40, "ISA pin must be a full SHA");
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = std::env::var_os("BENNETT_ISA_SPEC").map(PathBuf::from).unwrap_or_else(|| {
            root.join(format!(".tmp/gate/cache/bennett-isa-spec@{}/bennett-isa-spec", &sha[..12]))
        });
        let head = Command::new("git").arg("-C").arg(&path).args(["rev-parse", "HEAD"]).output()
            .unwrap_or_else(|e| panic!("ISA pin {sha} at {}: {e}", path.display()));
        let actual = String::from_utf8_lossy(&head.stdout);
        assert!(head.status.success() && actual.trim() == sha,
            "ISA pin {sha} required at {}; actual HEAD {:?}: {}. Materialise the gate pin with gate verify.",
            path.display(), actual.trim(), String::from_utf8_lossy(&head.stderr));
        path
    })
}

#[test]
fn published_inputs_are_from_the_gate_pin() {
    assert!(isa().join("spec/Bennett-ISA-Spec-0.9.md").is_file());
}

#[test]
fn a_wrong_checkout_is_rejected_with_expected_and_actual_heads() {
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "pins::published_inputs_are_from_the_gate_pin",
            "--nocapture",
        ])
        .env("BENNETT_ISA_SPEC", env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("ISA pin "), "{error}");
    assert!(error.contains("actual HEAD"), "{error}");
}
