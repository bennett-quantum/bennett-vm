//! 47t-a: typed coherent source admission, independent of execution.
#[path = "support/source09.rs"]
mod support;
use support::source;

fn actions() -> String {
    source(
        "action @callee (a:wire[2] mut) (r:wire in) (db:region[1,2] in) (n:i8, angle:f64, tk:token) ()\n^entry():\nx a[0]\naction @caller (x:wire[2] mut) (p:wire in) (m:region[1,2] in) (k:i8, b:f64, t:token) ()\n^entry():\naction_call @callee (x) (p) (m) (k, b, t)",
    )
}
#[test]
fn coherent_calls_require_exact_widths_types_and_region_roles() {
    let valid = actions();
    bennett_vm::load(&valid, true).unwrap();
    let mut errors = vec![];
    for (from, to) in [
        ("(x) (p) (m) (k, b, t)", "(x[0]) (p) (m) (k, b, t)"),
        ("(x) (p) (m) (k, b, t)", "(x) (x) (m) (k, b, t)"),
        ("(k, b, t)", "(b, b, t)"),
        ("(k, b, t)", "(k, k, t)"),
        ("(k, b, t)", "(k, b, k)"),
        ("(x) (p) (m)", "(x) (p) (x)"),
    ] {
        let text = valid.replace(from, to);
        if bennett_vm::load(&text, true).is_ok() {
            errors.push(format!("{from} -> {to}"));
        }
    }
    assert!(
        errors.is_empty(),
        "accepted invalid coherent calls: {errors:?}"
    );
    // Mutating a caller's in region through a callee is forbidden even at equal widths.
    let text = source(
        "action @callee () () (db:region[1,2] mut) () ()\n^entry():\ntmp = qalloc any 1\nqfree tmp\naction @caller () () (m:region[1,2] in) () ()\n^entry():\naction_call @callee () () (m) ()",
    );
    bennett_vm::load(&source("action @callee () () (db:region[1,2] mut) () ()\n^entry():\ntmp = qalloc any 1\nqfree tmp\naction @caller () () (m:region[1,2] mut) () ()\n^entry():\naction_call @callee () () (m) ()"), true).unwrap();
    assert!(bennett_vm::load(&text, true).is_err());
}
#[test]
fn controlled_adjoint_calls_are_admitted_and_still_checked() {
    let text = actions().replace(
        "action_call @callee (x)",
        "ctl p adj action_call @callee (x)",
    );
    bennett_vm::load(&text, true).unwrap();
    assert!(bennett_vm::load(&text.replace("(k, b, t)", "(b, b, t)"), true).is_err());
}
#[test]
fn full_qinplace_checks_actual_token_wire_memory_and_argument_types() {
    let functions = "oracle @f (v:wire[2] in, p:wire in) (t:wire[2]) (fr:wire[4]) (db:region[1,2] in) (n:i8)\n^entry():\nlift.retval v\noracle @g (v:wire[2] in, p:wire in) (t:wire[2]) (fr:wire[4]) (db:region[1,2] in) (n:i8)\n^entry():\nlift.retval v\naction @caller (x:wire[2] mut) (p:wire in) (m:region[1,2] in) (k:i8, tk:token) ()\n^entry():\nqinplace @f @g tk x (p) (m) (k)";
    let valid = source(&functions.replace("region[1,2]", "region[1,8]"));
    bennett_vm::load(&valid, true).unwrap();
    let mut errors = vec![];
    for (from, to) in [
        ("tk x (p)", "k x (p)"),
        ("tk x (p)", "tk x[0] (p)"),
        ("tk x (p)", "tk x (x)"),
        ("(m) (k)", "(x) (k)"),
        ("(m) (k)", "(m) (tk)"),
    ] {
        if bennett_vm::load(&valid.replace(from, to), true).is_ok() {
            errors.push(format!("{from} -> {to}"));
        }
    }
    assert!(errors.is_empty(), "accepted invalid qinplace: {errors:?}");
}

#[test]
fn huge_regions_are_type_checked_by_shape_without_enumerating_cells() {
    let valid = source(
        "action @callee () () (db:region[63,1] in) () ()\n^entry():\ntmp = qalloc any 1\nqfree tmp\naction @caller () () (m:region[63,1] in) () ()\n^entry():\naction_call @callee () () (m) ()",
    );
    bennett_vm::load(&valid, true).unwrap();
    // Equal cell width cannot hide a different address domain.
    let invalid = valid.replace("m:region[63,1]", "m:region[62,1]");
    assert!(bennett_vm::load(&invalid, true).is_err());
}

#[test]
fn in_wires_cannot_be_modified_through_a_callee() {
    let invalid = source(
        "action @callee (a:wire mut) () () () ()\n^entry():\nx a\naction @caller (t:wire mut) (p:wire in) () () ()\n^entry():\naction_call @callee (p) () () ()",
    );
    let e = bennett_vm::load(&invalid, true).unwrap_err();
    assert!(e.contains("in wire"), "{e}");
    bennett_vm::load(&invalid.replace("@callee (p) ()", "@callee (t) ()"), true).unwrap();
}

