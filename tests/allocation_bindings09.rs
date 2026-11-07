//! Reallocated linear names retain distinct source lifetimes and metadata.
use crate::source09 as support;
use bennett_vm::{
    json::Json,
    ref_library::{Catalog, CleanupEntry, CleanupRealisation, Leaf},
    sched::Selector,
};
use std::collections::BTreeMap;

fn object(body: &str) -> bennett_vm::object::Object {
    let bounds = body
        .lines()
        .filter_map(|l| l.strip_prefix("qloop "))
        .map(|l| l.split_whitespace().next().unwrap())
        .collect::<Vec<_>>()
        .join(",");
    let text = support::source(&format!(
        "action @root (a:wire[2] mut, t:wire mut) (p:wire in) (r:region[0,2] mut) () ()\nregs {{}} bounds {{{bounds}}}\n^entry():\n{body}"
    ));
    bennett_vm::load(&text, false).unwrap().0
}

fn assert_identity(leaves: &[Leaf]) {
    let names = ["a[0]", "a[1]", "t[0]", "p[0]", "r[0]", "r[1]"];
    for basis in 0..64 {
        for borrowed in 0..4 {
            let mut values = names
                .iter()
                .enumerate()
                .map(|(bit, n)| (n.to_string(), basis & (1 << bit) != 0))
                .collect::<BTreeMap<_, _>>();
            let original = values.clone();
            let mut acquired = BTreeMap::new();
            for leaf in leaves {
                match leaf.operation.as_str() {
                    "alloc" => {
                        for (bit, wire) in leaf.wires.iter().enumerate() {
                            let value = leaf.state.as_deref() == Some("borrowed")
                                && borrowed & (1 << bit) != 0;
                            assert!(values.insert(wire.clone(), value).is_none());
                            assert!(acquired.insert(wire.clone(), value).is_none());
                        }
                    }
                    "release" => {
                        for wire in &leaf.wires {
                            assert_eq!(values.remove(wire), acquired.remove(wire));
                        }
                    }
                    "x" | "cx" | "ccx" | "mcx" => {
                        let (target, controls) = leaf.wires.split_last().unwrap();
                        if controls.iter().chain(&leaf.controls).all(|w| values[w]) {
                            *values.get_mut(target).unwrap() ^= true;
                        }
                    }
                    "swap" => {
                        if leaf.controls.iter().all(|w| values[w]) {
                            let (a, b) = (values[&leaf.wires[0]], values[&leaf.wires[1]]);
                            values.insert(leaf.wires[0].clone(), b);
                            values.insert(leaf.wires[1].clone(), a);
                        }
                    }
                    op => panic!("unexpected permutation leaf {op}"),
                }
            }
            assert!(acquired.is_empty());
            assert_eq!(values, original, "basis={basis}, borrowed={borrowed}");
        }
    }
}

#[test]
fn completed_lifetimes_can_reuse_names_with_different_shapes_and_kinds() {
    let cases = [
        "d = qalloc any 1\nqfree d\nd = qalloc any 1\nqfree d",
        "d = qalloc cold 1\nx d\nx d\nqfree d\nd = qalloc hot 2\nx d[1]\nx d[1]\nqfree d",
        "d = qcopy a\nquncopy a d\nd = qcopy p\nquncopy p d",
        "d = qload r ()\nqunload r () d\nd = qalloc any 1\nqfree d",
        "d = qborrow any 1\ncx d t\ncx d t\nqreturn d\nd = qalloc any 2\nqfree d",
        "qif p {\nd = qalloc any 1\nqfree d\nd = qalloc any 2\n} {\nqfree d\n}",
        "qloop 3 {\nd = qalloc any 1\nqfree d\nd = qalloc any 2\nqfree d\n}",
        "d = qalloc any 1\nqloop 3 {\nqfree d\nd = qalloc any 1\n}\nqfree d",
        "d = qalloc cold 1\nqloop 1 {\nqfree d\nd = qalloc hot 1\n}\nqfree d",
        "d = qalloc cold 1\nqloop 0 {\nqfree d\nd = qalloc hot 1\n}\nqfree d",
    ];
    let catalog = Catalog::published().unwrap();
    for body in cases {
        let object = object(body);
        for adjoint in [false, true] {
            let leaves = catalog
                .lower_action(&object, "root", &Json::Arr(vec![]), adjoint, 0)
                .unwrap_or_else(|e| panic!("adjoint={adjoint}, {body}: {e}"));
            let mut live = BTreeMap::new();
            for leaf in &leaves {
                if leaf.operation == "alloc" {
                    assert!(
                        live.insert(
                            leaf.allocation.clone().unwrap(),
                            (leaf.wires.len(), leaf.class.clone(), leaf.state.clone())
                        )
                        .is_none()
                    );
                } else if leaf.operation == "release" {
                    assert_eq!(
                        live.remove(leaf.allocation.as_ref().unwrap()),
                        Some((leaf.wires.len(), leaf.class.clone(), leaf.state.clone()))
                    );
                }
            }
            assert!(live.is_empty());
            assert_identity(&leaves);
            let expected: Vec<_> = if body.contains("cold") {
                if body.contains("qloop 0") {
                    vec![(1, "cold")]
                } else if body.contains("hot 2") {
                    vec![(1, "cold"), (2, "hot")]
                } else {
                    vec![(1, "cold"), (1, "hot")]
                }
            } else {
                vec![]
            };
            if !expected.is_empty() {
                let mut actual = leaves
                    .iter()
                    .filter(|l| l.operation == "alloc")
                    .map(|l| (l.wires.len(), l.class.as_deref().unwrap()))
                    .collect::<Vec<_>>();
                if adjoint {
                    actual.reverse();
                }
                assert_eq!(actual, expected, "{body}");
            }
        }
    }
}

