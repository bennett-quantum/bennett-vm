//! Differential harness against the reference interpreter binary.
//!
//! The oracle is used only through its CLI (brief rule). Paths default to
//! the sibling `bennett-isa-spec` checkout and may be overridden with
//! `BENNETT_ISA_REF` / `BENNETT_FIXTURES`. A missing oracle is a failing
//! test, never a skip.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
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

fn run(bin: &Path, args: &[String]) -> Output {
    Command::new(bin)
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("BROWSER")
        .output()
        .unwrap_or_else(|e| panic!("cannot run {}: {e}", bin.display()))
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
    assert!(refr.exists(), "reference binary missing at {}", refr.display());
    let a = run(&refr, args);
    let b = run(Path::new(vm()), args);
    assert!(
        a.status.code() == b.status.code() && a.stdout == b.stdout,
        "DIVERGENCE {case}\nargs: {args:?}\n--- bennett-isa-ref ---\n{}\n--- bennett-vm ---\n{}",
        show(&a),
        show(&b)
    );
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
        s(&["run-oracle", &fx("inc8.bisa"), "inc8", "--input", "8", "--budget", "3"]),
        s(&["run-oracle", &fx("inc8.bisa"), "inc8", "--input", "255", "--budget", "3"]),
        s(&["run-oracle", &fx("inc8.bisa"), "inc8", "--input", "8", "--budget", "2"]),
        s(&["run-oracle", &fx("inc8.bisa"), "inc8", "--input", "8", "--budget", "0"]),
        s(&["run-oracle", &fx("sum_db.bisa"), "sum_db", "--input", "3", "--mem", "db=0102030405060708090a0b0c0d0e0f10", "--budget", "1000,4080,512,64"]),
        s(&["run-oracle", &fx("sum_db.bisa"), "sum_db", "--input", "16", "--mem", "db=0102030405060708090a0b0c0d0e0f10", "--budget", "10000,4080,272,64"]),
        s(&["run-oracle", &fx("sum_db.bisa"), "sum_db", "--input", "16", "--mem", "db=0102030405060708090a0b0c0d0e0f10", "--budget", "10000,4080,271,64"]),
        s(&["run-oracle", &fx("sum_db.bisa"), "sum_db", "--input", "17", "--mem", "db=0102030405060708090a0b0c0d0e0f10"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "add", "--input", "255,2"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "sub", "--input", "0,1"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "mul", "--input", "128,3"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "udiv", "--input", "3,0"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "sdiv", "--input", "128,255"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "sdiv", "--input", "249,2"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "srem", "--input", "249,2"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "ashr", "--input", "128,8"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "casts", "--input", "128,0"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "parmov", "--input", "3,9"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "jump", "--input", "0,0"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "jump", "--input", "1,9"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "badjump", "--input", "1,2"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "fault0", "--input", "1,2"]),
        s(&["run-oracle", &fx("arithmetic.bisa"), "fault10", "--input", "1,2"]),
        s(&["run-oracle", &fx("heap.bisa"), "heap", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "zeroalloc", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "afterfree", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "pinned", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "doublefree", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "badstore", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "outoftable", "--input", ""]),
        s(&["run-oracle", &fx("heap.bisa"), "faultif", "--input", ""]),
        s(&["run-oracle", &fx("mutual.bisa"), "main", "--input", ""]),
        s(&["run-oracle", &fx("nested.bisa"), "nested", "--input", "8", "--arg", "token:3"]),
        s(&["run-oracle", &fx("nested.bisa"), "nested", "--input", "8", "--arg", "token:2"]),
        s(&["run-oracle", &fx("nested.bisa"), "cancel", "--input", "8", "--arg", "token:2"]),
        s(&["run-oracle", &fx("nested.bisa"), "ignore", "--input", "8", "--arg", "token:2"]),
        s(&["run-oracle", &fx("tiny_memory.bisa"), "tiny", "--input", "0", "--mem", "db=ff"]),
        s(&["run-oracle", &fx("tiny_memory.bisa"), "tiny", "--input", "1", "--mem", "db=03"]),
        s(&["run-oracle", &fx("actions.bisa"), "lifted", "--input", "2"]),
        s(&["run-oracle", &fx("wide.bisa"), "add257", "--input", "1606938044258990275541962092341162602522202993782792835301379,1267650600228229401496703205381", "--budget", "10"]),
        s(&["run-oracle", &fx("wide.bisa"), "div257", "--input", "1606938044258990275541962092341162602522202993782792835301379,1267650600228229401496703205381", "--budget", "10"]),
        s(&["run-oracle", &fx("wide.bisa"), "call257", "--input", "5", "--budget", "10,0,4,64"]),
        s(&["run-oracle", &fx("wide.bisa"), "call257", "--input", "5", "--budget", "10,0,5,1"]),
        s(&["run-oracle", &fx("wide.bisa"), "fault257", "--input", "5"]),
        s(&["run-oracle", &fx("wide.bisa"), "fault_small257", "--input", "1"]),
    ];
    for (i, args) in cases.iter().enumerate() {
        assert_bytes(&format!("run-oracle case {i}"), args);
    }
}
