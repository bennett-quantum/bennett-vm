//! Fresh cleanup traversal owns contiguous caller spaces (ISA pass X/K80).
use crate::source09 as support;
use bennett_vm::{
    json::{self, Json},
    ref_library::{Catalog, CleanupEntry, CleanupRealisation},
    sched::Selector,
};

fn replay_identity(e: &bennett_vm::ref_library::ActionExpansion, values: &[(&str, bool)]) {
    replay_identity_with_borrow(e, values, false);
}
fn replay_identity_with_borrow(
    e: &bennett_vm::ref_library::ActionExpansion,
    values: &[(&str, bool)],
    borrowed: bool,
) {
    let mut state = values
        .iter()
        .map(|(n, v)| (n.to_string(), *v))
        .collect::<std::collections::BTreeMap<_, _>>();
    let original = state.clone();
    for l in &e.leaves {
        match l.operation.as_str() {
            "alloc" => {
                for w in &l.wires {
                    let value = l.state.as_deref() == Some("borrowed") && borrowed;
                    assert!(state.insert(w.clone(), value).is_none());
                }
            }
            "release" => {
                for w in &l.wires {
                    let value = l.state.as_deref() == Some("borrowed") && borrowed;
                    assert_eq!(state.remove(w), Some(value));
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
            op => panic!("unexpected {op}"),
        }
    }
    assert_eq!(state, original);
}

fn recompute(
    functions: &str,
    point: &str,
    adjoint: bool,
) -> bennett_vm::ref_library::ActionExpansion {
    let (object, ids) = bennett_vm::load(&support::source(functions), false).unwrap();
    let selector = json::parse(&format!(
        r#"{{"path":[{{"symbol":"{}","point":"{point}","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#,
        ids.interface["root"]
    )).unwrap();
    let expansion = Catalog::published()
        .unwrap()
        .lower_action_with_cleanup(
            &object,
            "root",
            &Json::Arr(vec![]),
            adjoint,
            0,
            &[CleanupEntry {
                selector: Selector::parse(&selector).unwrap(),
                realisation: CleanupRealisation::Recompute,
            }],
        )
        .unwrap();
    assert!(
        expansion
            .cleanups
            .iter()
            .any(|c| c.realisation == CleanupRealisation::Recompute
                && c.address.path[0].point == point),
        "the requested cleanup must actually own a recompute carrier at {point}"
    );
    expansion
}

#[test]
fn fresh_primitive_uncomputation_has_emitted_ordinals_not_raw_inverse_ordinals() {
    let functions = "action @root (x:wire mut, z:wire mut) () () () ()\n^entry():\nd = qalloc any 1\ncx x d\ncx z d\ncx z d\ncx x d\nqfree d";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:0" } else { "entry:5" },
            adjoint,
        );
        let replacement = &e.leaves[3..];
        assert_eq!(
            replacement
                .iter()
                .map(|l| l.address.as_ref().unwrap().generated)
                .collect::<Vec<_>>(),
            (0..8).collect::<Vec<_>>()
        );
        assert_eq!(
            replacement
                .iter()
                .map(|l| l.operation.as_str())
                .collect::<Vec<_>>(),
            ["alloc", "cx", "cx", "cx", "cx", "cx", "release", "release"]
        );
    }
}

