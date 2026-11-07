//! Published contract witnesses, consumed only through the pinned artifacts.
use std::process::Command;

fn published_case(suite: &str, name: &str) {
    use bennett_vm::json::{self, Json};
    let root = crate::pins::isa().join(format!("conformance/cases/{suite}/{name}"));
    let case = json::parse(&std::fs::read_to_string(root.join("case.json")).unwrap()).unwrap();
    let object = root.join(case.get("object").unwrap().as_str().unwrap());
    let args = case.get("command").unwrap().as_arr().unwrap().iter().map(|v| {
        v.as_str().unwrap().replace("{object}", object.to_str().unwrap())
    }).collect::<Vec<_>>();
    let out = Command::new(env!("CARGO_BIN_EXE_bennett-vm")).args(&args).output().unwrap();
    let expected = case.get("expect").unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(expected.get("exit").unwrap().as_u64().unwrap() as i32), "{name}: {stderr}");
    if expected.get("stdout_empty") == Some(&Json::Bool(true)) {
        assert!(out.stdout.is_empty(), "{name}: {:?}", out.stdout);
    }
    if let Some(value) = expected.get("stdout_json") {
        let actual = json::parse(std::str::from_utf8(&out.stdout).unwrap().trim()).unwrap();
        for (key, value) in value.as_obj().unwrap() {
            assert_eq!(actual.get(key), Some(value), "{name}: {key}");
        }
    }
    if let Some(parts) = expected.get("stderr_contains") {
        for part in parts.as_arr().unwrap() {
            assert!(stderr.contains(part.as_str().unwrap()), "{name}: {stderr}");
        }
    }
}

#[test]
fn published_oracle_token_partitions() {
    for suffix in ["000", "001", "010", "011", "012"] {
        published_case("C", &format!("C-k91-three-tokens-{suffix}"));
    }
    published_case("C", "C-k91-token-free-oracle");
    published_case("G", "G-k91-reverse-same");
}

#[test]
fn published_schedule_and_certificate_partitions() {
    for suffix in ["duplicate-carrier-same", "invalid-arity", "invalid-negative", "invalid-gap", "invalid-first-label", "wrong-partition-certificate"] {
        published_case("F", &format!("F-k91-{suffix}"));
    }
}

#[test]
fn published_checker_obligations() {
    for suffix in ["duplicate-component", "missing-expected"] {
        published_case("D", &format!("D-k84-{suffix}"));
    }
    let path = crate::pins::isa().join("conformance/fixtures/accepted-k84-09/baseline.bisa");
    bennett_vm::load(&std::fs::read_to_string(path).unwrap(), false).unwrap();
}
