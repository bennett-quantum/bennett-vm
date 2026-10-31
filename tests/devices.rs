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
    assert!(
        bennett_vm::load(&raw, false)
            .unwrap_err()
            .contains("complete 512-byte sectors")
    );
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

#[test]
fn complete_sector_encoding_preserves_order_and_zero_presence() {
    use bennett_vm::device::block_image;
    assert!(block_image("").unwrap().is_empty());
    let mut bytes = vec![0u8; 1024];
    for (i, b) in bytes[..512].iter_mut().enumerate() {
        *b = (i % 256) as u8;
    }
    let image = bennett_vm::sha256::to_hex(&bytes);
    let sectors = block_image(&image).unwrap();
    assert_eq!(sectors.len(), 2);
    assert_eq!(sectors[&0], bytes[..512]);
    assert_eq!(sectors[&1], vec![0; 512]);
    assert!(!sectors.contains_key(&2)); // all-zero image sector is present; later sectors absent
    for n in [1, 511, 513, 1023] {
        assert!(
            block_image(&"00".repeat(n))
                .unwrap_err()
                .contains("complete 512-byte sectors")
        );
    }
    for bad in ["0".repeat(1023), "GG".repeat(512), "AA".repeat(512)] {
        assert!(block_image(&bad).is_err());
    }
}
