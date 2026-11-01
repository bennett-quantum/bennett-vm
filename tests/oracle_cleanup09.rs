//! Source-formed K64 constructors and K65 lifted cleanup choices.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::{
    json::{self},
    ref_library::{Catalog, CleanupEntry, CleanupRealisation},
    sched::Selector,
};

#[test]
fn source_constructor_lifted_pair_has_one_nonemitting_carrier_per_step_phase() {
    let text = support::source(
        "oracle @root (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs { r:reg[1], fr:reg[4] }\n^entry():\nlift.oracle_call @id tk (x) r fr () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x",
    );
    let (obj, ids) = bennett_vm::load(&text, false).unwrap();
    let catalog = Catalog::published().unwrap();
    for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
        let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"entry:1","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#, ids.interface["root"])).unwrap();
        let e = catalog
            .lower_oracle_with_cleanup(
                &obj,
                "root",
                &json::parse(r#"[["1","0","0","0"]]"#).unwrap(),
                &json::parse(r#"{"token":["3","0","0","0"]}"#).unwrap(),
                0,
                &[CleanupEntry {
                    selector: Selector::parse(&selector).unwrap(),
                    realisation: choice,
                }],
            )
            .unwrap();
        assert!(e.cleanup_pairing_complete);
        assert_eq!(e.cleanups.len(), 6);
        for c in &e.cleanups {
            assert_eq!(c.realisation, choice);
            assert_eq!(c.address.path.len(), 1);
            assert_eq!(c.address.path[0].point, "entry:1");
            assert_eq!(c.computing_point.point, "entry:0");
            assert_eq!(c.address.generated, 0);
            assert!((1..=3).contains(&c.address.step.unwrap()));
            assert!(!c.controls.is_empty());
        }
        assert!(
            e.leaves
                .iter()
                .all(|l| l.address.as_ref().unwrap().kind
                    != bennett_vm::sched::OperationKind::Cleanup)
        );
    }
}

fn replay(
    leaves: &[bennett_vm::ref_library::Leaf],
    mut state: std::collections::BTreeMap<String, bool>,
) -> std::collections::BTreeMap<String, bool> {
    for l in leaves {
        match l.operation.as_str() {
            "alloc" => {
                for w in &l.wires {
                    assert!(state.insert(w.clone(), false).is_none());
                }
            }
            "release" => {
                for w in &l.wires {
                    assert_eq!(state.remove(w), Some(false), "dirty {w}");
                }
            }
            "x" | "cx" | "ccx" | "mcx" => {
                let (target, controls) = l.wires.split_last().unwrap();
                if controls.iter().chain(&l.controls).all(|w| state[w]) {
                    *state.get_mut(target).unwrap() ^= true;
                }
            }
            "swap" => {
                if l.controls.iter().all(|w| state[w]) {
                    let (a, b) = (state[&l.wires[0]], state[&l.wires[1]]);
                    state.insert(l.wires[0].clone(), b);
                    state.insert(l.wires[1].clone(), a);
                }
            }
            op => panic!("unexpected gate {op}"),
        }
    }
    state
}

#[test]
fn controlled_reverse_and_recompute_constructors_restore_every_private_bit() {
    let text = support::source(
        "oracle @root (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs { r:reg[1], fr:reg[4] }\n^entry():\nr = lift.mov x\nlift.oracle_call @id tk (x) r fr () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x",
    );
    let (obj, ids) = bennett_vm::load(&text, false).unwrap();
    let catalog = Catalog::published().unwrap();
    for steps in [0, 3, 4, 5] {
        for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"entry:2","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#, ids.interface["root"])).unwrap();
            let e = catalog
                .lower_oracle_with_cleanup(
                    &obj,
                    "root",
                    &json::parse(r#"[["1","0","0","0"]]"#).unwrap(),
                    &json::parse(&format!(r#"{{"token":["{steps}","0","0","0"]}}"#)).unwrap(),
                    1,
                    &[CleanupEntry {
                        selector: Selector::parse(&selector).unwrap(),
                        realisation: choice,
                    }],
                )
                .unwrap();
            assert_eq!(e.cleanups.len(), 2 * steps);
            for c in &e.cleanups {
                assert_eq!(c.controls[0], "$controls[0]");
            }
            for bits in 0..128 {
                let mut state = std::collections::BTreeMap::from([
                    ("x[0]".to_string(), bits & 1 != 0),
                    ("t[0]".into(), bits & 2 != 0),
                    ("$controls[0]".into(), bits & 64 != 0),
                ]);
                for i in 0..4 {
                    state.insert(format!("f[{i}]"), bits & (4 << i) != 0);
                }
                let mut expected = state.clone();
                if bits & 64 != 0 {
                    if steps >= 4 {
                        *expected.get_mut("t[0]").unwrap() ^= bits & 1 != 0;
                    } else {
                        *expected.get_mut("f[0]").unwrap() ^= true;
                    }
                }
                assert_eq!(
                    replay(&e.leaves, state),
                    expected,
                    "{steps} {choice:?} {bits}"
                );
            }
        }
    }
}

#[test]
fn source_constructor_rejects_bad_pairs_and_invalid_context_before_selection() {
    let catalog = Catalog::published().unwrap();
    for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
        for rows in [
            "lift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
            "lift.oracle_call @id tk (x) r fr () ()\nlift.oracle_uncall @id tk (x) s fr () ()\nlift.retval r",
        ] {
            let (obj, ids) = bennett_vm::load(&support::source(&format!("oracle @root (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs {{ r:reg[1], s:reg[1], fr:reg[4] }}\n^entry():\n{rows}\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x")), false).unwrap();
            let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"*","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#, ids.interface["root"])).unwrap();
            let error = catalog
                .lower_oracle_with_cleanup(
                    &obj,
                    "root",
                    &json::parse(r#"[["1","0","0","0"]]"#).unwrap(),
                    &json::parse(r#"{"token":["3","0","0","0"]}"#).unwrap(),
                    0,
                    &[CleanupEntry {
                        selector: Selector::parse(&selector).unwrap(),
                        realisation: choice,
                    }],
                )
                .unwrap_err();
            assert!(error.contains("§5.5"), "{error}");
        }
    }
    let (obj, _) = bennett_vm::load(
        &support::source(
            "oracle @root (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x",
        ),
        false,
    )
    .unwrap();
    for context in [
        r#"{}"#,
        r#"{"token":["1","0","0"]}"#,
        r#"{"token":["01","0","0","0"]}"#,
        r#"{"token":["1","1","0","0"]}"#,
        r#"{"token":["1","0","0","0"],"extra":0}"#,
    ] {
        assert!(
            catalog
                .lower_oracle_with_cleanup(
                    &obj,
                    "root",
                    &json::parse("[]").unwrap(),
                    &json::parse(context).unwrap(),
                    0,
                    &[]
                )
                .is_err(),
            "{context}"
        );
    }
}

#[test]
fn nested_oracle_carriers_keep_last_oracle_step_and_fresh_call_ownership() {
    let (obj, ids) = bennett_vm::load(&support::source("oracle @root (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs { r:reg[1], fr:reg[4] }\n^entry():\nlift.oracle_call @middle tk (x) r fr () (tk)\nlift.oracle_uncall @middle tk (x) r fr () (tk)\nlift.retval r\noracle @middle (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs { r:reg[1], fr:reg[4] }\n^entry():\nlift.oracle_call @id tk (x) r fr () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x"), false).unwrap();
    let catalog = Catalog::published().unwrap();
    for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
        // Each middle invocation gets B=1: the outer B=0/1 is independent.
        // The middle's uncall still has an explicit predicated carrier.
        let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"entry:1","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#, ids.interface["root"])).unwrap();
        let e = catalog
            .lower_oracle_with_cleanup(
                &obj,
                "root",
                &json::parse(r#"[["1","0","0","0"]]"#).unwrap(),
                &json::parse(r#"{"token":["1","0","0","0"]}"#).unwrap(),
                1,
                &[CleanupEntry {
                    selector: Selector::parse(&selector).unwrap(),
                    realisation: choice,
                }],
            )
            .unwrap();
        let nested = e
            .cleanups
            .iter()
            .filter(|c| c.address.path.len() == 2)
            .collect::<Vec<_>>();
        assert_eq!(
            nested.len(),
            if choice == CleanupRealisation::Reverse {
                8
            } else {
                12
            }
        );
        for c in nested {
            assert_eq!(c.address.path[1].symbol, ids.interface["middle"]);
            assert_eq!(c.address.path[1].point, "entry:1");
            assert_eq!(c.address.step, Some(1));
            assert!(c.controls.iter().any(|w| w == "$controls[0]"));
            assert!(c.address.path[0].call_index.is_some());
        }
        let fresh_calls = e
            .leaves
            .iter()
            .filter_map(|l| {
                let a = l.address.as_ref().unwrap();
                (a.path.len() > 1 && a.path[0].point == "entry:1")
                    .then(|| a.path[0].call_index.unwrap())
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            fresh_calls,
            if choice == CleanupRealisation::Reverse {
                [0].into()
            } else {
                [0, 1].into()
            }
        );
        for bits in 0..8 {
            let state = std::collections::BTreeMap::from([
                ("x[0]".to_string(), bits & 1 != 0),
                ("t[0]".into(), bits & 2 != 0),
                ("$controls[0]".into(), bits & 4 != 0),
                ("f[0]".into(), false),
                ("f[1]".into(), false),
                ("f[2]".into(), false),
                ("f[3]".into(), false),
            ]);
            let mut expected = state.clone();
            if bits & 4 != 0 {
                *expected.get_mut("f[0]").unwrap() = true;
            }
            assert_eq!(replay(&e.leaves, state), expected);
        }
    }
}

#[test]
fn source_rows_preserve_assignment_capture_widths_and_branch_successors() {
    let programs = [
        "a = lift.select p x y\nb = lift.trunc.i1 a\nc = lift.zext.i2 b\nd = lift.sext.i2 b\n(a, r) = lift.parmov (c, d)\nlift.retval r",
        "r = lift.add x y\nr = lift.xor r x\nqbranch p ^left ^right\n^left():\nlift.retval r\n^right():\nr = lift.sub r y\nlift.retval r",
    ];
    let catalog = Catalog::published().unwrap();
    for rows in programs {
        let (obj, _) = bennett_vm::load(&support::source(&format!("oracle @root (x:wire[2] in, y:wire[2] in, p:wire in) (t:wire[2]) (f:wire[4]) () ()\nregs {{ a:reg[2], b:reg[1], c:reg[2], d:reg[2], r:reg[2] }}\n^entry():\n{rows}")), false).unwrap();
        let e = catalog
            .lower_oracle_with_cleanup(
                &obj,
                "root",
                &json::parse("[]").unwrap(),
                &json::parse(r#"{"token":["6","0","0","0"]}"#).unwrap(),
                0,
                &[],
            )
            .unwrap();
        for bits in 0..32 {
            let mut state = std::collections::BTreeMap::new();
            for (name, width, value) in [
                ("x", 2, bits & 3),
                ("y", 2, (bits >> 2) & 3),
                ("p", 1, bits >> 4),
                ("t", 2, 3),
                ("f", 4, 9),
            ] {
                for i in 0..width {
                    state.insert(format!("{name}[{i}]"), value & (1 << i) != 0);
                }
            }
            let out = bennett_vm::oracle::run_oracle(
                &obj,
                "root",
                &[
                    bennett_vm::bigint::BigUint::from_u64(bits & 3),
                    bennett_vm::bigint::BigUint::from_u64((bits >> 2) & 3),
                    bennett_vm::bigint::BigUint::from_u64(bits >> 4),
                ],
                &mut [],
                &[],
                bennett_vm::value::Budget {
                    steps: 6,
                    heap: 0,
                    stack: 0,
                    depth: 0,
                },
            )
            .unwrap();
            let mut expected = state.clone();
            for i in 0..2 {
                *expected.get_mut(&format!("t[{i}]")).unwrap() ^= out.payload.bit(i);
            }
            for i in 0..4 {
                *expected.get_mut(&format!("f[{i}]")).unwrap() ^= out.code.bit(i);
            }
            assert_eq!(replay(&e.leaves, state), expected, "{rows} {bits}");
        }
    }
}

#[test]
fn nested_constructor_keeps_fixed_classical_values_separate_from_readable_bits() {
    let (obj, _) = bennett_vm::load(&support::source("oracle @root (x:wire in) (t:wire) (f:wire[4]) () (tk:token, a:i1)\nregs { r:reg[1], fr:reg[4] }\n^entry():\nlift.oracle_call @id tk (a) r fr () (a)\nlift.oracle_uncall @id tk (a) r fr () (a)\nlift.retval a\noracle @id (x:wire in) (t:wire) (f:wire[4]) () (a:i1)\n^entry():\nlift.retval a"), false).unwrap();
    let catalog = Catalog::published().unwrap();
    for a in 0..2 {
        let e = catalog
            .lower_oracle_with_cleanup(
                &obj,
                "root",
                &json::parse(&format!(r#"[["1","0","0","0"],"{a}"]"#)).unwrap(),
                &json::parse(r#"{"token":["3","0","0","0"]}"#).unwrap(),
                0,
                &[],
            )
            .unwrap();
        for x in [false, true] {
            let state = std::collections::BTreeMap::from([
                ("x[0]".to_string(), x),
                ("t[0]".into(), false),
                ("f[0]".into(), false),
                ("f[1]".into(), false),
                ("f[2]".into(), false),
                ("f[3]".into(), false),
            ]);
            let mut expected = state.clone();
            expected.insert("t[0]".into(), a != 0);
            assert_eq!(replay(&e.leaves, state), expected);
        }
    }
}
