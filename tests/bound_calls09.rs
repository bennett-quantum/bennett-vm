//! K63 checks call occurrences after the row's library has been bound.
use crate::source09 as support;
use bennett_vm::{
    json::{self, Json},
    ref_library::{Catalog, CleanupEntry, CleanupRealisation},
    sched::{self, Selector},
};

fn start_document(path: Json, region: &str) -> Json {
    Json::obj(vec![
        (
            "bindings",
            Json::obj(vec![("region", Json::Str(region.into()))]),
        ),
        (
            "entries",
            Json::Arr(vec![Json::obj(vec![
                ("path", path),
                ("step", Json::Str("*".into())),
                ("phase", Json::Str("*".into())),
                ("generated", Json::Str("*".into())),
                ("kind", Json::Str("start".into())),
                ("start", Json::obj(vec![("time", Json::u64(0))])),
            ])]),
        ),
    ])
}
fn point(symbol: &str, row: &str, call: Option<u64>) -> Json {
    let mut fields = vec![
        ("symbol", Json::Str(symbol.into())),
        ("point", Json::Str(row.into())),
        ("iteration", Json::Arr(vec![])),
        ("instance", Json::Arr(vec![])),
    ];
    if let Some(call) = call {
        fields.push(("call_index", Json::u64(call)));
    }
    Json::obj(fields)
}

fn ownership_object() -> (bennett_vm::object::Object, bennett_vm::identity::Identities) {
    let text = support::source(
        "action @root (x:wire[1] mut) () () () ()\n^entry():\naction_call @left (x) () () ()\naction @left (w:wire[1] mut) () () () ()\nregs { } bounds {0}\n^entry():\nqloop 0 {\nx w\n}\naction @right (w:wire[1] mut) () () () ()\n^entry():\nx w",
    );
    bennett_vm::load(&text, false).unwrap()
}

#[test]
fn root_source_ignores_mutated_ast_call_targets() {
    let (mut obj, ids) = ownership_object();
    let root = obj.functions.iter_mut().find(|f| f.name == "root").unwrap();
    root.blocks[0].instrs[0].operands[0] = "@right".into();
    let expansion = Catalog::published()
        .unwrap()
        .lower_action_with_cleanup(&obj, "root", &Json::Arr(vec![]), false, 0, &[])
        .unwrap();
    let domain = expansion
        .calls
        .domains()
        .iter()
        .find(|d| d.path.len() == 1 && d.path[0].symbol == ids.interface["root"])
        .unwrap();
    assert_eq!(
        &domain.targets[0].closure,
        ids.closure["left"].as_ref().unwrap()
    );
    assert!(expansion.leaves.is_empty());
}

#[test]
fn root_source_rejects_forged_rows_and_identities() {
    let (mut obj, ids) = ownership_object();
    let expansion = Catalog::published()
        .unwrap()
        .lower_action_with_cleanup(&obj, "root", &Json::Arr(vec![]), false, 0, &[])
        .unwrap();
    let root = obj.functions.iter_mut().find(|f| f.name == "root").unwrap();
    let mut row = root.blocks[0].instrs[0].clone();
    row.line += 1;
    root.blocks[0].instrs.push(row);
    let doc = start_document(
        Json::Arr(vec![
            point(&ids.interface["root"], "entry:1", Some(0)),
            point(&ids.interface["left"], "entry:0", None),
        ]),
        "root",
    );
    assert!(sched::validate_bound_entries(&doc, &obj, &ids, &expansion.calls).is_err());
    let mut wrong_ids = ids.clone();
    wrong_ids.interface.insert("root".into(), "f".repeat(64));
    let doc = start_document(Json::Arr(vec![]), "root");
    sched::validate_bound_entries(&doc, &obj, &ids, &expansion.calls).unwrap();
    assert!(sched::validate_bound_entries(&doc, &obj, &wrong_ids, &expansion.calls).is_err());
}

