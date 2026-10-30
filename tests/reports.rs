//! §10 ownership and §13 producer-supplied coverage, with exact expected bytes.
use bennett_vm::{json, launch::*};
use std::{
    ffi::{CStr, CString},
    path::Path,
    ptr,
};
fn object() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("bennett-isa-spec/reference-interpreter/tests/fixtures/registers.bisa"),
    )
    .unwrap()
}
fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}
#[test]
fn coverage_is_owned_canonical_and_outlives_program() {
    unsafe {
        // §13: entries report dialect source classifications; §10: accessor pointers
        // are valid until owning object freed. No execution-derived counts invented.
        let raw = r#"{"entries":[{"classification":"trapped","count":2,"fault_code":8,"point":"entry:0","reason":"unsupported source operation","source":"dialect.test"}]}"#;
        let text = format!("{}\nsection coverage\n{raw}\n", object());
        let mut p = ptr::null_mut();
        assert_eq!(
            bennett_load(text.as_ptr(), text.len(), ptr::null(), 0, &mut p),
            0
        );
        let mut report = ptr::null_mut();
        assert_eq!(bennett_coverage_get(p, &mut report), 0);
        let mut n = 0;
        let bytes = bennett_coverage_jcs(report, &mut n);
        assert_eq!(std::slice::from_raw_parts(bytes, n), raw.as_bytes());
        assert_eq!(bennett_unload(p), 0);
        assert_eq!(std::slice::from_raw_parts(bytes, n), raw.as_bytes());
        bennett_coverage_free(report);
    }
}
#[test]
fn absent_coverage_names_driver_and_invalid_inputs_are_atomic() {
    unsafe {
        let text = object();
        let mut p = ptr::null_mut();
        assert_eq!(
            bennett_load(text.as_ptr(), text.len(), ptr::null(), 0, &mut p),
            0
        );
        let mut out = ptr::dangling_mut();
        assert_ne!(bennett_coverage_get(p, &mut out), 0);
        assert!(out.is_null());
        assert!(
            CStr::from_ptr(bennett_last_error())
                .to_str()
                .unwrap()
                .contains("bennett-driver")
        );
        let mut token = 0;
        assert_eq!(
            bennett_budget(p, c("toggle").as_ptr(), 3, 0, 0, 0, &mut token),
            0
        );
        let mut estimate = ptr::dangling_mut();
        assert_eq!(
            bennett_estimate_run(p, c("missing").as_ptr(), token, &mut estimate),
            11
        );
        assert!(estimate.is_null());
        assert_eq!(
            bennett_estimate_run(p, c("toggle").as_ptr(), token, &mut estimate),
            5
        );
        assert!(
            CStr::from_ptr(bennett_last_error())
                .to_str()
                .unwrap()
                .contains("bennett-estimate")
        );
        assert_eq!(bennett_unload(p), 0);
    }
}
#[test]
fn malformed_coverage_is_rejected() {
    // §9.1 metadata is not an extension bag; §13 closed classifications.
    let bad=json::parse(r#"{"entries":[{"point":"entry:0","source":"d","classification":"ignored","reason":"bad","count":1}]}"#).unwrap();
    let text = format!("{}\nsection coverage\n{}\n", object(), bad.to_jcs());
    assert!(bennett_vm::load(&text, false).unwrap_err().contains("§13"));
}
