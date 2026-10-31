//! Independent ISA 0.9 §11.1 consumer witnesses; no producer implementation used.
use bennett_vm::{
    json::{self, Json},
    ref_library::{Bindings, Catalog},
};
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
#[test]
fn full_catalog_identity_and_k57_default() {
    let c = Catalog::published().unwrap();
    assert_eq!(c.len(), 219);
    assert_eq!(
        c.hash(),
        "5de4e4995350313e50217880ba1b95de39cd043283b726337d4894efb005d197"
    );
    bennett_vm::sched::require_library_version(None, "0.9").unwrap();
    assert!(bennett_vm::sched::require_library(None).is_err()); // 0.8 never inherits 0.9
    let raw = c.document().to_jcs();
    assert!(Catalog::parse(&(raw.clone() + "\n")).is_err());
    assert!(Catalog::parse(&raw.replace("0.8 §4.5: cx", "changed §4.5: cx")).is_err());
    assert!(
        bennett_vm::sched::require_library_version(
            Some(&j(r#"{"name":"ref","version":"0.9","hash":"00"}"#)),
            "0.9"
        )
        .is_err()
    );
}
#[test]
fn qxor_full_domain_and_inverse_ordinals() {
    let c = Catalog::published().unwrap();
    let mut b = Bindings::new();
    b.wire("s", 2);
    b.wire("t", 2);
    let leaves = c.expand("qxor", &[j("2")], &b).unwrap();
    assert_eq!(leaves.len(), 2);
    assert_eq!(leaves[0].wires, ["s[0]", "t[0]"]);
    for s in 0..4 {
        for t in 0..4 {
            let mut state = t;
            for l in &leaves {
                let bit = l.ordinal;
                if s & (1 << bit) != 0 {
                    state ^= 1 << bit;
                }
            }
            assert_eq!(state, t ^ s);
            for l in leaves.iter().rev() {
                if s & (1 << l.ordinal) != 0 {
                    state ^= 1 << l.ordinal;
                }
            }
            assert_eq!(state, t);
        }
    }
    b.body(
        "B",
        j(r#"[{"inverse":true,"body":[{"invoke":"qxor","args":[2]}]}]"#),
    )
    .unwrap();
    let rev = c.expand("adj", &[j("\"B\"")], &b).unwrap();
    assert_eq!(rev.iter().map(|l| l.ordinal).collect::<Vec<_>>(), [1, 0]);
}
#[test]
fn typed_binding_width_alias_and_angle_checks() {
    let c = Catalog::published().unwrap();
    let mut b = Bindings::new();
    b.wire("a", 3);
    let l = c.expand(
        "bind.qxor",
        &[
            j("2"),
            j(r#"{"range":"a","from":0,"to":2}"#),
            j(r#"["a[2]","a[2]"]"#),
        ],
        &b,
    );
    assert!(l.is_err());
    assert!(
        c.expand("bind.qxor", &[j("2"), j(r#""a""#), j(r#""a""#)], &b)
            .is_err()
    );
    let l = c
        .expand(
            "gate.rx",
            &[j(r#""0x8000000000000000""#), j(r#""a[0]""#), j("[]")],
            &b,
        )
        .unwrap();
    assert_eq!(l[0].angle, Some(1 << 63));
    assert!(
        c.expand(
            "gate.rx",
            &[j(r#""0x7ff0000000000000""#), j(r#""a[0]""#), j("[]")],
            &b
        )
        .is_err()
    );
}
#[test]
fn k62_source_adjoint_keeps_instruction_lifetimes() {
    let c = Catalog::published().unwrap();
    let mut b = Bindings::new();
    b.wire("x", 1);
    let template = |target: &str| {
        j(&format!(
            r#"[{{"parameters":[{{"name":"x","type":"wire","width":1}},{{"name":"t","type":"wire","width":1}},{{"name":"f","type":"wire","width":4}}],"body":[{{"gate":"cx","wires":["x","{target}"]}}]}}]"#
        ))
    };
    let g = template("t");
    let h = template("f[0]");
    let fwd = c
        .expand(
            "qinplace",
            &[j("1"), j("4"), j(r#""x""#), g.clone(), h.clone(), j("[]")],
            &b,
        )
        .unwrap();
    let adj = c
        .expand(
            "qinplace",
            &[j("1"), j("4"), j(r#""x""#), h, g, j("[]")],
            &b,
        )
        .unwrap();
    for l in [&fwd, &adj] {
        assert_eq!(
            l.iter().map(|x| x.operation.as_str()).collect::<Vec<_>>(),
            ["alloc", "alloc", "cx", "swap", "cx", "release", "release"]
        );
        assert_eq!(l[0].wires.len(), 1);
        assert_eq!(l[1].wires.len(), 4);
        assert_eq!(l[5].wires, l[0].wires);
        assert_eq!(l[6].wires, l[1].wires);
    }
    assert_eq!(adj[2].wires[1], adj[1].wires[0]);
    assert_ne!(adj[2].wires, fwd[2].wires);
}
#[test]
fn finite_higher_order_repetition_is_not_a_dependency_cycle() {
    let c = Catalog::published().unwrap();
    let mut b = Bindings::new();
    b.wire("a", 1);
    b.body("B",j(r#"[{"invoke":"qloop","args":[3,[{"invoke":"qloop","args":[2,[{"gate":"x","wires":["a"]}]]}]]}]"#)).unwrap();
    assert_eq!(c.expand("adj", &[j(r#""B""#)], &b).unwrap().len(), 6);
    assert!(
        b.body("bad", j(r#"[{"inverse":false,"body":[]}]"#))
            .is_err()
    );
}
#[test]
fn reacquisition_has_fresh_identity_in_both_traversals() {
    let c = Catalog::published().unwrap();
    let mut b = Bindings::new();
    b.body("B",j(r#"[{"declare":"t","width":1,"class":{"class":"any"},"state":"clean","body":[{"acquire":"t"},{"relinquish":"t"},{"acquire":"t"},{"relinquish":"t"}]}]"#)).unwrap();
    let l = c.expand("adj", &[j(r#""B""#)], &b).unwrap();
    assert_eq!(l[0].wires, l[1].wires);
    assert_eq!(l[2].wires, l[3].wires);
    assert_ne!(l[0].wires, l[2].wires);
    b.body("I", j(r#"[{"inverse":true,"body":[{"splice":"B"}]}]"#))
        .unwrap();
    let inverse = c.expand("adj", &[j(r#""I""#)], &b).unwrap();
    assert_eq!(
        inverse.iter().map(|l| l.ordinal).collect::<Vec<_>>(),
        [3, 2, 1, 0]
    );
    assert_eq!(inverse[0].operation, "alloc");
    assert_eq!(inverse[0].wires, inverse[1].wires);
    assert_eq!(inverse[2].wires, inverse[3].wires);
    assert_ne!(inverse[0].wires, inverse[2].wires);
}

#[test]
fn source_image_and_classical_event_bindings() {
    let c = Catalog::published().unwrap();
    let b = Bindings::new();
    let template = j(r#"[{"parameters":[{"name":"r","type":"wire","width":16}],"body":[]}]"#);
    let l = c
        .expand(
            "qregion",
            &[j("2"), j("8"), j(r#""8102""#), template, j("[]")],
            &b,
        )
        .unwrap();
    assert_eq!(
        l.iter().map(|x| x.operation.as_str()).collect::<Vec<_>>(),
        ["alloc", "x", "x", "x", "x", "x", "x", "release"]
    );
    assert_eq!(l[1].wires, [l[0].wires[0].clone()]);
    assert_eq!(l[2].wires, [l[0].wires[7].clone()]);
    assert_eq!(l[3].wires, [l[0].wires[9].clone()]);
    assert_eq!(l[4].wires, l[3].wires);
    assert_eq!(l[7].wires, l[0].wires);
    let event = j(r#"{"instruction":"a = const.i8 7","point":"entry:0"}"#);
    let l = c.expand("cc.const", &[event], &b).unwrap();
    assert_eq!(l[0].operation, "const.i8");
    assert_eq!(
        l[0].event.as_ref().unwrap().to_jcs(),
        r#"{"instruction":"a = const.i8 7","point":"entry:0"}"#
    );
    assert!(
        c.expand(
            "cc.add",
            &[j(r#"{"instruction":"a = const.i8 7","point":"entry:0"}"#)],
            &b
        )
        .is_err()
    );
    assert!(
        c.expand(
            "qregion",
            &[j("2"), j("8"), j(r#""810X""#), j("[]"), j("[]")],
            &b
        )
        .is_err()
    );
}

#[test]
fn declaration_aliases_keep_class_state_and_inverse_lifetimes() {
    let c = Catalog::published().unwrap();
    let mut b = Bindings::new();
    b.body("B",j(r#"[{"declare":"d","width":2,"class":{"class":"ancilla"},"state":"clean","body":[{"invoke":"qalloc","args":["d"]},{"gate":"x","wires":["d[1]"]},{"gate":"x","wires":["d[1]"]},{"invoke":"qfree","args":["d"]}]}]"#)).unwrap();
    let f = c.expand("adj", &[j(r#""B""#)], &b).unwrap();
    assert_eq!(f[0].class.as_deref(), Some("ancilla"));
    assert_eq!(f[0].wires, f[3].wires);
    b.body("I", j(r#"[{"inverse":true,"body":[{"splice":"B"}]}]"#))
        .unwrap();
    let i = c.expand("adj", &[j(r#""I""#)], &b).unwrap();
    assert_eq!(
        i.iter().map(|l| l.ordinal).collect::<Vec<_>>(),
        [3, 2, 1, 0]
    );
    assert_eq!(i[0].operation, "alloc");
    assert_eq!(i[0].class.as_deref(), Some("ancilla"));
    assert_eq!(i[0].wires, i[3].wires);
}
