//! K64 annotates the authenticated stream; raw descriptors are not source admission.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::{
    json::{self, Json},
    ref_library::{Bindings, Catalog, Leaf},
};
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
fn fixture(steps: usize, addressed: bool, inverse: bool) -> Vec<Leaf> {
    let text = support::source(
        "oracle @o (x:wire[1] in) (t:wire[1]) (f:wire[4]) () ()\n^entry():\nlift.retval x",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let mut env = Bindings::new();
    env.wire("tgt", 1);
    env.wire("fr", 4);
    env.wire("x", 1);
    if addressed {
        env.address_oracle(&obj, "o").unwrap();
    }
    let names = ["pc", "next", "faulted", "code", "result"];
    let mut state = vec![];
    let mut declarations = vec![];
    for (name, width) in names
        .into_iter()
        .map(|n| (n.to_string(), if n == "code" { 4 } else { 1 }))
        .chain((0..steps).map(|i| (format!("frame{i}"), 1)))
    {
        state.push(Json::obj(vec![
            ("width", Json::u64(width)),
            ("initial", j("0")),
            ("I", j("\"\"")),
            ("storage", Json::Str(name.clone())),
        ]));
        declarations.push((name, width));
    }
    let frames = (0..steps)
        .map(|i| {
            Json::obj(vec![
                ("index", Json::u64(i as u64)),
                ("frame", Json::Str(format!("frame{i}"))),
            ])
        })
        .collect();
    let mut body = j(&format!(
        r#"[{{"invoke":"oracle.construct","args":[1,1,1,{steps},4,1,"pc","next","faulted","code","result","tgt","fr",{},{},[{{"parameters":[{{"name":"f","type":"wire","width":1}}],"body":[{{"gate":"x","wires":["x"]}}]}}],[{{"parameters":[{{"name":"f","type":"wire","width":1}},{{"name":"n","type":"wire","width":1}}],"body":[{{"gate":"x","wires":["x"]}}]}}]]}}]"#,
        Json::Arr(state).to_jcs(),
        Json::Arr(frames).to_jcs()
    ));
    if inverse {
        body = Json::Arr(vec![Json::obj(vec![
            ("inverse", Json::Bool(true)),
            ("body", body),
        ])]);
    }
    for (name, width) in declarations.into_iter().rev() {
        body = Json::Arr(vec![Json::obj(vec![
            ("declare", Json::Str(name)),
            ("width", Json::u64(width)),
            ("class", j(r#"{"class":"any"}"#)),
            ("state", j("\"clean\"")),
            ("body", body),
        ])]);
    }
    env.body("B", body).unwrap();
    Catalog::published()
        .unwrap()
        .expand("adj", &[j("\"B\"")], &env)
        .unwrap()
}
#[test]
fn k64_zero_budget_retains_nonstep_support_and_raw_stream() {
    for steps in 0..=2 {
        let addressed = fixture(steps, true, false);
        let raw = fixture(steps, false, false);
        assert_eq!(addressed.len(), raw.len());
        for (a, b) in addressed.iter().zip(raw) {
            let mut a = a.clone();
            a.address = None;
            assert_eq!(a, b, "addressing cannot change any emitted event");
        }
        for point in ["$construct", "$commit", "$release"] {
            let group = addressed
                .iter()
                .filter(|l| l.address.as_ref().unwrap().path[0].point == point)
                .collect::<Vec<_>>();
            assert!(!group.is_empty());
            for l in group {
                let a = l.address.as_ref().unwrap();
                assert_eq!(a.step, None);
                assert_eq!(a.phase, "");
                assert!(a.path[0].iteration.is_empty());
                assert!(a.path[0].instance.is_empty());
            }
        }
        assert_eq!(
            addressed
                .iter()
                .filter(|l| l.address.as_ref().unwrap().path[0].point == "$step")
                .count()
                > 0,
            steps > 0
        );
    }
}
#[test]
fn k64_step_support_excludes_rows_and_rows_next_share_ordinals() {
    let leaves = fixture(2, true, false);
    for t in 1..=2 {
        for phase in ["fwd", "rev"] {
            let group = leaves
                .iter()
                .filter(|l| {
                    let a = l.address.as_ref().unwrap();
                    a.step == Some(t) && a.phase == phase
                })
                .collect::<Vec<_>>();
            let mut ord = group
                .iter()
                .filter_map(|l| {
                    let a = l.address.as_ref().unwrap();
                    (a.path[0].point == "$step").then_some(a.generated)
                })
                .collect::<Vec<_>>();
            ord.sort();
            assert_eq!(ord, (0..ord.len()).collect::<Vec<_>>());
            let row = group
                .iter()
                .filter(|l| l.address.as_ref().unwrap().path[0].point == "entry:0")
                .collect::<Vec<_>>();
            assert_eq!(row.len(), 2);
            assert_eq!(
                row.iter()
                    .map(|l| l.address.as_ref().unwrap().generated)
                    .collect::<Vec<_>>(),
                if phase == "fwd" {
                    vec![0, 1]
                } else {
                    vec![1, 0]
                }
            );
            let zero = group
                .iter()
                .find(|l| {
                    let a = l.address.as_ref().unwrap();
                    a.path[0].point == "$step" && a.generated == 0
                })
                .unwrap();
            assert_eq!(
                zero.operation,
                if phase == "fwd" { "alloc" } else { "release" }
            );
        }
    }
    let reversed = fixture(2, true, true);
    for (f, r) in leaves.iter().rev().zip(reversed) {
        let mut a = f.address.clone().unwrap();
        a.kind = r.address.as_ref().unwrap().kind;
        assert_eq!(Some(a), r.address);
    }
}

fn paired_text(rows: &str) -> String {
    support::source(&format!(
        "oracle @root (x:wire in) (t:wire) (f:wire[4]) () (tk:token, other:token)\nregs {{ r:reg[1], s:reg[1], fr:reg[4], fs:reg[4], pc:reg[4], offset:reg[4] }}\n^entry():\n{rows}\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x"
    ))
}

fn paired_source(rows: &str) -> bennett_vm::object::Object {
    // Parsed only: these pairs test the address-binding stage, which must refuse
    // them on its own; `load_verdicts_match_the_binding_stage` holds load to the
    // same verdicts.
    bennett_vm::object::parse(&paired_text(rows)).unwrap()
}

/// The pairing tests below start from parsed objects, so this keeps what `load`
/// used to establish for them: every pair the binding stage admits loads, and every
/// pair it refuses is refused by load under §5.5, not by some other rule.
#[test]
fn load_verdicts_match_the_binding_stage() {
    for rows in [
        "lift.oracle_call @id tk (x) r fr () ()\nlift.oracle_call @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "lift.oracle_call @id tk (x) r fr () ()\nqbranch x ^left ^right\n^left():\nqjump ^join\n^right():\nqjump ^join\n^join():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "lift.oracle_call @id tk (x) r fr () ()\nqjump ^body\n^body():\nlift.oracle_call @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) s fs () ()\nqbranch x ^body ^done\n^done():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        // `tgt` and `fr'` are part of the match (ruling R1, as the reference): a pair
        // may cross another on different targets, and an unmatched call is retained.
        "lift.oracle_call @id tk (x) r fr () ()\nlift.oracle_call @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "lift.oracle_call @id tk (x) r fr () ()\nlift.oracle_call @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.oracle_uncall @id tk (x) s fs () ()\nlift.retval r",
    ] {
        bennett_vm::load(&paired_text(rows), false).unwrap();
    }
    for rows in [
        "lift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "lift.oracle_call @id tk (x) r fr () ()\nlift.oracle_uncall @id other (x) r fr () ()\nlift.retval r",
        "s = lift.mov x\nlift.oracle_call @id tk (s) r fr () ()\ns = lift.mov x\nlift.oracle_uncall @id tk (s) r fr () ()\nlift.retval r",
        "qbranch x ^left ^right\n^left():\nlift.oracle_call @id tk (x) r fr () ()\nqjump ^join\n^right():\nqjump ^join\n^join():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "qbranch x ^left ^right\n^left():\nlift.oracle_call @id tk (x) r fr () ()\nqjump ^join\n^right():\nlift.oracle_call @id tk (x) r fr () ()\nqjump ^join\n^join():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "lift.oracle_call @id tk (x) r fr () ()\nqjump ^body\n^body():\nlift.oracle_uncall @id tk (x) r fr () ()\nqbranch x ^body ^done\n^done():\nlift.retval r",
    ] {
        let error = bennett_vm::load(&paired_text(rows), false).err().unwrap();
        assert!(error.contains("§5.5"), "{rows}: {error}");
    }
}

#[test]
fn lifted_uncall_source_pairing_is_lifo_and_keeps_exact_target_locations() {
    let object = paired_source(
        "lift.oracle_call @id tk (x) r fr () ()\nlift.oracle_call @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
    );
    let mut env = Bindings::new();
    env.address_oracle(&object, "root").unwrap();
    let pairs = env.oracle_cleanup_pairs().unwrap();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].computing_point.point, "entry:1");
    assert_eq!(pairs[0].cleanup_point.point, "entry:2");
    assert_eq!(pairs[1].computing_point.point, "entry:0");
    assert_eq!(pairs[1].cleanup_point.point, "entry:3");
    for pair in pairs {
        assert_eq!(pair.computing_point.symbol, pair.cleanup_point.symbol);
        assert!(pair.cleanup_point.instance.is_empty());
        assert!(pair.cleanup_point.call_index.is_none());
    }
    // `tgt` and `fr'` are part of the match at 0.8 and 0.9, as in the reference
    // (ruling R1, 2026-11-02): an uncall on r pairs with the call on r even past a
    // later call on s, which is retained; two pairs on different targets may cross.
    let skipped = paired_source(
        "lift.oracle_call @id tk (x) r fr () ()\nlift.oracle_call @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
    );
    env.address_oracle(&skipped, "root").unwrap();
    let pairs = env.oracle_cleanup_pairs().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].computing_point.point, "entry:0");
    assert_eq!(pairs[0].cleanup_point.point, "entry:2");
    let crossing = paired_source(
        "lift.oracle_call @id tk (x) r fr () ()\nlift.oracle_call @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.oracle_uncall @id tk (x) s fs () ()\nlift.retval r",
    );
    env.address_oracle(&crossing, "root").unwrap();
    let pairs = env.oracle_cleanup_pairs().unwrap();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].computing_point.point, "entry:0");
    assert_eq!(pairs[0].cleanup_point.point, "entry:2");
    assert_eq!(pairs[1].computing_point.point, "entry:1");
    assert_eq!(pairs[1].cleanup_point.point, "entry:3");
}

