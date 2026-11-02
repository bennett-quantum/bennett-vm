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
    let functions = format!(
        "action @root (x:wire[2] mut, z:wire mut) () () () ()\n^entry():\n{body}\nperm_action @mix (a:wire[2] mut) () () () ()\n^entry():\ncx a[0] a[1]"
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
    let restored = "ctl z action_call @mix (x) () () ()\nx z\nx z\nctl z adj action_call @mix (x) () () ()";
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
fn restored_memory_values_do_not_restore_readonly_cell_versions() {
    let functions = "action @root (x:wire mut) (a:wire in, z:wire in) (r:region[1,1] mut) () ()\n^entry():\naction_call @read (x) (a) (r) ()\nqstore r a z\nqstore r a z\nadj action_call @read (x) (a) (r) ()\nperm_action @read (t:wire mut) (a:wire in) (r:region[1,1] in) () ()\n^entry():\nd = qload r a\ncx d t\nqunload r a d";
    let (object, _) = bennett_vm::load(&support::source(functions), false).unwrap();
    let catalog = Catalog::published().unwrap();
    for adjoint in [false, true] {
        let e = catalog.lower_action_with_cleanup(&object, "root", &Json::Arr(vec![]), adjoint, 0, &[]).unwrap();
        assert!(e.cleanups.iter().all(|c| c.address.path.len() != 1), "only the callee's intrinsic load cleanup has a carrier");
    }
}

#[test]
fn release_coalescing_requires_the_original_readonly_input_versions() {
    let body = "d = qalloc any 1\ncx x[0] d\nx x[0]\ncx x[0] d\nqfree d";
    for adjoint in [false, true] {
        let e = expand(body, adjoint, &[]).unwrap();
        assert!(!e.cleanup_pairing_complete);
        assert!(e.cleanups.is_empty(), "a dirty lifetime is not a proved terminal inverse chain");
    }
}
