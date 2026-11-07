//! Saved journal contract from the reference README; fixture is published ISA 0.8.
use bennett_vm::json::{self, Json};
use std::{
    path::PathBuf,
    process::{Command, Output},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn dir(name: &str) -> PathBuf {
    let path = root().join(".tmp/saved-tape").join(name);
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn fixture() -> PathBuf {
    crate::pins::isa().join("reference-interpreter/tests/fixtures/classical.bisa")
}
fn run(command: &str, path: &std::path::Path, entry: &str, options: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
        .args([
            command,
            path.to_str().unwrap(),
            entry,
            "--entropy",
            "0b1621",
        ])
        .args(options)
        .output()
        .unwrap()
}
fn good(out: Output) -> (Vec<u8>, Json) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc = json::parse(std::str::from_utf8(&out.stdout).unwrap()).unwrap();
    (out.stdout, doc)
}
fn bad(out: Output) {
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("replay mismatch"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn saved_five_device_tape_replays_byte_identically_and_resets() {
    let dir = dir("devices");
    let path = dir.join("tape.json");
    let path = path.to_str().unwrap();
    let (stdout, value) = good(run("run-cc", &fixture(), "effects", &["--tape-out", path]));
    let tape = json::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(value.get("tape"), Some(&tape));
    assert_eq!(tape.as_arr().unwrap().len(), 5);
    assert_eq!(value.get("console_hex"), Some(&Json::Str("0b1621".into())));
    assert_eq!(
        good(run(
            "run-cc",
            &fixture(),
            "effects",
            &["--replay-tape", path]
        ))
        .0,
        stdout
    );
    let (reverse, reset) = good(run("reverse", &fixture(), "effects", &[]));
    assert_eq!(
        good(run(
            "reverse",
            &fixture(),
            "effects",
            &["--replay-tape", path]
        ))
        .0,
        reverse
    );
    for key in ["equals_entry", "memory_equals_entry"] {
        assert_eq!(reset.get(key), Some(&Json::Bool(true)));
    }
    assert_eq!(reset.get("first_difference"), Some(&Json::Null));
    assert_eq!(reset.get("console_hex"), Some(&Json::Str("".into())));
    assert_eq!(reset.get("effect_tape_length"), Some(&Json::u64(0)));
    assert_eq!(reset.get("entry_memory_digest"), reset.get("memory_digest"));
}
#[test]
fn replay_rejects_changed_bytes_operands_order_and_exhaustion_without_output() {
    let dir = dir("corruption");
    let (_, value) = good(run("run-cc", &fixture(), "effects", &[]));
    let original = value.get("tape").unwrap().as_arr().unwrap();
    for case in ["bytes", "args", "dev", "op", "order", "missing", "extra"] {
        let mut tape = original.clone();
        match case {
            "order" => tape.swap(0, 1),
            "missing" => {
                tape.pop();
            }
            "extra" => tape.push(tape[0].clone()),
            field => {
                if let Json::Obj(m) = &mut tape[0] {
                    m.insert(
                        field.into(),
                        match field {
                            "bytes" => Json::Str("001621".into()),
                            "args" => Json::Arr(vec![Json::u64(17), Json::u64(3)]),
                            "dev" => Json::Str("console".into()),
                            _ => Json::Str("write".into()),
                        },
                    );
                }
            }
        }
        let path = dir.join(format!("{case}.json"));
        std::fs::write(&path, Json::Arr(tape).to_jcs()).unwrap();
        let output = dir.join("must-not-overwrite.json");
        std::fs::write(&output, "sentinel").unwrap();
        bad(run(
            "run-cc",
            &fixture(),
            "effects",
            &[
                "--replay-tape",
                path.to_str().unwrap(),
                "--tape-out",
                output.to_str().unwrap(),
            ],
        ));
        assert_eq!(std::fs::read_to_string(output).unwrap(), "sentinel");
    }
}
#[test]
fn journal_retains_cancelled_effects_and_reverse_saves_forward_prefix() {
    let dir = dir("journal");
    let tape = dir.join("tape.json");
    let tape = tape.to_str().unwrap();
    let (stdout, value) = good(run("run-cc", &fixture(), "nested", &["--tape-out", tape]));
    assert_eq!(value.get("tape"), Some(&Json::Arr(vec![])));
    assert_eq!(
        json::parse(&std::fs::read_to_string(tape).unwrap())
            .unwrap()
            .as_arr()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        good(run(
            "run-cc",
            &fixture(),
            "nested",
            &["--replay-tape", tape]
        ))
        .0,
        stdout
    );
    let (stdout, _) = good(run(
        "reverse",
        &fixture(),
        "effects",
        &["--steps", "4", "--tape-out", tape],
    ));
    assert_eq!(
        json::parse(&std::fs::read_to_string(tape).unwrap())
            .unwrap()
            .as_arr()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        good(run(
            "reverse",
            &fixture(),
            "effects",
            &["--steps", "4", "--replay-tape", tape]
        ))
        .0,
        stdout
    );
}
#[test]
fn failed_effect_adds_no_journal_entry_and_empty_tape_replays_fault() {
    let dir = dir("fault");
    let path = dir.join("tape.json");
    let path = path.to_str().unwrap();
    // Override entropy rather than accepting the fixture helper's three bytes.
    let invoke = |opts: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
            .args(["run-cc", fixture().to_str().unwrap(), "entropy_short"])
            .args(opts)
            .output()
            .unwrap()
    };
    let (stdout, value) = good(invoke(&["--tape-out", path]));
    assert_eq!(value.get("fault"), Some(&Json::u64(10)));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "[]\n");
    assert_eq!(good(invoke(&["--replay-tape", path])).0, stdout);
}
#[test]
fn malformed_tapes_and_duplicate_options_fail_loudly() {
    let dir = dir("schema");
    let path = dir.join("bad.json");
    let path = path.to_str().unwrap();
    for text in [
        "{}",
        "[{}]",
        r#"[{"args":["01"],"bytes":"","dev":"clock","op":"read"}]"#,
        r#"[{"args":[1.0],"bytes":"","dev":"clock","op":"read"}]"#,
        r#"[{"args":[],"bytes":"GG","dev":"x","op":"x"}]"#,
    ] {
        std::fs::write(path, text).unwrap();
        let out = run("run-cc", &fixture(), "effects", &["--replay-tape", path]);
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains("tape"));
    }
    for option in ["--tape-out", "--replay-tape"] {
        let out = run(
            "run-cc",
            &fixture(),
            "effects",
            &[option, path, option, path],
        );
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
    }
}

