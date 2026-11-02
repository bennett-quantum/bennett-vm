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
            // Parsed only: the constructor must refuse these on its own (load does too).
            let obj = bennett_vm::object::parse(&support::source(&format!("oracle @root (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs {{ r:reg[1], s:reg[1], fr:reg[4] }}\n^entry():\n{rows}\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x"))).unwrap();
            let ids = bennett_vm::identity::compute(&obj).unwrap();
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

#[test]
fn action_oracle_calls_capture_each_source_token_under_controls_and_adjoints() {
    let (obj, ids) = bennett_vm::load(&support::source("action @root (t:wire mut, f:wire[4] mut) (x:wire in, p:wire in) () (short:token, enough:token) ()\n^entry():\nctl p oracle_call @outer short (x) t f () (enough)\nctl p oracle_call @outer enough (x) t f () (enough)\noracle @outer (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs { r:reg[1], fr:reg[4] }\n^entry():\nlift.oracle_call @id tk (x) r fr () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval x\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x"), false).unwrap();
    let catalog = Catalog::published().unwrap();
    for adjoint in [false, true] {
        for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"entry:1","iteration":"*","call_index":0}},{{"symbol":"{}","point":"entry:1","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#, ids.interface["root"], ids.interface["outer"])).unwrap();
            let e = catalog
                .lower_action_with_cleanup(
                    &obj,
                    "root",
                    &json::parse(r#"[["0","0","0","0"],["3","0","0","0"]]"#).unwrap(),
                    adjoint,
                    0,
                    &[CleanupEntry {
                        selector: Selector::parse(&selector).unwrap(),
                        realisation: choice,
                    }],
                )
                .unwrap();
            // B=0 emits no steps/carriers, B=3 emits one carrier per step/phase.
            assert_eq!(e.cleanups.len(), 6);
            for c in &e.cleanups {
                assert_eq!(c.realisation, choice);
                assert_eq!(c.address.path.len(), 2);
                assert_eq!(c.address.path[0].symbol, ids.interface["root"]);
                assert_eq!(c.address.path[0].point, "entry:1");
                assert_eq!(c.address.path[0].call_index, Some(0));
                assert_eq!(c.address.path[1].symbol, ids.interface["outer"]);
                assert_eq!(c.address.path[1].point, "entry:1");
                assert!(c.controls.iter().any(|w| w == "p[0]"));
            }
            for bits in 0..128 {
                let mut state = std::collections::BTreeMap::from([
                    ("x[0]".to_string(), bits & 1 != 0),
                    ("t[0]".into(), bits & 2 != 0),
                    ("p[0]".into(), bits & 64 != 0),
                ]);
                for i in 0..4 {
                    state.insert(format!("f[{i}]"), bits & (4 << i) != 0);
                }
                let mut expected = state.clone();
                if bits & 64 != 0 {
                    *expected.get_mut("t[0]").unwrap() ^= bits & 1 != 0;
                    *expected.get_mut("f[0]").unwrap() ^= true;
                }
                assert_eq!(replay(&e.leaves, state), expected, "{adjoint} {bits}");
            }
        }
    }
}

#[test]
fn action_oracle_source_arguments_keep_the_admission_type_guard() {
    let text = support::source(
        "action @root (t:wire mut, f:wire[4] mut) (x:wire in) () (tk:token) ()\n^entry():\noracle_call @pick tk (x) t f () (-1)\noracle @pick (x:wire in) (t:wire) (f:wire[4]) () (a:i1)\n^entry():\nlift.retval a",
    );
    // Source argument slots require typed names. Lowering must not bypass this
    // guard merely because a raw catalog value could encode the same bits.
    let error = bennett_vm::load(&text, false).expect_err("untyped source argument admitted");
    assert!(
        error.contains("call classical/token argument type mismatch"),
        "{error}"
    );
}

