//! Reallocated linear names retain distinct source lifetimes and metadata.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::{json::Json, ref_library::Catalog};
use std::collections::BTreeMap;

fn object(body: &str) -> bennett_vm::object::Object {
    let bounds = body.lines().filter_map(|l| l.strip_prefix("qloop "))
        .map(|l| l.split_whitespace().next().unwrap()).collect::<Vec<_>>().join(",");
    let text = support::source(&format!("action @root (a:wire[2] mut, t:wire mut) (p:wire in) (r:region[0,2] mut) () ()\nregs {{}} bounds {{{bounds}}}\n^entry():\n{body}"));
    bennett_vm::load(&text, false).unwrap().0
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
            let leaves = catalog.lower_action(&object, "root", &Json::Arr(vec![]), adjoint, 0)
                .unwrap_or_else(|e| panic!("adjoint={adjoint}, {body}: {e}"));
            let mut live = BTreeMap::new();
            for leaf in &leaves {
                if leaf.operation == "alloc" {
                    assert!(live.insert(leaf.allocation.clone().unwrap(), (leaf.wires.len(), leaf.class.clone(), leaf.state.clone())).is_none());
                } else if leaf.operation == "release" {
                    assert_eq!(live.remove(leaf.allocation.as_ref().unwrap()), Some((leaf.wires.len(), leaf.class.clone(), leaf.state.clone())));
                }
            }
            assert!(live.is_empty());
            let expected: Vec<_> = if body.contains("cold") {
                if body.contains("qloop 0") { vec![(1, "cold")] }
                else if body.contains("hot 2") { vec![(1, "cold"), (2, "hot")] }
                else { vec![(1, "cold"), (1, "hot")] }
            } else { vec![] };
            if !expected.is_empty() {
                let mut actual = leaves.iter().filter(|l| l.operation == "alloc")
                    .map(|l| (l.wires.len(), l.class.as_deref().unwrap())).collect::<Vec<_>>();
                if adjoint { actual.reverse(); }
                assert_eq!(actual, expected, "{body}");
            }
        }
    }
}

#[test]
fn loop_carried_lifetimes_retain_each_acquisitions_class() {
    let object = object("d = qalloc cold 1\nqloop 3 {\nqfree d\nd = qalloc hot 1\n}\nqfree d");
    let catalog = Catalog::published().unwrap();
    for adjoint in [false, true] {
        let leaves = catalog.lower_action(&object, "root", &Json::Arr(vec![]), adjoint, 0).unwrap();
        let mut acquired = leaves.iter().filter(|l| l.operation == "alloc").map(|l| l.class.as_deref().unwrap()).collect::<Vec<_>>();
        let mut released = leaves.iter().filter(|l| l.operation == "release").map(|l| l.class.as_deref().unwrap()).collect::<Vec<_>>();
        if adjoint { acquired.reverse(); released.reverse(); }
        assert_eq!(acquired, ["cold", "hot", "hot", "hot"]);
        assert_eq!(released, acquired);
    }
}

#[test]
fn inverse_execution_restores_each_visited_allocation_class() {
    use bennett_vm::{action::{QuantumState, RegionWires, execute_action_inner}, json, target::Target};
    let object = object("d = qalloc cold 1\nqloop 3 {\nqfree d\nd = qalloc hot 1\n}\nqfree d");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../bennett-isa-spec/conformance/cases/E/E-matching-first-entry/target.json");
    let mut target = json::parse(std::fs::read_to_string(path).unwrap().trim_end()).unwrap();
    let Json::Obj(fields) = &mut target else { panic!("target object"); };
    fields.insert("classes".into(), json::parse(r#"{"fast":{"capacity":16,"dirty_rate":1,"qubit_second":1},"cold":{"capacity":2,"dirty_rate":1,"qubit_second":1},"hot":{"capacity":2,"dirty_rate":1,"qubit_second":1}}"#).unwrap());
    let target = Target::parse(&target.to_jcs()).unwrap();
    for adjoint in [false, true] {
        let mut state = QuantumState::new();
        state.set_target(target.clone()).unwrap();
        let wires = state.target_alloc(6, "fast").unwrap();
        execute_action_inner(&object, &mut state, "root", &[wires[..2].to_vec(), vec![wires[2]]], &[vec![wires[3]]], &[RegionWires { a:0,w:2,cells:vec![wires[4..].to_vec()] }], &[], adjoint, &[]).unwrap();
        let accounting = state.target_accounting().unwrap();
        for class in ["cold", "hot"] {
            assert_eq!(accounting.get("peak_qubits").unwrap().get(class).and_then(Json::as_str), Some("1"), "adjoint={adjoint}, class={class}");
        }
    }
}
