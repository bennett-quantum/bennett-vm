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
        if let Json::Obj(dm) = &mut d {
            if let Json::Arr(bs) = dm.get_mut("bundles").unwrap() {
                for (b, p) in bs.iter_mut().zip(f.lists[..3].iter().flatten()) {
                    if let Json::Obj(bm) = b {
                        bm.insert("name".into(), Json::Str(p.name.clone()));
                    }
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
