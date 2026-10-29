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
