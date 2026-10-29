//! C11 consumer of the published §10 declarations. Expectations live in C.
use bennett_vm::{json::Json, object};
use std::{path::Path, process::Command};
fn fixture() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures");
    let mut obj =
        object::parse(&std::fs::read_to_string(root.join("classical.bisa")).unwrap()).unwrap();
    for file in ["registers.bisa", "inc8.bisa"] {
        let other = object::parse(&std::fs::read_to_string(root.join(file)).unwrap()).unwrap();
        for (name, mut sym) in other.symbols {
            sym.index = obj.symbols.len();
            obj.symbols.insert(name, sym);
        }
        obj.functions.extend(other.functions);
    }
    let additions = [
        (
            "echo",
            "x:i9, d:f64, q:qreg[2], t:token",
            "a:i9, b:f64, c:qreg[2], z:token",
            "ret x d q t",
        ),
        ("effect_run", "", "r:i64", "r = call @effects\nret r"),
        ("short_run", "", "", "call @entropy_short\nret"),
        ("fault_run", "", "r:i8", "r = call @divzero\nret r"),
        (
            "mutate",
            "q:qreg[1]",
            "",
            "action_call @toggle (q) () () ()\nret",
        ),
        ("readq", "q:qreg[1]", "r:i1", "r = measure q\nret r"),
        (
            "tree",
            "",
            "",
            "q = qreg.new any 1\ncall @qcallee q\nn = const.i16 1024\np = alloc n\ncall @f p\nuncall @f (p) ()\nret",
        ),
        ("newq", "", "q:qreg[1]", "q = qreg.new any 1\nret q"),
        (
            "oracle_run",
            "t:token",
            "r:i4, v:i8",
            "x = qreg.new any 8\ny = qreg.new any 8\nf = qreg.new any 4\noracle_call @inc8 t (x) y f () ()\nr = measure f\nv = measure y\nz = measure x\nret r v",
        ),
    ];
    let mut additions = additions.to_vec();
    additions.push((
        "qcallee",
        "q:qreg[1]",
        "",
        "action_call @toggle (q) () () ()\nret",
    ));
    let mut functions = obj
        .functions
        .iter()
        .map(|f| f.canonical.clone())
        .collect::<Vec<_>>();
    for (name, params, results, body) in additions {
        let types = |s: &str| {
            Json::Arr(
                s.split(", ")
                    .filter(|s| !s.is_empty())
                    .map(|s| Json::Str(s.split_once(':').unwrap().1.into()))
                    .collect(),
            )
        };
        let kind = if name == "qcallee" { "r" } else { "cc" };
        let sig = Json::obj(vec![
            ("abi", Json::Str("0.8".into())),
            ("kind", Json::Str(kind.into())),
            ("tier", Json::Str(kind.into())),
            ("params", types(params)),
            ("results", types(results)),
        ]);
        let sym = object::Symbol {
            index: obj.symbols.len(),
            name: name.into(),
            kind: kind.into(),
            signature: sig,
            stored_hash: "0".repeat(64),
        };
        obj.symbols.insert(name.into(), sym);
        functions.push(format!(
            "fn {kind} @{name} ({params}) ({results}) () () ()\n^entry():\n{body}\n"
        ));
    }
    let syms = |hashes: Option<&bennett_vm::identity::Identities>| {
        Json::Obj(
            obj.symbols
                .values()
                .map(|s| {
                    (
                        s.index.to_string(),
                        Json::obj(vec![
                            ("name", Json::Str(s.name.clone())),
                            ("kind", Json::Str(s.kind.clone())),
                            ("signature", s.signature.clone()),
                            (
                                "interface_hash",
                                Json::Str(
                                    hashes
                                        .map(|h| h.interface[&s.name].clone())
                                        .unwrap_or_else(|| "0".repeat(64)),
                                ),
                            ),
                        ]),
                    )
                })
                .collect(),
        )
        .to_jcs()
    };
    let original = std::fs::read_to_string(root.join("classical.bisa")).unwrap();
    let suffix = original.split_once("section devices").unwrap().1;
    let build = |symbols: String| {
        format!(
            "{}\n\nsection symbols\n{symbols}\n\nsection functions\n{}\n\nsection devices{}",
            original.lines().next().unwrap(),
            functions
                .iter()
                .map(|s| s.trim_end())
                .collect::<Vec<_>>()
                .join("\n"),
            suffix
        )
    };
    let text = build(syms(None));
    let (_, ids) = bennett_vm::load(&text, true).unwrap();
    build(syms(Some(&ids)))
}
#[test]
fn c11_launch_contract() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = repo.join(".tmp/c-abi");
    std::fs::create_dir_all(&tmp).unwrap();
    let object = tmp.join("launch.bisa");
    std::fs::write(&object, fixture()).unwrap();
    // Separate target directory avoids recursively taking cargo test's lock.
    let build = Command::new("cargo")
        .args(["build", "--offline", "--lib", "--manifest-path"])
        .arg(repo.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(tmp.join("target"))
        .env("TMPDIR", &tmp)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let exe = tmp.join("launch-abi");
    let output = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(repo.join("include"))
        .arg("-I")
        .arg(repo.parent().unwrap().join("bennett-isa-spec/include"))
        .arg(repo.join("tests/launch_abi.c"))
        .arg(tmp.join("target/debug/libbennett_vm.a"))
        .args(["-ldl", "-lpthread", "-lm", "-o"])
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(exe).arg(object).output().unwrap();
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn c11_sched_contract() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = repo.join(".tmp/sched-abi");
    std::fs::create_dir_all(&tmp).unwrap();
    let root = repo
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/cases");
    let raw = std::fs::read_to_string(root.join("objects/inc8.bisa")).unwrap();
    let (_, ids) = bennett_vm::load(&raw, false).unwrap();
    let object = tmp.join("inc8.bisa");
    std::fs::write(&object, raw).unwrap();
    let target = tmp.join("target.json");
    std::fs::write(
        &target,
        std::fs::read_to_string(root.join("E/E-matching-first-entry/target.json"))
            .unwrap()
            .trim_end(),
    )
    .unwrap();
    let out = Command::new("cargo")
        .args(["build", "--offline", "--lib", "--manifest-path"])
        .arg(repo.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(tmp.join("target"))
        .env("TMPDIR", &tmp)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let exe = tmp.join("sched-abi");
    let out = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(repo.join("include"))
        .arg("-I")
        .arg(repo.parent().unwrap().join("bennett-isa-spec/include"))
        .arg(repo.join("tests/sched_abi.c"))
        .arg(tmp.join("target/debug/libbennett_vm.a"))
        .args(["-ldl", "-lpthread", "-lm", "-o"])
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new(exe)
        .arg(object)
        .arg(target)
        .arg(ids.closure["inc8"].as_ref().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    println!("{}", String::from_utf8_lossy(&out.stdout));
}
