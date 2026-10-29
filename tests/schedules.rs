//! §11 hand-derived scheduling consumer witnesses; no oracle command exists.
use bennett_vm::{
    json::{self, Json},
    sched::{self, Operation, Point, Selector},
    target::Rational as R,
};
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
#[test]
fn specificity_is_lexicographic_and_document_order_independent() {
    // §11 "Specificity is the lexicographic tuple"; H's corrected expectation.
    let point = Point {
        symbol: "a".repeat(64),
        point: "entry:0".into(),
        iteration: vec![0],
    };
    let op = Operation {
        path: vec![point.clone()],
        step: Some(1),
        phase: "fwd".into(),
        generated: 0,
    };
    let a = Selector {
        path: vec![(point.symbol.clone(), point.point.clone(), Some(vec![0]))],
        step: None,
        phase: None,
        generated: None,
    };
    let b = Selector {
        path: vec![(point.symbol, point.point, None)],
        step: Some(1),
        phase: None,
        generated: None,
    };
    assert_eq!(
        sched::select(&[a.clone(), b.clone()], &op).unwrap(),
        Some(0)
    );
    assert_eq!(sched::select(&[b, a.clone()], &op).unwrap(), Some(1));
    assert!(
        sched::select(&[a.clone(), a], &op)
            .unwrap_err()
            .contains("maximal")
    );
}
#[test]
fn defaults_and_timing_fail_loud() {
    // §11 uncovered positive-error operations share epsilon equally; exact ops get 0.
    assert_eq!(
        sched::epsilon_share(&"0.01".parse::<R>().unwrap(), 4).unwrap(),
        "0.0025".parse().unwrap()
    );
    assert_eq!(
        sched::epsilon_share(&"0".parse::<R>().unwrap(), 0).unwrap(),
        "0".parse().unwrap()
    );
    assert!(
        sched::validate_start(&"0".parse().unwrap(), &["1".parse().unwrap()])
            .unwrap_err()
            .contains("dependency")
    );
}
#[test]
fn unknown_expansion_is_rejected_even_without_explicit_binding() {
    // §11: "A consumer that does not implement the bound library rejects".
    assert!(
        sched::require_library(Some(&j(
            r#"{"name":"no-such-library","version":"0","hash":"00"}"#
        )))
        .unwrap_err()
        .contains("E_SCHEDULE_INFEASIBLE")
    );
    assert!(
        sched::require_library(None)
            .unwrap_err()
            .contains("ref@0.8")
    );
}
#[test]
fn schedule_validates_paths_and_cleanup_before_library_rejection() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(
        repo.parent()
            .unwrap()
            .join("bennett-isa-spec/conformance/cases/objects/sched_inc8.bisa"),
    )
    .unwrap();
    let (obj, ids) = bennett_vm::load(&text, false).unwrap();
    let document = sched::object_schedule(&obj).unwrap().unwrap();
    sched::validate_entries(&document, &obj, &ids).unwrap();
    let mut d = document.clone();
    if let Json::Obj(m) = &mut d {
        m.insert("entries".into(),j(r#"[{"kind":"cleanup","path":[],"step":"*","phase":"*","generated":"*","cleanup":{"realisation":"measure"}}]"#));
    }
    assert!(
        sched::validate_entries(&d, &obj, &ids)
            .unwrap_err()
            .contains("cleanup")
    );
    let mut d = document;
    if let Json::Obj(m) = &mut d
        && let Json::Arr(es) = m.get_mut("entries").unwrap()
        && let Json::Obj(e) = &mut es[0]
        && let Json::Arr(p) = e.get_mut("path").unwrap()
        && let Json::Obj(p) = &mut p[0]
    {
        p.insert("point".into(), Json::Str("entry:3".into()));
    }
    assert!(
        sched::validate_entries(&d, &obj, &ids)
            .unwrap_err()
            .contains("point")
    );
}
#[test]
fn lower_specificity_ties_do_not_invalidate_unique_maximum() {
    let point = Point {
        symbol: "a".repeat(64),
        point: "entry:0".into(),
        iteration: vec![],
    };
    let op = Operation {
        path: vec![point.clone()],
        step: Some(1),
        phase: "fwd".into(),
        generated: 0,
    };
    let low = Selector {
        path: vec![],
        step: None,
        phase: None,
        generated: None,
    };
    let high = Selector {
        path: vec![(point.symbol, point.point, Some(vec![]))],
        step: None,
        phase: None,
        generated: None,
    };
    assert_eq!(
        sched::select(&[low.clone(), low, high], &op).unwrap(),
        Some(2)
    );
}

#[test]
fn all_schedule_bindings_are_checked_before_consumption() {
    use bennett_vm::{sched::Bindings, target::Target};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/cases");
    let (object, ids) = bennett_vm::load(
        &std::fs::read_to_string(root.join("objects/inc8.bisa")).unwrap(),
        false,
    )
    .unwrap();
    let t = Target::parse(
        std::fs::read_to_string(root.join("E/E-matching-first-entry/target.json"))
            .unwrap()
            .trim_end(),
    )
    .unwrap();
    let params = j("{}");
    let epsilon = "0".parse().unwrap();
    let b = Bindings {
        region: "inc8",
        variant: "forward",
        inherited_controls: 0,
        budget: [3, 0, 0, 0],
        params: &params,
        epsilon: &epsilon,
    };
    let binding = Json::obj(vec![
        (
            "closure_hash",
            Json::Str(ids.closure["inc8"].clone().unwrap()),
        ),
        ("target_hash", Json::Str(t.hash.clone())),
        ("region", Json::Str("inc8".into())),
        ("variant", Json::Str("forward".into())),
        ("inherited_controls", Json::u64(0)),
        ("budget", j("[3,0,0,0]")),
        (
            "params_hash",
            Json::Str(bennett_vm::sha256::sha256_hex(b"{}")),
        ),
        ("epsilon", Json::u64(0)),
        ("scheduler", j(r#"{"name":"test","version":"0.8"}"#)),
    ]);
    let doc = Json::obj(vec![("bindings", binding), ("entries", j("[]"))]);
    assert!(
        sched::consume(&doc, &object, &ids, &t, &b)
            .unwrap_err()
            .contains("ref@0.8")
    );
    for field in [
        "closure_hash",
        "target_hash",
        "region",
        "variant",
        "inherited_controls",
        "budget",
        "params_hash",
        "epsilon",
    ] {
        let mut bad = doc.clone();
        if let Json::Obj(m) = &mut bad
            && let Json::Obj(binding) = m.get_mut("bindings").unwrap()
        {
            let value = match field {
                "inherited_controls" | "epsilon" => Json::u64(1),
                "budget" => j("[4,0,0,0]"),
                _ => Json::Str("wrong".into()),
            };
            binding.insert(field.into(), value);
        }
        let error = sched::consume(&bad, &object, &ids, &t, &b).unwrap_err();
        assert!(
            error.contains(&format!("binding {field} mismatch")),
            "{error}"
        );
    }
}

#[test]
fn data_symbol_is_not_an_instruction_path() {
    // §11 path is a chain of program points, never a data entry.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let (obj, ids) = bennett_vm::load(
        &std::fs::read_to_string(
            root.join("bennett-isa-spec/conformance/cases/objects/actions.bisa"),
        )
        .unwrap(),
        false,
    )
    .unwrap();
    let doc = j(&format!(
        r#"{{"entries":[{{"kind":"start","start":{{"time":0}},"path":[{{"symbol":"{}","point":"entry:0","iteration":[]}}],"step":"*","phase":"*","generated":"*"}}]}}"#,
        ids.interface["image"]
    ));
    assert!(
        sched::validate_entries(&doc, &obj, &ids)
            .unwrap_err()
            .contains("not a function")
    );
}