#[test]
fn bound_paths_reject_wrong_callees_indices_objects_and_unbound_domains() {
    let text = support::source(
        "action @root (x:wire[1] mut, z:wire[2] mut) () () () ()\n^entry():\naction_call @left (x) () () ()\naction_call @right (z) () () ()\naction @left (w:wire[1] mut) () () () ()\nregs { } bounds {0}\n^entry():\nqloop 0 {\nx w\n}\naction @right (w:wire[2] mut) () () () ()\n^entry():\nx w[0]",
    );
    let (obj, ids) = bennett_vm::load(&text, false).unwrap();
    let expansion = Catalog::published()
        .unwrap()
        .lower_action_with_cleanup(&obj, "root", &Json::Arr(vec![]), false, 0, &[])
        .unwrap();
    for (owner, index, callee, expected) in [
        ("entry:0", 0, "left", true),
        ("entry:1", 0, "right", true),
        ("entry:0", 0, "right", false),
        ("entry:1", 0, "left", false),
        ("entry:0", 1, "left", false),
        ("entry:1", 1, "right", false),
    ] {
        let doc = start_document(
            Json::Arr(vec![
                point(&ids.interface["root"], owner, Some(index)),
                point(&ids.interface[callee], "entry:0", None),
            ]),
            "root",
        );
        assert_eq!(
            sched::validate_bound_entries(&doc, &obj, &ids, &expansion.calls).is_ok(),
            expected,
            "{owner}/{index}/{callee}"
        );
    }
    let doc = start_document(Json::Arr(vec![]), "root");
    assert!(sched::validate_bound_entries(&doc, &obj, &ids, &Default::default()).is_err());
    let detached = start_document(
        Json::Arr(vec![point(&ids.interface["left"], "entry:0", None)]),
        "root",
    );
    assert!(
        sched::validate_bound_entries(&detached, &obj, &ids, &expansion.calls).is_err(),
        "callee source point needs its invocation prefix"
    );
    let changed = support::source(
        text.split_once("section functions\n")
            .unwrap()
            .1
            .replace("x w[0]", "x w[1]")
            .trim_end(),
    );
    let (changed, changed_ids) = bennett_vm::load(&changed, false).unwrap();
    assert!(sched::validate_bound_entries(&doc, &changed, &changed_ids, &expansion.calls).is_err());
}

