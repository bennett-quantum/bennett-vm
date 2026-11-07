//! Explicit multirow inverse sequences bind before their first cleanup leaf.
use crate::source09 as support;
use bennett_vm::{
    json::{self, Json},
    ref_library::{ActionExpansion, Catalog, CleanupEntry, CleanupRealisation},
    sched::Selector,
};
fn lower(body: &str, recompute: bool, adjoint: bool) -> ActionExpansion {
    let text = support::source(&format!(
        "action @root (x:wire[1] mut, z:wire[1] mut) () () () ()\n^entry():\n{body}"
    ));
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let entries = if recompute {
        vec![CleanupEntry {
            selector: Selector::parse(
                &json::parse(r#"{"path":[],"step":"*","phase":"*","generated":0}"#).unwrap(),
            )
            .unwrap(),
            realisation: CleanupRealisation::Recompute,
        }]
    } else {
        vec![]
    };
    Catalog::published()
        .unwrap()
        .lower_action_with_cleanup(&obj, "root", &Json::Arr(vec![]), adjoint, 0, &entries)
        .unwrap()
}
fn replay(e: &ActionExpansion, input: usize) {
    let mut state = std::collections::BTreeMap::from([
        ("x[0]".to_string(), input & 1 != 0),
        ("z[0]".to_string(), input & 2 != 0),
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
            "cx" => {
                if state[&l.wires[0]] && l.controls.iter().all(|w| state[w]) {
                    *state.get_mut(&l.wires[1]).unwrap() ^= true;
                }
            }
            op => panic!("unexpected {op}"),
        }
    }
    assert_eq!(state, original);
}
#[test]
fn whole_multirow_cleanup_has_one_carrier_and_preserves_reverse_row_points() {
    let body = "d = qalloc any 1\ncx x d\ncx z d\ncx z d\ncx x d\nqfree d";
    for adjoint in [false, true] {
        let reverse = lower(body, false, adjoint);
        assert!(reverse.cleanup_pairing_complete);
        assert_eq!(reverse.cleanups.len(), 1);
        let c = &reverse.cleanups[0];
        assert_eq!(c.address.generated, 0);
        assert_eq!(
            c.address.path[0].point,
            if adjoint { "entry:0" } else { "entry:5" }
        );
        assert_eq!(
            c.computing_point.point,
            if adjoint { "entry:5" } else { "entry:0" }
        );
        assert_eq!(reverse.leaves.len(), 6);
        let points = reverse
            .leaves
            .iter()
            .map(|l| l.address.as_ref().unwrap().path[0].point.clone())
            .collect::<Vec<_>>();
        let mut expected = (0..6).map(|i| format!("entry:{i}")).collect::<Vec<_>>();
        if adjoint {
            expected.reverse();
        }
        assert_eq!(points, expected);
        let recompute = lower(body, true, adjoint);
        assert!(recompute.cleanup_pairing_complete);
        assert_eq!(recompute.cleanups.len(), 1);
        assert_eq!(recompute.cleanups[0].address, c.address);
        assert_eq!(recompute.leaves.len(), 11);
        // Newly instantiated recompute leaves belong to the cleanup source
        // instruction; written reverse leaves keep their original row points.
        let leaves = &recompute.leaves[3..];
        // Pass X/K80 requires fresh, emitted-order ordinals for the whole
        // replacement; raw inverse ordinal retention applies only to reversal
        // of an already expanded trace.
        for (n, leaf) in (0..8).zip(leaves) {
            let a = leaf.address.as_ref().unwrap();
            assert_eq!(a.path[0].point, c.address.path[0].point);
            assert_eq!(a.generated, n);
        }
        for input in 0..4 {
            replay(&reverse, input);
            replay(&recompute, input);
        }
    }
}
#[test]
fn copy_computation_and_explicit_xor_release_bind_the_same_value() {
    let body = "d = qcopy x\ncx z d\ncx z d\nqxor x d\nqfree d";
    for adjoint in [false, true] {
        for recompute in [false, true] {
            let e = lower(body, recompute, adjoint);
            assert!(e.cleanup_pairing_complete);
            assert_eq!(e.cleanups.len(), 1);
            assert_eq!(
                e.cleanups[0].address.path[0].point,
                if adjoint { "entry:0" } else { "entry:4" }
            );
            for input in 0..4 {
                replay(&e, input);
            }
        }
    }
}

#[test]
fn nonempty_nonadjacent_action_adjoint_pair_has_a_whole_call_carrier() {
    let text = support::source(
        "action @root (x:wire mut, z:wire mut) () () () ()\n^entry():\naction_call @flip (x, z) () () ()\nx z\nx z\nadj action_call @flip (x, z) () () ()\nperm_action @flip (y:wire mut, w:wire mut) () () () ()\n^entry():\ncx y w",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    for adjoint in [false, true] {
        let e = Catalog::published()
            .unwrap()
            .lower_action_with_cleanup(&obj, "root", &Json::Arr(vec![]), adjoint, 0, &[])
            .unwrap();
        assert!(e.cleanup_pairing_complete);
        assert_eq!(e.cleanups.len(), 1);
        assert_eq!(
            e.cleanups[0].address.path[0].point,
            if adjoint { "entry:0" } else { "entry:3" }
        );
        assert_eq!(e.cleanups[0].address.path[0].call_index, None);
        assert_eq!(e.leaves.len(), 4);
        assert_eq!(e.leaves[0].invocation[0].call_index, 0);
        assert_eq!(e.leaves[3].invocation[0].call_index, 0);
    }
}

#[test]
fn recompute_rejects_other_values_and_unknown_nonempty_call_input_versions() {
    let text = support::source(
        "action @root (x:wire mut) () () () ()\n^entry():\naction_call @flip (x) () () ()\nadj action_call @flip (x) () () ()\nperm_action @flip (y:wire mut) () () () ()\n^entry():\nx y",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let entries = [CleanupEntry {
        selector: Selector::parse(
            &json::parse(r#"{"path":[],"step":"*","phase":"*","generated":0}"#).unwrap(),
        )
        .unwrap(),
        realisation: CleanupRealisation::Recompute,
    }];
    let error = Catalog::published()
        .unwrap()
        .lower_action_with_cleanup(&obj, "root", &Json::Arr(vec![]), false, 0, &entries)
        .unwrap_err();
    assert!(
        error.contains("exact clean-input computing sequence"),
        "{error}"
    );

    let text = support::source(
        "action @root () () () () ()\n^entry():\nd = qalloc any 1\nh d\nh d\nqfree d",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let c = Catalog::published().unwrap();
    let reverse = c
        .lower_action_with_cleanup(&obj, "root", &Json::Arr(vec![]), false, 0, &[])
        .unwrap();
    assert!(reverse.cleanup_pairing_complete);
    assert_eq!(reverse.cleanups.len(), 1);
    assert_eq!(reverse.leaves.len(), 4);
    let error = c
        .lower_action_with_cleanup(&obj, "root", &Json::Arr(vec![]), false, 0, &entries)
        .unwrap_err();
    assert!(error.contains("permutation computing sequence"), "{error}");
}

#[test]
fn multirow_carriers_keep_effective_controls_iterations_classes_and_ties() {
    let text = support::source(
        "action @root (x:wire mut) (p:wire in) () () ()\nregs { } bounds {2}\n^entry():\nqloop 2 {\nqpred p\nd = qalloc fast 1\ncx x d\ncx x d\nqfree d\nqunpred p\n}",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let selector = Selector::parse(
        &json::parse(r#"{"path":[],"step":"*","phase":"*","generated":0}"#).unwrap(),
    )
    .unwrap();
    let entry = CleanupEntry {
        selector,
        realisation: CleanupRealisation::Recompute,
    };
    for adjoint in [false, true] {
        let e = Catalog::published()
            .unwrap()
            .lower_action_with_cleanup(
                &obj,
                "root",
                &Json::Arr(vec![]),
                adjoint,
                0,
                std::slice::from_ref(&entry),
            )
            .unwrap();
        assert!(e.cleanup_pairing_complete);
        assert_eq!(e.cleanups.len(), 2);
        for (index, carrier) in e.cleanups.iter().enumerate() {
            assert_eq!(
                carrier.address.path[0].iteration,
                [if adjoint { 1 - index } else { index }]
            );
            assert_eq!(carrier.controls, ["p[0]"]);
        }
        assert!(
            e.leaves
                .iter()
                .filter(|l| l.operation == "alloc")
                .all(|l| l.class.as_deref() == Some("fast"))
        );
        assert!(
            e.leaves
                .iter()
                .filter(|l| l.operation == "cx")
                .all(|l| l.controls == ["p[0]"])
        );
        let error = Catalog::published()
            .unwrap()
            .lower_action_with_cleanup(
                &obj,
                "root",
                &Json::Arr(vec![]),
                adjoint,
                0,
                &[entry.clone(), entry.clone()],
            )
            .unwrap_err();
        assert!(error.contains("multiple maximal"), "{error}");
    }
}

#[test]
fn controlled_call_cleanup_carrier_includes_the_call_control() {
    let text = support::source(
        "action @root (x:wire mut) (p:wire in) () () ()\n^entry():\nctl p action_call @flip (x) () () ()\nctl p adj action_call @flip (x) () () ()\nperm_action @flip (y:wire mut) () () () ()\n^entry():\nx y",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    for adjoint in [false, true] {
        let e = Catalog::published()
            .unwrap()
            .lower_action_with_cleanup(&obj, "root", &Json::Arr(vec![]), adjoint, 0, &[])
            .unwrap();
        assert!(e.cleanup_pairing_complete);
        assert_eq!(e.cleanups.len(), 1);
        assert_eq!(e.cleanups[0].controls, ["p[0]"]);
        assert!(e.leaves.iter().all(|l| l.controls == ["p[0]"]));
    }
}

#[test]
fn unbound_list_and_nested_target_writes_are_not_mistaken_for_empty_computations() {
    for body in [
        "d = qalloc any 1\naction_call @flip (d) () () ()\naction_call @flip (d) () () ()\nqfree d",
        "d = qalloc any 1\nqloop 2 {\nx d\n}\nqfree d",
    ] {
        let bounds = if body.contains("qloop") {
            "regs { } bounds {2}\n"
        } else {
            ""
        };
        let text = support::source(&format!(
            "action @root () () () () ()\n{bounds}^entry():\n{body}\nperm_action @flip (y:wire mut) () () () ()\n^entry():\nx y"
        ));
        let (obj, _) = bennett_vm::load(&text, false).unwrap();
        let e = Catalog::published()
            .unwrap()
            .lower_action_with_cleanup(&obj, "root", &Json::Arr(vec![]), false, 0, &[])
            .unwrap();
        assert!(!e.cleanup_pairing_complete, "unbound writes: {body}");
    }
}
