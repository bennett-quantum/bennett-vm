//! Hand-derived §7.3 device-sector boundary test (not reference code).
use bennett_vm::json::{self, Json};
use std::{path::Path, process::Command};
#[path = "support/device.rs"]
mod device_fixture;
use device_fixture::block_object;
#[test]
fn block_image_reads_a_whole_sector_and_reverses() {
    // §7.3: block uses "512-byte sectors"; a read replaces all 512 destination bytes.
    let raw = block_object(true);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join(".tmp/full-block.bisa");
    std::fs::write(&path, &raw).unwrap();
    let run = |cmd: &str| {
        Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
            .arg(cmd)
            .arg(&path)
            .arg("effects")
            .output()
            .unwrap()
    };
    let out = run("run-cc");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result = json::parse(std::str::from_utf8(&out.stdout).unwrap().trim()).unwrap();
    assert_eq!(
        result.get("results").unwrap().as_arr().unwrap()[0].get("value"),
        Some(&Json::Str("165".into()))
    );
    let out = run("reverse");
    assert!(out.status.success());
    let result = json::parse(std::str::from_utf8(&out.stdout).unwrap().trim()).unwrap();
    assert_eq!(result.get("equals_entry"), Some(&Json::Bool(true)));
}

#[test]
fn incomplete_block_images_are_rejected_before_execution() {
    let raw = block_object(false);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/invalid-block-sector.bisa");
    std::fs::write(&path, raw).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
        .arg("run-cc")
        .arg(path)
        .arg("effects")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("§7.3: block image must have complete 512-byte sectors")
    );
}