#[test]
fn fresh_call_uncomputation_has_separate_ordered_occurrences_and_callee_spaces() {
    let functions = "action @root (x:wire mut, z:wire mut) () () () ()\n^entry():\nd = qalloc any 1\naction_call @copy (d) (x) () ()\naction_call @copy (d) (z) () ()\nadj action_call @copy (d) (z) () ()\nadj action_call @copy (d) (x) () ()\nqfree d\nperm_action @copy (t:wire mut) (s:wire in) () () ()\n^entry():\ncx s t";
    for adjoint in [false, true] {
        let point = if adjoint { "entry:0" } else { "entry:5" };
        let e = recompute(functions, point, adjoint);
        assert!(e.cleanup_pairing_complete);
        assert_eq!(e.cleanups.len(), 1);
        let replacement = &e.leaves[3..];
        let calls = replacement
            .iter()
            .filter(|l| !l.invocation.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(
            calls
                .iter()
                .map(|l| l.invocation[0].call_index)
                .collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        for (index, l) in calls.iter().enumerate() {
            let a = l.address.as_ref().unwrap();
            assert_eq!(a.path[0].point, point);
            assert_eq!(a.path[0].call_index, Some(index));
            assert_eq!(a.path[1].point, "entry:0");
            assert_eq!(a.path[1].call_index, None);
            assert_eq!(a.generated, 0);
            assert_eq!(l.invocation[0].adjoint, index >= 2);
        }
        let caller = replacement
            .iter()
            .filter(|l| l.invocation.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(
            caller
                .iter()
                .map(|l| l.address.as_ref().unwrap().generated)
                .collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
    }
}

#[test]
fn nonempty_call_recompute_restores_exact_nonzero_pre_call_inputs() {
    let functions = "action @root (x:wire mut, z:wire mut) (p:wire in) () () ()\n^entry():\nd = qcopy x\nctl p action_call @flip (d) (z) () ()\nctl p adj action_call @flip (d) (z) () ()\nx z\nx z\nquncopy x d\nperm_action @flip (t:wire mut) (s:wire in) () () ()\n^entry():\ncx s t\nx t";
    for adjoint in [false, true] {
        let point = if adjoint { "entry:1" } else { "entry:2" };
        let e = recompute(functions, point, adjoint);
        assert!(e.cleanup_pairing_complete);
        assert_eq!(e.cleanups.len(), 2);
        let selected = e
            .cleanups
            .iter()
            .find(|c| c.address.path[0].point == point)
            .unwrap();
        assert_eq!(selected.controls, ["p[0]"]);
        for input in 0..8 {
            let mut state = std::collections::BTreeMap::from([
                ("x[0]".to_string(), input & 1 != 0),
                ("z[0]".to_string(), input & 2 != 0),
                ("p[0]".to_string(), input & 4 != 0),
            ]);
            let original = state.clone();
            for l in &e.leaves {
                match l.operation.as_str() {
                    "alloc" => {
                        for w in &l.wires {
                            assert!(state.insert(w.clone(), false).is_none());
                        }
                    }
                    "release" => {
                        for w in &l.wires {
                            assert_eq!(state.remove(w), Some(false));
                        }
                    }
                    "x" | "cx" => {
                        let active = l.controls.iter().all(|c| state[c])
                            && (l.operation == "x" || state[&l.wires[0]]);
                        if active {
                            *state.get_mut(l.wires.last().unwrap()).unwrap() ^= true;
                        }
                    }
                    op => panic!("unexpected {op}"),
                }
            }
            assert_eq!(state, original, "input={input}, adjoint={adjoint}");
        }
    }
}

#[test]
fn intrinsic_copy_and_load_pairs_retain_separate_primitive_inverse_rows() {
    for load in [false, true] {
        let (compute, cleanup) = if load {
            ("d = qload db x", "qunload db x d")
        } else {
            ("d = qcopy x", "quncopy x d")
        };
        let functions = format!(
            "action @root (x:wire mut, z:wire mut) () (db:region[1,1] in) () ()\n^entry():\n{compute}\ncx z d\ncx z d\n{cleanup}"
        );
        for adjoint in [false, true] {
            let point = if adjoint { "entry:0" } else { "entry:3" };
            let e = recompute(&functions, point, adjoint);
            assert!(e.cleanup_pairing_complete);
            assert_eq!(e.cleanups.len(), 1);
            // K81 keeps the primitive inverse outside the intrinsic cleanup.
            let old_inverse = if adjoint { "entry:1" } else { "entry:2" };
            assert!(
                e.leaves
                    .iter()
                    .any(|l| l.address.as_ref().unwrap().path[0].point == old_inverse)
            );
            for bits in 0..16 {
                let mut state = std::collections::BTreeMap::from([
                    ("x[0]".to_string(), bits & 1 != 0),
                    ("z[0]".to_string(), bits & 2 != 0),
                    ("db[0]".to_string(), bits & 4 != 0),
                    ("db[1]".to_string(), bits & 8 != 0),
                ]);
                let original = state.clone();
                for l in &e.leaves {
                    match l.operation.as_str() {
                        "alloc" => {
                            for w in &l.wires {
                                assert!(state.insert(w.clone(), false).is_none());
                            }
                        }
                        "release" => {
                            for w in &l.wires {
                                assert_eq!(state.remove(w), Some(false));
                            }
                        }
                        "x" | "cx" | "ccx" | "mcx" => {
                            let (target, controls) = l.wires.split_last().unwrap();
                            if controls.iter().chain(&l.controls).all(|w| state[w]) {
                                *state.get_mut(target).unwrap() ^= true;
                            }
                        }
                        op => panic!("unexpected {op}"),
                    }
                }
                assert_eq!(
                    state, original,
                    "load={load}, adjoint={adjoint}, bits={bits}"
                );
            }
        }
    }
}

#[test]
fn nested_loop_computation_replay_uses_carrier_coordinates_only() {
    let functions = "action @root (x:wire mut) () () () ()\nregs {} bounds {2,2}\n^entry():\nd = qalloc fast 1\nqloop 2 {\ncx x d\n}\nqloop 2 {\ncx x d\n}\nqfree d";
    for adjoint in [false, true] {
        let point = if adjoint { "entry:0" } else { "entry:5" };
        let e = recompute(functions, point, adjoint);
        assert!(e.cleanup_pairing_complete);
        assert_eq!(e.cleanups.len(), 1);
        assert_eq!(e.leaves.len(), 11);
        for (ordinal, leaf) in e.leaves[3..].iter().enumerate() {
            let a = leaf.address.as_ref().unwrap();
            assert_eq!(a.path[0].point, point);
            assert_eq!(a.path[0].iteration, []);
            assert_eq!(a.generated, ordinal);
        }
        assert!(
            e.leaves
                .iter()
                .filter(|l| l.operation == "alloc")
                .all(|l| l.class.as_deref() == Some("fast"))
        );
        for x in [false, true] {
            replay_identity(&e, &[("x[0]", x)]);
        }
    }
}

#[test]
fn fresh_nested_conditional_inverse_has_contiguous_caller_ordinals() {
    let functions = "action @root (x:wire mut, z:wire mut) (p:wire in) () () ()\n^entry():\nd = qalloc any 1\nqif p {\ncx x d\n} {\ncx z d\n}\nqif p {\ncx x d\n} {\ncx z d\n}\nqfree d";
    for adjoint in [false, true] {
        let point = if adjoint { "entry:0" } else { "entry:7" };
        let e = recompute(functions, point, adjoint);
        assert!(e.cleanup_pairing_complete);
        assert_eq!(e.cleanups.len(), 1);
        let leaves = e
            .leaves
            .iter()
            .filter(|l| l.address.as_ref().unwrap().path[0].point == point)
            .collect::<Vec<_>>();
        assert!(!leaves.is_empty());
        assert_eq!(
            leaves
                .iter()
                .map(|l| l.address.as_ref().unwrap().generated)
                .collect::<Vec<_>>(),
            (0..leaves.len()).collect::<Vec<_>>()
        );
        for bits in 0..8 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("z[0]", bits & 2 != 0),
                    ("p[0]", bits & 4 != 0),
                ],
            );
        }
    }
}

#[test]
fn multi_target_call_recompute_restores_all_pre_call_values() {
    let functions = "action @root (x:wire mut, z:wire mut) () () () ()\n^entry():\nd = qcopy x\ne = qcopy z\naction_call @pair (d, e) () () ()\nadj action_call @pair (d, e) () () ()\nx z\nx z\nquncopy z e\nquncopy x d\nperm_action @pair (a:wire mut, b:wire mut) () () () ()\n^entry():\nswap a b\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:2" } else { "entry:3" },
            adjoint,
        );
        assert!(e.cleanup_pairing_complete);
        assert_eq!(e.cleanups.len(), 3);
        for bits in 0..4 {
            replay_identity(&e, &[("x[0]", bits & 1 != 0), ("z[0]", bits & 2 != 0)]);
        }
    }
}

