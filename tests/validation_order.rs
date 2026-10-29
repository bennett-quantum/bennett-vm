#[test]
fn spec_rules_precede_stored_identity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/cases/objects");
    let cases = [
        ("f_borrow_neg1", "5.6", "the frame is V; W; V⁻¹; W"),
        (
            "f_borrow_neg2",
            "5.6",
            "controls are all in a set X with X ∩ B = ∅",
        ),
        ("f_borrow_neg3", "5.6", "target is in B"),
        ("f_borrow_neg4", "5.6", "no swap appears in the frame"),
        (
            "f_disjoint_pred",
            "3.3",
            "every wire of the effective control context is distinct from every target",
        ),
        ("f_width_mismatch", "3.3", "binop/icmp operands equal width"),
        (
            "f_in_wire_target",
            "5.1",
            "in wires in actions appear only as controls, as targets of diagonal gates, as qcopy/qxor sources, or as in arguments of calls",
        ),
        (
            "f_lex_double_space",
            "16.1",
            "tokens separated by exactly one space",
        ),
    ];
    for (file, rule, sentence) in cases {
        let text = std::fs::read_to_string(root.join(format!("{file}.bisa"))).unwrap();
        let error = bennett_vm::load(&text, false).unwrap_err();
        assert!(
            error.contains(&format!("§{rule}")),
            "{file}: {sentence}: {error}"
        );
    }
}
