//! Segment cancellation retains later instances and reverses without replay.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::json::{self, Json};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn run(bin: &Path, command: &str, object: &Path, engine: &str) -> Json {
    let output = Command::new(bin)
        .arg(command)
        .arg(object)
        .arg("main")
        .env("BENNETT_VM_ENGINE", engine)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{command}, {engine}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut result = json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    // As in the existing differential harness, memory digests are opaque
    // fingerprints. Validate their shape and each engine's entry relation;
    // the ISA does not prescribe a shared digest serialization.
    let digest = result.get("memory_digest").unwrap().as_str().unwrap();
    assert!(
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    if command != "run-cc" {
        let entry = result.get("entry_memory_digest").unwrap().as_str().unwrap();
        assert!(
            entry.len() == 64
                && entry
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_eq!(
            result.get("memory_equals_entry"),
            Some(&Json::Bool(entry == digest))
        );
    }
    let Json::Obj(fields) = &mut result else {
        panic!("CLI outcome must be an object");
    };
    fields.remove("memory_digest");
    fields.remove("entry_memory_digest");
    result
}

#[test]
fn cancellation_keeps_later_definitions_in_both_engines() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = root.join(format!(".tmp/segment-instances-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let reference = std::env::var_os("BENNETT_ISA_REF")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("../bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref")
        });
    let vm = Path::new(env!("CARGO_BIN_EXE_bennett-vm"));
    let expected = json::parse(r#"[{"type":"i8","value":"9"}]"#).unwrap();
    for version in ["0.8", "0.9"] {
        let encode = |functions: &str| {
            if version == "0.9" {
                support::source(functions)
            } else {
                support::source_at(version, functions)
            }
        };
        let definitions = "fn r @id () (r:i8) () () ()\n^entry():\nr = const.i8 2\nret r\nfn cc @main () (r:i8) () () ()\n^entry():\n";
        let repeated = scratch.join(format!("repeated-{version}.bisa"));
        std::fs::write(&repeated, encode(&format!("{definitions}x = const.i8 1\nx = call @id\nsaved = const.i8 2\nx = const.i8 9\nuncall @id () (saved)\nret x"))).unwrap();
        // The reference still rejects multiple classical definitions despite
        // ISA 0.9 §3.4. Use an equivalent distinct-name program as the oracle;
        // the consumer must retain the current instance of the reused name.
        let distinct = scratch.join(format!("distinct-{version}.bisa"));
        std::fs::write(&distinct, encode(&format!("{definitions}old = const.i8 1\ncalled = call @id\nsaved = const.i8 2\nlater = const.i8 9\nuncall @id () (saved)\nret later"))).unwrap();
        for command in ["run-program", "run-cc", "reverse"] {
            let oracle = run(&reference, command, &distinct, "interpreter");
            assert_eq!(oracle.get("results"), Some(&expected));
            for engine in ["interpreter", "jit"] {
                let actual = run(vm, command, &repeated, engine);
                let canonical = run(vm, command, &distinct, engine);
                assert_eq!(canonical, oracle, "{command}, {engine}, ISA {version}");
                assert_eq!(actual.get("results"), Some(&expected));
                for field in [
                    "status",
                    "fault",
                    "console_hex",
                    "clock",
                    "effect_tape_length",
                    "tape",
                    "host_requests",
                    "equals_entry",
                ] {
                    assert_eq!(
                        actual.get(field),
                        oracle.get(field),
                        "{field}: {command}, {engine}, ISA {version}"
                    );
                }
                if command == "reverse" {
                    assert_eq!(actual.get("equals_entry"), Some(&Json::Bool(true)));
                    assert_eq!(actual.get("first_difference"), Some(&Json::Null));
                }
            }
        }
    }
    std::fs::remove_dir_all(scratch).unwrap();
}