#[test]
fn pre_call_input_reconstruction_keeps_original_predicate_scopes() {
    let functions = "action @root (x:wire mut, z:wire mut) (p:wire in) () () ()\n^entry():\nd = qcopy x\ne = qcopy z\nqpred p\nx d\nqunpred p\naction_call @mix (d, e) () () ()\nadj action_call @mix (d, e) () () ()\nqpred p\nx d\nqunpred p\nx z\nx z\nquncopy z e\nquncopy x d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:5" } else { "entry:6" },
            adjoint,
        );
        assert!(e.cleanup_pairing_complete);
        for bits in 0..8 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("z[0]", bits & 2 != 0),
                    ("p[0]", bits & 4 != 0),
                ],
            );
        }
    }
}

#[test]
fn paired_calls_reconstruct_owned_inputs_across_enclosing_predicate_boundaries() {
    let functions = "action @root (x:wire mut, z:wire mut) (p:wire in) () () ()\n^entry():\nd = qcopy x\nqpred p\naction_call @mix (d) (z) () ()\nadj action_call @mix (d) (z) () ()\nqunpred p\nquncopy x d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:2" } else { "entry:3" },
            adjoint,
        );
        assert!(e.cleanup_pairing_complete);
        assert!(
            e.cleanups
                .iter()
                .any(|c| c.realisation == CleanupRealisation::Recompute)
        );
        for bits in 0..8 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("z[0]", bits & 2 != 0),
                    ("p[0]", bits & 4 != 0),
                ],
            );
        }
    }
}

