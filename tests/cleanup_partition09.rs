//! K81 partitions source inverse occurrences before cleanup selection.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::{
    json::{self, Json},
    ref_library::{ActionExpansion, Catalog, CleanupEntry, CleanupRealisation},
    sched::Selector,
};
use std::collections::{BTreeMap, BTreeSet};

fn expand(
    body: &str,
    adjoint: bool,
    choices: &[(&str, CleanupRealisation)],
) -> Result<ActionExpansion, String> {
    let bounds = body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("qloop "))
        .map(|row| row.split_whitespace().next().unwrap())
        .collect::<Vec<_>>()
        .join(",");
    let functions = format!(
        "action @root (x:wire[2] mut, z:wire mut) () () () ()\nregs {{}} bounds {{{bounds}}}\n^entry():\n{body}\nperm_action @mix (a:wire[2] mut) () () () ()\n^entry():\ncx a[0] a[1]\nperm_action @pair (a:wire mut, b:wire mut) () () () ()\n^entry():\nx a\ncx a b"
    );
    let (object, ids) = bennett_vm::load(&support::source(&functions), false)?;
    let entries = choices.iter().map(|(point, realisation)| {
        let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"{point}","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#, ids.interface["root"])).unwrap();
        CleanupEntry { selector: Selector::parse(&selector).unwrap(), realisation: *realisation }
    }).collect::<Vec<_>>();
    Catalog::published()?.lower_action_with_cleanup(
        &object,
        "root",
        &Json::Arr(vec![]),
        adjoint,
        0,
        &entries,
    )
}
fn assert_identity(e: &ActionExpansion) {
    for bits in 0..8 {
        let mut state = BTreeMap::from([
            ("x[0]".to_string(), bits & 1 != 0),
            ("x[1]".to_string(), bits & 2 != 0),
            ("z[0]".to_string(), bits & 4 != 0),
        ]);
        let original = state.clone();
        for leaf in &e.leaves {
            match leaf.operation.as_str() {
                "alloc" => {
                    for w in &leaf.wires {
                        assert!(state.insert(w.clone(), false).is_none());
                    }
                }
                "release" => {
                    for w in &leaf.wires {
                        assert_eq!(state.remove(w), Some(false));
                    }
                }
                "x" | "cx" | "ccx" | "mcx" => {
                    let (target, controls) = leaf.wires.split_last().unwrap();
                    if controls.iter().chain(&leaf.controls).all(|w| state[w]) {
                        *state.get_mut(target).unwrap() ^= true;
                    }
                }
                op => panic!("unexpected {op}"),
            }
        }
        assert_eq!(state, original);
    }
}

#[test]
fn intrinsic_boundary_keeps_call_and_copy_cleanup_independently_selectable() {
    let body = "d = qcopy x\naction_call @mix (d) () () ()\nadj action_call @mix (d) () () ()\nquncopy x d";
    for adjoint in [false, true] {
        let (call, copy) = if adjoint {
            ("entry:1", "entry:0")
        } else {
            ("entry:2", "entry:3")
        };
        for call_choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            for copy_choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
                let e = expand(body, adjoint, &[(call, call_choice), (copy, copy_choice)]).unwrap();
                assert!(e.cleanup_pairing_complete);
                assert_eq!(e.cleanups.len(), 2);
                assert_eq!(
                    e.cleanups
                        .iter()
                        .map(|c| c.address.path[0].point.as_str())
                        .collect::<BTreeSet<_>>(),
                    BTreeSet::from([call, copy])
                );
                assert_identity(&e);
            }
        }
        assert!(
            expand(
                body,
                adjoint,
                &[
                    (call, CleanupRealisation::Reverse),
                    (call, CleanupRealisation::Recompute)
                ]
            )
            .is_err()
        );
    }
}

#[test]
fn terminal_call_inverse_belongs_to_release_before_call_pair_selection() {
    let body = "d = qalloc any 2\naction_call @mix (d) () () ()\nadj action_call @mix (d) () () ()\nqfree d";
    for adjoint in [false, true] {
        let point = if adjoint { "entry:0" } else { "entry:3" };
        for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            let e = expand(body, adjoint, &[(point, choice)]).unwrap();
            assert!(e.cleanup_pairing_complete);
            assert_eq!(e.cleanups.len(), 1);
            assert_eq!(e.cleanups[0].address.path[0].point, point);
            assert_identity(&e);
        }
    }
}

#[test]
fn an_intervening_unmatched_computation_breaks_the_call_pair() {
    let body = "action_call @mix (x) () () ()\ncx x[0] x[1]\nadj action_call @mix (x) () () ()";
    for adjoint in [false, true] {
        let e = expand(body, adjoint, &[]).unwrap();
        assert!(
            e.cleanups.is_empty(),
            "textual inverse is not the current occurrence"
        );
        assert_eq!(e.leaves.len(), 3);
    }
}