#[test]
fn action_oracle_outputs_invalidate_earlier_external_input_versions() {
    let (obj, ids) = bennett_vm::load(&support::source("action @root (x:wire[2] mut, f:wire[4] mut) (z:wire[2] in) () (tk:token) ()\n^entry():\nd = qcopy x\noracle_call @id tk (z) x f () ()\naction_call @mix (d) () () ()\nadj action_call @mix (d) () () ()\noracle_call @id tk (z) x f () ()\nquncopy x d\nperm_action @mix (d:wire[2] mut) () () () ()\n^entry():\ncx d[0] d[1]\noracle @id (x:wire[2] in) (t:wire[2]) (f:wire[4]) () ()\n^entry():\nlift.retval x"), false).unwrap();
    let catalog = Catalog::published().unwrap();
    let params = json::parse(r#"[["1","0","0","0"]]"#).unwrap();
    let reverse = catalog
        .lower_action_with_cleanup(&obj, "root", &params, false, 0, &[])
        .unwrap();
    assert!(reverse.cleanup_pairing_complete);
    for bits in 0..256 {
        let mut state = std::collections::BTreeMap::new();
        for (name, width, value) in [
            ("x", 2, bits & 3),
            ("z", 2, (bits >> 2) & 3),
            ("f", 4, bits >> 4),
        ] {
            for i in 0..width {
                state.insert(format!("{name}[{i}]"), value & (1 << i) != 0);
            }
        }
        assert_eq!(replay(&reverse.leaves, state.clone()), state);
    }
    let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"entry:3","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#,ids.interface["root"])).unwrap();
    let error = match catalog.lower_action_with_cleanup(
        &obj,
        "root",
        &params,
        false,
        0,
        &[CleanupEntry {
            selector: Selector::parse(&selector).unwrap(),
            realisation: CleanupRealisation::Recompute,
        }],
    ) {
        Err(error) => error,
        Ok(_) => panic!("recompute silently substituted a later external input version"),
    };
    assert!(
        error.contains("exact clean-input computing sequence"),
        "{error}"
    );
}

fn assert_map(text: &str, params: &str, budget: [u64; 4], inputs: &[(&str, usize)], cases: usize) {
    assert_map_choice(text, params, budget, inputs, cases, None);
}
fn assert_map_choice(
    text: &str,
    params: &str,
    budget: [u64; 4],
    inputs: &[(&str, usize)],
    cases: usize,
    choice: Option<CleanupRealisation>,
) {
    use bennett_vm::{bigint::BigUint, value::Budget};
    let (obj, _) = bennett_vm::load(text, false).unwrap();
    let catalog = Catalog::published().unwrap();
    let parameters = json::parse(params).unwrap();
    let values = bennett_vm::invocation::decode_args(
        &parameters,
        &obj.function("root").unwrap().lists[4]
            .iter()
            .map(|p| p.ty.clone())
            .collect::<Vec<_>>(),
        obj.ptr_width,
    )
    .unwrap();
    let context = json::parse(&format!(
        r#"{{"token":["{}","{}","{}","{}"]}}"#,
        budget[0], budget[1], budget[2], budget[3]
    ))
    .unwrap();
    let entries = choice
        .into_iter()
        .map(|realisation| CleanupEntry {
            selector: Selector::parse(
                &json::parse(r#"{"path":[],"step":"*","phase":"*","generated":0}"#).unwrap(),
            )
            .unwrap(),
            realisation,
        })
        .collect::<Vec<_>>();
    let e = catalog
        .lower_oracle_with_cleanup(&obj, "root", &parameters, &context, 0, &entries)
        .unwrap();
    if let Some(choice) = choice {
        assert!(e.cleanup_pairing_complete);
        assert!(!e.cleanups.is_empty());
        assert!(e.cleanups.iter().all(|c| c.realisation == choice));
    }
    let target = match obj.function("root").unwrap().lists[1][0].ty {
        bennett_vm::object::Type::Wire(w) => w,
        _ => panic!(),
    };
    for bits in 0..cases {
        let mut state = std::collections::BTreeMap::new();
        let mut shift = 0;
        let mut ins = vec![];
        for &(name, width) in inputs {
            let v = (bits >> shift) & ((1 << width) - 1);
            ins.push(BigUint::from_u64(v as u64));
            for i in 0..width {
                state.insert(format!("{name}[{i}]"), v & (1 << i) != 0);
            }
            shift += width;
        }
        for (name, width) in [("t", target), ("f", 4)] {
            for i in 0..width {
                state.insert(format!("{name}[{i}]"), i % 2 == 0);
            }
        }
        let out = bennett_vm::oracle::run_oracle(
            &obj,
            "root",
            &ins,
            &mut [],
            &values,
            Budget {
                steps: budget[0],
                heap: budget[1],
                stack: budget[2],
                depth: budget[3],
            },
        )
        .unwrap();
        let mut expected = state.clone();
        for (name, width, value) in [("t", target, out.payload), ("f", 4, out.code)] {
            for i in 0..width {
                *expected.get_mut(&format!("{name}[{i}]")).unwrap() ^= value.bit(i);
            }
        }
        assert_eq!(
            replay(&e.leaves, state),
            expected,
            "input {bits}, budget {budget:?}"
        );
    }
}

#[test]
fn source_memory_descriptors_restore_heap_table_and_displaced_bytes() {
    let text = support::source(
        "oracle @root (x:wire[8] in) (t:wire[8]) (f:wire[4]) () ()\nregs { p:reg[2], n:reg[2], r:reg[8] }\n^entry():\nn = lift.const.i2 1\np = lift.alloc n\nlift.store.i8 p x\nr = lift.load.i8 p\nlift.boundscheck p n\nlift.free p\nlift.retval r",
    );
    let text = support::sign(&format!("{}\nsection layout\n{}\n",text.replace("ptr_width=32","ptr_width=2"),json::parse(r#"{"ptr_width":2,"H":2,"A_tab":1,"S_cells":3,"null_size":1,"heap_base":2,"heap_limit_max":1,"globals":{},"interface_regions":{}}"#).unwrap().to_jcs()));
    for steps in [0, 2, 4, 7, 8] {
        assert_map(&text, "[]", [steps, 1, 0, 0], &[("x", 8)], 4);
    }
    assert_map(&text, "[]", [7, 0, 0, 0], &[("x", 8)], 2);
}

#[test]
fn mapped_memory_lifted_cleanup_preserves_interface_cells_for_both_choices() {
    let raw = support::source(
        "oracle @root () (t:wire[8]) (f:wire[4]) (m:region[0,8] in) (tk:token)\nregs { r:reg[8], fr:reg[4] }\n^entry():\nlift.oracle_call @read tk () r fr (m) ()\nlift.oracle_uncall @read tk () r fr (m) ()\nlift.retval r\noracle @read () (t:wire[8]) (f:wire[4]) (m:region[0,8] in) ()\nregs { p:reg[2], r:reg[8] }\n^entry():\np = lift.addr @m\nr = lift.load.i8 p\nlift.retval r",
    );
    let end = raw.find("\nsection layout").unwrap();
    let layout = json::parse(r#"{"ptr_width":2,"H":2,"A_tab":1,"S_cells":3,"null_size":1,"heap_base":3,"heap_limit_max":0,"globals":{},"interface_regions":{"root":{"m":{"base":1}},"read":{"m":{"base":2}}}}"#).unwrap();
    let text = support::sign(&format!(
        "{}\nsection layout\n{}\n",
        raw[..end].replace("ptr_width=32", "ptr_width=2"),
        layout.to_jcs()
    ));
    let (obj, ids) = bennett_vm::load(&text, false).unwrap();
    let selector=json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"entry:1","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#,ids.interface["root"])).unwrap();
    for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
        let e = Catalog::published()
            .unwrap()
            .lower_oracle_with_cleanup(
                &obj,
                "root",
                &json::parse(r#"[["3","0","0","0"]]"#).unwrap(),
                &json::parse(r#"{"token":["3","0","0","0"]}"#).unwrap(),
                1,
                &[CleanupEntry {
                    selector: Selector::parse(&selector).unwrap(),
                    realisation: choice,
                }],
            )
            .unwrap();
        assert_eq!(e.cleanups.len(), 6);
        for byte in [0, 1, 128, 255] {
            for ctl in [false, true] {
                let mut state = std::collections::BTreeMap::from([("$controls[0]".into(), ctl)]);
                for (name, w, v) in [("m", 8, byte), ("t", 8, 165), ("f", 4, 10)] {
                    for i in 0..w {
                        state.insert(format!("{name}[{i}]"), v & (1 << i) != 0);
                    }
                }
                assert_eq!(
                    replay(&e.leaves, state.clone()),
                    state,
                    "{choice:?} {byte} {ctl}"
                );
            }
        }
    }
}

fn stack_layout(text: &str) -> String {
    let layout=json::parse(r#"{"ptr_width":2,"H":2,"A_tab":0,"S_cells":4,"null_size":1,"heap_base":2,"heap_limit_max":0,"globals":{},"interface_regions":{}}"#).unwrap();
    support::sign(&format!(
        "{}\nsection layout\n{}\n",
        text.replace("ptr_width=32", "ptr_width=2"),
        layout.to_jcs()
    ))
}
#[test]
fn fixed_lproc_instances_capture_arguments_and_restore_call_return_counters() {
    let text = stack_layout(&support::source(
        "oracle @root (x:wire in) (t:wire) (f:wire[4]) () (a:i1, b:i1)\nregs { r:reg[1], s:reg[1] }\n^entry():\nr = lift.call @work (x) () (a) ()\ns = lift.call @work (r) () (b) ()\nlift.retval s\nlproc @work (x:reg[1]) (r:reg[1]) () (a:i1) ()\n^entry():\nr = lift.xor x a\nlift.ret r",
    ));
    for steps in [1, 3, 7] {
        assert_map(&text, r#"["1","0"]"#, [steps, 0, 16, 2], &[("x", 1)], 2);
    }
    for (stack, depth) in [(0, 2), (16, 0)] {
        assert_map(&text, r#"["1","0"]"#, [8, 0, stack, depth], &[("x", 1)], 2);
    }
}
#[test]
fn recursive_lproc_capture_and_return_restore_self_aliases_and_budget_faults() {
    let text = stack_layout(&support::source(
        "oracle @root (x:wire[2] in) (t:wire[2]) (f:wire[4]) () ()\nregs { r:reg[2] }\n^entry():\nr = lift.call @walk (x)\nlift.retval r\nlproc @walk (x:reg[2]) (r:reg[2]) () () ()\nregs { zero:reg[2], one:reg[2], p:reg[1] }\n^entry():\np = lift.icmp.eq x zero\nqbranch p ^done ^again\n^again():\none = lift.const.i2 1\nx = lift.sub x one\nr = lift.call @walk (x)\nlift.ret r\n^done():\nlift.ret x",
    ));
    for steps in [1, 9, 14] {
        assert_map(&text, "[]", [steps, 0, 16, 2], &[("x", 2)], 4);
    }
}

#[test]
fn lproc_lifted_uncall_carriers_keep_fixed_instance_keys_for_both_choices() {
    let text = stack_layout(&support::source(
        "oracle @root (x:wire in) (t:wire) (f:wire[4]) () (tk:token, a:i1, b:i1)\nregs { r:reg[1], s:reg[1] }\n^entry():\nr = lift.call @work (x) (tk) (a) ()\ns = lift.call @work (r) (tk) (b) ()\nlift.retval s\nlproc @work (x:reg[1]) (r:reg[1]) (tk:token) (a:i1) ()\nregs { fr:reg[4] }\n^entry():\nlift.oracle_call @id tk (x) r fr () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nr = lift.xor x a\nlift.ret r\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x",
    ));
    let (obj, ids) = bennett_vm::load(&text, false).unwrap();
    let selector=json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"entry:1","instance":[0,1],"iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#,ids.interface["work"])).unwrap();
    for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
        let e = Catalog::published()
            .unwrap()
            .lower_oracle_with_cleanup(
                &obj,
                "root",
                &json::parse(r#"[["1","0","0","0"],"1","0"]"#).unwrap(),
                &json::parse(r#"{"token":["11","0","16","2"]}"#).unwrap(),
                0,
                &[CleanupEntry {
                    selector: Selector::parse(&selector).unwrap(),
                    realisation: choice,
                }],
            )
            .unwrap();
        assert!(e.cleanup_pairing_complete);
        for key in [vec![0, 1], vec![0, 2]] {
            let carriers = e
                .cleanups
                .iter()
                .filter(|c| c.address.path[0].instance == key)
                .collect::<Vec<_>>();
            assert_eq!(carriers.len(), 22);
            for c in carriers {
                assert_eq!(
                    c.realisation,
                    if key == vec![0, 1] {
                        choice
                    } else {
                        CleanupRealisation::Reverse
                    }
                );
            }
        }
        for x in [false, true] {
            let state = std::collections::BTreeMap::from([
                ("x[0]".into(), x),
                ("t[0]".into(), false),
                ("f[0]".into(), false),
                ("f[1]".into(), false),
                ("f[2]".into(), false),
                ("f[3]".into(), false),
            ]);
            let mut expected = state.clone();
            expected.insert("t[0]".into(), !x);
            assert_eq!(replay(&e.leaves, state), expected);
        }
    }
}

#[test]
fn source_global_image_and_pinned_entry_survive_store_load_and_fault_reversal() {
    for rows in [
        "p = lift.addr @seed\nlift.store.i8 p x\nr = lift.load.i8 p\nlift.retval r",
        "p = lift.addr @seed\nr = lift.load.i8 p\nlift.free p\nlift.retval r",
    ] {
        let raw = support::source(&format!(
            "oracle @root (x:wire[8] in) (t:wire[8]) (f:wire[4]) () ()\nregs {{ p:reg[2], r:reg[8] }}\n^entry():\n{rows}"
        ));
        let mut lines = raw
            .replace("ptr_width=32", "ptr_width=2")
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let symbol_line = lines.iter().position(|l| l == "section symbols").unwrap() + 1;
        let json::Json::Obj(ref mut symbols) = json::parse(&lines[symbol_line]).unwrap() else {
            panic!()
        };
        symbols.insert("1".into(),json::parse(r#"{"name":"seed","kind":"data","signature":{"abi":"0.9","kind":"data","size":1,"tier":"-"},"interface_hash":"0000000000000000000000000000000000000000000000000000000000000000"}"#).unwrap());
        lines[symbol_line] = json::Json::Obj(symbols.clone()).to_jcs();
        let layout=json::parse(r#"{"ptr_width":2,"H":2,"A_tab":1,"S_cells":3,"null_size":1,"heap_base":2,"heap_limit_max":1,"globals":{"seed":{"base":1,"size":1,"image":"seed"}},"interface_regions":{}}"#).unwrap();
        let text = support::sign(&format!(
            "{}\n\nsection data\nseed 81\n\nsection layout\n{}\n",
            lines.join("\n"),
            layout.to_jcs()
        ));
        for steps in [2, 3, 4] {
            assert_map(&text, "[]", [steps, 1, 0, 0], &[("x", 8)], 4);
        }
    }
}

#[test]
fn source_flagged_division_and_fault_rows_preserve_effective_causes_and_prefixes() {
    for op in ["udiv", "urem", "sdiv", "srem"] {
        let text = support::source(&format!(
            "oracle @root (a:wire[2] in, b:wire[2] in) (t:wire[2]) (f:wire[4]) () ()\nregs {{ r:reg[2] }}\n^entry():\nr = lift.{op} a b\nlift.retval r"
        ));
        for steps in [1, 2, 3] {
            assert_map(&text, "[]", [steps, 0, 0, 0], &[("a", 2), ("b", 2)], 16);
        }
        let text = support::source(&format!(
            "oracle @root (a:wire[2] in, b:wire[2] in) (t:wire[2]) (f:wire[4]) () ()\nregs {{ r:reg[2] }}\n^entry():\nr = lift.mov a\nr = lift.{op} r b\nlift.retval r"
        ));
        assert_map(&text, "[]", [3, 0, 0, 0], &[("a", 2), ("b", 2)], 16);
    }
    for cause in [0, 4, 9] {
        let text = support::source(&format!(
            "oracle @root (p:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.faultif p {cause}\nlift.retval p"
        ));
        for steps in [0, 1, 2, 3] {
            assert_map(&text, "[]", [steps, 0, 0, 0], &[("p", 1)], 2);
        }
        let text = support::source(&format!(
            "oracle @root () (t:wire) (f:wire[4]) () ()\n^entry():\nfault {cause}"
        ));
        for steps in [0, 1, 2] {
            assert_map(&text, "[]", [steps, 0, 0, 0], &[], 1);
        }
    }
}
#[test]
fn source_indirect_rows_keep_canonical_interior_targets_and_instance_ranges() {
    let text = support::source(
        "oracle @root (a:wire[4] in) (t:wire[4]) (f:wire[4]) () ()\nregs { r:reg[4] }\n^entry():\nqjumpr a\n^done():\nr = lift.const.i4 7\nlift.retval r",
    );
    for steps in [0, 1, 2, 3, 4] {
        assert_map(&text, "[]", [steps, 0, 0, 0], &[("a", 4)], 16);
    }
    let text = stack_layout(&support::source(
        "oracle @root (a:wire[4] in) (t:wire[4]) (f:wire[4]) () ()\nregs { r:reg[4] }\n^entry():\nr = lift.call @jump (a)\nlift.retval r\nlproc @jump (a:reg[4]) (r:reg[4]) () () ()\n^entry():\nqjumpr a\n^done():\nr = lift.const.i4 9\nlift.ret r",
    ));
    assert_map(&text, "[]", [5, 0, 16, 2], &[("a", 4)], 16);
}

#[test]
fn indirect_pair_carriers_execute_computed_cycles_under_controls_and_adjoints() {
    for body in [
        "pc = lift.label ^done\none = lift.const.i4 1\npc = lift.add pc one\nlift.oracle_call @id tk (x) r fr () ()\nqjumpr pc\n^done():\nr = lift.mov x\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval x",
        "pc = lift.label ^body\nqjumpr pc\n^body():\nlift.oracle_call @id tk (x) r fr () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nqjumpr pc",
    ] {
        let text = support::source(&format!(
            "oracle @outer (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs {{ r:reg[1], fr:reg[4], pc:reg[4], one:reg[4] }}\n^entry():\n{body}\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x"
        ));
        let (obj, ids) = bennett_vm::load(&text, false).unwrap();
        let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"{}","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#, ids.interface["outer"], if body.contains("^done") {"done:1"} else {"body:1"})).unwrap();
        for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            let e = Catalog::published()
                .unwrap()
                .lower_oracle_with_cleanup(
                    &obj,
                    "outer",
                    &json::parse(r#"[["1","0","0","0"]]"#).unwrap(),
                    &json::parse(r#"{"token":["8","0","0","0"]}"#).unwrap(),
                    1,
                    &[CleanupEntry {
                        selector: Selector::parse(&selector).unwrap(),
                        realisation: choice,
                    }],
                )
                .unwrap();
            assert!(e.cleanup_pairing_complete);
            assert_eq!(e.cleanups.len(), 16);
            assert!(e.cleanups.iter().all(
                    |c| c.realisation == choice && c.controls.contains(&"$controls[0]".into())
                ));
            for adjoint in [false, true] {
                let leaves = if adjoint {
                    e.leaves
                        .iter()
                        .rev()
                        .cloned()
                        .map(|mut leaf| {
                            leaf.operation = match leaf.operation.as_str() {
                                "alloc" => "release".into(),
                                "release" => "alloc".into(),
                                _ => leaf.operation,
                            };
                            leaf
                        })
                        .collect::<Vec<_>>()
                } else {
                    e.leaves.clone()
                };
                for bits in 0..128 {
                    let mut state = std::collections::BTreeMap::from([
                        ("x[0]".into(), bits & 1 != 0),
                        ("t[0]".into(), bits & 2 != 0),
                        ("$controls[0]".into(), bits & 64 != 0),
                    ]);
                    for i in 0..4 {
                        state.insert(format!("f[{i}]"), bits & (4 << i) != 0);
                    }
                    let mut expected = state.clone();
                    if bits & 64 != 0 {
                        if body.contains("^done") {
                            *expected.get_mut("t[0]").unwrap() ^= bits & 1 != 0;
                        } else {
                            *expected.get_mut("f[0]").unwrap() ^= true;
                        }
                    }
                    assert_eq!(
                        replay(&leaves, state),
                        expected,
                        "{adjoint} {choice:?} {bits}"
                    );
                }
            }
        }
    }
}

#[test]
fn indirect_lproc_pairs_keep_canonical_instance_offsets_and_fault_exits() {
    // Two fixed keys put ^done at distinct canonical row indices. A local
    // numeric zero is outside either worker, and must fault rather than enter
    // the oracle or another instance. Both retain authenticated source pairs.
    for jump in ["pc", "zero"] {
        let path = if jump == "pc" {
            "qjumpr pc\n^done():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.ret x"
        } else {
            "lift.oracle_uncall @id tk (x) r fr () ()\nqjumpr zero\n^done():\nlift.ret x"
        };
        let text = stack_layout(&support::source(&format!(
            "oracle @root (x:wire in) (t:wire) (f:wire[4]) () (tk:token, a:i1, b:i1)\nregs {{ r:reg[1] }}\n^entry():\nqbranch x ^left ^right\n^left():\nr = lift.call @work (x) (tk) (a) ()\nqjump ^end\n^right():\nr = lift.call @work (x) (tk) (b) ()\nqjump ^end\n^end():\nlift.retval r\nlproc @work (x:reg[1]) (r:reg[1]) (tk:token) (a:i1) ()\nregs {{ fr:reg[4], pc:reg[5], zero:reg[5] }}\n^entry():\npc = lift.label ^done\nzero = lift.const.i5 0\nlift.oracle_call @id tk (x) r fr () ()\n{path}\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x"
        )));
        // Mutually exclusive calls still execute both fixed keys across x=0/1,
        // with one eight-cell activation record rather than two live records.
        let text = support::sign(&text.replace("\"S_cells\":4", "\"S_cells\":3"));
        let (obj, _) = bennett_vm::load(&text, false).unwrap();
        let mut bindings = bennett_vm::ref_library::Bindings::new();
        bindings.address_oracle(&obj, "root").unwrap();
        let pairs = bindings.oracle_cleanup_pairs().unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].cleanup_point.instance, [0, 1]);
        assert_eq!(pairs[1].cleanup_point.instance, [0, 2]);
        // Source execution and complete private release, including the second
        // instance's distinct canonical target and the out-of-range fault.
        for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            assert_map_choice(
                &text,
                r#"[["1","0","0","0"],"0","1"]"#,
                [10, 0, 8, 2],
                &[("x", 1)],
                2,
                Some(choice),
            );
        }
    }
}
#[test]
fn source_permutation_rows_use_the_semantic_callee_and_source_adjoint() {
    let text = support::source(
        "oracle @root (x:wire[2] in) (t:wire[2]) (f:wire[4]) () (a:i1)\nregs { r:reg[2] }\n^entry():\nr = lift.mov x\nlift.perm_call @mix (r) () () (a)\nlift.retval r\nperm_action @mix (r:wire[2] mut) () () (a:i1) ()\n^entry():\nx r[0]\ncx r[0] r[1]",
    );
    for a in [0, 1] {
        for steps in [2, 3] {
            assert_map(
                &text,
                &format!(r#"["{a}"]"#),
                [steps, 0, 0, 0],
                &[("x", 2)],
                4,
            );
        }
    }
}

#[test]
fn zero_literal_fault_causes_are_traps_in_source_execution() {
    use bennett_vm::{bigint::BigUint, value::Budget};
    for conditional in [false, true] {
        let rows = if conditional {
            "lift.faultif p 0\nlift.retval p"
        } else {
            "fault 0"
        };
        let (obj, _) = bennett_vm::load(
            &support::source(&format!(
                "oracle @root (p:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\n{rows}"
            )),
            false,
        )
        .unwrap();
        for steps in [0, 1, 2, 3] {
            for p in [0, 1] {
                let out = bennett_vm::oracle::run_oracle(
                    &obj,
                    "root",
                    &[BigUint::from_u64(p)],
                    &mut [],
                    &[],
                    Budget {
                        steps,
                        heap: 0,
                        stack: 0,
                        depth: 0,
                    },
                )
                .unwrap();
                let expected = if steps == 0 {
                    1
                } else if !conditional || p == 1 {
                    4
                } else if steps == 1 {
                    1
                } else {
                    0
                };
                assert_eq!(
                    out.code,
                    BigUint::from_u64(expected),
                    "{conditional} {steps} {p}"
                );
            }
        }
    }
}
