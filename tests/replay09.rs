use bennett_vm::{
    certificate::{self, Verdict},
    json::{self, Json},
};
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
fn object() -> bennett_vm::object::Object {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    bennett_vm::load(
        &std::fs::read_to_string(
            root.join("bennett-isa-spec/reference-interpreter/tests/fixtures/registers.bisa"),
        )
        .unwrap(),
        false,
    )
    .unwrap()
    .0
}
fn claim(o: &bennett_vm::object::Object, kind: &str) -> Json {
    let h = bennett_vm::identity::compute(o).unwrap().closure["toggle"]
        .clone()
        .unwrap();
    j(&format!(
        r#"{{"kind":"{kind}","subject":{{"closure_hash":"{h}","variant":"forward","inherited_controls":0}},"invocation":{{"args":[],"budget":["3","0","0","0"]}},"domain":{{"bundles":[{{"name":"t","width":1,"lo":0,"hi":1}}],"borrowed":[],"regions":[]}},"expected":{{"table":[{{"before":{{"bundles":["0"],"borrowed":[],"regions":[]}},"after":{{"bundles":["1"],"borrowed":[],"regions":[]}}}},{{"before":{{"bundles":["1"],"borrowed":[],"regions":[]}},"after":{{"bundles":["0"],"borrowed":[],"regions":[]}}}}]}},"sampler":{{"algorithm":"xoshiro256**","seed":["1","2","3","4"],"count":8}}}}"#
    ))
}
#[test]
fn standalone_claim_binds_subject_invocation_and_full_rows() {
    let o = object();
    let c = claim(&o, "exhaustive");
    assert_eq!(
        certificate::verify_claim09(&o, "toggle", &c, &[]).unwrap(),
        Verdict::Proof
    );
    for key in ["invocation", "subject", "expected"] {
        let mut bad = c.clone();
        if let Json::Obj(m) = &mut bad {
            m.remove(key);
        }
        assert!(certificate::verify_claim09(&o, "toggle", &bad, &[]).is_err());
    }
    for (from, to) in [
        (r#""args":[]"#, r#""args":["0"]"#),
        (
            r#""budget":["3","0","0","0"]"#,
            r#""budget":["3","1","0","0"]"#,
        ),
        (r#""variant":"forward""#, r#""variant":"""#),
    ] {
        assert!(
            certificate::verify_claim09(&o, "toggle", &j(&c.to_jcs().replace(from, to)), &[])
                .is_err()
        );
    }
}
#[test]
fn sampled_is_evidence_and_validates_even_undrawn_rows() {
    let o = object();
    let c = claim(&o, "sampled");
    assert!(matches!(
        certificate::verify_claim09(&o, "toggle", &c, &[]).unwrap(),
        Verdict::Evidence { .. }
    ));
    // Shape/range/order validation covers all table rows before drawing samples.
    for (from, to) in [
        (r#""bundles":["1"]"#, r#""bundles":["01"]"#),
        (r#""borrowed":[]"#, r#""borrowed":["0"]"#),
        (r#""bundles":["1"]"#, r#""bundles":["2"]"#),
    ] {
        assert!(
            certificate::verify_claim09(&o, "toggle", &j(&c.to_jcs().replace(from, to)), &[])
                .is_err()
        );
    }
}
#[test]
fn borrowed_labels_cannot_cover_an_omitted_interface_component() {
    let o = object();
    let raw = o.text.replace(
        "x t\n",
        "b = qborrow any 1\ncx b t\ncx b t\nqreturn b\nx t\n",
    );
    let o = bennett_vm::load(&raw, true).unwrap().0;
    let mut c = claim(&o, "exhaustive");
    if let Json::Obj(m) = &mut c {
        m.insert(
            "domain".into(),
            j(r#"{"bundles":[],"borrowed":[{"name":"t","width":1,"lo":0,"hi":1}],"regions":[]}"#),
        );
        m.insert("expected".into(),j(r#"{"table":[{"before":{"bundles":["0"],"borrowed":["0"],"regions":[]},"after":{"bundles":["1"],"borrowed":["0"],"regions":[]}},{"before":{"bundles":["0"],"borrowed":["1"],"regions":[]},"after":{"bundles":["1"],"borrowed":["1"],"regions":[]}}]}"#));
    }
    let v = certificate::verify_claim09(&o, "toggle", &c, &[]).unwrap();
    assert!(
        matches!(v,Verdict::Evidence{ref checked,ref covered,ref domain} if checked.to_decimal()=="2" && covered.to_decimal()=="2" && domain.to_decimal()=="4")
    );
    if let Json::Obj(m) = &mut c {
        m.insert("domain".into(),j(r#"{"bundles":[],"borrowed":[{"name":"renamed","width":1,"lo":0,"hi":1}],"regions":[]}"#));
    }
    assert_eq!(
        certificate::verify_claim09(&o, "toggle", &c, &[]).unwrap(),
        v
    );
}