#[test]
fn reused_copy_names_keep_each_cleanup_choice_and_input_width() {
    let object = object("d = qcopy a\nquncopy a d\nd = qcopy p\nquncopy p d");
    let catalog = Catalog::published().unwrap();
    for adjoint in [false, true] {
        let default = catalog
            .lower_action_with_cleanup(&object, "root", &Json::Arr(vec![]), adjoint, 0, &[])
            .unwrap();
        assert_eq!(default.cleanups.len(), 2);
        for first in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
            for second in [CleanupRealisation::Reverse, CleanupRealisation::Recompute] {
                let entries = default
                    .cleanups
                    .iter()
                    .zip([first, second])
                    .map(|(carrier, realisation)| CleanupEntry {
                        selector: Selector::parse(&Json::obj(vec![
                            (
                                "path",
                                Json::Arr(vec![Json::obj(vec![
                                    ("symbol", Json::Str(carrier.address.path[0].symbol.clone())),
                                    ("point", Json::Str(carrier.address.path[0].point.clone())),
                                    ("iteration", Json::Str("*".into())),
                                ])]),
                            ),
                            ("step", Json::Str("*".into())),
                            ("phase", Json::Str("*".into())),
                            ("generated", Json::u64(0)),
                        ]))
                        .unwrap(),
                        realisation,
                    })
                    .collect::<Vec<_>>();
                let expansion = catalog
                    .lower_action_with_cleanup(
                        &object,
                        "root",
                        &Json::Arr(vec![]),
                        adjoint,
                        0,
                        &entries,
                    )
                    .unwrap();
                assert_eq!(
                    expansion
                        .cleanups
                        .iter()
                        .map(|c| c.realisation)
                        .collect::<Vec<_>>(),
                    [first, second]
                );
                assert_identity(&expansion.leaves);
            }
        }
    }
}

#[test]
fn loop_carried_copy_slots_keep_the_default_trace_without_claiming_a_future_owner() {
    let object = object("d = qcopy a\nqloop 3 {\nquncopy a d\nd = qcopy a\n}\nquncopy a d");
    let catalog = Catalog::published().unwrap();
    for adjoint in [false, true] {
        let expansion = catalog
            .lower_action_with_cleanup(&object, "root", &Json::Arr(vec![]), adjoint, 0, &[])
            .unwrap();
        assert!(!expansion.cleanup_pairing_complete);
        assert!(expansion.cleanups.is_empty());
        assert_identity(&expansion.leaves);
    }
}

