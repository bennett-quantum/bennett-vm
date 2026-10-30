//! Differential harness against the reference interpreter binary.
//!
//! The oracle is used only through its CLI (brief rule). Paths default to
//! the sibling `bennett-isa-spec` checkout and may be overridden with
//! `BENNETT_ISA_REF` / `BENNETT_FIXTURES`. A missing oracle is a failing
//! test, never a skip.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn record(args: &[String], out: &Output) {
    if std::env::var_os("BENNETT_HARNESS_COUNTS").is_some() {
        eprintln!(
            "HARNESS {} {}",
            args[0],
            if out.status.success() {
                "json"
            } else {
                "diagnostic"
            }
        );
    }
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn reference() -> PathBuf {
    if let Ok(p) = std::env::var("BENNETT_ISA_REF") {
        return PathBuf::from(p);
    }
    workspace().join("bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref")
}

fn fixtures() -> PathBuf {
    if let Ok(p) = std::env::var("BENNETT_FIXTURES") {
        return PathBuf::from(p);
    }
    workspace().join("bennett-isa-spec/reference-interpreter/tests/fixtures")
}

fn vm() -> &'static str {
    env!("CARGO_BIN_EXE_bennett-vm")
}

fn raw_run(bin: &Path, args: &[String], mode: &str) -> Output {
    Command::new(bin)
        .args(args)
        .env("BENNETT_VM_ENGINE", mode)
        .env("BENNETT_JIT_AUDIT", "1")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("BROWSER")
        .output()
        .unwrap_or_else(|e| panic!("cannot run {}: {e}", bin.display()))
}

