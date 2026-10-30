//! Run the complete published harness unchanged; retain unavailable cases.
use std::{path::Path, process::Command};

fn run_harness(cases: &Path, mode: &str) -> (Option<i32>, String) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let harness = repo
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/target/debug/bennett-isa-conformance");
    let output = Command::new(&harness)
        .args(["--bin", env!("CARGO_BIN_EXE_bennett-vm"), "--cases"])
        .arg(cases)
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
    (
        output.status.code(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

fn unavailable_cases(report: &str, cases: &Path) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for line in report
        .lines()
        .filter(|line| line.starts_with("UNAVAILABLE "))
    {
        let id = line
            .strip_prefix("UNAVAILABLE ")
            .unwrap()
            .split(':')
            .next()
            .unwrap();
        let suite = id.split('-').next().unwrap();
        let path = cases.join(suite).join(id).join("case.json");
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let case = bennett_vm::json::parse(&text)?;
        if case.get("id").and_then(|v| v.as_str()) != Some(id) {
            return Err(format!("{id}: published case id mismatch"));
        }
        let unavailable = case.get("unavailable");
        let reason = unavailable
            .and_then(|v| v.get("reason"))
            .and_then(|v| v.as_str())
            .filter(|reason| !reason.trim().is_empty())
            .ok_or_else(|| format!("{id}: published unavailable.reason must be non-empty"))?;
        let deliverable = unavailable
            .and_then(|v| v.get("deliverable"))
            .and_then(|v| v.as_u64());
        if !deliverable.is_some_and(|n| (8..=11).contains(&n))
            && !reason.contains("b-isa-spec-")
            && !reason.contains("bennett-")
        {
            return Err(format!(
                "{id}: unavailable must name a bead or staged deliverable"
            ));
        }
        if ids.iter().any(|seen| seen == id) {
            return Err(format!("{id}: duplicate UNAVAILABLE outcome"));
        }
        ids.push(id.to_owned());
    }
    Ok(ids)
}

fn assert_rows(report: &str, unavailable: usize) {
    for (suite, pass) in [
        ("A", 21),
        ("B", 35),
        ("C", 339),
        ("D", 7),
        ("E", 0),
        ("F", 33),
        ("G", 10),
        ("H", 0),
        ("all", 445),
    ] {
        let prefix = format!("| {suite} |");
        let rows: Vec<_> = report.lines().filter(|l| l.starts_with(&prefix)).collect();
        assert_eq!(rows.len(), 1, "missing/duplicate suite {suite}: {report}");
        let columns: Vec<_> = rows[0].split('|').map(str::trim).collect();
        assert_eq!(columns.len(), 7, "{report}");
        assert_eq!(columns[2].parse::<usize>().unwrap(), pass, "{report}");
        assert_eq!(columns[3], "0", "{report}");
        let count = columns[4].parse::<usize>().unwrap();
        assert_eq!(
            columns[5].parse::<usize>().unwrap(),
            pass + count,
            "{report}"
        );
        let actual = report
            .lines()
            .filter(|l| {
                if suite == "all" {
                    l.starts_with("UNAVAILABLE ")
                } else {
                    l.starts_with(&format!("UNAVAILABLE {suite}-"))
                }
            })
            .count();
        assert_eq!(count, actual, "{report}");
        if suite == "all" {
            assert_eq!(count, unavailable, "{report}");
        }
    }
    assert!(!report.lines().any(|l| l.starts_with("FAIL ")), "{report}");
}

fn published_cases() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/cases")
}

#[test]
fn published_conformance_report() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(repo.join(".tmp")).unwrap();
    for mode in ["interpreter", "jit"] {
        let (exit, report) = run_harness(&published_cases(), mode);
        std::fs::write(repo.join(format!(".tmp/conformance-{mode}.log")), &report).unwrap();
        let ids = unavailable_cases(&report, &published_cases()).unwrap();
        assert_rows(&report, ids.len());
        assert_eq!(exit, Some(if ids.is_empty() { 0 } else { 2 }), "{report}");
        println!(
            "{mode}: 445 available cases pass; {} unavailable: {}",
            ids.len(),
            ids.join(", ")
        );
    }
}

fn copy_cases(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_cases(&entry.path(), &path);
        } else {
            std::fs::copy(entry.path(), path).unwrap();
        }
    }
}

#[test]
fn corrupted_unavailable_reason_is_rejected() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!(".tmp/conformance-corrupt-{}", std::process::id()));
    copy_cases(&published_cases(), &scratch);
    let id = "E-topology-grid-09";
    let path = scratch.join("E").join(id).join("case.json");
    let mut case = bennett_vm::json::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let bennett_vm::json::Json::Obj(ref mut fields) = case else {
        panic!("{id}: case must be an object");
    };
    let bennett_vm::json::Json::Obj(fields) = fields.get_mut("unavailable").unwrap() else {
        panic!("{id}: unavailable must be an object");
    };
    fields.insert("reason".into(), bennett_vm::json::Json::Str("  ".into()));
    std::fs::write(path, case.to_jcs()).unwrap();
    for mode in ["interpreter", "jit"] {
        let (exit, report) = run_harness(&scratch, mode);
        // The published binary accepts this metadata: our consumer must refuse it.
        assert_eq!(exit, Some(2), "{report}");
        assert_rows(
            &report,
            report
                .lines()
                .filter(|l| l.starts_with("UNAVAILABLE "))
                .count(),
        );
        assert_eq!(
            unavailable_cases(&report, &scratch),
            Err(format!(
                "{id}: published unavailable.reason must be non-empty"
            ))
        );
    }
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn metadata_unknown_fields_are_load_errors() {
    let root = published_cases().join("objects");
    for name in ["f_cert_unknown_field.bisa", "f_sched_unknown_field.bisa"] {
        let text = std::fs::read_to_string(root.join(name)).unwrap();
        let e = bennett_vm::load(&text, false).unwrap_err();
        assert!(e.contains("§9.1"), "{name}: {e}");
    }
}
