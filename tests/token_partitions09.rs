//! Published contract witnesses, consumed only through the pinned artifacts.
use std::process::Command;

fn published_case(suite: &str, name: &str) {
    use bennett_vm::json::{self, Json};
    let root = crate::pins::isa().join(format!("conformance/cases/{suite}/{name}"));
    let case = json::parse(&std::fs::read_to_string(root.join("case.json")).unwrap()).unwrap();
    let object = root.join(case.get("object").unwrap().as_str().unwrap());
    let args = case
        .get("command")
        .unwrap()
        .as_arr()
        .unwrap()
        .iter()
        .map(|v| {
            v.as_str()
                .unwrap()
                .replace("{object}", object.to_str().unwrap())
        })
        .collect::<Vec<_>>();
    let out = Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
        .args(&args)
        .output()
        .unwrap();
    let expected = case.get("expect").unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(expected.get("exit").unwrap().as_u64().unwrap() as i32),
        "{name}: {stderr}"
    );
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
    for suffix in [
        "duplicate-carrier-same",
        "invalid-arity",
        "invalid-negative",
        "invalid-gap",
        "invalid-first-label",
        "wrong-partition-certificate",
    ] {
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

#[test]
fn certificate_cannot_follow_changed_source_without_rebinding() {
    let path = crate::pins::isa().join("conformance/fixtures/accepted-k84-09/baseline.bisa");
    let text = std::fs::read_to_string(path)
        .unwrap()
        .replace("x t\n", "x t\nx t\nx t\n");
    let text = crate::source09::sign(&text);
    let error = bennett_vm::load(&text, false).unwrap_err();
    assert!(error.contains("object_hash"), "{error}");
}

#[test]
fn malformed_control_counts_do_not_default_to_zero() {
    let path = crate::pins::isa().join("conformance/fixtures/accepted-k84-09/baseline.bisa");
    let original = std::fs::read_to_string(path).unwrap();
    for value in ["-1", "null", "true", "\"0\""] {
        let text = original.replace(
            "\"inherited_controls\":0",
            &format!("\"inherited_controls\":{value}"),
        );
        let error = bennett_vm::load(&text, false).unwrap_err();
        assert!(error.contains("inherited_controls"), "{value}: {error}");
    }
}

#[test]
fn source_cleanup_uses_identity_through_forwarding_and_adjoint() {
    use bennett_vm::{json, ref_library::Catalog};
    let source = crate::source09::source(
        "oracle @identity (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x\nperm_action @pair (t:wire mut, f:wire[4] mut) (x:wire in) () (a:token, b:token) ()\n^entry():\noracle_call @identity a (x) t f () ()\noracle_call @identity b (x) t f () ()\nperm_action @outer (t:wire mut, f:wire[4] mut) (x:wire in) () (a:token, b:token) ()\n^entry():\naction_call @pair (t, f) (x) () (a, b)",
    );
    let (object, _) = bennett_vm::load(&source, false).unwrap();
    let catalog = Catalog::parse(
        &std::fs::read_to_string(crate::pins::isa().join("spec/ref-0.9.json")).unwrap(),
    )
    .unwrap();
    let args = json::parse(r#"[["1","0","0","0"],["1","0","0","0"]]"#).unwrap();
    for name in ["pair", "outer"] {
        for adjoint in [false, true] {
            for (labels, count) in [("[0,0]", 1), ("[0,1]", 0)] {
                let labels = json::parse(labels).unwrap();
                let expansion = catalog
                    .lower_action_with_token_aliases(
                        &object,
                        name,
                        &args,
                        adjoint,
                        0,
                        &[],
                        Some(&labels),
                    )
                    .unwrap();
                assert_eq!(
                    expansion.cleanups.len(),
                    count,
                    "{name} adjoint={adjoint} {labels:?}"
                );
                if count != 0 {
                    assert_eq!(expansion.cleanups[0].address.generated, 0);
                    assert_eq!(
                        expansion.cleanups[0].address.path.last().unwrap().point,
                        if adjoint { "entry:0" } else { "entry:1" }
                    );
                }
            }
        }
    }
    assert!(
        catalog
            .lower_action(&object, "pair", &args, false, 0)
            .unwrap_err()
            .contains("token_aliases")
    );
}

#[test]
fn partitions_reject_invalid_requests_and_preserve_fresh_identities() {
    use bennett_vm::{
        json, token_aliases,
        value::{Budget, Value},
    };
    let budget = Budget {
        steps: 1,
        ..Budget::default()
    };
    for labels in [
        "[1,1]",
        "[0,2]",
        "[0,-1]",
        "[0]",
        "[0,0,0]",
        "[0,0.0]",
        "[0,\"0\"]",
    ] {
        let mut args = [Value::Token(budget), Value::Token(budget)];
        assert!(
            token_aliases::bind(&mut args, None, Some(&json::parse(labels).unwrap())).is_err(),
            "{labels}"
        );
    }
    for labels in ["[0, 0]", "[0,0]\n"] {
        assert!(token_aliases::parse(Some(labels)).is_err());
    }
    let mut same = [Value::Token(budget), Value::Token(budget)];
    let aliases = json::parse("[0,0]").unwrap();
    token_aliases::bind(&mut same, None, Some(&aliases)).unwrap();
    assert_eq!(same[0], same[1]);
    let old = same[0].clone();
    token_aliases::bind(&mut same, None, Some(&aliases)).unwrap();
    assert_ne!(
        same[0], old,
        "new tuple-only request must create fresh identities"
    );
    assert_eq!(token_aliases::partition(None, &same).unwrap(), aliases);
    let mut unequal = [
        Value::Token(budget),
        Value::Token(Budget { steps: 2, ..budget }),
    ];
    let before = unequal.clone();
    assert!(
        token_aliases::bind(&mut unequal, None, Some(&aliases))
            .unwrap_err()
            .contains("unequal tuples")
    );
    assert_eq!(
        unequal, before,
        "invalid partition must not change argument identities"
    );
}

#[test]
fn schedule_consumption_includes_the_partition() {
    use bennett_vm::{
        json::{self, Json},
        sched,
        target::Target,
    };
    let (object, ids) = bennett_vm::load(
        &crate::source09::source(
            "perm_action @p (t:wire mut) () () (a:token, b:token) ()\n^entry():\nx t",
        ),
        false,
    )
    .unwrap();
    let target = Target::parse(
        std::fs::read_to_string(
            crate::pins::isa().join("conformance/cases/E/E-matching-first-entry/target.json"),
        )
        .unwrap()
        .trim_end(),
    )
    .unwrap();
    let params = json::parse(r#"[["1","0","0","0"],["1","0","0","0"]]"#).unwrap();
    let same = json::parse("[0,0]").unwrap();
    let distinct = json::parse("[0,1]").unwrap();
    let epsilon = "0".parse().unwrap();
    let bindings = sched::Bindings {
        region: "p",
        variant: "forward",
        inherited_controls: 0,
        budget: [0; 4],
        params: &params,
        token_aliases: Some(&same),
        epsilon: &epsilon,
    };
    let doc = Json::obj(vec![
        ("entries", Json::Arr(vec![])),
        (
            "bindings",
            Json::obj(vec![
                ("closure_hash", Json::Str(ids.closure["p"].clone().unwrap())),
                ("target_hash", Json::Str(target.hash.clone())),
                ("region", Json::Str("p".into())),
                ("variant", Json::Str("forward".into())),
                ("inherited_controls", Json::u64(0)),
                ("budget", json::parse("[0,0,0,0]").unwrap()),
                (
                    "params_hash",
                    Json::Str(bennett_vm::invocation::params_hash(&params)),
                ),
                ("token_aliases", same.clone()),
                ("epsilon", Json::u64(0)),
                (
                    "scheduler",
                    json::parse(r#"{"name":"test","version":"0.9"}"#).unwrap(),
                ),
            ]),
        ),
    ]);
    sched::consume(&doc, &object, &ids, &target, &bindings).unwrap();
    let different = sched::Bindings {
        token_aliases: Some(&distinct),
        ..bindings
    };
    assert!(
        sched::consume(&doc, &object, &ids, &target, &different)
            .unwrap_err()
            .contains("token_aliases mismatch")
    );
    let missing = sched::Bindings {
        token_aliases: None,
        ..bindings
    };
    assert!(
        sched::consume(&doc, &object, &ids, &target, &missing)
            .unwrap_err()
            .contains("token_aliases")
    );
}

#[test]
fn alias_aware_estimate_validates_before_the_existing_resource_boundary() {
    use bennett_vm::launch::*;
    use std::{ffi::CString, ptr};
    let source = crate::source09::source(
        "oracle @o (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x\nperm_action @p (t:wire mut) () () (a:token, b:token) ()\n^entry():\nx t",
    );
    unsafe {
        let mut program = ptr::null_mut();
        assert_eq!(
            bennett_load(source.as_ptr(), source.len(), ptr::null(), 0, &mut program),
            0
        );
        let mut token = 0;
        assert_eq!(
            bennett_budget(
                program,
                CString::new("o").unwrap().as_ptr(),
                1,
                0,
                0,
                0,
                &mut token
            ),
            0
        );
        let name = CString::new("p").unwrap();
        let variant = CString::new("forward").unwrap();
        let params = br#"[["1","0","0","0"],["2","0","0","0"]]"#;
        let mut out = ptr::null_mut();
        assert_eq!(
            bennett_estimate_run(program, name.as_ptr(), token, &mut out),
            11
        );
        for (aliases, expected) in [(b"[0,0]".as_slice(), 11), (b"[0,1]".as_slice(), 5)] {
            assert_eq!(
                bennett_estimate_run_with_token_aliases(
                    program,
                    name.as_ptr(),
                    token,
                    variant.as_ptr(),
                    0,
                    params.as_ptr(),
                    params.len(),
                    aliases.as_ptr(),
                    aliases.len(),
                    &mut out
                ),
                expected
            );
            assert!(out.is_null());
        }
        bennett_unload(program);
    }
}

#[test]
fn scheduler_partition_setter_rejects_noncanonical_labels() {
    use std::{ffi::c_void, ptr};
    unsafe extern "C" {
        fn bennett_sched_request_new(out: *mut *mut c_void) -> i32;
        fn bennett_sched_request_set_token_aliases(
            request: *mut c_void,
            bytes: *const u8,
            len: usize,
        ) -> i32;
        fn bennett_sched_request_free(request: *mut c_void);
    }
    unsafe {
        let mut request = ptr::null_mut();
        assert_eq!(bennett_sched_request_new(&mut request), 0);
        for labels in ["[]", "[0]", "[0,0]", "[0,1]", "[0,1,0]"] {
            assert_eq!(
                bennett_sched_request_set_token_aliases(request, labels.as_ptr(), labels.len()),
                0,
                "{labels}"
            );
        }
        for labels in ["[1]", "[0,2]", "[0,-1]", "[0,\"0\"]", "[0, 1]", "null"] {
            assert_eq!(
                bennett_sched_request_set_token_aliases(request, labels.as_ptr(), labels.len()),
                11,
                "{labels}"
            );
        }
        bennett_sched_request_free(request);
    }
}