#[test]
fn nested_predicate_call_history_restores_source_versions_in_both_directions() {
    let functions = "action @root (x:wire mut, z:wire mut) (p:wire in, q:wire in) () () ()\n^entry():\nd = qcopy x\nqpred p\nx d\nqpred q\naction_call @mix (d) (z) () ()\nadj action_call @mix (d) (z) () ()\nqunpred q\nx d\nqunpred p\nquncopy x d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    for adjoint in [false, true] {
        let point = if adjoint { "entry:4" } else { "entry:5" };
        let e = recompute(functions, point, adjoint);
        assert!(e.cleanup_pairing_complete);
        let carrier = e
            .cleanups
            .iter()
            .find(|c| c.realisation == CleanupRealisation::Recompute)
            .unwrap();
        assert_eq!(carrier.address.path.last().unwrap().point, point);
        assert_eq!(carrier.address.generated, 0);
        assert_eq!(carrier.controls.len(), 2);
        for bits in 0..16 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("z[0]", bits & 2 != 0),
                    ("p[0]", bits & 4 != 0),
                    ("q[0]", bits & 8 != 0),
                ],
            );
        }
    }
}

#[test]
fn call_recompute_rebuilds_transitive_owned_histories_and_readonly_actuals() {
    let functions = "action @root (x:wire mut, z:wire mut) () () () ()\n^entry():\ne = qcopy x\nd = qcopy e\ng = qalloc any 1\nx e\naction_call @mix (d, g) (e, z) () ()\nadj action_call @mix (d, g) (e, z) () ()\nx e\nqfree g\nquncopy e d\nquncopy x e\nperm_action @mix (a:wire mut, b:wire mut) (c:wire in, v:wire in) () () ()\n^entry():\ncx a b\ncx c a\ncx v a";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:4" } else { "entry:5" },
            adjoint,
        );
        assert!(
            e.cleanups
                .iter()
                .any(|c| c.realisation == CleanupRealisation::Recompute)
        );
        for bits in 0..4 {
            replay_identity(&e, &[("x[0]", bits & 1 != 0), ("z[0]", bits & 2 != 0)]);
        }
    }
}

#[test]
fn call_recompute_rebuilds_local_mutable_region_image_and_stores() {
    let functions = "action @root () (x:wire[8] in, a:wire in) () () ()\n^entry():\nqregion db 1 8 @seed\nqstore db a x\naction_call @mix () (x, a) (db) ()\nadj action_call @mix () (x, a) (db) ()\nqstore db a x\nperm_action @mix () (v:wire[8] in, k:wire in) (m:region[1,8] mut) () ()\n^entry():\nqstore m k v";
    // Nonzero, asymmetric cells exercise initial image preparation/restoration.
    let mut source = support::source(functions);
    let mut lines = source.lines().map(str::to_string).collect::<Vec<_>>();
    let n = lines.iter().position(|l| l == "section symbols").unwrap() + 1;
    let Json::Obj(mut symbols) = json::parse(&lines[n]).unwrap() else {
        panic!()
    };
    symbols.insert("2".into(), json::parse(r#"{"name":"seed","kind":"data","signature":{"abi":"0.9","kind":"data","size":2,"tier":"-"},"interface_hash":"0000000000000000000000000000000000000000000000000000000000000000"}"#).unwrap());
    lines[n] = Json::Obj(symbols).to_jcs();
    source = support::sign(&format!(
        "{}\n\nsection data\nseed 8102\n",
        lines.join("\n")
    ));
    let (object, ids) = bennett_vm::load(&source, false).unwrap();
    for adjoint in [false, true] {
        let point = if adjoint { "entry:2" } else { "entry:3" };
        let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"{point}","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#, ids.interface["root"])).unwrap();
        let e = Catalog::published()
            .unwrap()
            .lower_action_with_cleanup(
                &object,
                "root",
                &Json::Arr(vec![]),
                adjoint,
                0,
                &[CleanupEntry {
                    selector: Selector::parse(&selector).unwrap(),
                    realisation: CleanupRealisation::Recompute,
                }],
            )
            .unwrap();
        assert!(e.cleanup_pairing_complete);
        assert!(
            e.cleanups
                .iter()
                .any(|c| c.realisation == CleanupRealisation::Recompute)
        );
        for bits in 0..512 {
            let mut values = (0..8)
                .map(|n| (format!("x[{n}]"), bits & (1 << n) != 0))
                .collect::<Vec<_>>();
            values.push(("a[0]".into(), bits & 256 != 0));
            replay_identity(
                &e,
                &values
                    .iter()
                    .map(|(n, v)| (n.as_str(), *v))
                    .collect::<Vec<_>>(),
            );
        }
    }
}

#[test]
fn call_recompute_preserves_sliced_target_aliases() {
    let functions = "action @root (z:wire mut) (x:wire[2] in) () () ()\n^entry():\nd = qcopy x\nx z\nx z\naction_call @mix (d[0], d[1]) () () ()\nadj action_call @mix (d[0], d[1]) () () ()\nquncopy x d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:3" } else { "entry:4" },
            adjoint,
        );
        assert!(e.cleanup_pairing_complete);
        assert!(
            e.cleanups
                .iter()
                .any(|c| c.realisation == CleanupRealisation::Recompute)
        );
        for bits in 0..4 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("x[1]", bits & 2 != 0),
                    ("z[0]", false),
                ],
            );
        }
    }
}