#[test]
fn lifted_uncall_source_pairing_rejects_missing_or_changed_invocation_inputs() {
    for rows in [
        "lift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "lift.oracle_call @id tk (x) r fr () ()\nlift.oracle_uncall @id other (x) r fr () ()\nlift.retval r",
        "s = lift.mov x\nlift.oracle_call @id tk (s) r fr () ()\ns = lift.mov x\nlift.oracle_uncall @id tk (s) r fr () ()\nlift.retval r",
    ] {
        let mut env = Bindings::new();
        env.address_oracle(&paired_source(rows), "root").unwrap();
        let error = env.oracle_cleanup_pairs().unwrap_err();
        assert!(error.contains("§5.5"), "{error}");
    }
}

#[test]
fn lifted_uncall_source_pairing_joins_cfg_paths_without_inventing_a_call() {
    let prefix = "lift.oracle_call @id tk (x) r fr () ()\nqbranch x ^left ^right\n^left():\nqjump ^join\n^right():\nqjump ^join\n^join():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r";
    let mut env = Bindings::new();
    env.address_oracle(&paired_source(prefix), "root").unwrap();
    let pairs = env.oracle_cleanup_pairs().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].computing_point.point, "entry:0");
    assert_eq!(pairs[0].cleanup_point.point, "join:0");
    for rows in [
        "qbranch x ^left ^right\n^left():\nlift.oracle_call @id tk (x) r fr () ()\nqjump ^join\n^right():\nqjump ^join\n^join():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "qbranch x ^left ^right\n^left():\nlift.oracle_call @id tk (x) r fr () ()\nqjump ^join\n^right():\nlift.oracle_call @id tk (x) r fr () ()\nqjump ^join\n^join():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
    ] {
        env.address_oracle(&paired_source(rows), "root").unwrap();
        assert!(env.oracle_cleanup_pairs().unwrap_err().contains("§5.5"));
    }
}

