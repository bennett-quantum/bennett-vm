//! K65 whole-pair selection happens before any cleanup leaves are expanded.
use crate::source09 as support;
use bennett_vm::{
    json::{self, Json},
    ref_library::{Catalog, CleanupEntry, CleanupRealisation},
    sched::{OperationKind, Selector},
};
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
fn entry(generated: Json, choice: CleanupRealisation) -> CleanupEntry {
    CleanupEntry {
        selector: Selector::parse(&Json::obj(vec![
            ("path", j("[]")),
            ("step", j("\"*\"")),
            ("phase", j("\"*\"")),
            ("generated", generated),
        ]))
        .unwrap(),
        realisation: choice,
    }
}
fn lower(
    body: &str,
    entries: &[CleanupEntry],
    adjoint: bool,
) -> Result<bennett_vm::ref_library::ActionExpansion, String> {
    let bounds = if body.contains("qloop 2") { "2" } else { "" };
    let text = support::source(&format!(
        "action @root (x:wire[2] mut) () () () ()\nregs {{ }} bounds {{{bounds}}}\n^entry():\n{body}"
    ));
    let (obj, _) = bennett_vm::load(&text, false)?;
    Catalog::published()?.lower_action_with_cleanup(&obj, "root", &j("[]"), adjoint, 0, entries)
}
#[test]
fn k65_whole_copy_pair_uses_one_nonemitting_ordinal_zero_carrier() {
    let body = "d = qcopy x\nquncopy x d";
    let reverse = lower(body, &[], false).unwrap();
    let recompute = lower(body, &[entry(j("0"), CleanupRealisation::Recompute)], false).unwrap();
    assert_eq!(reverse.cleanups.len(), 1);
    assert_eq!(recompute.cleanups.len(), 1);
    let a = &reverse.cleanups[0];
    let b = &recompute.cleanups[0];
    assert_eq!(a.address, b.address);
    assert_eq!(a.address.kind, OperationKind::Cleanup);
    assert_eq!(a.address.generated, 0);
    assert_eq!(a.address.path[0].point, "entry:1");
    assert_eq!(a.computing_point.point, "entry:0");
    assert_eq!(a.realisation, CleanupRealisation::Reverse);
    assert_eq!(b.realisation, CleanupRealisation::Recompute);
    assert_eq!(reverse.leaves.len(), 6);
    assert_eq!(recompute.leaves.len(), 12);
    assert!(
        recompute
            .leaves
            .iter()
            .all(|l| l.address.as_ref().unwrap().kind != OperationKind::Cleanup)
    );
    let ignored = lower(body, &[entry(j("1"), CleanupRealisation::Recompute)], false).unwrap();
    assert_eq!(ignored.leaves, reverse.leaves);
    for value in 0..4 {
        let mut state = std::collections::BTreeMap::from([
            ("x[0]".to_string(), value & 1 != 0),
            ("x[1]".to_string(), value & 2 != 0),
        ]);
        for l in &recompute.leaves {
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
                "cx" => {
                    if state[&l.wires[0]] {
                        *state.get_mut(&l.wires[1]).unwrap() ^= true;
                    }
                }
                op => panic!("unexpected {op}"),
            }
        }
        assert_eq!(state.len(), 2);
        assert_eq!(state["x[0]"], value & 1 != 0);
        assert_eq!(state["x[1]"], value & 2 != 0);
    }
}
#[test]
fn k65_duplicate_carriers_reject_before_cleanup_expansion_and_adjoint_keeps_points() {
    let body = "d = qcopy x\nquncopy x d";
    let entries = [
        entry(j("0"), CleanupRealisation::Reverse),
        entry(j("0"), CleanupRealisation::Recompute),
    ];
    assert!(
        lower(body, &entries, false)
            .unwrap_err()
            .contains("multiple maximal")
    );
    let adjoint = lower(body, &[], true).unwrap();
    assert_eq!(adjoint.cleanups.len(), 1);
    assert_eq!(adjoint.cleanups[0].address.path[0].point, "entry:0");
    let loops = lower("qloop 2 {\nd = qcopy x\nquncopy x d\n}", &[], false).unwrap();
    assert_eq!(loops.cleanups.len(), 2);
    for (i, c) in loops.cleanups.iter().enumerate() {
        assert_eq!(c.address.path[0].iteration, [i]);
        assert_eq!(c.address.generated, 0);
    }
}
#[test]
fn k65_empty_inverse_sequence_has_a_release_point_carrier() {
    let e = lower("d = qalloc any 2\nqfree d", &[], false).unwrap();
    assert_eq!(e.cleanups.len(), 1);
    assert_eq!(e.cleanups[0].address.path[0].point, "entry:1");
    assert_eq!(e.cleanups[0].address.generated, 0);
    assert_eq!(e.leaves.len(), 2);
}
#[test]
fn k65_nested_callee_selection_uses_the_full_containing_chain() {
    let text = support::source(
        "action @root (x:wire[2] mut) () () () ()\n^entry():\naction_call @callee (x) () () ()\naction @callee (y:wire[2] mut) () () () ()\n^entry():\nd = qcopy y\nquncopy y d",
    );
    let (obj, ids) = bennett_vm::load(&text, false).unwrap();
    let mut specific = entry(j("0"), CleanupRealisation::Recompute);
    specific.selector.path = vec![
        bennett_vm::sched::PathSelector {
            symbol: ids.interface["root"].clone(),
            point: "entry:0".into(),
            iteration: Some(vec![]),
            instance: Some(vec![]),
            call_index: Some(0),
        },
        bennett_vm::sched::PathSelector {
            symbol: ids.interface["callee"].clone(),
            point: "entry:1".into(),
            iteration: Some(vec![]),
            instance: Some(vec![]),
            call_index: None,
        },
    ];
    let e = Catalog::published()
        .unwrap()
        .lower_action_with_cleanup(&obj, "root", &j("[]"), false, 0, &[specific])
        .unwrap();
    assert_eq!(e.cleanups.len(), 1);
    assert_eq!(e.cleanups[0].address.path.len(), 2);
    assert_eq!(e.cleanups[0].realisation, CleanupRealisation::Recompute);
    assert_eq!(e.leaves.len(), 12);
}
#[test]
fn k65_empty_adjoint_call_has_a_carrier_even_with_no_emitted_leaves() {
    let text = support::source(
        "action @root () (p:wire[1] in) () () ()\n^entry():\naction_call @empty () (p) () ()\nadj action_call @empty () (p) () ()\nperm_action @empty () (p:wire[1] in) () () ()\n^entry():\nqpred p\nqunpred p",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    for entries in [vec![], vec![entry(j("0"), CleanupRealisation::Recompute)]] {
        let e = Catalog::published()
            .unwrap()
            .lower_action_with_cleanup(&obj, "root", &j("[]"), false, 0, &entries)
            .unwrap();
        assert!(e.leaves.is_empty());
        assert_eq!(e.cleanups.len(), 1);
        assert_eq!(e.cleanups[0].address.path[0].point, "entry:1");
        assert_eq!(e.cleanups[0].address.path[0].call_index, None);
    }
}
#[test]
fn k65_recomputed_load_is_a_full_domain_clean_map_under_control() {
    let text = support::source(
        "action @root (a:wire[1] mut) () (db:region[1,2] in) () ()\n^entry():\nd = qload db a\nqunload db a d",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let c = Catalog::published().unwrap();
    for choice in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
        let e = c
            .lower_action_with_cleanup(&obj, "root", &j("[]"), false, 1, &[entry(j("0"), choice)])
            .unwrap();
        assert_eq!(e.cleanups.len(), 1);
        assert_eq!(e.cleanups[0].realisation, choice);
        assert_eq!(e.cleanups[0].controls, ["$controls[0]"]);
        for bits in 0..64 {
            let mut state = std::collections::BTreeMap::from([
                ("a[0]".to_string(), bits & 1 != 0),
                ("$controls[0]".to_string(), bits & 2 != 0),
            ]);
            for bit in 0..4 {
                state.insert(format!("db[{bit}]"), bits & (4 << bit) != 0);
            }
            let before = state.clone();
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
                        let (t, controls) = l.wires.split_last().unwrap();
                        if controls.iter().chain(&l.controls).all(|w| state[w]) {
                            *state.get_mut(t).unwrap() ^= true;
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
            assert_eq!(state, before, "{choice:?} at {bits}");
        }
    }
}
#[test]
fn k65_zero_trip_pairs_do_not_generate_carriers_or_ties() {
    let text = support::source(
        "action @root (x:wire[2] mut) () () () ()\nregs { } bounds {0}\n^entry():\nqloop 0 {\nd = qcopy x\nquncopy x d\n}",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let entries = [
        entry(j("0"), CleanupRealisation::Reverse),
        entry(j("0"), CleanupRealisation::Recompute),
    ];
    let e = Catalog::published()
        .unwrap()
        .lower_action_with_cleanup(&obj, "root", &j("[]"), false, 0, &entries)
        .unwrap();
    assert!(e.leaves.is_empty());
    assert!(e.cleanups.is_empty());
}
#[test]
fn default_lowering_preserves_explicit_multirow_inverse_sequences() {
    // Binding the whole explicit inverse must preserve the existing default
    // source trace and its adjoint, while enabling the selected recompute.
    let body = "d = qcopy x\nqxor x d\nqfree d";
    for adjoint in [false, true] {
        let e = lower(body, &[], adjoint).unwrap();
        assert_eq!(e.leaves.len(), 6);
        assert!(e.cleanup_pairing_complete);
        assert_eq!(e.cleanups.len(), 1);
        let selected = lower(
            body,
            &[entry(j("0"), CleanupRealisation::Recompute)],
            adjoint,
        )
        .unwrap();
        assert!(selected.cleanup_pairing_complete);
        assert_eq!(selected.cleanups[0].address, e.cleanups[0].address);
        assert_eq!(selected.leaves.len(), 12);
    }
}
#[test]
fn dormant_unresolved_multirow_pairs_do_not_reject_a_cleanup_policy() {
    let text = support::source(
        "action @root (x:wire[2] mut) () () () ()\nregs { } bounds {0}\n^entry():\nqloop 0 {\nd = qcopy x\nqxor x d\nqfree d\n}",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let e = Catalog::published()
        .unwrap()
        .lower_action_with_cleanup(
            &obj,
            "root",
            &j("[]"),
            false,
            0,
            &[entry(j("0"), CleanupRealisation::Recompute)],
        )
        .unwrap();
    assert!(e.leaves.is_empty());
    assert!(e.cleanups.is_empty());
    assert!(e.cleanup_pairing_complete);
}