#[test]
fn older_external_history_versions_are_reconstructed_on_fresh_wires() {
    let functions = "action @root (x:wire mut) () () () ()\n^entry():\nd = qcopy x\nx x\ng = qalloc any 1\naction_call @mix (d, g) () () ()\nadj action_call @mix (d, g) () () ()\nqfree g\nx x\nquncopy x d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:2" } else { "entry:5" },
            adjoint,
        );
        for x in [false, true] {
            replay_identity(&e, &[("x[0]", x)]);
        }
    }
}

#[test]
fn external_history_cancellation_preserves_effective_predicate_scopes() {
    let functions = "action @root (x:wire mut) (p:wire in, q:wire in) () () ()\n^entry():\nd = qcopy x\ng = qalloc any 1\nqpred p\nx x\nqunpred p\nqpred q\nx x\nqunpred q\naction_call @mix (d, g) () () ()\nadj action_call @mix (d, g) () () ()\nqpred q\nx x\nqunpred q\nqpred p\nx x\nqunpred p\nqfree g\nquncopy x d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:8" } else { "entry:9" },
            adjoint,
        );
        for bits in 0..8 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("p[0]", bits & 2 != 0),
                    ("q[0]", bits & 4 != 0),
                ],
            );
        }
    }
}

#[test]
fn call_cleanup_checks_external_history_versions_through_the_cleanup_point() {
    let functions = "action @root (x:wire mut) () () () ()\n^entry():\nd = qcopy x\ng = qalloc any 1\naction_call @mix (d, g) () () ()\nx x\nadj action_call @mix (d, g) () () ()\nqfree g\nx x\nquncopy x d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:1" } else { "entry:5" },
            adjoint,
        );
        for x in [false, true] {
            replay_identity(&e, &[("x[0]", x)]);
        }
    }
}

#[test]
fn call_input_history_keeps_live_borrowed_dependencies_without_fresh_borrowing() {
    let functions = "action @root () () () () ()\n^entry():\nb = qborrow any 1\nd = qcopy b\ng = qalloc any 1\naction_call @mix (d, g) (b) () ()\nadj action_call @mix (d, g) (b) () ()\nqfree g\nquncopy b d\nqreturn b\nperm_action @mix (a:wire mut, c:wire mut) (v:wire in) () () ()\n^entry():\ncx a c\ncx v a";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:2" } else { "entry:5" },
            adjoint,
        );
        assert!(
            e.cleanups
                .iter()
                .any(|c| c.realisation == CleanupRealisation::Recompute)
        );
        assert_eq!(
            e.leaves
                .iter()
                .filter(|l| l.operation == "alloc" && l.state.as_deref() == Some("borrowed"))
                .count(),
            1
        );
        for borrowed in [false, true] {
            replay_identity_with_borrow(&e, &[], borrowed);
        }
    }
}

#[test]
fn call_input_history_requires_original_effective_predicate_versions() {
    let functions = "action @root (p:wire mut) () () () ()\n^entry():\nd = qalloc any 1\nqpred p\nx d\nqunpred p\nx p\ng = qalloc any 1\naction_call @mix (d, g) () () ()\nadj action_call @mix (d, g) () () ()\nqfree g\nx p\nqpred p\nx d\nqunpred p\nqfree d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:5" } else { "entry:8" },
            adjoint,
        );
        for p in [false, true] {
            replay_identity(&e, &[("p[0]", p)]);
        }
    }
}

#[test]
fn nested_call_history_tracks_outer_acquisitions_and_each_source_iteration() {
    let functions = "action @root (x:wire mut, z:wire mut) () () () ()\nregs {} bounds {3,2}\n^entry():\nd = qcopy x\nqloop 3 {\nx d\nqloop 2 {\naction_call @mix (d) (z) () ()\nadj action_call @mix (d) (z) () ()\n}\n}\nx d\nquncopy x d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    for adjoint in [false, true] {
        let point = if adjoint { "entry:4" } else { "entry:5" };
        let e = recompute(functions, point, adjoint);
        assert!(e.cleanup_pairing_complete);
        let iterations = e
            .cleanups
            .iter()
            .filter(|c| c.realisation == CleanupRealisation::Recompute)
            .map(|c| c.address.path.last().unwrap().iteration.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            iterations,
            (0..3)
                .flat_map(|i| (0..2).map(move |j| vec![i, j]))
                .collect()
        );
        for iteration in &iterations {
            let ordinals = e
                .leaves
                .iter()
                .filter_map(|l| l.address.as_ref())
                .filter(|a| {
                    a.path.len() == 1
                        && a.path[0].point == point
                        && a.path[0].iteration == *iteration
                })
                .map(|a| a.generated)
                .collect::<Vec<_>>();
            assert!(!ordinals.is_empty());
            assert_eq!(ordinals, (0..ordinals.len()).collect::<Vec<_>>());
        }
        for bits in 0..4 {
            replay_identity(&e, &[("x[0]", bits & 1 != 0), ("z[0]", bits & 2 != 0)]);
        }
    }
}