#[test]
fn lifted_uncall_source_pairing_handles_balanced_cycles_and_detects_underflow_cycles() {
    let rows = "lift.oracle_call @id tk (x) r fr () ()\nqjump ^body\n^body():\nlift.oracle_call @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) s fs () ()\nqbranch x ^body ^done\n^done():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r";
    let mut env = Bindings::new();
    env.address_oracle(&paired_source(rows), "root").unwrap();
    let pairs = env.oracle_cleanup_pairs().unwrap();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].computing_point.point, "body:0");
    assert_eq!(pairs[0].cleanup_point.point, "body:1");
    assert_eq!(pairs[1].computing_point.point, "entry:0");
    assert_eq!(pairs[1].cleanup_point.point, "done:0");
    let rows = "lift.oracle_call @id tk (x) r fr () ()\nqjump ^body\n^body():\nlift.oracle_uncall @id tk (x) r fr () ()\nqbranch x ^body ^done\n^done():\nlift.retval r";
    env.address_oracle(&paired_source(rows), "root").unwrap();
    assert!(
        env.oracle_cleanup_pairs()
            .unwrap_err()
            .contains("no matching call")
    );
}

#[test]
fn lifted_uncall_source_pairing_preserves_lproc_instance_ownership() {
    let text = support::source(
        "oracle @root (x:wire in) (t:wire) (f:wire[4]) (db:region[0,8] in) (tk:token, a:i1, b:i1)\nregs { r:reg[1] }\n^entry():\nr = lift.call @worker (x) (tk) (a) ()\nr = lift.call @worker (x) (tk) (b) ()\nlift.retval r\nlproc @worker (u:reg[1]) (r:reg[1]) (k:token token) (v:i1 classical) ()\nregs { fr:reg[4] }\n^entry():\nlift.oracle_call @id k (u) r fr () (v)\nlift.oracle_uncall @id k (u) r fr () (v)\nlift.ret r\noracle @id (x:wire in) (t:wire) (f:wire[4]) () (a:i1)\n^entry():\nlift.retval x",
    );
    let (object, ids) = bennett_vm::load(&text, false).unwrap();
    let mut env = Bindings::new();
    env.address_oracle(&object, "root").unwrap();
    let pairs = env.oracle_cleanup_pairs().unwrap();
    assert_eq!(pairs.len(), 2);
    for (pair, instance) in pairs.iter().zip([vec![0, 1], vec![0, 2]]) {
        assert_eq!(pair.computing_point.symbol, ids.interface["worker"]);
        assert_eq!(pair.cleanup_point.symbol, ids.interface["worker"]);
        assert_eq!(pair.computing_point.instance, instance);
        assert_eq!(pair.cleanup_point.instance, instance);
        assert_eq!(pair.computing_point.point, "entry:0");
        assert_eq!(pair.cleanup_point.point, "entry:1");
    }
    // Relinking another object must not retarget a previously captured proof.
    env.link(&paired_source("lift.retval x")).unwrap();
    assert_eq!(env.oracle_cleanup_pairs().unwrap(), pairs);
}