#[test]
fn controlled_adjoint_edges_participate_in_acyclicity() {
    let invalid = source(
        "action @recur (t:wire mut) (p:wire in) () () ()\n^entry():\nctl p adj action_call @recur (t) (p) () ()",
    );
    let e = bennett_vm::load(&invalid, true).unwrap_err();
    assert!(e.contains("cycle"), "{e}");
}

#[test]
fn rotation_parameters_must_be_f64() {
    let valid = source(
        "action @a (t:wire mut) () () (angle:f64, tk:token, n:i8) ()\n^entry():\nrx angle t",
    );
    bennett_vm::load(&valid, true).unwrap();
    for param in ["tk", "n", "missing"] {
        let e = bennett_vm::load(&valid.replace("rx angle t", &format!("rx {param} t")), true)
            .unwrap_err();
        assert!(e.contains("3.3"), "{e}");
    }
}

#[test]
fn literal_angles_keep_the_published_float_spelling() {
    for angle in ["0.7", "1.0", "1e+21", "-0.0"] {
        let valid = source(&format!(
            "action @a (t:wire mut) () () () ()\n^entry():\nrx {angle} t"
        ));
        bennett_vm::load(&valid, true).unwrap();
    }
    for angle in ["0.70", "1.0e0", "00.7"] {
        let invalid = source(&format!(
            "action @a (t:wire mut) () () () ()\n^entry():\nrx {angle} t"
        ));
        let e = bennett_vm::load(&invalid, true).unwrap_err();
        assert!(e.contains("16.1"), "{e}");
    }
}

#[test]
fn memory_rows_check_shapes_roles_and_disjoint_operands() {
    let good = source(
        "action @a (addr:wire[2] mut, v:wire[8] mut) () (m:region[2,8] mut) () ()\n^entry():\nqstore m addr v\nqexchm m addr v\nd = qload m addr\nqunload m addr d",
    );
    bennett_vm::load(&good, true).unwrap();
    for text in [
        good.replace("qstore m addr v", "qstore m v addr"),
        good.replace("qexchm m addr v", "qexchm missing addr v"),
        good.replace("region[2,8] mut", "region[2,8] in"),
        good.replace("qunload m addr d", "qunload m addr addr"),
    ] {
        assert!(bennett_vm::load(&text, true).is_err(), "admitted {text}");
    }
}
#[test]
fn linear_local_lifetimes_are_checked_even_without_borrow_frames() {
    for body in [
        "qfree x",
        "a = qalloc any 1\nqfree a\nx a",
        "a = qalloc any 1\na = qalloc any 1\nqfree a",
        "a = qalloc any 1",
        "a = qborrow any 1\nqfree a",
        "a = qalloc any 1\nqreturn a",
        "qpred x\nqunpred x\nqfree x",
    ] {
        let text = source(&format!(
            "action @a (x:wire mut) () () () ()\n^entry():\n{body}"
        ));
        assert!(bennett_vm::load(&text, true).is_err(), "admitted {body}");
    }
    let good = source(
        "action @a (x:wire mut) () () () ()\nregs { } bounds {2}\n^entry():\na = qalloc any 1\nqfree a\na = qalloc any 1\nqfree a\nqloop 2 {\nb = qcopy x\nquncopy x b\n}",
    );
    bennett_vm::load(&good, true).unwrap();
}
#[test]
fn function_wide_predicates_and_zero_width_regions_are_checked() {
    let text = source(
        "action @a (x:wire mut) (p:wire in) () () ()\n^entry():\nqpred p\n^tail():\nx x\nqunpred p",
    );
    bennett_vm::load(&text, true).unwrap();
    for body in ["qpred x", "qregion m 1 0", "a = qalloc any 0\nqfree a"] {
        let bad = source(&format!(
            "action @a (x:wire mut) () () () ()\n^entry():\n{body}"
        ));
        assert!(bennett_vm::load(&bad, true).is_err(), "admitted {body}");
    }
}
#[test]
fn qinplace_inputs_are_disjoint_from_the_effective_control_context() {
    let text = source(
        "oracle @f (v:wire in, p:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval v\noracle @g (v:wire in, p:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval v\naction @a (x:wire mut) (p:wire in) () (tk:token) ()\n^entry():\nqpred p\nqinplace @f @g tk x (p) () ()\nqunpred p",
    );
    let e = bennett_vm::load(&text, true).unwrap_err();
    assert!(e.contains("qinplace"), "{e}");
}
#[test]
fn one_trip_loop_can_replace_a_returned_borrow_with_an_owned_local() {
    let one = source(
        "action @a () () () () ()\nregs {  } bounds {1}\n^entry():\na = qborrow any 1\nqloop 1 {\nqreturn a\na = qalloc any 1\n}\nqfree a",
    );
    bennett_vm::load(&one, true).unwrap();
    let two = one
        .replace("bounds {1}", "bounds {2}")
        .replace("qloop 1", "qloop 2");
    assert!(bennett_vm::load(&two, true).is_err());
}