#[test]
fn call_histories_cross_both_branch_arms_without_losing_outer_versions() {
    let functions = "action @root (x:wire mut, z:wire mut) (p:wire in) () () ()\n^entry():\nd = qcopy x\nqif p {\nx d\naction_call @mix (d) (z) () ()\nadj action_call @mix (d) (z) () ()\nx d\n} {\ncx z d\naction_call @mix (d) (x) () ()\nadj action_call @mix (d) (x) () ()\ncx z d\n}\nquncopy x d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    for adjoint in [false, true] {
        // qif source-adjoints its two formed arms through the published inverse recipe.
        for point in ["entry:4", "entry:8"] {
            let e = recompute(functions, point, adjoint);
            assert!(e.cleanup_pairing_complete);
            assert!(
                e.cleanups
                    .iter()
                    .any(|c| c.realisation == CleanupRealisation::Recompute
                        && !c.controls.is_empty())
            );
            for bits in 0..8 {
                replay_identity(
                    &e,
                    &[
                        ("x[0]", bits & 1 != 0),
                        ("z[0]", bits & 2 != 0),
                        ("p[0]", bits & 4 != 0),
                    ],
                );
            }
        }
    }
}

#[test]
fn nested_local_call_acquisition_reconstructs_its_outer_owned_dependency() {
    let functions = "action @root (x:wire mut, z:wire mut) () () () ()\nregs {} bounds {3}\n^entry():\nd = qcopy x\nqloop 3 {\nx d\ne = qcopy d\naction_call @mix (e) (z) () ()\nadj action_call @mix (e) (z) () ()\nquncopy d e\n}\nx d\nquncopy x d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:4" } else { "entry:5" },
            adjoint,
        );
        assert!(e.cleanup_pairing_complete);
        for bits in 0..4 {
            replay_identity(&e, &[("x[0]", bits & 1 != 0), ("z[0]", bits & 2 != 0)]);
        }
    }
}

#[test]
fn earlier_conditional_iterations_are_replayed_without_new_cleanup_carriers() {
    let functions = "action @root (x:wire mut, z:wire mut) (p:wire in) () () ()\nregs {} bounds {3,3}\n^entry():\nd = qcopy x\nqloop 3 {\nqif p {\nx d\naction_call @mix (d) (z) () ()\nadj action_call @mix (d) (z) () ()\n} {\ncx z d\n}\n}\nqloop 3 {\nqif p {\nx d\n} {\ncx z d\n}\n}\nquncopy x d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    for adjoint in [false, true] {
        let e = recompute(functions, "entry:5", adjoint);
        assert_eq!(
            e.cleanups
                .iter()
                .filter(|c| c.realisation == CleanupRealisation::Recompute)
                .count(),
            3
        );
        for bits in 0..8 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("z[0]", bits & 2 != 0),
                    ("p[0]", bits & 4 != 0),
                ],
            );
        }
    }
}

#[test]
fn else_history_does_not_substitute_the_temporarily_inverted_predicate() {
    let functions = "action @root (x:wire mut) (p:wire in) () () ()\nregs {  } bounds {0}\n^entry():\nd = qcopy p\nqif p {\nqloop 0 {\nx x\n}\n} {\naction_call @mix (d) (x) () ()\nadj action_call @mix (d) (x) () ()\n}\nquncopy p d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    let (object, _) = bennett_vm::load(&support::source(functions), false).unwrap();
    for adjoint in [false, true] {
        let selected = recompute(functions, "entry:5", adjoint);
        let normal = Catalog::published()
            .unwrap()
            .lower_action_with_cleanup(&object, "root", &Json::Arr(vec![]), adjoint, 0, &[])
            .unwrap();
        for bits in 0..4 {
            replay_identity(&normal, &[("x[0]", bits & 1 != 0), ("p[0]", bits & 2 != 0)]);
            replay_identity(
                &selected,
                &[("x[0]", bits & 1 != 0), ("p[0]", bits & 2 != 0)],
            );
        }
    }
}