#[test]
fn lifted_uncall_source_pairing_allows_intervening_memory_writes() {
    // §5.5 permits this source pair; it does not promise to clear fr after
    // changing the cells. §6 window/realisation checks remain separate.
    let text = support::source(
        "oracle @root (x:wire in) (t:wire) (f:wire[4]) (db:region[0,8] in) (tk:token)\nregs { r:reg[1], fr:reg[4], p:reg[32], v:reg[8] }\n^entry():\np = lift.const.i32 16\nv = lift.const.i8 1\nlift.oracle_call @id tk (x) r fr (db) ()\nlift.store.i8 p v\nlift.oracle_uncall @id tk (x) r fr (db) ()\nlift.retval r\noracle @id (x:wire in) (t:wire) (f:wire[4]) (db:region[0,8] in) ()\n^entry():\nlift.retval x",
    );
    let (object, _) = bennett_vm::load(&text, false).unwrap();
    let mut env = Bindings::new();
    env.address_oracle(&object, "root").unwrap();
    let pairs = env.oracle_cleanup_pairs().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].computing_point.point, "entry:2");
    assert_eq!(pairs[0].cleanup_point.point, "entry:4");
}

#[test]
fn lifted_uncall_indirect_proof_obligation_does_not_remove_raw_provenance() {
    let object = paired_source(
        "pc = lift.label ^entry\nlift.oracle_call @id tk (x) s fs () ()\nlift.oracle_uncall @id tk (x) s fs () ()\nqjumpr pc",
    );
    let mut env = Bindings::new();
    env.address_oracle(&object, "root").unwrap();
    let pairs = env.oracle_cleanup_pairs().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].computing_point.point, "entry:1");
    assert_eq!(pairs[0].cleanup_point.point, "entry:2");
    // This proof authenticates source ownership only. The raw catalog stream
    // still requires source/descriptor consistency (the K64 tests above).
    assert!(env.body("forged", j(r#"[{"$cleanup":{"target":"s","point":"entry:1","computing":[],"uncomputing":[],"perm":true},"body":[]}]"#)).is_err());
    assert!(
        env.body(
            "forged",
            j(r#"[{"$source":{"point":"entry:2"},"body":[]}]"#)
        )
        .is_err()
    );
}

#[test]
fn indirect_pairs_use_computed_interior_targets_and_balanced_value_paths() {
    for rows in [
        "pc = lift.label ^done\nlift.oracle_call @id tk (x) r fr () ()\nqjumpr pc\n^done():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "pc = lift.label ^done\noffset = lift.const.i4 1\npc = lift.add pc offset\nlift.oracle_call @id tk (x) r fr () ()\nqjumpr pc\n^done():\nr = lift.mov x\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "lift.oracle_call @id tk (x) r fr () ()\nqbranch x ^left ^right\n^left():\npc = lift.label ^join\nqjumpr pc\n^right():\npc = lift.label ^join\nqjumpr pc\n^join():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "pc = lift.label ^body\nqjumpr pc\n^body():\nlift.oracle_call @id tk (x) r fr () ()\nlift.oracle_uncall @id tk (x) r fr () ()\nqjumpr pc",
    ] {
        // The second witness jumps to the interior cleanup row by arithmetic.
        let mut env = Bindings::new();
        env.address_oracle(&paired_source(rows), "root").unwrap();
        assert_eq!(env.oracle_cleanup_pairs().unwrap().len(), 1, "{rows}");
    }
}

#[test]
fn indirect_pairs_reject_underflow_ambiguity_successors_and_input_writes() {
    for rows in [
        "pc = lift.label ^body\nlift.oracle_call @id tk (x) r fr () ()\nqjumpr pc\n^body():\nlift.oracle_uncall @id tk (x) r fr () ()\nqjumpr pc",
        "pc = lift.label ^join\nqbranch x ^left ^right\n^left():\nlift.oracle_call @id tk (x) r fr () ()\nqjumpr pc\n^right():\nlift.oracle_call @id tk (x) r fr () ()\nqjumpr pc\n^join():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r",
        "pc = lift.label ^done\nlift.oracle_call @id tk (x) r fr () ()\nqjumpr pc\n^done():\nlift.oracle_uncall @id tk (x) s fs () ()\nlift.retval r",
        "s = lift.mov x\npc = lift.label ^done\nlift.oracle_call @id tk (s) r fr () ()\nqjumpr pc\n^done():\ns = lift.mov x\nlift.oracle_uncall @id tk (s) r fr () ()\nlift.retval r",
    ] {
        let mut env = Bindings::new();
        env.address_oracle(&paired_source(rows), "root").unwrap();
        assert!(env.oracle_cleanup_pairs().unwrap_err().contains("§5.5"));
    }
}

#[test]
fn recursive_instance_capture_invalidates_an_indirect_pairs_readable_version() {
    let text = support::source(
        "oracle @root (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs { r:reg[1] }\n^entry():\nr = lift.call @work (x) (tk) () ()\nlift.retval r\nlproc @work (x:reg[1]) (r:reg[1]) (tk:token) () ()\nregs { fr:reg[4], pc:reg[5] }\n^entry():\npc = lift.label ^done\nlift.oracle_call @id tk (x) r fr () ()\nr = lift.call @work (r) (tk) () ()\npc = lift.label ^done\nqjumpr pc\n^done():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.ret r\noracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x",
    );
    let text = support::sign(&format!(
        "{}\nsection layout\n{}\n",
        text,
        j(r#"{"ptr_width":32,"H":6,"A_tab":0,"S_cells":4,"null_size":16,"heap_base":32,"heap_limit_max":0,"globals":{},"interface_regions":{}}"#).to_jcs()
    ));
    let (object, _) = bennett_vm::load(&text, false).unwrap();
    let mut env = Bindings::new();
    env.address_oracle(&object, "root").unwrap();
    assert!(
        env.oracle_cleanup_pairs()
            .unwrap_err()
            .contains("intervening row writes")
    );
}
