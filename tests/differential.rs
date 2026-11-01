//! Differential harness against the reference interpreter binary.
//!
//! The oracle is used only through its CLI (brief rule). Paths default to
//! the sibling `bennett-isa-spec` checkout and may be overridden with
//! `BENNETT_ISA_REF` / `BENNETT_FIXTURES`. A missing oracle is a failing
//! test, never a skip.

#[allow(dead_code)]
#[path = "support/source09.rs"]
mod source09;
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
fn cc_oracle_then_gate_preserves_xor_interpreter() {
    cc_oracle_then_gate_preserves_xor("interpreter");
}

#[test]
fn cc_oracle_then_gate_preserves_xor_jit() {
    cc_oracle_then_gate_preserves_xor("jit");
}

fn cc_oracle_then_gate_object(label: &str) -> String {
    use bennett_vm::json::Json;
    let text = std::fs::read_to_string(fixtures().join("inc8.bisa")).unwrap();
    let mut lines = text.lines().map(String::from).collect::<Vec<_>>();
    let i = lines.iter().position(|l| l == "section symbols").unwrap() + 1;
    let Json::Obj(mut symbols) = bennett_vm::json::parse(&lines[i]).unwrap() else {
        panic!("symbols must be an object")
    };
    for name in ["mix", "flip"] {
        symbols.insert(
            symbols.len().to_string(),
            Json::obj(vec![
                ("interface_hash", Json::Str("0".repeat(64))),
                ("kind", Json::Str("action".into())),
                ("name", Json::Str(name.into())),
                ("signature", bennett_vm::json::parse(r#"{"abi":"0.8","args":[],"bounds":[],"in":[],"kind":"action","mem":[],"mut":[8],"tier":"q"}"#).unwrap()),
            ]),
        );
    }
    lines[i] = Json::Obj(symbols).to_jcs();
    let text = lines.join("\n")
        + "\naction @mix (q:wire[8] mut) () () () ()\n^entry():\nh q[0]\n"
        + "action @flip (q:wire[8] mut) () () () ()\n^entry():\nx q[0]\n";
    let text = add_cc(
        &text,
        "oracle_then_gate",
        "tk:token",
        vec!["token"],
        "a:i8, b:i8, c:i4",
        vec!["i8", "i8", "i4"],
        "x = qreg.new any 8\nt = qreg.new any 8\nf = qreg.new any 4\naction_call @mix (x) () () ()\noracle_call @inc8 tk (x) t f () ()\naction_call @flip (t) () () ()\na = measure x\nb = measure t\nc = measure f\nret a b c",
    );
    temp_object(label, &text)
}

fn cc_oracle_then_gate_preserves_xor(engine: &str) {
    let path = cc_oracle_then_gate_object(&format!("oracle-then-gate-{engine}"));
    for (draw, expected) in [("0.1", ["0", "0", "0"]), ("0.9", ["1", "3", "0"])] {
        let args = s(&[
            "run-program",
            &path,
            "oracle_then_gate",
            "--arg",
            "token:3",
            "--draws",
            draw,
        ]);
        let out = raw_run(Path::new(vm()), &args, engine);
        assert!(out.status.success(), "{engine}: {}", show(&out));
        let json =
            bennett_vm::json::parse(std::str::from_utf8(&out.stdout).unwrap().trim()).unwrap();
        let actual = json
            .get("results")
            .unwrap()
            .as_arr()
            .unwrap()
            .iter()
            .map(|v| v.get("value").unwrap().as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            actual, expected,
            "{engine} draw={draw}: oracle XOR must survive the following gate"
        );
    }
}

#[test]
fn cc_oracle_then_gate_differential() {
    let path = cc_oracle_then_gate_object("oracle-then-gate-differential");
    for budget in ["0", "2", "3"] {
        for draw in ["0.1", "0.9"] {
            assert_outcome(
                "CC oracle XOR followed by action gate",
                &s(&[
                    "run-program",
                    &path,
                    "oracle_then_gate",
                    "--arg",
                    &format!("token:{budget}"),
                    "--draws",
                    draw,
                ]),
            );
        }
    }
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

// ---- VM defect beads from the 2026-11-01 CC review and differential harness ----

/// A signed object at `version` ("0.8" or "0.9") in a per-test file.
fn signed_object(label: &str, version: &str, functions: &str) -> String {
    let text = source09::source_at(version, functions);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".tmp")
        .join(format!("{label}-{version}-{}.bisa", std::process::id()));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    let path = path.display().to_string();
    // The reference has neither measure.iN nor qreg[*] yet (b-isa-spec-fuk.7) and
    // miscomputes 0.9 layout identities (b-isa-spec-7fd): such objects are judged
    // by the 0.9 text.
    if !functions.contains("measure.i")
        && !functions.contains("qreg[*]")
        && !(version == "0.9" && functions.contains("region["))
    {
        assert_bytes(label, &s(&["check", &path]));
    }
    let checked = raw_run(Path::new(vm()), &s(&["check", &path]), "interpreter");
    assert!(checked.status.success(), "{label}: {}", show(&checked));
    path
}

/// The JSON of one successful VM run under `engine`, with fail-loud context.
fn vm_json(engine: &str, args: &[String]) -> bennett_vm::json::Json {
    let out = raw_run(Path::new(vm()), args, engine);
    assert!(out.status.success(), "{engine} {args:?}: {}", show(&out));
    bennett_vm::json::parse(std::str::from_utf8(&out.stdout).unwrap().trim()).unwrap()
}

const ENGINES: [&str; 2] = ["interpreter", "jit"];

const SPINNING_ORACLE: &str = "perm_action @perm (t:wire mut) () () () ()\n^entry():\nx t\noracle @orc (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nqjump ^entry\n";

/// bennett-vm-erb: §5.2 writes survive a later action_call on another register.
#[test]
fn oracle_fault_code_survives_a_later_action_call() {
    let measured = signed_object(
        "erb-measure",
        "0.8",
        &format!(
            "{SPINNING_ORACLE}fn cc @main (tk:token) (r0:i4) () () ()\n^entry():\nq = qreg.new any 1\no = qreg.new any 1\nf = qreg.new any 4\noracle_call @orc tk (q) o f () ()\naction_call @perm (q) () () ()\nm = measure f\nret m"
        ),
    );
    let args = s(&["run-cc", &measured, "main", "--arg", "token:64"]);
    assert_outcome("erb measure", &args);
    for engine in ENGINES {
        let json = vm_json(engine, &args);
        let m = &json.get("measurements").unwrap().as_arr().unwrap()[0];
        assert_eq!(m.get("bits").unwrap().as_str(), Some("1000"), "{engine}");
    }
    let freed = signed_object(
        "erb-free",
        "0.8",
        &format!(
            "{SPINNING_ORACLE}fn cc @main (tk:token) () () () ()\n^entry():\nq = qreg.new any 1\no = qreg.new any 1\nf = qreg.new any 4\noracle_call @orc tk (q) o f () ()\naction_call @perm (q) () () ()\nqreg.free f\nret"
        ),
    );
    let args = s(&["run-cc", &freed, "main", "--arg", "token:64"]);
    assert_outcome("erb unclean free", &args);
    for engine in ENGINES {
        let json = vm_json(engine, &args);
        assert_eq!(json.get("fault").unwrap().as_u64(), Some(4), "{engine}");
    }
    let reversed = signed_object(
        "erb-reverse",
        "0.8",
        &format!(
            "{SPINNING_ORACLE}fn cc @main (tk:token) () () () ()\n^entry():\nq = qreg.new any 1\no = qreg.new any 1\nf = qreg.new any 4\noracle_call @orc tk (q) o f () ()\naction_call @perm (q) () () ()\nret"
        ),
    );
    let args = s(&["reverse", &reversed, "main", "--arg", "token:64"]);
    assert_outcome("erb reverse", &args);
    for engine in ENGINES {
        let json = vm_json(engine, &args);
        assert_eq!(
            json.get("equals_entry"),
            Some(&bennett_vm::json::Json::Bool(true)),
            "{engine}"
        );
    }
}

const ALIAS_PRELUDE: &str = "perm_action @flip (t:wire mut) () () () ()\n^entry():\nx t\naction @flip4 (t:wire[4] mut) () () () ()\n^entry():\nx t[0]\naction @cxact (t:wire mut) (c:wire in) () () ()\n^entry():\ncx c t\naction @two (a:wire mut, b:wire mut) () () () ()\n^entry():\nx a\nx b\naction @ins (t:wire mut) (c:wire in, d:wire in) () () ()\n^entry():\nccx c d t\noracle @orc (x:wire[4] in) (t:wire[4]) (f:wire[4]) () ()\n^entry():\nlift.retval x\noracle @orc1 (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x\n";

/// Every region-call shape whose slots are filled by two names of one register.
const REGION_PRELUDE: &str = "action @reg_mut (t:wire mut) () (m:region[0,1] mut) () ()\n^entry():\nx t\naction @reg_in (t:wire mut) () (m:region[0,1] in) () ()\n^entry():\nx t\n";

const ALIAS_CALLS: [(&str, &str); 6] = [
    ("mut-in", "action_call @cxact (a) (b) () ()"),
    ("mut-mut", "action_call @two (a, b) () () ()"),
    ("in-in", "action_call @ins (c) (a, b) () ()"),
    ("ctl-target", "ctl a action_call @flip (b) () () ()"),
    ("ctl-in-legal", "ctl a action_call @cxact (c) (b) () ()"),
    ("adj-ctl-target", "ctl a adj action_call @flip (b) () () ()"),
];

/// bennett-vm-3tx: §3.3 region-call ledger row and "Registers, handles and
/// aliases": overlap is a trap before any effect, a read-only control overlap
/// is legal.
#[test]
fn region_call_overlaps_trap_before_any_effect() {
    for (name, call) in ALIAS_CALLS {
        for (kind, version) in [("cc", "0.9"), ("r", "0.9"), ("cc", "0.8")] {
            let path = signed_object(
                &format!("3tx-{name}-{kind}"),
                version,
                &format!(
                    "{ALIAS_PRELUDE}fn {kind} @g (a:qreg[1], b:qreg[1], c:qreg[1]) () () () ()\n^entry():\n{call}\nret\nfn cc @main () (r:i1, s:i1) () () ()\n^entry():\nq = qreg.new any 1\nw = qreg.new any 1\naction_call @flip (q) () () ()\ncall @g q q w\nr = measure q\ns = measure w\nret r s"
                ),
            );
            for command in ["run-cc", "run-program"] {
                assert_outcome(
                    &format!("3tx {name} {kind} {version} {command}"),
                    &s(&[command, &path, "main", "--draws", "0.1,0.1"]),
                );
            }
            if version == "0.9" {
                let args = s(&["run-cc", &path, "main", "--draws", "0.1,0.1"]);
                for engine in ENGINES {
                    let json = vm_json(engine, &args);
                    let expected = if name == "ctl-in-legal" {
                        None
                    } else {
                        Some(4)
                    };
                    assert_eq!(
                        json.get("fault").unwrap().as_u64(),
                        expected,
                        "{engine} {name} {kind}"
                    );
                }
            }
        }
    }
    // Reversal of a run that trapped restores the register; the oracle slots
    // and registers used through a stale or viewed alias trap likewise.
    let bodies = [
        (
            "oracle-in-tgt",
            "oracle_call @orc tk (a) b c () ()",
            Some(4),
        ),
        (
            "oracle-tgt-fault",
            "oracle_call @orc tk (c) a b () ()",
            Some(4),
        ),
        (
            "oracle-in-fault",
            "oracle_call @orc tk (a) c b () ()",
            Some(4),
        ),
        ("mut-mem", "action_call @reg_mut (e) () (f) ()", Some(4)),
        (
            "ctl-mem-in-legal",
            "ctl e action_call @reg_in (d) () (f) ()",
            None,
        ),
        (
            "viewed-alias",
            "v = qreg.slice a 0 1\naction_call @flip4 (b) () () ()\naction_call @flip (v) () () ()",
            Some(4),
        ),
        (
            "freed-alias",
            "qreg.free a\naction_call @flip4 (b) () () ()",
            Some(4),
        ),
        (
            "ctl-mem-mut",
            "ctl e action_call @reg_mut (d) () (f) ()",
            Some(4),
        ),
        (
            "ctl-oracle-tgt",
            "ctl e oracle_call @orc1 tk (d) f z () ()",
            Some(4),
        ),
        (
            "ctl-oracle-in-legal",
            "ctl e oracle_call @orc1 tk (f) d z () ()",
            None,
        ),
        (
            "ctl-view-consumed",
            "vw = qreg.slice c 0 1\nctl vw action_call @flip (d) () () ()\naction_call @flip4 (c) () () ()\naction_call @flip4 (c) () () ()",
            None,
        ),
    ];
    for (name, body, expected) in bodies {
        let path = signed_object(
            &format!("3tx-{name}"),
            "0.9",
            &format!(
                "{ALIAS_PRELUDE}{}fn cc @g (tk:token, a:qreg[4], b:qreg[4], c:qreg[4], d:qreg[1], z:qreg[4], e:qreg[1], f:qreg[1]) () () () ()\n^entry():\n{body}\nret\nfn cc @main (tk:token) () () () ()\n^entry():\nq = qreg.new any 4\nw = qreg.new any 4\nd = qreg.new any 1\nz = qreg.new any 4\no = qreg.new any 1\ncall @g tk q q w d z o o\nret",
                if body.contains("reg_") {
                    REGION_PRELUDE
                } else {
                    ""
                }
            ),
        );
        // A 0.9 layout defeats the reference's identity check (b-isa-spec-7fd):
        // those shapes are judged by the §3.3 text alone.
        for command in ["run-cc", "reverse", "run-program"] {
            let mut args = s(&[command, &path, "main", "--arg", "token:64"]);
            if command == "run-program" {
                args.push("--reverse".into());
            }
            if !body.contains("reg_") {
                assert_outcome(&format!("3tx {name} {command}"), &args);
            }
        }
        let args = s(&["run-cc", &path, "main", "--arg", "token:64"]);
        for engine in ENGINES {
            let json = vm_json(engine, &args);
            assert_eq!(
                json.get("fault").unwrap().as_u64(),
                expected,
                "{engine} {name}"
            );
        }
    }
    // §3.3 binding: a qreg[*] of the wrong width at a slot traps, §4.9 a use with
    // no current instance likewise; measure.iN checks N before measuring, no draw.
    let path = signed_object(
        "3tx-dynamic",
        "0.9",
        &format!(
            "{ALIAS_PRELUDE}fn cc @dynamic (q:qreg[*]) () () () ()\n^entry():\naction_call @flip (q) () () ()\nret\nfn cc @width () () () () ()\n^entry():\nq = qreg.new any 2\ncall @dynamic q\nret\nfn cc @measure3 (q:qreg[*]) (r:i3) () () ()\n^entry():\nr = measure.i3 q\nret r\nfn cc @measure2 (q:qreg[*]) (r:i2) () () ()\n^entry():\nr = measure.i2 q\nret r\nfn cc @bad_measure () (r:i3) () () ()\n^entry():\nq = qreg.new any 2\nr = call @measure3 q\nret r\nfn cc @good_measure () (r:i2) () () ()\n^entry():\nq = qreg.new any 2\nr = call @measure2 q\nret r\nfn cc @no_instance (c:i1) () () () ()\n^entry():\nq = qreg.new any 1\nbr c ^define() ^use()\n^define():\nw = qreg.new any 1\nbr ^use()\n^use():\naction_call @flip (w) () () ()\nret"
        ),
    );
    for (entry, extra, fault) in [
        ("width", vec![], Some(4)),
        ("bad_measure", vec![], Some(4)),
        ("good_measure", vec![], None),
        ("no_instance", vec!["--arg", "0"], Some(4)),
    ] {
        let mut args = s(&["run-cc", &path, entry, "--draws", "0.1"]);
        args.extend(s(&extra));
        for engine in ENGINES {
            let json = vm_json(engine, &args);
            assert_eq!(
                json.get("fault").unwrap().as_u64(),
                fault,
                "{engine} {entry}"
            );
            assert_eq!(
                json.get("measurements").unwrap().as_arr().unwrap().len(),
                usize::from(entry == "good_measure"),
                "{engine} {entry}"
            );
        }
    }
}

fn bell_measure_program(measure: &str) -> String {
    format!(
        "action @bell (t:wire[2] mut) () () () ()\n^entry():\nh t[0]\ncx t[0] t[1]\nfn cc @measurement () (r:i2) () () ()\n^entry():\nq = qreg.new any 2\naction_call @bell (q) () () ()\nr = {measure} q\nret r\nfn cc @measuring () (r:i2) () () ()\n^entry():\nr = call @measurement\nret r\nfn cc @bad_uncall () (r:i2) () () ()\n^entry():\nr = call @measuring\nuncall @measuring () (r)\nret r"
    )
}

/// bennett-vm-4ke: measure.iN is a measure for §7.4 uncall and §10 reversal.
#[test]
fn measure_with_width_suffix_blocks_uncall_and_reversal() {
    let path = signed_object("4ke", "0.9", &bell_measure_program("measure.i2"));
    // The reference has no measure.iN yet (b-isa-spec-fuk.7): the 0.9 text is the
    // oracle here, and the plain-measure twin of the same program is differential.
    let plain = signed_object("4ke-plain", "0.8", &bell_measure_program("measure"));
    for entry in ["measurement", "measuring", "bad_uncall"] {
        for command in ["run-cc", "reverse", "run-program"] {
            assert_outcome(
                &format!("4ke plain {command} {entry}"),
                &s(&[command, &plain, entry, "--draws", "0.1"]),
            );
            // Same outcome as the plain twin, up to the operator spelling.
            let args = |p: &str| s(&[command, p, entry, "--draws", "0.1"]);
            for engine in ENGINES {
                let twin = raw_run(Path::new(vm()), &args(&plain), engine);
                let suffixed = raw_run(Path::new(vm()), &args(&path), engine);
                assert_eq!(
                    twin.status.code(),
                    suffixed.status.code(),
                    "{engine} {command} {entry}\n{}",
                    show(&suffixed)
                );
            }
        }
    }
    let args = s(&["run-cc", &path, "bad_uncall", "--draws", "0.1"]);
    for engine in ENGINES {
        let json = vm_json(engine, &args);
        assert_eq!(json.get("fault").unwrap().as_u64(), Some(4), "{engine}");
        let out = raw_run(
            Path::new(vm()),
            &s(&["reverse", &path, "measuring", "--draws", "0.1"]),
            engine,
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && stderr.contains("E_NOT_REVERSIBLE"),
            "{engine}: {}",
            show(&out)
        );
    }
}

/// bennett-vm-w63: 0.9 §10 FAILED -> REVERSED through run-program; 0.8's
/// eligibility is COMPLETED only and stays.
#[test]
fn failed_runs_reverse_through_run_program_at_0_9() {
    let functions = "fn cc @boom () (r:i64) () () ()\n^entry():\nx = const.i64 1\ny = const.i64 0\nr = udiv x y\nret r\nfn r @inner (d:i64) (r:i64) () () ()\n^entry():\nx = const.i64 1\nr = udiv x d\nret r\nfn cc @nested () (r:i64) () () ()\n^entry():\nd = const.i64 0\nq = qreg.new any 1\nr = call @inner d\nret r\nfn cc @measured () (r:i1, s:i64) () () ()\n^entry():\nq = qreg.new any 1\nr = measure q\nd = const.i64 0\ns = call @inner d\nret r s";
    for version in ["0.8", "0.9"] {
        let path = signed_object("w63", version, functions);
        for entry in ["boom", "nested", "measured"] {
            for flags in [&["--reverse"][..], &[][..]] {
                let mut args = s(&["run-program", &path, entry, "--draws", "0.1"]);
                args.extend(s(flags));
                assert_outcome(&format!("w63 {version} {entry} {flags:?}"), &args);
            }
            assert_outcome(
                &format!("w63 {version} reverse {entry}"),
                &s(&["reverse", &path, entry, "--draws", "0.1"]),
            );
        }
    }
    let path = signed_object("w63-engines", "0.9", functions);
    let args = s(&["run-program", &path, "nested", "--reverse"]);
    for engine in ENGINES {
        let json = vm_json(engine, &args);
        assert_eq!(json.get("status").unwrap().as_str(), Some("REVERSED"));
        assert_eq!(
            json.get("equals_entry"),
            Some(&bennett_vm::json::Json::Bool(true)),
            "{engine}"
        );
    }
    let path = signed_object("w63-engines-08", "0.8", functions);
    let args = s(&["run-program", &path, "nested", "--reverse"]);
    for engine in ENGINES {
        let out = raw_run(Path::new(vm()), &args, engine);
        assert!(!out.status.success(), "{engine}: {}", show(&out));
    }
}

/// bennett-vm-w63 through the §10 ABI: bennett_reverse of a FAILED run.
#[test]
fn launch_reverse_accepts_failed_runs_at_0_9() {
    use bennett_vm::launch::*;
    const OK: i32 = 0;
    const INVALID: i32 = 8;
    const NOT_REVERSIBLE: i32 = 9;
    let functions = "fn r @inner (d:i64) (r:i64) () () ()\n^entry():\nx = const.i64 1\nr = udiv x d\nret r\nfn cc @nested () (r:i64) () () ()\n^entry():\nd = const.i64 0\nq = qreg.new any 1\nr = call @inner d\nret r\nfn cc @measured () (r:i1, s:i64) () () ()\n^entry():\nq = qreg.new any 1\nr = measure q\nd = const.i64 0\ns = call @inner d\nret r s";
    for (version, entry, expected) in [
        ("0.9", "nested", OK),
        ("0.9", "measured", NOT_REVERSIBLE),
        ("0.8", "nested", INVALID),
    ] {
        let text = source09::source_at(version, functions);
        unsafe {
            let mut p = std::ptr::null_mut();
            let status = bennett_load(text.as_ptr(), text.len(), std::ptr::null(), 0, &mut p);
            assert_eq!(status, OK, "{version}");
            let mut args = std::ptr::null_mut();
            assert_eq!(bennett_args_new(0, &mut args), OK);
            let name = std::ffi::CString::new(entry).unwrap();
            let mut run = 0;
            let status = bennett_run(p, name.as_ptr(), args, std::ptr::null(), 0, &mut run);
            assert_eq!(status, OK);
            let (mut result, mut request) = (std::ptr::null_mut(), std::ptr::null_mut());
            assert_eq!(bennett_wait(p, run, &mut result, &mut request), OK);
            assert_eq!(bennett_result_nfaults(result), 1);
            assert_eq!(bennett_result_fault(result, 0), 5);
            assert_eq!(bennett_reverse(p, run), expected, "{version} {entry}");
            if expected == OK {
                // REVERSED: a second reversal is the state error, not a repeat.
                assert_eq!(bennett_reverse(p, run), INVALID, "{version} {entry}");
            }
            assert_eq!(bennett_run_close(p, run), OK);
            bennett_result_free(result);
            bennett_args_free(args);
            assert_eq!(bennett_unload(p), OK);
        }
    }
}

/// bennett-vm-h6j: a transfer no host can allocate is a diagnostic (exit 1),
/// never a panic.
#[test]
fn unallocatable_transfers_are_diagnostics_not_panics() {
    for fixture in ["r21/cc-length.bisa", "r21/r-length.bisa"] {
        let path = fx(fixture);
        for command in ["run-cc", "reverse", "run-program"] {
            let args = s(&[command, &path, "f"]);
            assert_outcome(&format!("h6j {command} {fixture}"), &args);
            for engine in ENGINES {
                let out = raw_run(Path::new(vm()), &args, engine);
                let stderr = String::from_utf8_lossy(&out.stderr);
                if command == "run-program" && fixture.starts_with("r21/r-") {
                    continue;
                }
                assert!(
                    out.status.code() == Some(1) && stderr.contains("host limit"),
                    "{engine} {command} {fixture}: {}",
                    show(&out)
                );
            }
        }
    }
}

/// bennett-vm-dwj: §4.1 K17, the first outcome whose cumulative weight is
/// strictly greater than draw x total, judged exactly at a CDF tie.
#[test]
fn measurement_draws_compare_exact_born_weights() {
    let functions = "action @single (t:wire mut) () () () ()\n^entry():\nh t\naction @h1 (t:wire[2] mut) () () () ()\n^entry():\nh t[1]\naction @h0h1 (t:wire[2] mut) () () () ()\n^entry():\nh t[0]\nh t[1]\naction @bell (t:wire[2] mut) () () () ()\n^entry():\nh t[0]\ncx t[0] t[1]\naction @phase (t:wire[2] mut) () () () ()\n^entry():\nh t[0]\nrz 0.5 t[0]\nh t[0]\nry 1.0 t[1]\nfn cc @one (a:i64) (r:i2) () () ()\n^entry():\nq = qreg.new any 2\naction_call @h1 (q) () () ()\nr = measure q\nret r\nfn cc @two (a:i64) (r:i2) () () ()\n^entry():\nq = qreg.new any 2\naction_call @h0h1 (q) () () ()\nr = measure q\nret r\nfn cc @bell_m (a:i64) (r:i2) () () ()\n^entry():\nq = qreg.new any 2\naction_call @bell (q) () () ()\nr = measure q\nret r\nfn cc @angle (a:i64) (r:i2) () () ()\n^entry():\nq = qreg.new any 2\naction_call @phase (q) () () ()\nr = measure q\nret r\nfn cc @sequence (a:i64) (r:i1, s:i1) () () ()\n^entry():\nq = qreg.new any 1\nw = qreg.new any 1\naction_call @single (q) () () ()\naction_call @single (w) () () ()\nr = measure q\ns = measure w\nret r s";
    for version in ["0.8", "0.9"] {
        let path = signed_object("dwj", version, functions);
        for entry in ["one", "two", "bell_m", "angle"] {
            for draw in [
                "0.0",
                "0.125",
                "0.25",
                "0.375",
                "0.5",
                "0.75",
                "0.875",
                "0.9999999",
            ] {
                assert_outcome(
                    &format!("dwj {version} {entry} {draw}"),
                    &s(&["run-cc", &path, entry, "--arg", "0", "--draws", draw]),
                );
            }
        }
        for draws in ["0.5,0.5", "0.0,0.5", "0.5,0.25", "0.25,0.75"] {
            assert_outcome(
                &format!("dwj {version} sequence {draws}"),
                &s(&["run-cc", &path, "sequence", "--arg", "0", "--draws", draws]),
            );
        }
    }
    let path = signed_object("dwj-engines", "0.8", functions);
    for (entry, draw, bits) in [
        ("one", "0.5", "01"),
        ("two", "0.25", "01"),
        ("two", "0.5", "10"),
        ("two", "0.75", "11"),
        ("bell_m", "0.5", "11"),
    ] {
        let args = s(&["run-cc", &path, entry, "--arg", "0", "--draws", draw]);
        for engine in ENGINES {
            let json = vm_json(engine, &args);
            let m = &json.get("measurements").unwrap().as_arr().unwrap()[0];
            assert_eq!(
                m.get("bits").unwrap().as_str(),
                Some(bits),
                "{engine} {entry} {draw}"
            );
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

#[test]
fn certificate_replay_requests_differential() {
    // §12.1 has no CLI verdict surface in the reference. Its run-oracle and
    // run-action surfaces do serve the underlying requests, including full
    // target/fault tuples and inverse pairs. Compare those, never invented CLI.
    let root = workspace().join("bennett-isa-spec/conformance/cases/objects");
    let pair = root.join("qinplace.bisa").display().to_string();
    for input in 0..4 {
        for name in ["inc2", "dec2"] {
            for budget in ["0", "2", "3"] {
                assert_outcome(
                    "§4.5 inverse pair F_B",
                    &s(&[
                        "run-oracle",
                        &pair,
                        name,
                        "--input",
                        &input.to_string(),
                        "--budget",
                        budget,
                    ]),
                );
            }
        }
        assert_coherent(&s(&[
            "run-action",
            &pair,
            "inc_inplace",
            "--mut",
            &input.to_string(),
            "--arg",
            "token:3,0,0,0",
        ]));
    }
    // Preserve complete tuples in every case of the scheduled-claim scenario;
    // the unscheduled semantic request is available even though the certificate
    // needs an unpublished schedule realisation.
    let tuple = root.join("t3_domains.bisa").display().to_string();
    for x in 0..4 {
        for t in 0..4 {
            for f in 0..16 {
                assert_coherent(&s(&[
                    "run-action",
                    &tuple,
                    "oracle_tuple",
                    "--mut",
                    &format!("{t},{f}"),
                    "--in",
                    &x.to_string(),
                    "--arg",
                    "token:3,0,0,0",
                ]));
            }
        }
    }
}