#[test]
fn replayed_independent_branch_histories_do_not_reapply_cleanup_policy() {
    let functions = "action @root (x:wire mut, z:wire mut) (p:wire in, q:wire in) () () ()\nregs {  } bounds {0}\n^entry():\nd = qcopy x\nqif q {\ncx z d\naction_call @mix (d) (x) () ()\nadj action_call @mix (d) (x) () ()\ncx z d\n} {\nx d\nx d\n}\nqif p {\naction_call @mix (d) (z) () ()\nadj action_call @mix (d) (z) () ()\n} {\nqloop 0 {\nx x\n}\n}\nquncopy x d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    for adjoint in [false, true] {
        let e = recompute(functions, "entry:10", adjoint);
        assert_eq!(
            e.cleanups
                .iter()
                .filter(|c| c.realisation == CleanupRealisation::Recompute)
                .count(),
            1
        );
        for bits in 0..16 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("z[0]", bits & 2 != 0),
                    ("p[0]", bits & 4 != 0),
                    ("q[0]", bits & 8 != 0),
                ],
            );
        }
    }
}

#[test]
fn bounded_while_histories_reconstruct_each_executed_prefix() {
    let functions = "action @root (x:wire mut, z:wire mut) (p:wire[3] in) () () ()\nregs {} bounds {3,3}\n^entry():\nd = qcopy x\nqloop 3 while p {\nx d\naction_call @mix (d) (z) () ()\nadj action_call @mix (d) (z) () ()\n}\nqloop 3 while p {\nx d\n}\nquncopy x d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    for adjoint in [false, true] {
        let e = recompute(functions, "entry:4", adjoint);
        assert_eq!(
            e.cleanups
                .iter()
                .filter(|c| c.realisation == CleanupRealisation::Recompute)
                .count(),
            3
        );
        for bits in 0..32 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("z[0]", bits & 2 != 0),
                    ("p[0]", bits & 4 != 0),
                    ("p[1]", bits & 8 != 0),
                    ("p[2]", bits & 16 != 0),
                ],
            );
        }
    }
}

#[test]
fn completed_conditional_inputs_use_their_earlier_predicate_version() {
    let functions = "action @root (p:wire mut) () () () ()\nregs {} bounds {0,0}\n^entry():\nd = qalloc any 1\nqif p {\nx d\n} {\nqloop 0 {\nx d\n}\n}\nx p\ng = qalloc any 1\naction_call @mix (d, g) () () ()\nadj action_call @mix (d, g) () () ()\nqfree g\nx p\nqif p {\nx d\n} {\nqloop 0 {\nx d\n}\n}\nqfree d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:6" } else { "entry:9" },
            adjoint,
        );
        for p in [false, true] {
            replay_identity(&e, &[("p[0]", p)]);
        }
    }
}

#[test]
fn mutable_enclosing_predicates_reconstruct_prior_iteration_arms() {
    let functions = "action @root (x:wire mut, z:wire mut, p:wire mut) () () () ()\nregs {} bounds {3,3}\n^entry():\nd = qcopy x\nqloop 3 {\nqif p {\nx d\naction_call @mix (d) (z) () ()\nadj action_call @mix (d) (z) () ()\n} {\ncx z d\n}\nx p\n}\nqloop 3 {\nx p\nqif p {\nx d\n} {\ncx z d\n}\n}\nquncopy x d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    for adjoint in [false, true] {
        let e = recompute(functions, "entry:5", adjoint);
        for bits in 0..8 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("z[0]", bits & 2 != 0),
                    ("p[0]", bits & 4 != 0),
                ],
            );
        }
    }
}

#[test]
fn nonpermutation_external_history_is_never_replayed_as_a_basis_snapshot() {
    let functions = "action @root (x:wire mut) () () () ()\n^entry():\nd = qcopy x\nh x\ng = qalloc any 1\naction_call @mix (d, g) () () ()\nadj action_call @mix (d, g) () () ()\nqfree g\nh x\nquncopy x d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    let text = support::source(functions);
    assert!(bennett_vm::load(&text, false).unwrap_err().contains("§6.2"));
    let object = bennett_vm::object::parse(&text).unwrap();
    let ids = bennett_vm::identity::compute(&object).unwrap();
    let catalog = Catalog::published().unwrap();
    for adjoint in [false, true] {
        let point = if adjoint { "entry:2" } else { "entry:5" };
        let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"{point}","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#, ids.interface["root"])).unwrap();
        let result = catalog.lower_action_with_cleanup(
            &object,
            "root",
            &Json::Arr(vec![]),
            adjoint,
            0,
            &[CleanupEntry {
                selector: Selector::parse(&selector).unwrap(),
                realisation: CleanupRealisation::Recompute,
            }],
        );
        assert!(matches!(result, Err(ref error) if error.contains("§6.2")));
        assert!(
            catalog
                .lower_action_with_cleanup(&object, "root", &Json::Arr(vec![]), adjoint, 0, &[])
                .unwrap_err()
                .contains("§6.2")
        );
    }
}