#[test]
fn nested_matched_inverse_occurrences_restore_call_target_versions() {
    let body = "action_call @mix (x) () () ()\nx x[0]\nx x[0]\nadj action_call @mix (x) () () ()";
    for adjoint in [false, true] {
        let e = expand(body, adjoint, &[]).unwrap();
        assert_eq!(e.cleanups.len(), 1);
        assert_identity(&e);
    }
}

#[test]
fn a_changed_effective_control_version_breaks_the_call_pair() {
    let body = "ctl z action_call @mix (x) () () ()\nx z\nctl z adj action_call @mix (x) () () ()";
    for adjoint in [false, true] {
        let e = expand(body, adjoint, &[]).unwrap();
        assert!(e.cleanups.is_empty());
    }
    let restored =
        "ctl z action_call @mix (x) () () ()\nx z\nx z\nctl z adj action_call @mix (x) () () ()";
    for adjoint in [false, true] {
        let e = expand(restored, adjoint, &[]).unwrap();
        assert_eq!(e.cleanups.len(), 1);
        assert_identity(&e);
    }
}

#[test]
fn one_changed_target_breaks_a_joint_multi_output_call_occurrence() {
    let body = "action_call @mix (x) () () ()\nx x[0]\nadj action_call @mix (x) () () ()\nx x[0]";
    for adjoint in [false, true] {
        let e = expand(body, adjoint, &[]).unwrap();
        assert!(e.cleanups.is_empty());
    }
}

#[test]
fn restored_memory_values_do_not_restore_produced_cell_versions() {
    let functions = "action @root (x:wire mut) (a:wire in, z:wire in) (r:region[1,1] mut) () ()\n^entry():\naction_call @read (x) (a) (r) ()\nqstore r a z\nqstore r a z\nadj action_call @read (x) (a) (r) ()\nperm_action @read (t:wire mut) (a:wire in) (r:region[1,1] mut) () ()\n^entry():\nd = qload r a\ncx d t\nqunload r a d";
    let (object, _) = bennett_vm::load(&support::source(functions), false).unwrap();
    let catalog = Catalog::published().unwrap();
    for adjoint in [false, true] {
        let e = catalog
            .lower_action_with_cleanup(&object, "root", &Json::Arr(vec![]), adjoint, 0, &[])
            .unwrap();
        assert!(
            e.cleanups.iter().all(|c| c.address.path.len() != 1),
            "only the callee's intrinsic load cleanup has a carrier"
        );
    }
}

#[test]
fn release_coalescing_requires_the_original_readonly_input_versions() {
    let body = "d = qalloc any 1\ncx x[0] d\nx x[0]\ncx x[0] d\nqfree d";
    let text = support::source(&format!(
        "action @root (x:wire[2] mut, z:wire mut) () () () ()\n^entry():\n{body}"
    ));
    // This program leaves d=1 for every input. Admission now rejects it
    // before a partial source trace can be formed; the Rust API must enforce
    // the same boundary when handed a parsed, unvalidated object.
    assert!(bennett_vm::load(&text, false).unwrap_err().contains("§6.2"));
    let object = bennett_vm::object::parse(&text).unwrap();
    let catalog = Catalog::published().unwrap();
    for adjoint in [false, true] {
        let error = catalog
            .lower_action_with_cleanup(&object, "root", &Json::Arr(vec![]), adjoint, 0, &[])
            .unwrap_err();
        assert!(error.contains("§6.2"), "adjoint={adjoint}: {error}");
    }
}

#[test]
fn disjoint_rows_interleave_with_a_terminal_inverse_without_changing_ownership() {
    let body = "d = qalloc any 1\ncx x[0] d\nx z\ncx x[0] d\nx z\nqfree d";
    for adjoint in [false, true] {
        let point = if adjoint { "entry:0" } else { "entry:5" };
        for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            let e = expand(body, adjoint, &[(point, choice)]).unwrap();
            assert!(e.cleanup_pairing_complete);
            assert_eq!(e.cleanups.len(), 1);
            assert_eq!(e.cleanups[0].address.path[0].point, point);
            for outside in ["entry:2", "entry:4"] {
                assert_eq!(
                    e.leaves
                        .iter()
                        .filter(|l| l.address.as_ref().unwrap().path[0].point == outside)
                        .count(),
                    1
                );
            }
            if choice == CleanupRealisation::Reverse {
                let mut expected = (0..6).map(|i| format!("entry:{i}")).collect::<Vec<_>>();
                if adjoint {
                    expected.reverse();
                }
                assert_eq!(
                    e.leaves
                        .iter()
                        .map(|l| l.address.as_ref().unwrap().path[0].point.clone())
                        .collect::<Vec<_>>(),
                    expected
                );
            }
            assert_identity(&e);
        }
    }
}

