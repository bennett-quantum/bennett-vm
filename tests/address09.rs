//! K49–K65 address and operation-domain witnesses, independent of target choices.
use bennett_vm::{
    json::{self, Json},
    sched::{self, Coordinate, Operation, OperationKind, Point, Selector},
};
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
fn selector(path: Json, step: Json, phase: &str, generated: Json) -> Selector {
    Selector::parse(&Json::obj(vec![
        ("path", path),
        ("step", step),
        ("phase", Json::Str(phase.into())),
        ("generated", generated),
    ]))
    .unwrap()
}
fn wild() -> Json {
    j(r#""*""#)
}
fn point(name: &str) -> Point {
    Point {
        symbol: "a".repeat(64),
        point: name.into(),
        ..Point::default()
    }
}
fn path(p: &Point, instance: Json, call: Json) -> Json {
    Json::Arr(vec![Json::obj(vec![
        ("symbol", Json::Str(p.symbol.clone())),
        ("point", Json::Str(p.point.clone())),
        ("iteration", j("[]")),
        ("instance", instance),
        ("call_index", call),
    ])])
}
#[test]
fn k52_reads_last_entered_oracle_only_for_proper_prefix() {
    let mut edge = point("entry:4");
    edge.call_index = Some(1);
    edge.instance = vec![0];
    edge.oracle = Some(Coordinate {
        step: Some(3),
        phase: "rev".into(),
    });
    let mut op = Operation {
        path: vec![edge.clone(), point("entry:0")],
        step: Some(1),
        phase: "fwd".into(),
        generated: 0,
        kind: OperationKind::Gate,
    };
    let root = selector(j("[]"), j("1"), "fwd", wild());
    let nested = selector(path(&edge, j("[0]"), j("1")), j("3"), "rev", wild());
    assert_eq!(
        sched::select(std::slice::from_ref(&root), &op).unwrap(),
        Some(0)
    );
    assert_eq!(
        sched::select(std::slice::from_ref(&nested), &op).unwrap(),
        Some(0)
    );
    op.path.pop();
    assert_eq!(
        sched::select(&[nested], &op).unwrap(),
        None,
        "terminal call point remains in containing invocation"
    );
    assert_eq!(sched::select(&[root], &op).unwrap(), Some(0));
}
#[test]
fn k51_nonstep_and_k63_occurrences_do_not_add_specificity() {
    let mut p = point("entry:0");
    p.instance = vec![0];
    p.call_index = Some(1);
    let op = Operation {
        path: vec![p.clone()],
        step: None,
        phase: String::new(),
        generated: 0,
        kind: OperationKind::Gate,
    };
    let exact = selector(path(&p, j("[0]"), j("1")), wild(), "*", wild());
    let broad = selector(path(&p, wild(), wild()), wild(), "*", wild());
    assert_eq!(
        sched::select(std::slice::from_ref(&exact), &op).unwrap(),
        Some(0)
    );
    assert_eq!(exact.specificity(), broad.specificity());
    assert!(
        sched::select(&[exact, broad], &op)
            .unwrap_err()
            .contains("multiple maximal")
    );
    for (instance, call, step, phase) in [
        (j("[1]"), j("1"), wild(), "*"),
        (j("[0]"), j("0"), wild(), "*"),
        (wild(), wild(), j("1"), "*"),
        (wild(), wild(), wild(), "rev"),
    ] {
        assert_eq!(
            sched::select(
                &[selector(path(&p, instance, call), step, phase, wild())],
                &op
            )
            .unwrap(),
            None
        );
    }
    let mut terminal = op;
    terminal.path[0].call_index = None;
    assert_eq!(
        sched::select(
            &[selector(path(&p, wild(), j("1")), wild(), "*", wild())],
            &terminal
        )
        .unwrap(),
        None
    );
}
#[test]
fn k54_and_k65_cleanup_carriers_are_disjoint_nonemitting_decisions() {
    let op = Operation {
        path: vec![point("entry:2")],
        step: None,
        phase: String::new(),
        generated: 0,
        kind: OperationKind::Cleanup,
    };
    let s = selector(j("[]"), wild(), "*", j("0"));
    for kind in [
        OperationKind::Gate,
        OperationKind::Alloc,
        OperationKind::Start,
    ] {
        assert_eq!(
            sched::select_kind(&[(kind, s.clone())], &op, kind).unwrap(),
            None
        );
    }
    assert_eq!(
        sched::select_kind(
            &[(OperationKind::Cleanup, s.clone())],
            &op,
            OperationKind::Cleanup
        )
        .unwrap(),
        Some(0)
    );
    let other = selector(j("[]"), wild(), "*", j("1"));
    assert_eq!(
        sched::select_kind(
            &[(OperationKind::Cleanup, other)],
            &op,
            OperationKind::Cleanup
        )
        .unwrap(),
        None
    );
    let gate = Operation {
        kind: OperationKind::Gate,
        ..op
    };
    assert_eq!(
        sched::select_kind(
            &[
                (OperationKind::Cleanup, s.clone()),
                (OperationKind::Start, s.clone()),
                (OperationKind::Gate, s)
            ],
            &gate,
            OperationKind::Gate
        )
        .unwrap(),
        Some(2),
        "decisions are selected independently by kind"
    );
}

#[path = "support/source09.rs"]
mod source09;
fn load(functions: &str) -> (bennett_vm::object::Object, bennett_vm::identity::Identities) {
    bennett_vm::load(&source09::source(functions.trim_end()), false).unwrap()
}
fn document(symbol: &str, point: &str, iteration: Json, instance: Json, call: Json) -> Json {
    let p = Json::obj(vec![
        ("symbol", Json::Str(symbol.into())),
        ("point", Json::Str(point.into())),
        ("iteration", iteration),
        ("instance", instance),
        ("call_index", call),
    ]);
    Json::obj(vec![
        ("bindings", j(r#"{"region":"root"}"#)),
        (
            "entries",
            Json::Arr(vec![Json::obj(vec![
                ("path", Json::Arr(vec![p])),
                ("step", wild()),
                ("phase", wild()),
                ("generated", wild()),
                ("kind", j(r#""start""#)),
                ("start", j(r#"{"time":0}"#)),
            ])]),
        ),
    ])
}
#[test]
fn k64_reserved_points_only_in_oracles_and_empty_source_coordinates() {
    let (obj, ids) = load(
        "oracle @root (x:wire[1] in) (t:wire[1]) (f:wire[4]) () ()\nregs { }\n^entry():\nlift.retval x\naction @a (x:wire[1] mut) () () () ()\n^entry():\nx x\n",
    );
    for p in ["$construct", "$step", "$commit", "$release"] {
        let doc = document(&ids.interface["root"], p, j("[]"), j("[]"), wild());
        sched::validate_entries(&doc, &obj, &ids).unwrap();
        for (it, ins, call) in [
            (j("[0]"), j("[]"), wild()),
            (j("[]"), j("[0]"), wild()),
            (j("[]"), j("[]"), j("0")),
        ] {
            assert!(
                sched::validate_entries(
                    &document(&ids.interface["root"], p, it, ins, call),
                    &obj,
                    &ids
                )
                .is_err()
            );
        }
        assert!(
            sched::validate_entries(
                &document(&ids.interface["a"], p, j("[]"), j("[]"), wild()),
                &obj,
                &ids
            )
            .is_err()
        );
    }
    assert!(
        sched::validate_entries(
            &document(
                &ids.interface["root"],
                "$dispatch",
                j("[]"),
                j("[]"),
                wild()
            ),
            &obj,
            &ids
        )
        .is_err()
    );
}
#[test]
fn source_iteration_and_semantic_call_domains_are_checked() {
    let (obj, ids) = load(
        "action @root (x:wire[1] mut) () () () ()\nregs { } bounds {2,3}\n^entry():\nqloop 2 {\nqloop 3 {\naction_call @a (x) () () ()\n}\n}\naction @a (x:wire[1] mut) () () () ()\n^entry():\nx x\n",
    );
    let hash = &ids.interface["root"];
    // Supply a distinct resolved closure identity at this API boundary, as
    // occurs when imports acquire closure pins. Source points still use interface.
    let mut resolved = ids.clone();
    let closure = "b".repeat(64);
    resolved
        .closure
        .insert("root".into(), Some(closure.clone()));
    assert_ne!(hash, &closure);
    assert!(
        sched::validate_entries(
            &document(&closure, "entry:2", j("[0,0]"), j("[]"), j("0")),
            &obj,
            &resolved
        )
        .is_err(),
        "program-point symbols are interface hashes, not closure hashes"
    );

    sched::validate_entries(
        &document(hash, "entry:2", j("[1,2]"), j("[]"), j("0")),
        &obj,
        &ids,
    )
    .unwrap();
    for (it, ins, call) in [
        (j("[2,0]"), j("[]"), j("0")),
        (j("[0,3]"), j("[]"), j("0")),
        (j("[1]"), j("[]"), j("0")),
        (j("[0,0]"), j("[0]"), j("0")),
        (j("[0,0]"), j("[]"), j("1")),
    ] {
        assert!(
            sched::validate_entries(&document(hash, "entry:2", it, ins, call), &obj, &ids).is_err()
        );
    }
    assert!(
        sched::validate_entries(
            &document(&ids.interface["a"], "entry:0", j("[]"), j("[]"), j("0")),
            &obj,
            &ids
        )
        .is_err(),
        "gate is not a semantic-call edge"
    );
}

#[test]
fn k49_fixed_rows_have_instances_but_lift_call_never_opens_an_edge() {
    let (obj, ids) = load(
        "oracle @root (x:wire[1] in) (t:wire[1]) (f:wire[4]) (db:region[0,8] in) (a:i1, b:i1)\nregs { r:reg[1] }\n^entry():\nr = lift.call @worker (x) () (a) ()\nr = lift.call @worker (x) () (a) ()\nr = lift.call @worker (x) () (b) ()\nlift.retval r\nlproc @worker (x:reg[1]) (r:reg[1]) () (k:i1 classical) ()\nregs { }\n^entry():\nr = lift.mov x\nlift.ret r",
    );
    let instances = bennett_vm::instances::for_oracle(&obj, "root").unwrap();
    assert_eq!(
        instances.iter().map(|v| v.key.clone()).collect::<Vec<_>>(),
        [vec![], vec![0], vec![1]]
    );
    for key in [j("[0]"), j("[1]")] {
        sched::validate_entries(
            &document(&ids.interface["worker"], "entry:0", j("[]"), key, wild()),
            &obj,
            &ids,
        )
        .unwrap();
    }
    for key in [j("[]"), j("[2]"), j("[0,0]")] {
        assert!(
            sched::validate_entries(
                &document(&ids.interface["worker"], "entry:0", j("[]"), key, wild()),
                &obj,
                &ids
            )
            .is_err()
        );
    }
    let mut doc = document(&ids.interface["root"], "entry:0", j("[]"), j("[]"), wild());
    if let Json::Obj(doc) = &mut doc
        && let Json::Arr(entries) = doc.get_mut("entries").unwrap()
        && let Json::Obj(e) = &mut entries[0]
        && let Json::Arr(path) = e.get_mut("path").unwrap()
    {
        path.push(j(&format!(
            r#"{{"symbol":"{}","point":"entry:0","iteration":[],"instance":[0]}}"#,
            ids.interface["worker"]
        )));
    }
    assert!(
        sched::validate_entries(&doc, &obj, &ids)
            .unwrap_err()
            .contains("invocation chain")
    );
}
#[test]
fn lifted_permutation_calls_open_semantic_edges_and_have_one_occurrence() {
    let (obj, ids) = load(
        "oracle @root (x:wire[1] in) (t:wire[1]) (f:wire[4]) () ()\nregs { r:reg[1] }\n^entry():\nr = lift.mov x\nlift.perm_call @flip (r) () () ()\nlift.retval r\nperm_action @flip (y:wire[1] mut) () () () ()\n^entry():\nx y",
    );
    let mut doc = document(&ids.interface["root"], "entry:1", j("[]"), j("[]"), j("0"));
    sched::validate_entries(&doc, &obj, &ids).unwrap();
    if let Json::Obj(d) = &mut doc
        && let Json::Arr(entries) = d.get_mut("entries").unwrap()
        && let Json::Obj(e) = &mut entries[0]
        && let Json::Arr(path) = e.get_mut("path").unwrap()
    {
        path.push(j(&format!(
            r#"{{"symbol":"{}","point":"entry:0","iteration":[],"instance":[]}}"#,
            ids.interface["flip"]
        )));
    }
    sched::validate_entries(&doc, &obj, &ids).unwrap();
    assert!(
        sched::validate_entries(
            &document(&ids.interface["root"], "entry:1", j("[]"), j("[]"), j("1")),
            &obj,
            &ids
        )
        .unwrap_err()
        .contains("call_index")
    );
}