#[test]
fn historical_external_permutation_calls_keep_symbols_and_operand_versions_distinct() {
    let functions = "action @root (a:wire mut, b:wire mut) () () () ()\n^entry():\nd = qcopy b\ng = qalloc any 1\naction_call @a (b) (a) () ()\nx a\naction_call @mix (d, g) () () ()\nadj action_call @mix (d, g) () () ()\nx a\nadj action_call @a (b) (a) () ()\nqfree g\nquncopy b d\nperm_action @a (t:wire mut) (s:wire in) () () ()\n^entry():\ncx s t\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:4" } else { "entry:5" },
            adjoint,
        );
        for bits in 0..4 {
            replay_identity(&e, &[("a[0]", bits & 1 != 0), ("b[0]", bits & 2 != 0)]);
        }
    }
}

#[test]
fn completed_bounded_while_history_reconstructs_released_condition_bundle() {
    let functions = "action @root (x:wire mut, z:wire mut) (p:wire[3] in) () () ()\nregs {} bounds {3,3}\n^entry():\nd = qcopy x\nc = qcopy p\nqloop 3 while c {\nx d\n}\nquncopy p c\naction_call @mix (d) (z) () ()\nadj action_call @mix (d) (z) () ()\ne = qcopy p\nqloop 3 while e {\nx d\n}\nquncopy p e\nquncopy x d\nperm_action @mix (a:wire mut) (b:wire in) () () ()\n^entry():\ncx b a";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:5" } else { "entry:6" },
            adjoint,
        );
        for bits in 0..32 {
            replay_identity(
                &e,
                &[
                    ("x[0]", bits & 1 != 0),
                    ("z[0]", bits & 2 != 0),
                    ("p[0]", bits & 4 != 0),
                    ("p[1]", bits & 8 != 0),
                    ("p[2]", bits & 16 != 0),
                ],
            );
        }
    }
}

#[test]
fn earlier_memory_versions_are_reconstructed_without_changing_the_live_region() {
    // This checks symbolic history formation and scratch restoration only.
    // A restored memory value does not restore its cell version or admit this
    // whole source region: the separate §6.2 admission check must reject the
    // later qunload after a cell-version advance, even for equal final bytes.
    let functions = "action @root (a:wire mut, z:wire mut) () (r:region[1,1] mut) () ()\n^entry():\nd = qload r a\ng = qalloc any 1\nqstore r a z\naction_call @mix (d, g) () () ()\nadj action_call @mix (d, g) () () ()\nqstore r a z\nqfree g\nqunload r a d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:3" } else { "entry:4" },
            adjoint,
        );
        for bits in 0..16 {
            replay_identity(
                &e,
                &[
                    ("a[0]", bits & 1 != 0),
                    ("z[0]", bits & 2 != 0),
                    ("r[0]", bits & 4 != 0),
                    ("r[1]", bits & 8 != 0),
                ],
            );
        }
    }
}

#[test]
fn historical_external_changes_retain_proven_live_owned_control_versions() {
    let functions = "action @root (x:wire mut) (q:wire in) () () ()\n^entry():\nd = qcopy x\np = qcopy q\ng = qalloc any 1\nqpred p\nx x\nqunpred p\naction_call @mix (d, g) () () ()\nadj action_call @mix (d, g) () () ()\nqpred p\nx x\nqunpred p\nqfree g\nquncopy q p\nquncopy x d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:6" } else { "entry:7" },
            adjoint,
        );
        for bits in 0..4 {
            replay_identity(&e, &[("x[0]", bits & 1 != 0), ("q[0]", bits & 2 != 0)]);
        }
    }
}

#[test]
fn historical_external_changes_reconstruct_mutated_live_owned_controls() {
    let functions = "action @root (x:wire mut) (q:wire in) () () ()\n^entry():\nd = qcopy x\np = qcopy q\ng = qalloc any 1\nqpred p\nx x\nqunpred p\nx p\naction_call @mix (d, g) () () ()\nadj action_call @mix (d, g) () () ()\nx p\nqpred p\nx x\nqunpred p\nqfree g\nquncopy q p\nquncopy x d\nperm_action @mix (a:wire mut, b:wire mut) () () () ()\n^entry():\ncx a b";
    for adjoint in [false, true] {
        let e = recompute(
            functions,
            if adjoint { "entry:7" } else { "entry:8" },
            adjoint,
        );
        for bits in 0..4 {
            replay_identity(&e, &[("x[0]", bits & 1 != 0), ("q[0]", bits & 2 != 0)]);
        }
    }
}
