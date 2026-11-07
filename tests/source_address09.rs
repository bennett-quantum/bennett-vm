//! Source-derived leaf addresses, never private call/step coordinates.
use crate::source09 as support;
use bennett_vm::{json::Json, ref_library::Catalog};
fn lower(body: &str, adjoint: bool) -> Vec<bennett_vm::ref_library::Leaf> {
    let bounds = body
        .lines()
        .filter_map(|l| l.strip_prefix("qloop "))
        .map(|l| l.split_whitespace().next().unwrap())
        .collect::<Vec<_>>()
        .join(",");
    let text = support::source(&format!(
        "action @root (x:wire[2] mut, z:wire[2] mut) () () () ()\nregs {{ }} bounds {{{bounds}}}\n^entry():\n{body}\naction @a (y:wire[2] mut) () () () ()\n^entry():\nx y[0]\nx y[1]"
    ));
    let (obj, ids) = bennett_vm::load(&text, false).unwrap();
    let leaves = Catalog::published()
        .unwrap()
        .lower_action(&obj, "root", &Json::Arr(vec![]), adjoint, 0)
        .unwrap();
    assert!(
        leaves
            .iter()
            .all(|l| l.address.as_ref().unwrap().path[0].symbol == ids.interface["root"])
    );
    leaves
}
#[test]
fn row_ordinals_and_call_occurrences_reset_at_each_source_point() {
    let leaves = lower(
        "action_call @a (x) () () ()\naction_call @a (x) () () ()",
        false,
    );
    assert_eq!(
        leaves.iter().map(|l| l.ordinal).collect::<Vec<_>>(),
        [0, 1, 0, 1]
    );
    for (i, l) in leaves.iter().enumerate() {
        let a = l.address.as_ref().expect("source leaf must be addressed");
        assert_eq!(a.path.len(), 2);
        assert_eq!(a.path[0].point, format!("entry:{}", i / 2));
        assert_eq!(a.path[0].call_index, Some(0));
        assert_eq!(a.path[1].point, format!("entry:{}", i % 2));
        assert_eq!(a.path[1].call_index, None);
        assert_eq!(l.invocation[0].call_index, 0);
        assert_eq!(a.generated, 0);
        assert_eq!(a.step, None);
        assert_eq!(a.phase, "");
    }
}
#[test]
fn nested_source_loop_iterations_are_not_catalog_constructor_iterations() {
    let leaves = lower(
        "qloop 2 {\nqloop 3 {\naction_call @a (x) () () ()\n}\n}",
        false,
    );
    assert_eq!(leaves.len(), 12);
    for (i, l) in leaves.iter().enumerate() {
        let a = l.address.as_ref().unwrap();
        assert_eq!(a.path[0].point, "entry:2");
        assert_eq!(a.path[0].iteration, [i / 6, (i / 2) % 3]);
        assert!(
            a.path[1].iteration.is_empty(),
            "callee has its own source loop domain"
        );
        assert_eq!(a.path[0].call_index, Some(0));
    }
    let qxor = lower("qxor x z", false);
    assert_eq!(qxor.len(), 2);
    assert!(
        qxor.iter()
            .all(|l| l.address.as_ref().unwrap().path[0].iteration.is_empty())
    );
    assert_eq!(qxor.iter().map(|l| l.ordinal).collect::<Vec<_>>(), [0, 1]);
}
#[test]
fn adjoint_keeps_forward_source_points_and_iteration_coordinates() {
    let body = "qloop 2 {\naction_call @a (x) () () ()\n}\nx x[0]";
    let forward = lower(body, false);
    let inverse = lower(body, true);
    assert_eq!(forward.len(), inverse.len());
    for (f, r) in forward.iter().rev().zip(&inverse) {
        assert_eq!(f.address, r.address);
        assert_eq!(
            f.address.as_ref().unwrap().generated,
            r.address.as_ref().unwrap().generated
        );
        assert_eq!(
            f.invocation[0..f.invocation.len()]
                .iter()
                .map(|c| c.call_index)
                .collect::<Vec<_>>(),
            r.invocation
                .iter()
                .map(|c| c.call_index)
                .collect::<Vec<_>>()
        );
    }
}
#[test]
fn implicit_qregion_support_retains_declaration_provenance() {
    let leaves = lower("qregion m 1 2\nx x[0]", false);
    assert!(
        leaves.iter().all(|l| l.address.is_some()),
        "implicit local image support must be addressed"
    );
    let support = leaves
        .iter()
        .filter(|l| l.operation == "alloc" || l.operation == "release")
        .collect::<Vec<_>>();
    assert_eq!(support.len(), 2);
    for (l, point) in support.iter().zip(["$construct", "$release"]) {
        let a = l.address.as_ref().unwrap();
        assert_eq!(a.path[0].point, point);
        assert!(a.path[0].iteration.is_empty());
        assert!(a.path[0].instance.is_empty());
        assert_eq!(a.generated, 0);
    }
}