// The unchanged reference comparison now obliges every pass separately and
// their composition, then executes the same invocation through native code.
fn run(bin: &Path, args: &[String]) -> Output {
    let baseline = raw_run(bin, args, "interpreter");
    if bin == Path::new(vm()) {
        let object_hash = std::fs::read(&args[1])
            .map(|b| bennett_vm::sha256::sha256_hex(&b))
            .unwrap_or_else(|_| "absent".into());
        for mode in [
            "inline",
            "structure",
            "reindex",
            "simplify",
            "optimised",
            "jit",
        ] {
            let transformed = raw_run(bin, args, mode);
            assert!(
                baseline.status.code() == transformed.status.code()
                    && baseline.stdout == transformed.stdout,
                "TRANSFORMATION DIVERGENCE {mode} {args:?}\nbaseline {}\ntransformed {}",
                show(&baseline),
                show(&transformed)
            );
            let without_audit = |output: &Output| {
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .filter(|line| !line.starts_with("JIT_AUDIT "))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            assert_eq!(
                without_audit(&baseline),
                without_audit(&transformed),
                "{mode}: diagnostic changed for {args:?}"
            );
            if mode == "jit"
                && baseline.status.success()
                && matches!(
                    args[0].as_str(),
                    "run-oracle" | "run-cc" | "run-program" | "step-check"
                )
            {
                let stderr = String::from_utf8_lossy(&transformed.stderr);
                assert!(
                    stderr
                        .lines()
                        .any(|l| l.starts_with("JIT_AUDIT native_runs=")
                            && !l.starts_with("JIT_AUDIT native_runs=0 ")),
                    "{args:?}: native execution must actually occur: {stderr}"
                );
            }
            if std::env::var_os("BENNETT_HARNESS_COUNTS").is_some() {
                let audit = String::from_utf8_lossy(&transformed.stderr);
                let audit = audit.lines().find(|l| l.starts_with("JIT_AUDIT ")).unwrap();
                eprintln!(
                    "JIT_HARNESS {mode} {} {} program={object_hash} {audit}",
                    args[0],
                    if transformed.status.success() {
                        "json"
                    } else {
                        "diagnostic"
                    }
                );
            }
        }
    }
    baseline
}

fn show(out: &Output) -> String {
    format!(
        "exit={:?}\nstdout={}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Byte-identical stdout and identical exit status.
fn assert_bytes(case: &str, args: &[String]) {
    let refr = reference();
    assert!(
        refr.exists(),
        "reference binary missing at {}",
        refr.display()
    );
    let a = run(&refr, args);
    let b = run(Path::new(vm()), args);
    assert!(
        a.status.code() == b.status.code() && a.stdout == b.stdout,
        "DIVERGENCE {case}\nargs: {args:?}\n--- bennett-isa-ref ---\n{}\n--- bennett-vm ---\n{}",
        show(&a),
        show(&b)
    );
    record(args, &a);
}

fn fixture_files() -> Vec<PathBuf> {
    let dir = fixtures();
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("fixtures dir {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "bisa"))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no .bisa fixtures under {}", dir.display());
    v
}

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

fn fx(name: &str) -> String {
    fixtures().join(name).display().to_string()
}

#[test]
fn check_and_hash_byte_identical_on_every_fixture() {
    for f in fixture_files() {
        let path = f.display().to_string();
        assert_bytes(&format!("check {path}"), &s(&["check", &path]));
        assert_bytes(&format!("hash {path}"), &s(&["hash", &path]));
    }
}

#[test]
fn run_oracle_differential() {
    let cases: Vec<Vec<String>> = vec![
        s(&[
            "run-oracle",
            &fx("inc8.bisa"),
            "inc8",
            "--input",
            "8",
            "--budget",
            "3",
        ]),
        s(&[
            "run-oracle",
            &fx("inc8.bisa"),
            "inc8",
            "--input",
            "255",
            "--budget",
            "3",
        ]),
        s(&[
            "run-oracle",
            &fx("inc8.bisa"),
            "inc8",
            "--input",
            "8",
            "--budget",
            "2",
        ]),
        s(&[
            "run-oracle",
            &fx("inc8.bisa"),
            "inc8",
            "--input",
            "8",
            "--budget",
            "0",
        ]),
        s(&[
            "run-oracle",
            &fx("sum_db.bisa"),
            "sum_db",
            "--input",
            "3",
            "--mem",
            "db=0102030405060708090a0b0c0d0e0f10",
            "--budget",
            "1000,4080,512,64",
        ]),
        s(&[
            "run-oracle",
            &fx("sum_db.bisa"),
            "sum_db",
            "--input",
            "16",
            "--mem",
            "db=0102030405060708090a0b0c0d0e0f10",
            "--budget",
            "10000,4080,272,64",
        ]),
        s(&[
            "run-oracle",
            &fx("sum_db.bisa"),
            "sum_db",
            "--input",
            "16",
            "--mem",
            "db=0102030405060708090a0b0c0d0e0f10",
            "--budget",
            "10000,4080,271,64",
        ]),
        s(&[
            "run-oracle",
            &fx("sum_db.bisa"),
            "sum_db",
            "--input",
            "17",
            "--mem",
            "db=0102030405060708090a0b0c0d0e0f10",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "add",
            "--input",
            "255,2",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "sub",
            "--input",
            "0,1",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "mul",
            "--input",
            "128,3",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "udiv",
            "--input",
            "3,0",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "sdiv",
            "--input",
            "128,255",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "sdiv",
            "--input",
            "249,2",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "srem",
            "--input",
            "249,2",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "ashr",
            "--input",
            "128,8",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "casts",
            "--input",
            "128,0",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "parmov",
            "--input",
            "3,9",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "jump",
            "--input",
            "0,0",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "jump",
            "--input",
            "1,9",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "badjump",
            "--input",
            "1,2",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "fault0",
            "--input",
            "1,2",
        ]),
        s(&[
            "run-oracle",
            &fx("arithmetic.bisa"),
            "fault10",
            "--input",
            "1,2",
        ]),
        s(&["run-oracle", &fx("heap.bisa"), "heap", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "zeroalloc", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "afterfree", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "pinned", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "doublefree", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "badstore", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "outoftable", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "faultif", "--input", ""]),
        s(&["run-oracle", &fx("mutual.bisa"), "main", "--input", ""]),
        s(&[
            "run-oracle",
            &fx("nested.bisa"),
            "nested",
            "--input",
            "8",
            "--arg",
            "token:3",
        ]),
        s(&[
            "run-oracle",
            &fx("nested.bisa"),
            "nested",
            "--input",
            "8",
            "--arg",
            "token:2",
        ]),
        s(&[
            "run-oracle",
            &fx("nested.bisa"),
            "cancel",
            "--input",
            "8",
            "--arg",
            "token:2",
        ]),
        s(&[
            "run-oracle",
            &fx("nested.bisa"),
            "ignore",
            "--input",
            "8",
            "--arg",
            "token:2",
        ]),
        s(&[
            "run-oracle",
            &fx("tiny_memory.bisa"),
            "tiny",
            "--input",
            "0",
            "--mem",
            "db=ff",
        ]),
        s(&[
            "run-oracle",
            &fx("tiny_memory.bisa"),
            "tiny",
            "--input",
            "1",
            "--mem",
            "db=03",
        ]),
        s(&["run-oracle", &fx("actions.bisa"), "lifted", "--input", "2"]),
        s(&[
            "run-oracle",
            &fx("wide.bisa"),
            "add257",
            "--input",
            "1606938044258990275541962092341162602522202993782792835301379,1267650600228229401496703205381",
            "--budget",
            "10",
        ]),
        s(&[
            "run-oracle",
            &fx("wide.bisa"),
            "div257",
            "--input",
            "1606938044258990275541962092341162602522202993782792835301379,1267650600228229401496703205381",
            "--budget",
            "10",
        ]),
        s(&[
            "run-oracle",
            &fx("wide.bisa"),
            "call257",
            "--input",
            "5",
            "--budget",
            "10,0,4,64",
        ]),
        s(&[
            "run-oracle",
            &fx("wide.bisa"),
            "call257",
            "--input",
            "5",
            "--budget",
            "10,0,5,1",
        ]),
        s(&["run-oracle", &fx("wide.bisa"), "fault257", "--input", "5"]),
        s(&[
            "run-oracle",
            &fx("wide.bisa"),
            "fault_small257",
            "--input",
            "1",
        ]),
    ];
    for (i, args) in cases.iter().enumerate() {
        assert_bytes(&format!("run-oracle case {i}"), args);
    }
}

/// Memory digests are opaque implementation fingerprints (the ISA does not
/// prescribe their encoding). Compare every other field, including trace.
fn assert_outcome(case: &str, args: &[String]) {
    let a = run(&reference(), args);
    let b = run(Path::new(vm()), args);
    assert_eq!(
        a.status.code(),
        b.status.code(),
        "{case}\nref {}\nvm {}",
        show(&a),
        show(&b)
    );
    if !a.status.success() {
        assert!(
            a.stdout.is_empty() && b.stdout.is_empty(),
            "diagnostic must not print JSON"
        );
        record(args, &a);
        return;
    }
    let mut aj = bennett_vm::json::parse(std::str::from_utf8(&a.stdout).unwrap().trim()).unwrap();
    let mut bj = bennett_vm::json::parse(std::str::from_utf8(&b.stdout).unwrap().trim()).unwrap();
    for j in [&mut aj, &mut bj] {
        if let bennett_vm::json::Json::Obj(m) = j {
            for key in ["memory_digest", "entry_memory_digest"] {
                if args[0] == "run-oracle" {
                    continue;
                }
                if key == "entry_memory_digest" && args[0] == "run-cc" {
                    continue;
                }
                let digest = m.remove(key).expect("required digest field");
                let text = digest.as_str().expect("digest must be a string");
                assert!(
                    text.len() == 64
                        && text
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                );
            }
        }
    }
    assert_eq!(
        aj,
        bj,
        "DIVERGENCE {case}\nargs {args:?}\nref {}\nvm {}",
        show(&a),
        show(&b)
    );
    record(args, &a);
}

#[test]
fn classical_control_and_launch_differential() {
    for entry in [
        "sum",
        "nested",
        "effects",
        "impure",
        "entropy_short",
        "unknown_effect",
        "memfault",
        "divzero",
        "host_invalid",
    ] {
        for command in ["run-cc", "reverse", "run-program"] {
            let mut args = s(&[command, &fx("classical.bisa"), entry, "--entropy", "616263"]);
            if entry == "sum" {
                args.extend(s(&["--args", "3"]));
            }
            assert_outcome(&format!("{command} classical {entry}"), &args);
        }
    }
    for entry in [
        "allocation",
        "views",
        "unclean",
        "measurement",
        "measuring",
        "bad_uncall",
    ] {
        for command in ["run-cc", "reverse", "run-program"] {
            let args = s(&[command, &fx("registers.bisa"), entry, "--draws", "0.9"]);
            assert_outcome(&format!("{command} registers {entry}"), &args);
        }
    }
}

fn temp_object(label: &str, text: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".tmp")
        .join(format!("{label}-{}.bisa", std::process::id()));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Construct only our test objects; all identities are independently checked
    // by the reference binary before execution.
    let (obj, ids) = bennett_vm::load(text, true).unwrap();
    let mut symbols = obj
        .symbols
        .values()
        .map(|s| {
            (s.index.to_string(), {
                let mut fields = std::collections::BTreeMap::new();
                fields.insert(
                    "interface_hash".into(),
                    bennett_vm::json::Json::Str(ids.interface[&s.name].clone()),
                );
                fields.insert("kind".into(), bennett_vm::json::Json::Str(s.kind.clone()));
                fields.insert("name".into(), bennett_vm::json::Json::Str(s.name.clone()));
                fields.insert("signature".into(), s.signature.clone());
                bennett_vm::json::Json::Obj(fields)
            })
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let _ = &mut symbols;
    let mut lines = text.lines().map(String::from).collect::<Vec<_>>();
    let index = lines.iter().position(|l| l == "section symbols").unwrap() + 1;
    lines[index] = bennett_vm::json::Json::Obj(symbols).to_jcs();
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    let path = path.display().to_string();
    assert_bytes(label, &s(&["check", &path]));
    path
}

fn add_cc(
    text: &str,
    name: &str,
    params: &str,
    param_types: Vec<&str>,
    results: &str,
    result_types: Vec<&str>,
    body: &str,
) -> String {
    use bennett_vm::json::Json;
    let mut lines = text.lines().map(String::from).collect::<Vec<_>>();
    let i = lines.iter().position(|l| l == "section symbols").unwrap() + 1;
    let Json::Obj(mut syms) = bennett_vm::json::parse(&lines[i]).unwrap() else {
        panic!()
    };
    syms.insert(
        syms.len().to_string(),
        Json::obj(vec![
            ("interface_hash", Json::Str("0".repeat(64))),
            ("kind", Json::Str("cc".into())),
            ("name", Json::Str(name.into())),
            (
                "signature",
                Json::obj(vec![
                    ("abi", Json::Str("0.8".into())),
                    ("tier", Json::Str("cc".into())),
                    ("kind", Json::Str("cc".into())),
                    (
                        "params",
                        Json::Arr(
                            param_types
                                .into_iter()
                                .map(|s| Json::Str(s.into()))
                                .collect(),
                        ),
                    ),
                    (
                        "results",
                        Json::Arr(
                            result_types
                                .into_iter()
                                .map(|s| Json::Str(s.into()))
                                .collect(),
                        ),
                    ),
                ]),
            ),
        ]),
    );
    lines[i] = Json::Obj(syms).to_jcs();
    let fi = lines.iter().position(|l| l == "section functions").unwrap();
    let end = (fi + 1..lines.len())
        .find(|i| lines[*i].starts_with("section "))
        .unwrap_or(lines.len());
    let insertion = if end == lines.len() { end } else { end - 1 };
    lines.insert(
        insertion,
        format!("fn cc @{name} ({params}) ({results}) () () ()\n^entry():\n{body}"),
    );
    lines.join("\n") + "\n"
}

#[test]
fn every_fixture_symbol_executes_through_cli() {
    use bennett_vm::object::{FnKind, Type};
    let mut counts = (0, 0, 0);
    for path in fixture_files() {
        let text = std::fs::read_to_string(&path).unwrap();
        let (obj, _) = bennett_vm::load(&text, false).unwrap();
        for f in &obj.functions {
            match f.kind {
                FnKind::Oracle => {
                    for budget in ["0", "1", "256"] {
                        let input = f.lists[0].iter().map(|_| "0").collect::<Vec<_>>().join(",");
                        let mut args = s(&[
                            "run-oracle",
                            path.to_str().unwrap(),
                            &f.name,
                            "--input",
                            &input,
                            "--budget",
                            budget,
                        ]);
                        for p in &f.lists[4] {
                            args.extend(s(&[
                                "--arg",
                                match p.ty {
                                    Type::Token => "token:64",
                                    Type::F64 => "f64:0.5",
                                    _ => "0",
                                },
                            ]));
                        }
                        assert_outcome(&format!("{} {} B={budget}", path.display(), f.name), &args);
                        counts.0 += 1;
                    }
                }
                FnKind::Cc | FnKind::R => {
                    // Nonempty inputs are covered by the explicit boundary cases.
                    if !f.lists[0].is_empty() {
                        continue;
                    }
                    let args = s(&[
                        "run-cc",
                        path.to_str().unwrap(),
                        &f.name,
                        "--entropy",
                        "616263",
                        "--draws",
                        "0.9",
                    ]);
                    // The host success case needs its explicit reply file below.
                    if f.name != "host" {
                        assert_outcome(&f.name, &args);
                        counts.1 += 1;
                    }
                }
                FnKind::Action | FnKind::PermAction => {
                    // Reach every action through a legal CC region call, with
                    // caller-owned handles covering every interface bundle/cell.
                    let mut body = String::new();
                    let mut groups = vec![];
                    let mut next = 0;
                    for list in &f.lists[..3] {
                        let mut names = vec![];
                        for p in list {
                            let n = match p.ty {
                                Type::Wire(n) => n,
                                Type::Region(a, w) => (1usize << a) * w,
                                _ => panic!(),
                            };
                            let name = format!("q{next}");
                            next += 1;
                            body += &format!("{name} = qreg.new any {n}\n");
                            names.push(name);
                        }
                        groups.push(format!("({})", names.join(", ")));
                    }
                    let params = f.lists[3]
                        .iter()
                        .map(|p| format!("{}:{}", p.name, p.ty.signature_text()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let types = f.lists[3]
                        .iter()
                        .map(|p| p.ty.signature_text())
                        .collect::<Vec<_>>();
                    let arg_names = f.lists[3]
                        .iter()
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                    body += &format!(
                        "action_call @{} {} ({arg_names})\n",
                        f.name,
                        groups.join(" ")
                    );
                    body += &format!(
                        "adj action_call @{} {} ({arg_names})\n",
                        f.name,
                        groups.join(" ")
                    );
                    for i in 0..next {
                        body += &format!("qreg.free q{i}\n");
                    }
                    body += "ret";
                    let text = add_cc(
                        &text,
                        "test_action",
                        &params,
                        types.iter().map(String::as_str).collect(),
                        "",
                        vec![],
                        &body,
                    );
                    let p = temp_object(&format!("action-{}", f.name), &text);
                    let mut args = s(&["run-cc", &p, "test_action"]);
                    for a in &f.lists[3] {
                        args.extend(s(&[
                            "--arg",
                            match a.ty {
                                Type::Token => "token:256",
                                Type::F64 => "f64:0.5",
                                _ => "0",
                            },
                        ]));
                    }
                    assert_outcome(&format!("action {}", f.name), &args);
                    counts.2 += 1;
                }
                FnKind::Lproc => {}
            }
        }
    }
    eprintln!(
        "all-symbol coverage: {} oracle inputs, {} CC/R entries, {} action/adjoint pairs",
        counts.0, counts.1, counts.2
    );
}

#[test]
fn retained_instances_survive_segment_cancellation() {
    let text = std::fs::read_to_string(fixtures().join("classical.bisa")).unwrap();
    let text = add_cc(
        &text,
        "reuse",
        "",
        vec![],
        "answer:i8",
        vec!["i8"],
        "n = const.i8 3\nx = call @sum n\ny = const.i8 7\nexpected = const.i8 6\nuncall @sum (n) (expected)\nret y",
    );
    let p = temp_object("reuse", &text);
    assert_outcome("retained successor", &s(&["run-cc", &p, "reuse"]));
    assert_outcome("retained successor reverse", &s(&["reverse", &p, "reuse"]));
}

#[test]
fn host_reply_and_prefix_reversal_differential() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/host-replies.json");
    std::fs::write(
        &p,
        r#"[{"service":"org.bennett.test","payload":"a50000","reply":"0102030405"}]"#,
    )
    .unwrap();
    for command in ["run-cc", "run-program", "reverse"] {
        assert_outcome(
            "host truncation",
            &s(&[
                command,
                &fx("classical.bisa"),
                "host",
                "--replies",
                p.to_str().unwrap(),
            ]),
        );
    }
    for step in 0..=22 {
        assert_outcome(
            "loop prefix",
            &s(&[
                "reverse",
                &fx("classical.bisa"),
                "sum",
                "--args",
                "3",
                "--steps",
                &step.to_string(),
            ]),
        );
        assert_outcome(
            "nested prefix",
            &s(&[
                "reverse",
                &fx("classical.bisa"),
                "nested",
                "--steps",
                &step.to_string(),
            ]),
        );
    }
}

#[test]
fn action_basis_measurements_differential() {
    use bennett_vm::object::{FnKind, Type};
    let mut cases = 0;
    for path in fixture_files() {
        let text = std::fs::read_to_string(&path).unwrap();
        let (obj, _) = bennett_vm::load(&text, false).unwrap();
        for f in obj
            .functions
            .iter()
            .filter(|f| matches!(f.kind, FnKind::Action | FnKind::PermAction))
        {
            let mut params = vec![];
            let mut types = vec![];
            let mut result_params = vec![];
            let mut result_types = vec![];
            let mut groups = vec![];
            let mut widths = vec![];
            for list in &f.lists[..3] {
                let mut names = vec![];
                for p in list {
                    let n = match p.ty {
                        Type::Wire(n) => n,
                        Type::Region(a, w) => (1usize << a) * w,
                        _ => panic!(),
                    };
                    let name = format!("q{}", widths.len());
                    params.push(format!("{name}:qreg[{n}]"));
                    types.push(format!("qreg[{n}]"));
                    names.push(name);
                    widths.push(n);
                }
                groups.push(format!("({})", names.join(", ")));
            }
            for p in &f.lists[3] {
                params.push(format!("{}:{}", p.name, p.ty.signature_text()));
                types.push(p.ty.signature_text());
            }
            // Designated idle set, deliberately prepared dirty in variant 1.
            params.push("idle:qreg[2]".into());
            types.push("qreg[2]".into());
            let arg_names = f.lists[3]
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            let mut body = format!(
                "action_call @{} {} ({arg_names})\n",
                f.name,
                groups.join(" ")
            );
            for (i, n) in widths.iter().enumerate() {
                body += &format!("r{i} = measure q{i}\n");
                result_params.push(format!("r{i}:i{n}"));
                result_types.push(format!("i{n}"));
            }
            body += "dirty = measure idle\n";
            result_params.push("dirty:i2".into());
            result_types.push("i2".into());
            body += &format!(
                "ret {}",
                (0..widths.len())
                    .map(|i| format!("r{i}"))
                    .chain(std::iter::once("dirty".into()))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            let t = add_cc(
                &text,
                "observe_action",
                &params.join(", "),
                types.iter().map(String::as_str).collect(),
                &result_params.join(", "),
                result_types.iter().map(String::as_str).collect(),
                &body,
            );
            let p = temp_object(&format!("observe-{}", f.name), &t);
            for variant in 0..3 {
                let mut args = s(&[
                    "run-program",
                    &p,
                    "observe_action",
                    "--draws",
                    "0.9,0.1,0.9,0.1,0.9,0.1,0.9,0.1",
                ]);
                for n in &widths {
                    let bits = (0..*n)
                        .map(|i| {
                            if variant == 1 || variant == 2 && i % 2 == 0 {
                                '1'
                            } else {
                                '0'
                            }
                        })
                        .collect::<String>();
                    args.extend(s(&["--arg", &format!("qreg:{bits}")]));
                }
                for a in &f.lists[3] {
                    args.extend(s(&[
                        "--arg",
                        match a.ty {
                            Type::Token => "token:256",
                            Type::F64 => "f64:0.5",
                            _ => "0",
                        },
                    ]));
                }
                args.extend(s(&[
                    "--arg",
                    if variant == 1 { "qreg:11" } else { "qreg:00" },
                ]));
                assert_outcome(&format!("observe {} variant {variant}", f.name), &args);
                cases += 1;
            }
        }
    }
    eprintln!("action observations: {cases}");
}

#[test]
fn spec_examples_are_extracted_and_diffed() {
    let spec =
        std::fs::read_to_string(workspace().join("bennett-isa-spec/spec/Bennett-ISA-Spec.md"))
            .unwrap();
    let mut count = 0;
    for fenced in spec
        .split("```\n")
        .filter(|s| s.starts_with("bennett-object "))
    {
        let text = fenced.split("```").next().unwrap();
        let p = temp_object(&format!("spec-example-{count}"), text);
        let name = if count == 0 { "inc8" } else { "sum_db" };
        assert_bytes(
            name,
            &s(&["run-oracle", &p, name, "--input", "3", "--budget", "256"]),
        );
        coherent_corpus(Path::new(&p));
        count += 1;
    }
    assert_eq!(count, 2, "every complete object in the spec must be diffed");
}

#[test]
fn malformed_classical_types_are_load_errors() {
    let text = std::fs::read_to_string(fixtures().join("classical.bisa")).unwrap();
    for (name, old, new) in [
        ("width", "one = const.i8 1", "one = const.i16 1"),
        ("ssa", "b = const.i8 0", "a = const.i8 0"),
        ("unknown", "r = udiv a b", "r = udiv a unknown"),
        ("return", "ret answer", "ret zero one"),
    ] {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(".tmp/invalid-{name}.bisa"));
        std::fs::write(&p, text.replace(old, new)).unwrap();
        assert_bytes(name, &s(&["hash", p.to_str().unwrap()]));
    }
}

#[test]
fn oracle_interface_memory_is_restored() {
    // The bounded computation may overwrite memory; the oracle construction
    // reverses that prefix before returning to its caller (§5.2).
    let text = std::fs::read_to_string(fixtures().join("tiny_memory.bisa"))
        .unwrap()
        .replace("q = lift.udiv v b", "lift.store.i8 p b\nq = lift.mov v");
    let (obj, _) = bennett_vm::load(&text, true).unwrap();
    let entry = vec![vec![bennett_vm::bigint::BigUint::from_u64(255)]];
    let mut mem = entry.clone();
    let b = bennett_vm::cliargs::default_budget(&obj, 256);
    let outcome = bennett_vm::oracle::run_oracle(
        &obj,
        "tiny",
        &[bennett_vm::bigint::BigUint::from_u64(1)],
        &mut mem,
        &[],
        b,
    )
    .unwrap();
    assert!(outcome.code.is_zero());
    assert_eq!(
        mem, entry,
        "§5.2: in regions unchanged after prefix reversal"
    );
}

#[test]
fn cc_oracle_xor_budgets_controls_and_arguments() {
    let text = std::fs::read_to_string(fixtures().join("inc8.bisa")).unwrap();
    let text = add_cc(
        &text,
        "invoke",
        "tk:token, x:qreg[8], t:qreg[8], f:qreg[4], c:qreg[1]",
        vec!["token", "qreg[8]", "qreg[8]", "qreg[4]", "qreg[1]"],
        "a:i8, b:i8, d:i4, e:i1",
        vec!["i8", "i8", "i4", "i1"],
        "ctl c oracle_call @inc8 tk (x) t f () ()\na = measure x\nb = measure t\nd = measure f\ne = measure c\nret a b d e",
    );
    let text = add_cc(
        &text,
        "uninvoke",
        "tk:token, x:qreg[8], t:qreg[8], f:qreg[4]",
        vec!["token", "qreg[8]", "qreg[8]", "qreg[4]"],
        "",
        vec![],
        "oracle_call @inc8 tk (x) t f () ()\nret",
    );
    let p = temp_object("invoke", &text);
    for budget in [0, 2, 3] {
        assert_outcome(
            "actual oracle involution reversal",
            &s(&[
                "run-program",
                &p,
                "uninvoke",
                "--reverse",
                "--arg",
                &format!("token:{budget}"),
                "--arg",
                "qreg:11111111",
                "--arg",
                "qreg:10100101",
                "--arg",
                "qreg:0101",
            ]),
        );
    }
    for budget in [0, 2, 3] {
        for x in ["00000000", "11111111"] {
            for control in ["0", "1"] {
                assert_outcome(
                    "controlled oracle XOR",
                    &s(&[
                        "run-program",
                        &p,
                        "invoke",
                        "--arg",
                        &format!("token:{budget}"),
                        "--arg",
                        &format!("qreg:{x}"),
                        "--arg",
                        "qreg:10100101",
                        "--arg",
                        "qreg:0101",
                        "--arg",
                        &format!("qreg:{control}"),
                    ]),
                );
            }
        }
    }
}

#[test]
fn classical_rows_and_boundary_faults() {
    let text = std::fs::read_to_string(fixtures().join("classical.bisa")).unwrap();
    for op in [
        "add", "sub", "mul", "and", "or", "xor", "shl", "lshr", "ashr", "udiv", "urem", "sdiv",
        "srem",
    ] {
        let t = add_cc(
            &text,
            "arithmetic",
            "a:i8, b:i8",
            vec!["i8", "i8"],
            "r:i8",
            vec!["i8"],
            &format!("r = {op} a b\nret r"),
        );
        let p = temp_object(op, &t);
        for (a, b) in [("255", "2"), ("128", "255"), ("128", "8"), ("3", "0")] {
            assert_outcome(
                op,
                &s(&["run-cc", &p, "arithmetic", "--args", &format!("{a},{b}")]),
            );
        }
    }
    for pred in ["eq", "ne", "ult", "ule", "slt", "sle"] {
        let t = add_cc(
            &text,
            "comparison",
            "a:i8, b:i8",
            vec!["i8", "i8"],
            "r:i1",
            vec!["i1"],
            &format!("r = icmp.{pred} a b\nret r"),
        );
        let p = temp_object(pred, &t);
        for (a, b) in [("255", "2"), ("128", "128")] {
            assert_outcome(
                pred,
                &s(&["run-cc", &p, "comparison", "--args", &format!("{a},{b}")]),
            );
        }
    }
    let t = add_cc(
        &text,
        "casts",
        "a:i8, b:i8, flag:i1",
        vec!["i8", "i8", "i1"],
        "r:i8, wide:i128, p:ptr, back:i128",
        vec!["i8", "i128", "ptr", "i128"],
        "wide = sext.i128 a\nsmall = trunc.i8 wide\nwider = zext.i128 b\nr = select flag small b\np = inttoptr wider\nback = ptrtoint.i128 p\nret r wide p back",
    );
    let p = temp_object("casts", &t);
    for args in ["-128,255,0", "-1,1,1", "128,0,1"] {
        assert_outcome("casts", &s(&["run-cc", &p, "casts", "--args", args]));
    }
    let t = add_cc(
        &text,
        "memory",
        "n:i16",
        vec!["i16"],
        "v:i16",
        vec!["i16"],
        "p = alloc n\nx = const.i16 65281\nstore.i16 p x\nv = load.i16 p\nfree p\nret v",
    );
    let p = temp_object("memory", &t);
    for n in ["0", "1", "2", "1024", "1025"] {
        for command in ["run-cc", "reverse", "run-program"] {
            assert_outcome("memory bounds", &s(&[command, &p, "memory", "--args", n]));
        }
    }
}

#[test]
fn history_reverses_actual_region_invocations() {
    let text = std::fs::read_to_string(fixtures().join("registers.bisa")).unwrap();
    let t = add_cc(
        &text,
        "unitary",
        "q:qreg[2]",
        vec!["qreg[2]"],
        "",
        vec![],
        "action_call @bell (q) () () ()\nret",
    );
    let t = add_cc(
        &t,
        "cancel_unitary",
        "q:qreg[2]",
        vec!["qreg[2]"],
        "r:i2",
        vec!["i2"],
        "call @unitary q\nx = const.i8 1\nuncall @unitary (q) ()\nr = measure q\nret r",
    );
    let p = temp_object("actual-adjoint", &t);
    for bits in ["00", "01", "10", "11"] {
        assert_outcome(
            "actual adjoint whole-run",
            &s(&[
                "run-program",
                &p,
                "unitary",
                "--arg",
                &format!("qreg:{bits}"),
                "--reverse",
            ]),
        );
        assert_outcome(
            "actual adjoint segment",
            &s(&[
                "run-program",
                &p,
                "cancel_unitary",
                "--arg",
                &format!("qreg:{bits}"),
            ]),
        );
        for step in [0, 1, 2] {
            assert_outcome(
                "actual adjoint prefix",
                &s(&[
                    "reverse",
                    &p,
                    "unitary",
                    "--arg",
                    &format!("qreg:{bits}"),
                    "--steps",
                    &step.to_string(),
                ]),
            );
        }
    }
}

/// Exact support and phase-sensitive component comparison. No global-phase
/// alignment: §4.4 requires equality including global phase. Tolerance applies
/// only to the binary64 display of exact amplitudes (absolute 2e-12).
fn assert_coherent(args: &[String]) {
    use bennett_vm::json::Json;
    fn compare(a: &Json, b: &Json, path: &str) {
        match (a, b) {
            (Json::Obj(a), Json::Obj(b)) => {
                assert_eq!(
                    a.keys().collect::<Vec<_>>(),
                    b.keys().collect::<Vec<_>>(),
                    "support/fields {path}"
                );
                for (k, v) in a {
                    compare(v, &b[k], &format!("{path}.{k}"));
                }
            }
            (Json::Arr(a), Json::Arr(b)) => {
                assert_eq!(a.len(), b.len(), "{path}");
                for (i, (a, b)) in a.iter().zip(b).enumerate() {
                    compare(a, b, &format!("{path}[{i}]"));
                }
            }
            (Json::Num(a), Json::Num(b)) if path.contains("state.") => {
                let a = a.parse::<f64>().unwrap();
                let b = b.parse::<f64>().unwrap();
                assert!(
                    (a - b).abs() <= 2e-12,
                    "phase-sensitive amplitude {path}: {a} != {b}"
                );
            }
            _ => assert_eq!(a, b, "{path}"),
        }
    }
    let a = run(&reference(), args);
    let b = run(Path::new(vm()), args);
    assert_eq!(
        a.status.code(),
        b.status.code(),
        "{args:?}\nref {}\nvm {}",
        show(&a),
        show(&b)
    );
    assert!(
        a.status.success(),
        "coherent fixture must execute, not merely agree on refusal: {args:?}\n{}",
        show(&a)
    );
    let aj = bennett_vm::json::parse(std::str::from_utf8(&a.stdout).unwrap().trim()).unwrap();
    let bj = bennett_vm::json::parse(std::str::from_utf8(&b.stdout).unwrap().trim()).unwrap();
    compare(&aj, &bj, &format!("{args:?}"));
    record(args, &a);
}
fn coherent_corpus(path: &Path) {
    use bennett_vm::object::{FnKind, Type};
    let text = std::fs::read_to_string(path).unwrap();
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    for f in &obj.functions {
        if f.kind == FnKind::Oracle {
            for budget in ["0", "1", "8", "64"] {
                let input = f.lists[0].iter().map(|_| "1").collect::<Vec<_>>().join(",");
                let mut args = s(&[
                    "reverse",
                    path.to_str().unwrap(),
                    &f.name,
                    "--input",
                    &input,
                    "--budget",
                    budget,
                ]);
                for p in &f.lists[4] {
                    args.extend(s(&[
                        "--arg",
                        match p.ty {
                            Type::Token => "token:32",
                            Type::F64 => "f64:0.5",
                            _ => "1",
                        },
                    ]));
                }
                assert_coherent(&args);
            }
            let mut args = s(&[
                "step-check",
                path.to_str().unwrap(),
                &f.name,
                "--samples",
                "4",
                "--seed",
                "1,2,3,4",
                "--budget",
                "64",
            ]);
            for p in &f.lists[4] {
                args.extend(s(&[
                    "--arg",
                    match p.ty {
                        Type::Token => "token:32",
                        Type::F64 => "f64:0.5",
                        _ => "1",
                    },
                ]));
            }
            assert_coherent(&args);
        } else if matches!(f.kind, FnKind::Action | FnKind::PermAction) {
            let mut n = 0;
            for list in &f.lists[..3] {
                for p in list {
                    n += match p.ty {
                        Type::Wire(w) => w,
                        Type::Region(a, w) => (1usize << a) * w,
                        _ => panic!(),
                    };
                }
            }
            let state = if n >= 2 {
                (0..4)
                    .map(|v| {
                        let bits = (0..n)
                            .map(|i| if i < 2 && v & (1 << i) != 0 { '1' } else { '0' })
                            .collect::<String>();
                        format!("{bits}={}", if v % 2 == 0 { "0.5" } else { "0,0.5" })
                    })
                    .collect::<Vec<_>>()
                    .join(";")
            } else {
                format!("{}=0,1", "0".repeat(n))
            };
            for cmd in ["run-action", "reverse"] {
                let mut args = s(&[cmd, path.to_str().unwrap(), &f.name, "--state", &state]);
                for p in &f.lists[3] {
                    args.extend(s(&[
                        "--arg",
                        match p.ty {
                            Type::Token => "token:64",
                            Type::F64 => "f64:0.5",
                            _ => "1",
                        },
                    ]));
                }
                assert_coherent(&args);
            }
            for basis in [
                "0".repeat(n),
                "1".repeat(n),
                (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect(),
            ] {
                for cmd in ["run-action", "reverse"] {
                    for adj in [false, true] {
                        let mut args =
                            s(&[cmd, path.to_str().unwrap(), &f.name, "--basis", &basis]);
                        if adj {
                            args.push("--adjoint".into());
                        }
                        for p in &f.lists[3] {
                            args.extend(s(&[
                                "--arg",
                                match p.ty {
                                    Type::Token => "token:64",
                                    Type::F64 => "f64:0.5",
                                    _ => "1",
                                },
                            ]));
                        }
                        assert_coherent(&args);
                    }
                }
            }
        }
    }
}
#[test]
fn coherent_commands_on_every_fixture() {
    for path in fixture_files() {
        coherent_corpus(&path);
    }
}

#[test]
fn published_d_and_g_cli_cases_differential() {
    // Re-run every runnable D/G command; staged scenarios have no oracle surface.
    let cases = workspace().join("bennett-isa-spec/conformance/cases");
    let mut compared = 0;
    for suite in ["D", "G"] {
        let mut dirs = std::fs::read_dir(cases.join(suite))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        dirs.sort();
        for dir in dirs {
            let doc =
                bennett_vm::json::parse(&std::fs::read_to_string(dir.join("case.json")).unwrap())
                    .unwrap();
            if doc.get("unavailable").is_some() {
                continue;
            }
            let Some(command) = doc.get("command").and_then(|v| v.as_arr()) else {
                panic!("available D/G case without command: {}", dir.display())
            };
            let object = dir
                .join(doc.get("object").unwrap().as_str().unwrap())
                .canonicalize()
                .unwrap();
            let args = command
                .iter()
                .map(|v| {
                    v.as_str()
                        .unwrap()
                        .replace("{object}", object.to_str().unwrap())
                        .replace("{dir}", dir.to_str().unwrap())
                })
                .collect::<Vec<_>>();
            if suite == "D" {
                assert_coherent(&args);
            } else {
                assert_outcome(doc.get("id").unwrap().as_str().unwrap(), &args);
            }
            compared += 1;
        }
    }
    assert_eq!(compared, 17, "all 7 D and 10 G CLI cases must participate");
}

#[path = "support/device.rs"]
mod device_fixture;
#[test]
fn block_image_boundaries_differential() {
    let raw = device_fixture::block_object(true);
    let path = temp_object("full-sector-block", &raw); // includes independent check comparison
    for command in ["run-cc", "reverse"] {
        assert_outcome("whole block sector", &s(&[command, &path, "effects"]));
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/incomplete-sector.bisa");
    std::fs::write(&path, device_fixture::block_object(false)).unwrap();
    let path = path.to_str().unwrap();
    assert_bytes("reject incomplete sector", &s(&["check", path]));
    assert_outcome(
        "reject incomplete sector before run",
        &s(&["run-cc", path, "effects"]),
    );
}

#[test]
fn jit_transformation_boundaries_differential() {
    // §5.7 T6; §3.6 observable canonical labels; §14.1 every row is a
    // budget step. These witnesses supplement, never replace, all fixtures.
    let base = std::fs::read_to_string(fixtures().join("inc8.bisa")).unwrap();
    let prefix = base.split("regs {").next().unwrap();
    let bodies = [
        (
            "fold-wrap",
            "regs { one:reg[8], r:reg[8] }\n^entry():\none = lift.const.i8 255\nr = lift.add one one\nlift.retval r\n",
        ),
        (
            "label-diamond",
            "regs { one:reg[8], r:reg[8], dest:reg[8], p:reg[1] }\n^entry():\ndest = lift.label ^left\nr = lift.mov dest\nqjumpr dest\n^left():\none = lift.const.i8 1\np = lift.icmp.eq x one\nqbranch p ^right ^exit\n^right():\nr = lift.add r one\nqjump ^exit\n^exit():\nlift.retval r\n",
        ),
        (
            "indirect-fault",
            "regs { dest:reg[8] }\n^entry():\ndest = lift.const.i8 255\nqjumpr dest\n",
        ),
        (
            "parallel-old-values",
            "regs { one:reg[8], r:reg[8] }\n^entry():\none = lift.const.i8 3\nr = lift.const.i8 5\n(one, r) = lift.parmov (r, one)\nr = lift.sub one r\nlift.retval r\n",
        ),
        (
            "loop-join",
            "regs { one:reg[8], r:reg[8], p:reg[1] }\n^entry():\none = lift.const.i8 1\nr = lift.const.i8 0\nqjump ^loop\n^loop():\nr = lift.add r one\np = lift.icmp.ult r x\nqbranch p ^loop ^exit\n^exit():\nlift.retval r\n",
        ),
    ];
    for (name, body) in bodies {
        let path = temp_object(&format!("jit-{name}"), &format!("{prefix}{body}"));
        for input in ["0", "1", "3", "255"] {
            for budget in ["0", "1", "2", "3", "5", "8", "18"] {
                assert_bytes(
                    name,
                    &s(&[
                        "run-oracle",
                        &path,
                        "inc8",
                        "--input",
                        input,
                        "--budget",
                        budget,
                    ]),
                );
            }
        }
        assert_coherent(&s(&[
            "step-check",
            &path,
            "inc8",
            "--exhaustive",
            "--max-bits",
            "8",
            "--budget",
            "8",
        ]));
    }
    let text = add_cc(
        &base,
        "fold",
        "",
        vec![],
        "r:i8",
        vec!["i8"],
        "a = const.i8 255\nb = const.i8 2\nr = add a b\nret r",
    );
    let path = temp_object("jit-cc-fold", &text);
    for command in ["run-cc", "run-program", "reverse"] {
        assert_outcome(
            "CC fold retains trace and history",
            &s(&[command, &path, "fold"]),
        );
    }
    for k in 0..=4 {
        assert_outcome(
            "CC folded history prefix",
            &s(&["reverse", &path, "fold", "--steps", &k.to_string()]),
        );
    }
}