#[test]
fn launch_replays_identically_and_both_reset_commands_restore_entry() {
    let dir = dir("launch");
    let raw = std::fs::read_to_string(fixture())
        .unwrap()
        .replace("n = const.i16 1024", "n = const.i16 512")
        .replacen(
            "len = hostcall org.bennett.test p n",
            "result = call @effects\nlen = const.i16 0",
            1,
        );
    // Adapt the published CC host entry to invoke the R device control.
    let (obj, ids) = bennett_vm::load(&raw, true).unwrap();
    let symbols = Json::Obj(
        obj.symbols
            .values()
            .map(|s| {
                (
                    s.index.to_string(),
                    Json::obj(vec![
                        ("interface_hash", Json::Str(ids.interface[&s.name].clone())),
                        ("kind", Json::Str(s.kind.clone())),
                        ("name", Json::Str(s.name.clone())),
                        ("signature", s.signature.clone()),
                    ]),
                )
            })
            .collect(),
    );
    let mut lines = raw.lines().map(String::from).collect::<Vec<_>>();
    let at = lines.iter().position(|l| l == "section symbols").unwrap() + 1;
    lines[at] = symbols.to_jcs();
    let obj = dir.join("launch.bisa");
    std::fs::write(&obj, lines.join("\n") + "\n").unwrap();
    let tape = dir.join("tape.json");
    let tape = tape.to_str().unwrap();
    let (stdout, _) = good(run("run-program", &obj, "host", &["--tape-out", tape]));
    assert_eq!(
        good(run("run-program", &obj, "host", &["--replay-tape", tape])).0,
        stdout
    );
    let (stdout, reset) = good(run(
        "run-program",
        &obj,
        "host",
        &["--reverse", "--replay-tape", tape],
    ));
    assert_eq!(
        good(run("run-program", &obj, "host", &["--reverse"])).0,
        stdout
    );
    let (_, history) = good(run("reverse", &obj, "host", &["--replay-tape", tape]));
    for reset in [reset, history] {
        assert_eq!(reset.get("equals_entry"), Some(&Json::Bool(true)));
        assert_eq!(reset.get("memory_equals_entry"), Some(&Json::Bool(true)));
        assert_eq!(reset.get("first_difference"), Some(&Json::Null));
        assert_eq!(reset.get("entry_memory_digest"), reset.get("memory_digest"));
        assert_eq!(reset.get("effect_tape_length"), Some(&Json::u64(0)));
        assert_eq!(reset.get("console_hex"), Some(&Json::Str("".into())));
    }
}