#[test]
fn recomputed_call_domains_validate_the_actual_cleanup_owner_and_callee() {
    let text = support::source(
        "action @root (x:wire mut, z:wire mut) () () () ()\n^entry():\nd = qalloc any 1\naction_call @copy (d) (x) () ()\naction_call @other (d) (z) () ()\nadj action_call @other (d) (z) () ()\nadj action_call @copy (d) (x) () ()\nqfree d\nperm_action @copy (t:wire mut) (s:wire in) () () ()\n^entry():\ncx s t\nperm_action @other (t:wire mut) (s:wire in) () () ()\n^entry():\ncx s t\nx t\nx t",
    );
    let (obj, ids) = bennett_vm::load(&text, false).unwrap();
    for adjoint in [false, true] {
        let point = if adjoint { "entry:0" } else { "entry:5" };
        let selector = json::parse(&format!(r#"{{"path":[{{"symbol":"{}","point":"{point}","iteration":"*"}}],"step":"*","phase":"*","generated":0}}"#, ids.interface["root"])).unwrap();
        let expansion = Catalog::published()
            .unwrap()
            .lower_action_with_cleanup(
                &obj,
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
        let domain = expansion
            .calls
            .domains()
            .iter()
            .find(|d| {
                d.path.len() == 1
                    && d.path[0].symbol == ids.interface["root"]
                    && d.path[0].point == point
            })
            .unwrap();
        assert_eq!(
            domain
                .targets
                .iter()
                .map(|t| t.interface.as_str())
                .collect::<Vec<_>>(),
            ["copy", "other", "other", "copy"].map(|n| ids.interface[n].as_str())
        );
        assert_eq!(
            domain.targets.iter().map(|t| t.adjoint).collect::<Vec<_>>(),
            [false, false, true, true]
        );
        for (index, expected) in [(0, true), (1, true), (2, true), (3, true), (4, false)] {
            let callee = if index == 1 || index == 2 {
                "other"
            } else {
                "copy"
            };
            let doc = json::parse(&format!(r#"{{"bindings":{{"region":"root"}},"entries":[{{"path":[{{"symbol":"{}","point":"{point}","iteration":[],"call_index":{index}}},{{"symbol":"{}","point":"entry:0","iteration":[]}}],"step":"*","phase":"*","generated":"*","kind":"start","start":{{"time":0}}}}]}}"#, ids.interface["root"], ids.interface[callee])).unwrap();
            assert_eq!(
                sched::validate_bound_entries(&doc, &obj, &ids, &expansion.calls).is_ok(),
                expected,
                "adjoint={adjoint} index={index}"
            );
        }
        let wrong_root = json::parse(r#"{"bindings":{"region":"copy"},"entries":[]}"#).unwrap();
        assert!(
            sched::validate_bound_entries(&wrong_root, &obj, &ids, &expansion.calls).is_err(),
            "call domains from root must not bind copy"
        );
    }
}

#[test]
fn nested_oracle_fixed_instances_belong_to_the_entered_invocation_without_lift_call_edges() {
    let text = support::source(
        "action @root (x:wire[1] mut, t:wire[1] mut, f:wire[4] mut) () (db:region[0,8] in) (tk:token, a:i1, b:i1) ()\n^entry():\noracle_call @outer tk (x) t f (db) (a, b)\noracle @outer (x:wire[1] in) (t:wire[1]) (f:wire[4]) (db:region[0,8] in) (a:i1, b:i1)\nregs { r:reg[1] }\n^entry():\nr = lift.call @worker (x) () (a) ()\nr = lift.call @worker (x) () (b) ()\nlift.retval r\nlproc @worker (x:reg[1]) (r:reg[1]) () (k:i1 classical) ()\nregs { }\n^entry():\nr = lift.mov x\nlift.perm_call @flip (r) () () ()\nlift.ret r\nperm_action @flip (y:wire[1] mut) () () () ()\n^entry():\nx y",
    );
    let (obj, ids) = bennett_vm::load(&text, false).unwrap();
    let params = json::parse(r#"[["1","0","0","0"],"0","1"]"#).unwrap();
    let expansion = Catalog::published()
        .unwrap()
        .lower_action_with_token_aliases(&obj, "root", &params, false, 0, &[], Some(&json::parse("[0]").unwrap()))
        .unwrap();
    for (instance, expected) in [
        ("[0]", true),
        ("[1]", true),
        ("[]", false),
        ("[2]", false),
        ("[0,0]", false),
    ] {
        let doc = json::parse(&format!(r#"{{"bindings":{{"region":"root"}},"entries":[{{"path":[{{"symbol":"{}","point":"entry:0","iteration":[],"instance":[],"call_index":0}},{{"symbol":"{}","point":"entry:0","iteration":[],"instance":{instance}}}],"step":"*","phase":"*","generated":"*","kind":"start","start":{{"time":0}}}}]}}"#,ids.interface["root"],ids.interface["worker"])).unwrap();
        assert_eq!(
            sched::validate_bound_entries(&doc, &obj, &ids, &expansion.calls).is_ok(),
            expected,
            "instance {instance}"
        );
    }
    let doc = json::parse(&format!(r#"{{"bindings":{{"region":"root"}},"entries":[{{"path":[{{"symbol":"{}","point":"entry:0","iteration":[],"call_index":0}},{{"symbol":"{}","point":"entry:0","iteration":[],"call_index":0}}],"step":"*","phase":"*","generated":"*","kind":"start","start":{{"time":0}}}}]}}"#,ids.interface["root"],ids.interface["outer"])).unwrap();
    assert!(
        sched::validate_bound_entries(&doc, &obj, &ids, &expansion.calls).is_err(),
        "lift.call must not become an edge"
    );
    for (instance, index, expected) in [
        (0, 0, true),
        (1, 0, true),
        (0, 1, false),
        (1, 1, false),
        (2, 0, false),
    ] {
        let mut worker = point(&ids.interface["worker"], "entry:1", Some(index));
        if let Json::Obj(fields) = &mut worker {
            fields.insert("instance".into(), Json::Arr(vec![Json::u64(instance)]));
        }
        let doc = start_document(
            Json::Arr(vec![
                point(&ids.interface["root"], "entry:0", Some(0)),
                worker,
                point(&ids.interface["flip"], "entry:0", None),
            ]),
            "root",
        );
        assert_eq!(
            sched::validate_bound_entries(&doc, &obj, &ids, &expansion.calls).is_ok(),
            expected,
            "worker instance={instance} call_index={index}"
        );
    }
}