#[test]
fn disjoint_structured_rows_retain_source_coordinates_across_cleanup_choices() {
    let conditional = "qif x[1] {\nx z\nx z\n} {\nx z\nx z\n}";
    let mut cases = vec![(
        conditional.to_string(),
        13,
        6,
        7,
        (1..=5).chain(8..=12).collect::<BTreeSet<_>>(),
    )];
    for count in [0, 1, 2, 4] {
        cases.push((
            format!("qloop {count} {{\nx z\nx z\n}}"),
            9,
            4,
            5,
            (1..=3).chain(6..=8).collect(),
        ));
    }
    for (outside, release, first, second, points) in cases {
        let body = format!("d = qalloc any 1\n{outside}\ncx x[0] d\ncx x[0] d\n{outside}\nqfree d");
        for adjoint in [false, true] {
            let carrier = format!("entry:{}", if adjoint { 0 } else { release });
            let reverse = expand(
                &body,
                adjoint,
                &[(carrier.as_str(), CleanupRealisation::Reverse)],
            )
            .unwrap();
            let fresh = expand(
                &body,
                adjoint,
                &[(carrier.as_str(), CleanupRealisation::Recompute)],
            )
            .unwrap();
            for e in [&reverse, &fresh] {
                assert!(e.cleanup_pairing_complete, "{outside}, adjoint={adjoint}");
                assert_eq!(e.cleanups.len(), 1, "{outside}, adjoint={adjoint}");
                assert_eq!(e.cleanups[0].address.path[0].point, carrier);
                assert_identity(e);
            }
            assert_eq!(fresh.cleanups[0].realisation, CleanupRealisation::Recompute);
            let outside_trace = |e: &ActionExpansion| {
                e.leaves
                    .iter()
                    .filter_map(|leaf| {
                        let address = leaf.address.as_ref().unwrap();
                        let point: usize = address.path[0]
                            .point
                            .strip_prefix("entry:")
                            .unwrap()
                            .parse()
                            .unwrap();
                        points.contains(&point).then(|| {
                            (
                                leaf.operation.clone(),
                                address.clone(),
                                leaf.wires.clone(),
                                leaf.controls.clone(),
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                outside_trace(&reverse),
                outside_trace(&fresh),
                "{outside}, adjoint={adjoint}"
            );
            let replaced = format!("entry:{}", if adjoint { first } else { second });
            assert_eq!(
                reverse
                    .leaves
                    .iter()
                    .filter(|l| l.address.as_ref().unwrap().path[0].point == replaced)
                    .count(),
                1
            );
            assert!(
                fresh
                    .leaves
                    .iter()
                    .all(|l| l.address.as_ref().unwrap().path[0].point != replaced)
            );
        }
    }
}

#[test]
fn disjoint_loop_cleanups_keep_their_own_carriers_and_choices() {
    for count in [0, 1, 2] {
        let body = format!(
            "d = qalloc any 1\nqloop {count} {{\na = qcopy z\nquncopy z a\n}}\ncx x[0] d\ncx x[0] d\nqloop {count} {{\nb = qcopy z\nquncopy z b\n}}\nqfree d"
        );
        for adjoint in [false, true] {
            let (outer, nested) = if adjoint {
                ("entry:0", ["entry:2", "entry:7"])
            } else {
                ("entry:9", ["entry:3", "entry:8"])
            };
            for outer_choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
                for nested_choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
                    let e = expand(
                        &body,
                        adjoint,
                        &[
                            (outer, outer_choice),
                            (nested[0], nested_choice),
                            (nested[1], nested_choice),
                        ],
                    )
                    .unwrap();
                    assert!(
                        e.cleanup_pairing_complete,
                        "count={count}, adjoint={adjoint}"
                    );
                    assert_eq!(e.cleanups.len(), 1 + 2 * count);
                    for carrier in &e.cleanups {
                        let point = &carrier.address.path[0];
                        if point.point == outer {
                            assert!(point.iteration.is_empty());
                            assert_eq!(carrier.realisation, outer_choice);
                        } else {
                            assert!(nested.contains(&point.point.as_str()));
                            assert_eq!(point.iteration.len(), 1);
                            assert!(point.iteration[0] < count);
                            assert_eq!(carrier.realisation, nested_choice);
                        }
                    }
                    for point in nested {
                        assert_eq!(
                            e.cleanups
                                .iter()
                                .filter(|c| c.address.path[0].point == point)
                                .count(),
                            count
                        );
                    }
                    assert_identity(&e);
                }
            }
        }
    }
}

#[test]
fn a_prior_cancelled_pair_is_outside_the_terminal_inverse_chain() {
    let body = "d = qalloc any 2\nx d[0]\nx d[0]\naction_call @mix (d) () () ()\nadj action_call @mix (d) () () ()\nqfree d";
    for adjoint in [false, true] {
        let point = if adjoint { "entry:0" } else { "entry:5" };
        for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            let e = expand(body, adjoint, &[(point, choice)]).unwrap();
            assert!(e.cleanup_pairing_complete);
            assert_identity(&e);
        }
    }
}

#[test]
fn joint_inverse_has_one_release_owner_and_later_release_is_independent() {
    let body = "a = qalloc any 1\nb = qalloc any 1\naction_call @pair (a, b) () () ()\nadj action_call @pair (a, b) () () ()\nqfree a\nqfree b";
    for adjoint in [false, true] {
        let (owner, later) = if adjoint {
            ("entry:1", "entry:0")
        } else {
            ("entry:4", "entry:5")
        };
        for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            let e = expand(
                body,
                adjoint,
                &[(owner, choice), (later, CleanupRealisation::Recompute)],
            )
            .unwrap();
            assert!(e.cleanup_pairing_complete);
            assert_eq!(e.cleanups.len(), 2);
            assert_eq!(
                e.cleanups
                    .iter()
                    .map(|c| c.address.path[0].point.as_str())
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from([owner, later])
            );
            assert_identity(&e);
        }
    }
}

#[test]
fn fixed_loop_visits_restore_call_versions_only_after_matched_inverse_visits() {
    for count in [0, 1, 2, 4] {
        let functions = format!(
            "action @root (x:wire[2] mut, z:wire mut) () () () ()\nregs {{}} bounds {{{count}}}\n^entry():\naction_call @mix (x) () () ()\nqloop {count} {{\nx x[0]\n}}\nadj action_call @mix (x) () () ()\nperm_action @mix (a:wire[2] mut) () () () ()\n^entry():\ncx a[0] a[1]"
        );
        let (object, _) = bennett_vm::load(&support::source(&functions), false).unwrap();
        let catalog = Catalog::published().unwrap();
        for adjoint in [false, true] {
            let e = catalog
                .lower_action_with_cleanup(&object, "root", &Json::Arr(vec![]), adjoint, 0, &[])
                .unwrap();
            assert_eq!(
                e.cleanups.len(),
                usize::from(count % 2 == 0),
                "count={count}, adjoint={adjoint}"
            );
            if count % 2 == 0 {
                assert_identity(&e);
            }
        }
    }
}

#[test]
fn joint_owner_preserves_the_other_live_targets_nonzero_entry_values() {
    let body = "a = qalloc any 1\nb = qalloc any 1\ncx x[0] b\naction_call @pair (a, b) () () ()\nadj action_call @pair (a, b) () () ()\nqfree a\ncx x[0] b\nqfree b";
    for adjoint in [false, true] {
        let (owner, later) = if adjoint {
            ("entry:1", "entry:0")
        } else {
            ("entry:5", "entry:7")
        };
        for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            let e = expand(
                body,
                adjoint,
                &[(owner, choice), (later, CleanupRealisation::Recompute)],
            )
            .unwrap_or_else(|error| panic!("adjoint={adjoint}, choice={choice:?}: {error}"));
            assert!(e.cleanup_pairing_complete);
            assert_eq!(e.cleanups.len(), 2);
            assert_eq!(e.cleanups[0].address.path[0].point, owner);
            assert_identity(&e);
        }
    }
}

#[test]
fn connected_joint_inverses_have_one_owner_in_each_traversal_direction() {
    let body = "a = qalloc any 1\nb = qalloc any 1\nc = qalloc any 1\naction_call @pair (a, b) () () ()\naction_call @pair (b, c) () () ()\nadj action_call @pair (b, c) () () ()\nadj action_call @pair (a, b) () () ()\nqfree a\nqfree b\nqfree c";
    for adjoint in [false, true] {
        let points = if adjoint {
            ["entry:2", "entry:1", "entry:0"]
        } else {
            ["entry:7", "entry:8", "entry:9"]
        };
        for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            let e = expand(
                body,
                adjoint,
                &[
                    (points[0], choice),
                    (points[1], CleanupRealisation::Recompute),
                    (points[2], CleanupRealisation::Recompute),
                ],
            )
            .unwrap_or_else(|error| panic!("adjoint={adjoint}, choice={choice:?}: {error}"));
            assert!(e.cleanup_pairing_complete);
            assert_eq!(e.cleanups.len(), 3);
            assert_eq!(
                e.cleanups
                    .iter()
                    .map(|c| c.address.path[0].point.as_str())
                    .collect::<Vec<_>>(),
                points
            );
            assert_identity(&e);
        }
    }
}