#[test]
fn loop_carried_lifetimes_retain_each_acquisitions_class() {
    let catalog = Catalog::published().unwrap();
    for (body, hot) in [
        (
            "d = qalloc cold 1\nqloop 3 {\nqfree d\nd = qalloc hot 1\n}\nqfree d",
            3,
        ),
        (
            "d = qalloc cold 1\nqloop 2 {\nqloop 2 {\nqfree d\nd = qalloc hot 1\n}\n}\nqfree d",
            4,
        ),
        (
            "d = qalloc cold 1\nqloop 2 while a {\nqfree d\nd = qalloc hot 1\n}\nqfree d",
            2,
        ),
    ] {
        let object = object(body);
        for adjoint in [false, true] {
            let leaves = catalog
                .lower_action(&object, "root", &Json::Arr(vec![]), adjoint, 0)
                .unwrap();
            let mut acquired = leaves
                .iter()
                .filter(|l| l.operation == "alloc")
                .map(|l| l.class.as_deref().unwrap())
                .collect::<Vec<_>>();
            let mut released = leaves
                .iter()
                .filter(|l| l.operation == "release")
                .map(|l| l.class.as_deref().unwrap())
                .collect::<Vec<_>>();
            if body.contains("while") {
                let (expected_acquired, expected_released) = if adjoint {
                    (["hot", "any", "hot", "cold"], ["hot", "hot", "any", "cold"])
                } else {
                    (["cold", "any", "hot", "hot"], ["cold", "hot", "any", "hot"])
                };
                assert_eq!(acquired, expected_acquired);
                assert_eq!(released, expected_released);
                assert_eq!(
                    leaves
                        .iter()
                        .filter(|l| l.operation == "alloc" && l.class.as_deref() == Some("any"))
                        .map(|l| l.wires.len())
                        .collect::<Vec<_>>(),
                    [2]
                );
                continue;
            }
            if adjoint {
                acquired.reverse();
                released.reverse();
            }
            assert_eq!(
                acquired,
                std::iter::once("cold")
                    .chain(std::iter::repeat_n("hot", hot))
                    .collect::<Vec<_>>()
            );
            assert_eq!(released, acquired);
        }
    }
}

#[test]
fn inverse_execution_restores_each_visited_allocation_class() {
    use bennett_vm::{
        action::{QuantumState, RegionWires, execute_action_inner},
        json,
        target::Target,
    };
    let path = crate::pins::isa().join("conformance/cases/E/E-matching-first-entry/target.json");
    let mut target = json::parse(std::fs::read_to_string(path).unwrap().trim_end()).unwrap();
    let Json::Obj(fields) = &mut target else {
        panic!("target object");
    };
    fields.insert("classes".into(), json::parse(r#"{"fast":{"capacity":16,"dirty_rate":1,"qubit_second":1},"cold":{"capacity":2,"dirty_rate":1,"qubit_second":1},"hot":{"capacity":2,"dirty_rate":1,"qubit_second":1}}"#).unwrap());
    let target = Target::parse(&target.to_jcs()).unwrap();
    for body in [
        "d = qalloc cold 1\nqloop 3 {\nqfree d\nd = qalloc hot 1\n}\nqfree d",
        "d = qalloc cold 1\nqloop 2 {\nqloop 2 {\nqfree d\nd = qalloc hot 1\n}\n}\nqfree d",
    ] {
        let object = object(body);
        for adjoint in [false, true] {
            let mut state = QuantumState::new();
            state.set_target(target.clone()).unwrap();
            let wires = state.target_alloc(6, "fast").unwrap();
            execute_action_inner(
                &object,
                &mut state,
                "root",
                &[wires[..2].to_vec(), vec![wires[2]]],
                &[vec![wires[3]]],
                &[RegionWires {
                    a: 0,
                    w: 2,
                    cells: vec![wires[4..].to_vec()],
                }],
                &[],
                adjoint,
                &[],
            )
            .unwrap();
            let accounting = state.target_accounting().unwrap();
            for class in ["cold", "hot"] {
                assert_eq!(
                    accounting
                        .get("peak_qubits")
                        .unwrap()
                        .get(class)
                        .and_then(Json::as_str),
                    Some("1"),
                    "adjoint={adjoint}, class={class}"
                );
            }
        }
    }
}

#[test]
fn inverse_while_visits_restore_bindings_even_when_the_gate_predicate_is_false() {
    use bennett_vm::{
        action::{QuantumState, RegionWires, execute_action_inner},
        bigint::BigUint,
    };
    let object =
        object("d = qalloc cold 1\nqloop 2 while a {\nqfree d\nd = qalloc hot 1\n}\nqfree d");
    for bits in 0..4 {
        for adjoint in [false, true] {
            let mut state = QuantumState::new();
            let wires = state.alloc_wires(6);
            state.prepare(&wires[..2], &BigUint::from_u64(bits));
            let before = state.clone();
            execute_action_inner(
                &object,
                &mut state,
                "root",
                &[wires[..2].to_vec(), vec![wires[2]]],
                &[vec![wires[3]]],
                &[RegionWires {
                    a: 0,
                    w: 2,
                    cells: vec![wires[4..].to_vec()],
                }],
                &[],
                adjoint,
                &[],
            )
            .unwrap();
            assert!(
                state.exact_equals(&before),
                "condition={bits}, adjoint={adjoint}"
            );
        }
    }
}
