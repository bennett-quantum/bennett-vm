//! Hand-derived ISA 0.8 §12 witnesses; no reference implementation code.
use bennett_vm::{
    certificate::{self, Verdict},
    json::{self, Json},
    value::Budget,
};
use std::path::Path;
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
fn obj() -> bennett_vm::object::Object {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures/registers.bisa");
    bennett_vm::load(&std::fs::read_to_string(p).unwrap(), false)
        .unwrap()
        .0
}
fn claim(kind: &str, hi: u64, table: &str) -> Json {
    j(&format!(
        r#"{{"kind":"{kind}","domain":{{"bundles":[{{"name":"t","width":1,"lo":0,"hi":{hi}}}],"borrowed":[],"regions":[]}},"expected":{{"table":{table}}},"sampler":{{"algorithm":"xoshiro256**","seed":["1","2","3","4"],"count":8}}}}"#
    ))
}
#[test]
fn full_tuple_and_domain_labels() {
    // §12.1: compare full tuples; §12.2: proof requires domain equal D.
    let o = obj();
    let c = claim("exhaustive", 1, "[[1],[0]]");
    assert_eq!(
        certificate::verify_claim(
            &o,
            "toggle",
            &c,
            &[],
            &Budget {
                steps: 3,
                heap: 0,
                stack: 0,
                depth: 0
            },
            &[]
        )
        .unwrap(),
        Verdict::Proof
    );
    let c = claim("exhaustive", 0, "[[1]]");
    assert!(matches!(
        certificate::verify_claim(
            &o,
            "toggle",
            &c,
            &[],
            &Budget {
                steps: 3,
                heap: 0,
                stack: 0,
                depth: 0
            },
            &[]
        )
        .unwrap(),
        Verdict::Evidence { .. }
    ));
    let c = claim("exhaustive", 1, "[[1],[1]]");
    assert!(
        certificate::verify_claim(
            &o,
            "toggle",
            &c,
            &[],
            &Budget {
                steps: 3,
                heap: 0,
                stack: 0,
                depth: 0
            },
            &[]
        )
        .unwrap_err()
        .contains("full tuple")
    );
}
#[test]
fn sampled_is_always_evidence_and_rejects_zero_seed() {
    // §12.1: replay xoshiro256**; sampled is evidence even if it happens to cover D.
    let o = obj();
    let c = claim("sampled", 1, "[[1],[0]]");
    assert!(matches!(
        certificate::verify_claim(
            &o,
            "toggle",
            &c,
            &[],
            &Budget {
                steps: 3,
                heap: 0,
                stack: 0,
                depth: 0
            },
            &[]
        )
        .unwrap(),
        Verdict::Evidence { .. }
    ));
    let c = j(&c
        .to_jcs()
        .replace(r#"["1","2","3","4"]"#, r#"["0","0","0","0"]"#));
    assert!(
        certificate::verify_claim(
            &o,
            "toggle",
            &c,
            &[],
            &Budget {
                steps: 3,
                heap: 0,
                stack: 0,
                depth: 0
            },
            &[]
        )
        .is_err()
    );
}
#[test]
fn executable_claims_require_real_bound_schedule() {
    // §12.1: schedule_hash required; §11: reject a library not implemented.
    let o = obj();
    for kind in ["numerical", "channel"] {
        let c = j(&format!(
            r#"{{"kind":"{kind}","subject":{{}},"bound":"0","precision_bits":64}}"#
        ));
        assert!(
            certificate::verify_claim(
                &o,
                "toggle",
                &c,
                &[],
                &Budget {
                    steps: 3,
                    heap: 0,
                    stack: 0,
                    depth: 0
                },
                &[]
            )
            .unwrap_err()
            .contains("schedule_hash")
        );
    }
}

#[test]
fn oracle_tuple_and_inverse_pair_totality() {
    // §12.1 table replay is independent of target scheduling. §4.5 requires both
    // compositions, with FAULT_NONE on every input (not only the current basis).
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let raw = std::fs::read_to_string(
        root.join("bennett-isa-spec/conformance/cases/objects/t3_domains.bisa"),
    )
    .unwrap();
    let o = bennett_vm::load(&raw, false).unwrap().0;
    let mut c = j(
        r#"{"kind":"exhaustive","domain":{"bundles":[{"name":"x","width":2,"lo":0,"hi":3},{"name":"t","width":2,"lo":0,"hi":3},{"name":"f","width":4,"lo":0,"hi":15}],"borrowed":[],"regions":[]},"expected":{"table":[]}}"#,
    );
    let table = (0..4)
        .flat_map(|x| {
            (0..4).flat_map(move |t| {
                (0..16).map(move |f| {
                    Json::Arr(vec![
                        Json::u64(x),
                        Json::u64(t ^ ((x + 1) % 4)),
                        Json::u64(f),
                    ])
                })
            })
        })
        .collect();
    if let Json::Obj(m) = c.get("expected").unwrap().clone() {
        let _ = m;
    }
    if let Json::Obj(m) = &mut c {
        m.insert(
            "expected".into(),
            Json::obj(vec![("table", Json::Arr(table))]),
        );
    }
    // Local parameter names come from the published object, not guessed aliases.
    let f = o.function("inc2").unwrap();
    if let Json::Obj(m) = &mut c {
        let mut d = m["domain"].clone();
        if let Json::Obj(dm) = &mut d
            && let Json::Arr(bs) = dm.get_mut("bundles").unwrap()
        {
            for (b, p) in bs.iter_mut().zip(f.lists[..3].iter().flatten()) {
                if let Json::Obj(bm) = b {
                    bm.insert("name".into(), Json::Str(p.name.clone()));
                }
            }
        }
        m.insert("domain".into(), d);
    }
    let b = Budget {
        steps: 3,
        heap: 0,
        stack: 0,
        depth: 0,
    };
    assert_eq!(
        certificate::verify_claim(&o, "inc2", &c, &[], &b, &[]).unwrap(),
        Verdict::Proof
    );
    assert!(
        certificate::verify_inverse_pair(&o, "inc2", "inc2", &b)
            .unwrap_err()
            .contains("every input")
    );
}

#[test]
fn verification_references_and_executor_do_not_trust_producer() {
    // §12.3: oracle/borrow_frames/inverse_pairs are checked claim indices.
    // A sampled claim cannot discharge a proof obligation, even over all D.
    let o = obj();
    let c = claim("exhaustive", 1, "[[1],[0]]");
    let region = Json::obj(vec![
        ("claims", Json::Arr(vec![c])),
        (
            "verification",
            j(r#"{"oracle":0,"borrow_frames":[],"inverse_pairs":[],"unitary":[],"channel":[]}"#),
        ),
    ]);
    let b = Budget {
        steps: 3,
        heap: 0,
        stack: 0,
        depth: 0,
    };
    assert_eq!(
        certificate::verify_region_claims(&o, "toggle", &region, &[], &b, &[]).unwrap(),
        vec![Verdict::Proof]
    );
    let wrong = j(&region.to_jcs().replace(r#""oracle":0"#, r#""oracle":99"#));
    assert!(
        certificate::verify_region_claims(&o, "toggle", &wrong, &[], &b, &[])
            .unwrap_err()
            .contains("index")
    );
    let adjoint_region = Json::obj(vec![
        (
            "claims",
            Json::Arr(vec![j(
                r#"{"kind":"structural","rule":"adjoint_table","witness":{}}"#,
            )]),
        ),
        ("verification", j(r#"{"oracle":0}"#)),
    ]);
    assert!(
        certificate::verify_region_claims(&o, "toggle", &adjoint_region, &[], &b, &[])
            .unwrap_err()
            .contains("different obligation")
    );
    let mut raw = o.text.clone();
    // A cert claiming a different object must be rejected before any gates run.
    raw.push_str(&format!("\nsection certificate\n{}\n",j(r#"{"versions":{},"producer":{},"composition":{},"bindings":{"object_hash":"wrong"},"regions":{}}"#).to_jcs()));
    let o = bennett_vm::load(&raw, false).unwrap().0;
    let mut state = bennett_vm::action::QuantumState::new();
    let wires = state.alloc_wires(1);
    let entry = state.clone();
    assert!(
        bennett_vm::action::execute_action_inner(
            &o,
            &mut state,
            "toggle",
            &[wires],
            &[],
            &[],
            &[],
            false,
            &[]
        )
        .unwrap_err()
        .contains("object_hash")
    );
    assert!(state.exact_equals(&entry));
}
#[test]
fn expected_program_resolves_and_library_claims_replay() {
    // §12.1: expected program is resolved by closure hash; local names irrelevant.
    let o = obj();
    let hash = bennett_vm::identity::compute(&o).unwrap().closure["toggle"]
        .clone()
        .unwrap();
    let mut c = claim("exhaustive", 1, "[[1],[0]]");
    if let Json::Obj(m) = &mut c {
        m.insert(
            "expected".into(),
            Json::obj(vec![("program", Json::Str(hash.clone()))]),
        );
    }
    let b = Budget {
        steps: 3,
        heap: 0,
        stack: 0,
        depth: 0,
    };
    assert_eq!(
        certificate::verify_claim(&o, "toggle", &c, &[], &b, &[]).unwrap(),
        Verdict::Proof
    );
    let c = j(&c.to_jcs().replace(&hash, &"a".repeat(64)));
    assert!(
        certificate::verify_claim(&o, "toggle", &c, &[], &b, &[])
            .unwrap_err()
            .contains("unresolved")
    );
}

#[test]
fn translation_witness_dirty_domain_and_library_replay() {
    // §5.6: V=cx t[0] b; W=cx b t[1]; V^-1; W. The map is
    // (t0,t1,b) -> (t0,t1 xor t0,b), for all dirty b, independent of b.
    let raw = obj().text.replace(
        "h t[0]\ncx t[0] t[1]",
        "b = qborrow any 1\ncx t[0] b\ncx b t[1]\ncx t[0] b\ncx b t[1]\nqreturn b",
    );
    let o = bennett_vm::load(&raw, true).unwrap().0;
    let c = j(
        r#"{"kind":"structural","rule":"translation_lemma","subclaims":[],"witness":{"V":["entry:1"],"W":["entry:2"],"X":["t[0]"],"B":["b"],"frame":["entry:1","entry:4"]}}"#,
    );
    let interface = bennett_vm::identity::compute(&o).unwrap().interface["bell"].clone();
    let mut c = c;
    if let Json::Obj(m) = &mut c
        && let Json::Obj(w) = m.get_mut("witness").unwrap()
    {
        for key in ["V", "W", "frame"] {
            if let Json::Arr(ps) = w.get_mut(key).unwrap() {
                for p in ps {
                    let (_, n) = p.as_str().unwrap().split_once(':').unwrap();
                    *p = Json::Arr(vec![
                        Json::Str(interface.clone()),
                        Json::Str("entry".into()),
                        Json::u64(n.parse().unwrap()),
                    ]);
                }
            }
        }
    }
    let b = Budget {
        steps: 3,
        heap: 0,
        stack: 0,
        depth: 0,
    };
    assert_eq!(
        certificate::verify_claim(&o, "bell", &c, &[], &b, &[]).unwrap(),
        Verdict::Proof
    );
    let wrong = j(&c.to_jcs().replace(r#""entry",4"#, r#""entry",3"#));
    assert!(certificate::verify_claim(&o, "bell", &wrong, &[], &b, &[]).is_err());
    let adjoint = j(r#"{"kind":"structural","rule":"adjoint_table","subclaims":[],"witness":{}}"#);
    assert_eq!(
        certificate::verify_claim(&o, "bell", &adjoint, &[], &b, &[]).unwrap(),
        Verdict::Proof
    );
    let hash = bennett_vm::identity::compute(&o).unwrap().closure["bell"]
        .clone()
        .unwrap();
    let cert = j(&format!(
        r#"{{"versions":{{}},"bindings":{{}},"producer":{{}},"composition":{{}},"regions":{{"{hash}":{{"claims":[{}]}}}}}}"#,
        c.to_jcs()
    ));
    let mut library = o.clone();
    library
        .text
        .push_str(&format!("\nsection certificate\n{}\n", cert.to_jcs()));
    library.has_certificate = true;
    let c = Json::obj(vec![
        ("kind", Json::Str("structural".into())),
        ("rule", Json::Str("inverse_pair_library".into())),
        (
            "witness",
            Json::obj(vec![
                ("library_closure_hash", Json::Str(hash)),
                ("claim_index", Json::u64(0)),
            ]),
        ),
    ]);
    assert_eq!(
        certificate::verify_claim(&o, "bell", &c, &[], &b, &[library]).unwrap(),
        Verdict::Proof
    );
    let d = j(
        r#"{"kind":"exhaustive","domain":{"bundles":[{"name":"t","width":2,"lo":0,"hi":3}],"borrowed":[{"name":"b","width":1,"lo":0,"hi":1}],"regions":[]},"expected":{"table":[[0,0],[0,1],[3,0],[3,1],[2,0],[2,1],[1,0],[1,1]]}}"#,
    );
    assert_eq!(
        certificate::verify_claim(&o, "bell", &d, &[], &b, &[]).unwrap(),
        Verdict::Proof
    );
}

#[test]
fn bound_executable_claim_refuses_unpublished_library() {
    // §12.1 subject binds the actual root schedule; §11 requires rejection of a
    // library the consumer does not implement, even for an integer-only toggle.
    let mut o = obj();
    let hash = bennett_vm::identity::compute(&o).unwrap().closure["toggle"]
        .clone()
        .unwrap();
    let schedule = j(r#"{"bindings":{"expansion":{"name":"ref","version":"0.8"}},"entries":[]}"#);
    o.text
        .push_str(&format!("\nsection schedule\n{}\n", schedule.to_jcs()));
    o.has_schedule = true;
    let schedule_hash = bennett_vm::sha256::sha256_hex(schedule.to_jcs().as_bytes());
    for kind in ["numerical", "channel", "exhaustive"] {
        let c = j(&format!(
            r#"{{"kind":"{kind}","expected":"oracle","subject":{{"closure_hash":"{hash}","variant":"forward","inherited_controls":0,"schedule_hash":"{schedule_hash}"}}}}"#
        ));
        let e =
            certificate::verify_claim(&o, "toggle", &c, &[], &Budget::default(), &[]).unwrap_err();
        assert!(e.contains("b-isa-spec-8r4") && e.contains("§11"), "{e}");
    }
}

#[test]
fn sampler_word_order_singletons_and_unbound_sampled_relation() {
    // §12.1: xoshiro256**, m consecutive words least-significant first;
    // ceil(log2(1))=0 so a singleton consumes no generator words.
    use bennett_vm::{bigint::BigUint as U, certificate::Xoshiro256};
    let mut rng = Xoshiro256([1, 2, 3, 4]);
    assert_eq!(
        rng.range(&U::from_u64(7), &U::from_u64(7)).unwrap(),
        U::from_u64(7)
    );
    assert_eq!(rng.word(), 11520);
    assert_eq!(rng.word(), 0);
    assert_eq!(rng.word(), 1509978240);
    let mut rng = Xoshiro256([1, 2, 3, 4]);
    let lo = U::from_u64(1).shl(80);
    let hi = lo.add(&U::from_u64(1).shl(65)).sub(&U::from_u64(1));
    assert_eq!(rng.range(&lo, &hi).unwrap(), lo.add(&U::from_u64(11520)));
    let mut c = claim("sampled", 1, "[[1],[0]]");
    if let Json::Obj(m) = &mut c {
        m.remove("expected");
    }
    let e =
        certificate::verify_claim(&obj(), "toggle", &c, &[], &Budget::default(), &[]).unwrap_err();
    assert!(e.contains("bennett-vm-clw") && e.contains("§12.1"));
}

#[test]
fn qinplace_checks_unobserved_inputs_before_mutating_state() {
    // §4.5 every input is required. A zero step budget faults universally; the
    // checker rejects before even allocating the instruction's target workspace.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let o = bennett_vm::load(
        &std::fs::read_to_string(
            root.join("bennett-isa-spec/conformance/cases/objects/qinplace.bisa"),
        )
        .unwrap(),
        false,
    )
    .unwrap()
    .0;
    let b = Budget {
        steps: 3,
        heap: 0,
        stack: 0,
        depth: 0,
    };
    assert_eq!(
        certificate::verify_inverse_pair(&o, "inc2", "dec2", &b).unwrap(),
        Verdict::Proof
    );
    let mut s = bennett_vm::action::QuantumState::new();
    let wires = s.alloc_wires(2);
    let entry = s.clone();
    let e = bennett_vm::action::execute_action_inner(
        &o,
        &mut s,
        "inc_inplace",
        &[wires],
        &[],
        &[],
        &[bennett_vm::value::Value::Token(Budget::default())],
        false,
        &[],
    )
    .unwrap_err();
    assert!(e.contains("every input") && e.contains("§4.5"));
    assert!(s.exact_equals(&entry));
    assert_eq!(s.wire_count(), 2);
}
