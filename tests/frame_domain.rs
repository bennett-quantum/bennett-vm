//! §5.6/K76: entry facts are constants and causal adjoint pops only.
use bennett_vm::load;
fn source(body: &str) -> String {
    let p = crate::pins::isa().join("reference-interpreter/tests/fixtures/control_flow.bisa");
    let text = std::fs::read_to_string(p).unwrap();
    let start = text.find("perm_action @borrowed").unwrap();
    let head_end = start + text[start..].find("^entry():\n").unwrap() + "^entry():\n".len();
    let end = text[head_end..]
        .find("\nsection ")
        .map_or(text.len(), |n| head_end + n);
    format!("{}{}\n{}", &text[..head_end], body, &text[end..])
}
#[test]
fn council_local_zero_frames_are_admitted() {
    for case in ["sol-oracle", "codex-oracle", "loop", "sol-qinplace"] {
        let p = crate::pins::isa().join(format!("reference-interpreter/tests/fixtures/r9-{case}.bisa"));
        load(&std::fs::read_to_string(p).unwrap(), false).unwrap_or_else(|e| panic!("{case}: {e}"));
    }
}
#[test]
fn only_causal_prefix_cancellation_establishes_local_constants() {
    let clear = "c = qalloc any 1\ncx x c\ncx x c\nb = qborrow any 1\ncx c b\nqreturn b\nqfree c";
    load(&source(clear), true).unwrap();
    // Algebra clears c, but neither repeated CX is topmost on all operands.
    let crossed = "c = qalloc any 1\ncx x c\ncx t c\ncx x c\ncx t c\nb = qborrow any 1\ncx c b\nqreturn b\ncx t c\ncx x c\ncx t c\ncx x c\nqfree c";
    let e = load(&source(crossed), true).unwrap_err();
    assert!(e.contains("§5.6"), "{e}");
}
#[test]
fn correlated_local_and_interface_are_independent_in_frame_domain() {
    let text = source("c = qcopy t\nb = qborrow any 1\ncx t b\ncx c b\nqreturn b\nquncopy t c");
    let e = load(&text, true).unwrap_err();
    assert!(e.contains("§5.6"), "{e}");
}

#[test]
fn overlapping_borrow_lifetimes_keep_separate_maximal_frames() {
    let text = source(
        "b = qborrow any 1\nd = qborrow any 1\ncx x d\ncx x b\ncx x b\nqreturn b\ncx x d\nqreturn d",
    );
    load(&text, true).unwrap();
}

#[test]
fn every_loop_visit_contributes_to_the_entry_domain() {
    // c is zero on the first visit, one on the second: the union must reject.
    let text = std::fs::read_to_string(crate::pins::isa().join("reference-interpreter/tests/fixtures/r9-loop.bisa"))
    .unwrap()
    .replace("qfree c\nc = qalloc any 1", "x c");
    let e = load(&text, true).unwrap_err();
    assert!(e.contains("§5.6"), "{e}");
}

#[test]
fn lifetime_boundary_invalidates_connected_reader_history() {
    let text = std::fs::read_to_string(crate::pins::isa().join("reference-interpreter/tests/fixtures/r9-sol-oracle.bisa"))
    .unwrap();
    let text = text.replacen(
        "oracle_call @o tk (c) t f () ()\n",
        "oracle_call @o tk (c) t f () ()\nqfree f\nf = qalloc any 4\n",
        1,
    );
    let e = load(&text, true).unwrap_err();
    assert!(e.contains("§5.6"), "{e}");
}

#[test]
fn disjoint_and_nested_adjoint_events_restore_constants() {
    let text = std::fs::read_to_string(crate::pins::isa().join("reference-interpreter/tests/fixtures/r9-sol-oracle.bisa"))
    .unwrap();
    let text = text.replace(
        "h c\noracle_call @o tk (c) t f () ()\noracle_call @o tk (c) t f () ()\nh c",
        "h c\nh t\ns c\nsdg c\nh c\nh t",
    );
    load(&text, true).unwrap();
}

#[test]
fn resolved_x_family_arities_share_exact_gate_identity() {
    let text = source(
        "c = qalloc any 1\ncx x c\nmcx (x) c\nb = qborrow any 1\ncx c b\nqreturn b\nmcx (x) c\ncx x c\nqfree c",
    );
    load(&text, true).unwrap();
}

#[test]
fn local_action_adjoints_resolve_indexed_aliases_and_inherited_controls() {
    let text = std::fs::read_to_string(crate::pins::isa().join("reference-interpreter/tests/fixtures/r9-sol-oracle.bisa"))
    .unwrap();
    let entry = format!(
        r#""9":{{"name":"turn","kind":"action","interface_hash":"{}","signature":{{"abi":"0.8","args":[],"bounds":[],"in":[],"kind":"action","mem":[],"mut":[1],"tier":"q"}}}},"#,
        "0".repeat(64)
    );
    let text = text.replacen("section symbols\n{", &format!("section symbols\n{{{entry}"), 1)
        .replace("oracle_call @o tk (c) t f () ()\noracle_call @o tk (c) t f () ()", "qpred t\naction_call @turn (c[0]) () () ()\nadj action_call @turn (c[0]) () () ()\nqunpred t");
    let text = format!(
        "{}\naction @turn (p:wire mut) () () () ()\n^entry():\nrx 0.25 p[0]\n",
        text.trim_end()
    );
    let text = text
        .lines()
        .map(|line| {
            if line.starts_with('{') {
                bennett_vm::json::parse(line).unwrap().to_jcs()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    load(&text, true).unwrap();
}

#[test]
fn public_region_verification_does_not_waive_the_frame_domain() {
    use bennett_vm::{
        certificate::{self, Verdict},
        json::Json,
        value::Budget,
    };
    let text = source(
        "c = qalloc any 1\ncx t c\ncx x c\ncx t c\ncx x c\nb = qborrow any 1\ncx c b\nqreturn b\ncx x c\ncx t c\ncx x c\ncx t c\nqfree c",
    );
    assert!(load(&text, true).unwrap_err().contains("§5.6"));
    // Exercise the public checker independently of load admission, as a host
    // with an unvalidated or modified parsed object can do.
    let obj = bennett_vm::object::parse(&text).unwrap();
    let claim = bennett_vm::json::parse(r#"{"kind":"exhaustive","domain":{"bundles":[{"name":"t","width":1,"lo":0,"hi":1},{"name":"x","width":1,"lo":0,"hi":1}],"borrowed":[{"name":"b","width":1,"lo":0,"hi":1}],"regions":[]},"expected":{"table":[[0,0,0],[0,0,1],[0,1,0],[0,1,1],[1,0,0],[1,0,1],[1,1,0],[1,1,1]]}}"#).unwrap();
    let budget = Budget {
        steps: 3,
        heap: 0,
        stack: 0,
        depth: 0,
    };
    assert_eq!(
        certificate::verify_claim(&obj, "borrowed", &claim, &[], &budget, &[]).unwrap(),
        Verdict::Proof
    );
    let region = Json::obj(vec![
        ("claims", Json::Arr(vec![claim])),
        (
            "verification",
            Json::obj(vec![("borrow_frames", Json::Arr(vec![Json::u64(0)]))]),
        ),
    ]);
    let e = certificate::verify_region_claims(&obj, "borrowed", &region, &[], &budget, &[])
        .unwrap_err();
    assert!(e.contains("§5.6"), "{e}");
}
