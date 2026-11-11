//! Published launch-estimate identity and explicit-accuracy vectors.
use bennett_vm::{
    estimate::{AccuracyRequest, ExplicitAccuracy, ResolvedRequest},
    invocation,
    json::{self, Json},
    target::{Rational, Target},
    value::Budget,
};
fn read(relative: &str) -> Json {
    json::parse(&std::fs::read_to_string(crate::pins::isa().join(relative)).unwrap()).unwrap()
}
fn oracle() -> (bennett_vm::object::Object, bennett_vm::identity::Identities) {
    let source = crate::source09::source(
        "oracle @o (x:wire in) (t:wire) (f:wire[4]) () (a:token, b:token)\n^entry():\nlift.retval x",
    );
    bennett_vm::load(&source, false).unwrap()
}
fn target() -> Target {
    Target::parse(&read("conformance/cases/E/E-matching-first-entry/target.json").to_jcs()).unwrap()
}
fn request(
    object: &bennett_vm::object::Object,
    budget: Budget,
    args: Json,
    aliases: Json,
) -> ResolvedRequest {
    ResolvedRequest::new(object, "o", "forward", 3, budget, args, Some(aliases)).unwrap()
}
fn native_target() -> Target {
    let mut value = target().document;
    let Json::Obj(ref mut fields) = value else {
        unreachable!()
    };
    let Json::Arr(gates) = fields.get_mut("gates").unwrap() else {
        unreachable!()
    };
    gates.push(json::parse(r#"{"classes":["*"],"controls":{"max":"unbounded","min":0},"cost":{"time":1},"epsilon":0,"op":"x"}"#).unwrap());
    Target::parse(&value.to_jcs()).unwrap()
}
#[test]
fn published_args_only_hash_and_separately_resolved_invocations() {
    let vector = read("conformance/cases/G/G-estimate-parameter-hash-09/binding.json");
    let (object, _) = oracle();
    let budget = invocation::decode_budget(vector.get("budget").unwrap()).unwrap();
    let args = vector.get("args").unwrap().clone();
    let aliases = vector.get("token_aliases").unwrap().as_arr().unwrap();
    let equal = request(&object, budget, args.clone(), aliases[0].clone());
    let distinct = request(&object, budget, args.clone(), aliases[1].clone());
    assert_eq!(
        equal.params_hash(),
        vector.get("params_hash").unwrap().as_str().unwrap()
    );
    assert_eq!(equal.params_hash(), distinct.params_hash());
    assert_ne!(equal.invocation(), distinct.invocation());
    for partition in ["[0,0,0]", "[0,0,1]", "[0,1,0]", "[0,1,1]", "[0,1,2]"] {
        let partition = json::parse(partition).unwrap();
        let resolved = request(&object, budget, args.clone(), partition.clone());
        assert_eq!(resolved.params_hash(), equal.params_hash());
        assert_eq!(resolved.token_aliases(), &partition);
    }
    assert!(
        ResolvedRequest::new(
            &object,
            "o",
            "forward",
            3,
            budget,
            args.clone(),
            Some(json::parse("[0,1]").unwrap())
        )
        .is_err()
    );

    let outer = request(
        &object,
        Budget { steps: 3, ..budget },
        args.clone(),
        aliases[1].clone(),
    );
    assert_eq!(outer.params_hash(), equal.params_hash());
    assert_ne!(outer.invocation(), distinct.invocation());
    let mut changed = args.clone();
    let Json::Arr(ref mut values) = changed else {
        unreachable!()
    };
    let Json::Arr(ref mut tuple) = values[1] else {
        unreachable!()
    };
    tuple[0] = Json::Str("3".into());
    let changed = request(&object, budget, changed, aliases[1].clone());
    assert_ne!(changed.params_hash(), equal.params_hash());
    assert_eq!(equal.args().len(), 2);
    assert_eq!(equal.budget(), budget);
}
#[test]
fn explicit_accuracy_is_exact_owned_and_not_resource_display_rounded() {
    let mut input = "0.123456789012345678901234567890123456789".to_string();
    let accuracy = ExplicitAccuracy::parse(&input).unwrap();
    input.clear();
    assert_eq!(
        accuracy.source(),
        "0.123456789012345678901234567890123456789"
    );
    assert_eq!(accuracy.decimal(), accuracy.source());
    assert_ne!(
        accuracy.decimal(),
        accuracy.value().upper_decimal34().unwrap()
    );
    for (text, canonical) in [
        ("0", "0"),
        ("-0", "0"),
        ("0.10", "0.1"),
        ("1e-1", "0.1"),
        ("2.500", "2.5"),
    ] {
        assert_eq!(ExplicitAccuracy::parse(text).unwrap().decimal(), canonical);
    }
    for bad in [
        "", " ", "NaN", "nan", "Infinity", "inf", "-0.1", "1/3", "0x01", "+0.1", "e1", "1e",
        "1.2.3", "0.1 ", "é",
    ] {
        assert!(ExplicitAccuracy::parse(bad).is_err(), "{bad:?}");
    }
}
#[test]
fn published_accuracy_controls_schedule_reuse_and_exact_default_shares() {
    let vector = read("conformance/cases/G/G-estimate-root-accuracy-09/accuracy.json");
    let binding = read("conformance/cases/G/G-estimate-parameter-hash-09/binding.json");
    let (object, ids) = oracle();
    let target = target();
    let req = request(
        &object,
        invocation::decode_budget(binding.get("budget").unwrap()).unwrap(),
        binding.get("args").unwrap().clone(),
        binding.get("token_aliases").unwrap().as_arr().unwrap()[1].clone(),
    );
    let accuracy =
        ExplicitAccuracy::parse(vector.get("request_epsilon").unwrap().as_str().unwrap()).unwrap();
    let req = AccuracyRequest::new(req, accuracy);
    let default = req
        .effective_schedule(&object, &ids, &target, None)
        .unwrap();
    assert!(!default.uses_attached());
    assert_eq!(
        default
            .document()
            .get("entries")
            .unwrap()
            .as_arr()
            .unwrap()
            .len(),
        0
    );
    for case in vector.get("cases").unwrap().as_arr().unwrap() {
        let mut attached = default.document().clone();
        let descriptor = case.get("schedule").unwrap();
        let given = if descriptor == &Json::Null {
            None
        } else {
            let Json::Obj(ref mut fields) = attached else {
                unreachable!()
            };
            let Json::Obj(bindings) = fields.get_mut("bindings").unwrap() else {
                unreachable!()
            };
            bindings.insert("epsilon".into(), descriptor.get("epsilon").unwrap().clone());
            if descriptor.get("other_bindings_match") == Some(&Json::Bool(false)) {
                bindings.insert("params_hash".into(), Json::Str("f".repeat(64)));
            }
            Some(&attached)
        };
        let effective = req
            .effective_schedule(&object, &ids, &target, given)
            .unwrap();
        assert_eq!(
            effective.uses_attached(),
            case.get("effective").unwrap().as_str() == Some("attached")
        );
        assert_eq!(
            req.accuracy().value(),
            &case
                .get("epsilon")
                .unwrap()
                .as_str()
                .unwrap()
                .parse::<Rational>()
                .unwrap()
        );
        let bindings = effective.document().get("bindings").unwrap();
        assert_eq!(
            bindings.get("params_hash").unwrap().as_str(),
            Some(req.request().params_hash())
        );
        assert_eq!(
            bindings
                .get("epsilon")
                .unwrap()
                .as_str()
                .unwrap()
                .parse::<Rational>()
                .unwrap(),
            *req.accuracy().value()
        );
        if effective.uses_attached() {
            assert_eq!(effective.document(), given.unwrap());
            assert_eq!(
                effective.hash(),
                bennett_vm::sha256::sha256_hex(given.unwrap().to_jcs().as_bytes())
            );
        } else {
            assert_eq!(effective.document(), default.document());
        }
    }
    let share = req
        .accuracy()
        .default_share(
            vector
                .get("inexact_operations")
                .unwrap()
                .as_usize()
                .unwrap(),
            false,
        )
        .unwrap();
    assert_eq!(share.fraction(), ("1".into(), "30".into()));
    assert!(req.accuracy().default_share(3, true).unwrap().is_zero());
    assert!(
        ExplicitAccuracy::parse("0")
            .unwrap()
            .default_share(0, false)
            .unwrap()
            .is_zero()
    );
    assert_eq!(
        ExplicitAccuracy::parse("10")
            .unwrap()
            .default_share(3, false)
            .unwrap()
            .fraction(),
        ("10".into(), "3".into())
    );
}
#[test]
fn feature_identifier_is_known_but_incomplete_estimation_is_not_advertised() {
    assert_eq!(
        bennett_vm::abi_features::ESTIMATE_ACCURACY_1,
        "estimate_accuracy/1"
    );
    unsafe {
        let mut bytes = std::ptr::null();
        let mut len = 0;
        assert_eq!(
            bennett_vm::abi_features::bennett_launch_features(&mut bytes, &mut len),
            0
        );
        let manifest =
            json::parse(std::str::from_utf8(std::slice::from_raw_parts(bytes, len)).unwrap())
                .unwrap();
        assert!(
            !manifest
                .get("features")
                .unwrap()
                .as_arr()
                .unwrap()
                .iter()
                .any(|v| v.as_str() == Some("estimate_accuracy/1"))
        );
    }
}
#[test]
fn additive_abi_rejects_invalid_explicit_accuracy_before_any_estimate() {
    use bennett_vm::launch::*;
    use std::{ffi::CString, ptr};
    let raw = crate::source09::source("perm_action @p (t:wire mut) () () () ()\n^entry():\nx t");
    unsafe {
        let mut program = ptr::null_mut();
        assert_eq!(
            bennett_load(raw.as_ptr(), raw.len(), ptr::null(), 0, &mut program),
            0
        );
        let mut token = 0;
        assert_eq!(
            bennett_budget(program, c"p".as_ptr(), 1, 0, 0, 0, &mut token),
            0
        );
        for input in [
            None,
            Some(""),
            Some("NaN"),
            Some("Infinity"),
            Some("-0.1"),
            Some("0.1 "),
        ] {
            let value = input.map(|s| CString::new(s).unwrap());
            let mut out = ptr::dangling_mut();
            let code = bennett_estimate_run_at_accuracy(
                program,
                c"p".as_ptr(),
                token,
                c"forward".as_ptr(),
                0,
                b"[]".as_ptr(),
                2,
                b"[]".as_ptr(),
                2,
                value.as_ref().map_or(ptr::null(), |v| v.as_ptr()),
                &mut out,
            );
            assert_eq!(code, 11, "{input:?}");
            assert!(out.is_null());
        }
        for input in [c"0", c"0.1", c"2.5"] {
            let mut out = ptr::dangling_mut();
            assert_eq!(
                bennett_estimate_run_at_accuracy(
                    program,
                    c"p".as_ptr(),
                    token,
                    c"forward".as_ptr(),
                    0,
                    b"[]".as_ptr(),
                    2,
                    b"[]".as_ptr(),
                    2,
                    input.as_ptr(),
                    &mut out
                ),
                5
            );
            assert!(out.is_null());
        }
        assert_eq!(bennett_unload(program), 0);
    }
}

#[test]
fn token_free_attached_schedule_may_use_the_published_implicit_partition() {
    let raw = crate::source09::source("perm_action @p (t:wire mut) () () () ()\n^entry():\nx t");
    let (object, ids) = bennett_vm::load(&raw, false).unwrap();
    let request = ResolvedRequest::new(
        &object,
        "p",
        "forward",
        0,
        Budget {
            steps: 1,
            heap: 0,
            stack: 0,
            depth: 0,
        },
        json::parse("[]").unwrap(),
        None,
    )
    .unwrap();
    let request = AccuracyRequest::new(request, ExplicitAccuracy::parse("0.1").unwrap());
    let target = target();
    let default = request
        .effective_schedule(&object, &ids, &target, None)
        .unwrap();
    let mut attached = default.document().clone();
    let Json::Obj(ref mut fields) = attached else {
        unreachable!()
    };
    let Json::Obj(bindings) = fields.get_mut("bindings").unwrap() else {
        unreachable!()
    };
    bindings.remove("token_aliases");
    assert!(
        request
            .effective_schedule(&object, &ids, &target, Some(&attached))
            .unwrap()
            .uses_attached()
    );
}

#[test]
fn explicit_entry_preserves_all_partitions_and_legacy_unavailable_behavior() {
    use bennett_vm::launch::*;
    use std::ptr;
    let raw = crate::source09::source(
        "perm_action @p (t:wire mut) () () (a:token, b:token, c:token) ()\n^entry():\nx t",
    );
    let mut target = target().document;
    let Json::Obj(ref mut fields) = target else {
        unreachable!()
    };
    let Json::Arr(gates) = fields.get_mut("gates").unwrap() else {
        unreachable!()
    };
    gates.push(json::parse(r#"{"classes":["*"],"controls":{"max":"unbounded","min":0},"cost":{"time":1},"epsilon":0,"op":"x"}"#).unwrap());
    let target = target.to_jcs();
    let vector = read("conformance/cases/G/G-estimate-parameter-hash-09/binding.json");
    let mut args = vector.get("args").unwrap().as_arr().unwrap().clone();
    args.push(args[0].clone());
    let params = Json::Arr(args).to_jcs();
    unsafe {
        let mut program = ptr::null_mut();
        assert_eq!(
            bennett_load(
                raw.as_ptr(),
                raw.len(),
                target.as_ptr(),
                target.len(),
                &mut program
            ),
            0,
            "{}",
            std::ffi::CStr::from_ptr(bennett_last_error()).to_string_lossy()
        );
        let mut token = 0;
        assert_eq!(
            bennett_budget(program, c"p".as_ptr(), 2, 0, 0, 0, &mut token),
            0
        );
        for aliases in ["[0,0,0]", "[0,0,1]", "[0,1,0]", "[0,1,1]", "[0,1,2]"] {
            let mut out = ptr::dangling_mut();
            assert_eq!(
                bennett_estimate_run_at_accuracy(
                    program,
                    c"p".as_ptr(),
                    token,
                    c"forward".as_ptr(),
                    3,
                    params.as_ptr(),
                    params.len(),
                    aliases.as_ptr(),
                    aliases.len(),
                    c"0.1".as_ptr(),
                    &mut out
                ),
                1
            );
            assert!(out.is_null());
            assert_eq!(
                bennett_estimate_run_with_token_aliases(
                    program,
                    c"p".as_ptr(),
                    token,
                    c"forward".as_ptr(),
                    3,
                    params.as_ptr(),
                    params.len(),
                    aliases.as_ptr(),
                    aliases.len(),
                    &mut out
                ),
                11
            );
            assert_eq!(
                std::ffi::CStr::from_ptr(bennett_last_error())
                    .to_str()
                    .unwrap(),
                bennett_vm::estimate::MISSING_ACCURACY_DIAGNOSTIC
            );
            assert!(out.is_null());
        }
        let mut out = ptr::dangling_mut();
        assert_eq!(
            bennett_estimate_run_at_accuracy(
                program,
                c"p".as_ptr(),
                token,
                c"forward".as_ptr(),
                3,
                params.as_ptr(),
                params.len(),
                b"[0,1]".as_ptr(),
                5,
                c"0.1".as_ptr(),
                &mut out
            ),
            11
        );
        assert!(out.is_null());
        assert_eq!(bennett_unload(program), 0);
    }
}

#[test]
fn every_other_request_binding_is_required_for_schedule_reuse() {
    let vector = read("conformance/cases/G/G-estimate-parameter-hash-09/binding.json");
    let (object, ids) = oracle();
    let target = target();
    let request = request(
        &object,
        invocation::decode_budget(vector.get("budget").unwrap()).unwrap(),
        vector.get("args").unwrap().clone(),
        vector.get("token_aliases").unwrap().as_arr().unwrap()[1].clone(),
    );
    let request = AccuracyRequest::new(request, ExplicitAccuracy::parse("0.1").unwrap());
    let default = request
        .effective_schedule(&object, &ids, &target, None)
        .unwrap();
    assert!(
        request
            .effective_schedule(&object, &ids, &target, Some(default.document()))
            .unwrap()
            .uses_attached()
    );
    for (field, value) in [
        ("closure_hash", Json::Str("f".repeat(64))),
        ("target_hash", Json::Str("f".repeat(64))),
        ("region", Json::Str("different".into())),
        ("variant", Json::Str("different".into())),
        ("inherited_controls", Json::u64(4)),
        ("budget", json::parse(r#"["3","0","0","0"]"#).unwrap()),
        ("params_hash", Json::Str("f".repeat(64))),
        ("token_aliases", json::parse("[0,0,0]").unwrap()),
    ] {
        let mut attached = default.document().clone();
        let Json::Obj(ref mut fields) = attached else {
            unreachable!()
        };
        let Json::Obj(bindings) = fields.get_mut("bindings").unwrap() else {
            unreachable!()
        };
        bindings.insert(field.into(), value);
        let effective = request
            .effective_schedule(&object, &ids, &target, Some(&attached))
            .unwrap();
        assert!(!effective.uses_attached(), "{field}");
        assert_eq!(effective.document(), default.document(), "{field}");
    }
}

#[test]
fn published_legacy_accuracy_resolution_never_invents_a_root_value() {
    let vector = read("conformance/cases/G/G-estimate-root-accuracy-09/accuracy.json");
    let binding = read("conformance/cases/G/G-estimate-parameter-hash-09/binding.json");
    let diagnostic = vector
        .get("legacy_missing_accuracy")
        .unwrap()
        .get("diagnostic")
        .unwrap()
        .as_str()
        .unwrap();
    let (object, ids) = oracle();
    let target = target();
    let request = request(
        &object,
        invocation::decode_budget(binding.get("budget").unwrap()).unwrap(),
        binding.get("args").unwrap().clone(),
        binding.get("token_aliases").unwrap().as_arr().unwrap()[1].clone(),
    );
    let template = AccuracyRequest::new(
        request.clone(),
        ExplicitAccuracy::parse(vector.get("request_epsilon").unwrap().as_str().unwrap()).unwrap(),
    )
    .effective_schedule(&object, &ids, &target, None)
    .unwrap();
    for case in vector.get("legacy_cases").unwrap().as_arr().unwrap() {
        let descriptor = case.get("schedule").unwrap();
        let mut document = template.document().clone();
        let attached = if descriptor == &Json::Null {
            None
        } else {
            let Json::Obj(ref mut fields) = document else {
                unreachable!()
            };
            let Json::Obj(bindings) = fields.get_mut("bindings").unwrap() else {
                unreachable!()
            };
            bindings.insert("epsilon".into(), descriptor.get("epsilon").unwrap().clone());
            if descriptor.get("other_bindings_match") == Some(&Json::Bool(false)) {
                bindings.insert("params_hash".into(), Json::Str("f".repeat(64)));
            }
            Some(&document)
        };
        let accuracy = request.legacy_accuracy(&object, &ids, Some(&target), attached);
        if case.get("resolution").unwrap().as_str() == Some("missing") {
            assert_eq!(accuracy.unwrap_err(), diagnostic);
        } else {
            let accuracy = accuracy.unwrap();
            assert_eq!(
                accuracy.source(),
                case.get("epsilon").unwrap().as_str().unwrap()
            );
            let effective = AccuracyRequest::new(request.clone(), accuracy)
                .effective_schedule(&object, &ids, &target, attached)
                .unwrap();
            assert!(effective.uses_attached());
            assert_eq!(effective.document(), attached.unwrap());
            assert_eq!(
                effective.hash(),
                bennett_vm::sha256::sha256_hex(attached.unwrap().to_jcs().as_bytes())
            );
        }
    }
}
#[test]
fn legacy_entry_without_schedule_returns_the_exact_published_diagnostic() {
    use bennett_vm::launch::*;
    use std::{ffi::CStr, ptr};
    let vector = read("conformance/cases/G/G-estimate-root-accuracy-09/accuracy.json");
    let diagnostic = vector
        .get("legacy_missing_accuracy")
        .unwrap()
        .get("diagnostic")
        .unwrap()
        .as_str()
        .unwrap();
    let raw =
        crate::source09::source("perm_action @p (t:wire mut) () () (n:i8) ()\n^entry():\nx t");
    unsafe {
        let mut program = ptr::null_mut();
        assert_eq!(
            bennett_load(raw.as_ptr(), raw.len(), ptr::null(), 0, &mut program),
            0
        );
        let mut token = 0;
        assert_eq!(
            bennett_budget(program, c"p".as_ptr(), 1, 0, 0, 0, &mut token),
            0
        );
        let params = br#"["1"]"#;
        let mut out = ptr::dangling_mut();
        assert_eq!(
            bennett_estimate_run_0_9(
                program,
                c"p".as_ptr(),
                token,
                c"forward".as_ptr(),
                0,
                params.as_ptr(),
                params.len(),
                &mut out
            ),
            11
        );
        assert!(out.is_null());
        assert_eq!(
            CStr::from_ptr(bennett_last_error()).to_str().unwrap(),
            diagnostic
        );
        out = ptr::dangling_mut();
        assert_eq!(
            bennett_estimate_run_with_token_aliases(
                program,
                c"p".as_ptr(),
                token,
                c"forward".as_ptr(),
                0,
                params.as_ptr(),
                params.len(),
                b"[]".as_ptr(),
                2,
                &mut out
            ),
            11
        );
        assert!(out.is_null());
        assert_eq!(
            CStr::from_ptr(bennett_last_error()).to_str().unwrap(),
            diagnostic
        );
        out = ptr::dangling_mut();
        assert_eq!(
            bennett_estimate_run_0_9(
                program,
                c"p".as_ptr(),
                token,
                c"forward".as_ptr(),
                0,
                b"[]".as_ptr(),
                2,
                &mut out
            ),
            11
        );
        assert!(out.is_null());
        assert_ne!(
            CStr::from_ptr(bennett_last_error()).to_str().unwrap(),
            diagnostic
        ); // argument validity precedes accuracy resolution
        assert_eq!(bennett_unload(program), 0);
    }
}

#[test]
fn legacy_entries_use_only_matching_schedule_accuracy_at_the_abi_boundary() {
    use bennett_vm::launch::*;
    use std::{ffi::CStr, ptr};
    let vector = read("conformance/cases/G/G-estimate-root-accuracy-09/accuracy.json");
    let diagnostic = vector
        .get("legacy_missing_accuracy")
        .unwrap()
        .get("diagnostic")
        .unwrap()
        .as_str()
        .unwrap();
    let source = crate::source09::source(
        "perm_action @p (t:wire mut) () () (n:i8) ()\n^entry():\nx t\nfn cc @main (q:qreg[1], n:i8) () () () ()\n^entry():\naction_call @p (q) () () (n)\nret",
    );
    let (object, ids) = bennett_vm::load(&source, false).unwrap();
    let target = native_target();
    let params = json::parse(r#"["1"]"#).unwrap();
    let budget = Budget {
        steps: 1,
        heap: 0,
        stack: 0,
        depth: 0,
    };
    let request =
        ResolvedRequest::new(&object, "p", "forward", 0, budget, params.clone(), None).unwrap();
    let template = AccuracyRequest::new(
        request,
        ExplicitAccuracy::parse(vector.get("request_epsilon").unwrap().as_str().unwrap()).unwrap(),
    )
    .effective_schedule(&object, &ids, &target, None)
    .unwrap();
    for case in vector.get("legacy_cases").unwrap().as_arr().unwrap() {
        let mut raw = source.clone();
        let descriptor = case.get("schedule").unwrap();
        if descriptor != &Json::Null {
            let mut schedule = template.document().clone();
            let Json::Obj(ref mut fields) = schedule else {
                unreachable!()
            };
            let Json::Obj(bindings) = fields.get_mut("bindings").unwrap() else {
                unreachable!()
            };
            bindings.insert("epsilon".into(), descriptor.get("epsilon").unwrap().clone());
            if descriptor.get("other_bindings_match") == Some(&Json::Bool(false)) {
                bindings.insert("params_hash".into(), Json::Str("f".repeat(64)));
            }
            raw.push_str(&format!("\nsection schedule\n{}\n", schedule.to_jcs()));
        }
        unsafe {
            let target_bytes = target.document.to_jcs();
            let mut program = ptr::null_mut();
            assert_eq!(
                bennett_load(
                    raw.as_ptr(),
                    raw.len(),
                    target_bytes.as_ptr(),
                    target_bytes.len(),
                    &mut program
                ),
                0,
                "{}",
                CStr::from_ptr(bennett_last_error()).to_string_lossy()
            );
            let mut token = 0;
            assert_eq!(
                bennett_budget(program, c"p".as_ptr(), 1, 0, 0, 0, &mut token),
                0
            );
            let mut qreg = 0;
            assert_eq!(bennett_qreg_new(program, c"fast".as_ptr(), 1, &mut qreg), 0);
            for aliases in [false, true] {
                let params_bytes = params.to_jcs();
                let mut out = ptr::dangling_mut();
                let code = if aliases {
                    bennett_estimate_run_with_token_aliases(
                        program,
                        c"p".as_ptr(),
                        token,
                        c"forward".as_ptr(),
                        0,
                        params_bytes.as_ptr(),
                        params_bytes.len(),
                        b"[]".as_ptr(),
                        2,
                        &mut out,
                    )
                } else {
                    bennett_estimate_run_0_9(
                        program,
                        c"p".as_ptr(),
                        token,
                        c"forward".as_ptr(),
                        0,
                        params_bytes.as_ptr(),
                        params_bytes.len(),
                        &mut out,
                    )
                };
                assert!(out.is_null());
                if case.get("resolution").unwrap().as_str() == Some("missing") {
                    assert_eq!(code, 11);
                    assert_eq!(
                        CStr::from_ptr(bennett_last_error()).to_str().unwrap(),
                        diagnostic
                    );
                } else {
                    assert_eq!(
                        code,
                        1,
                        "{}",
                        CStr::from_ptr(bennett_last_error()).to_string_lossy()
                    );
                    assert!(
                        !CStr::from_ptr(bennett_last_error())
                            .to_str()
                            .unwrap()
                            .contains("E_ESTIMATE_ACCURACY_REQUIRED")
                    );
                }
            }
            if descriptor != &Json::Null {
                let mut args = ptr::null_mut();
                assert_eq!(bennett_args_new(2, &mut args), 0);
                assert_eq!(bennett_args_set_qreg(args, 0, qreg), 0);
                assert_eq!(bennett_args_set_int(args, 1, 8, [1u8].as_ptr()), 0);
                let mut run = u64::MAX;
                assert_eq!(
                    bennett_run(program, c"main".as_ptr(), args, ptr::null(), 0, &mut run),
                    10
                );
                assert_eq!(run, u64::MAX);
                bennett_args_free(args);
            }
            assert_eq!(bennett_qreg_free(program, qreg), 0); // no quantum effect from failed estimate requests
            assert_eq!(bennett_unload(program), 0);
        }
    }
}
