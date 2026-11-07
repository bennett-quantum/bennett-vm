//! Dynamic region-call bindings must fault before touching wires or views.
use crate::source09 as support;
use bennett_vm::json::{self, Json};
use std::{path::Path, process::Command};

fn outcome(binary: &Path, path: &Path, engine: &str) -> Json {
    outcome_with_args(binary, path, engine, &[])
}
fn outcome_with_args(binary: &Path, path: &Path, engine: &str, extra: &[&str]) -> Json {
    let out = Command::new(binary)
        .args(["run-cc", path.to_str().unwrap(), "main", "--draws", "0.1"])
        .args(extra)
        .env("BENNETT_VM_ENGINE", engine)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    json::parse(std::str::from_utf8(&out.stdout).unwrap().trim()).unwrap()
}

fn alias(engine: &str) {
    for tier in ["cc", "r"] {
        for call in [
            "action_call @cxact (b) (a) () ()",
            "ctl a action_call @toggle (b) () () ()",
        ] {
            let text = support::source(&format!(
                "action @toggle (t:wire[1] mut) () () () ()\n^entry():\nx t\naction @cxact (t:wire[1] mut) (c:wire[1] in) () () ()\n^entry():\ncx c t\nfn {tier} @g (a:qreg[1], b:qreg[1]) () () () ()\n^entry():\n{call}\nret\nfn cc @main () () () () ()\n^entry():\nq = qreg.new any 1\naction_call @toggle (q) () () ()\ncall @g q q\nv = measure q\nret"
            ));
            let path = std::env::temp_dir().join(format!(
                "alias09-{engine}-{tier}-{}.bisa",
                std::process::id()
            ));
            std::fs::write(&path, text).unwrap();
            let reference = crate::pins::isa().join("reference-interpreter/target/debug/bennett-isa-ref");
            for bin in [
                &reference,
                &Path::new(env!("CARGO_BIN_EXE_bennett-vm")).to_path_buf(),
            ] {
                let result = outcome(bin, &path, engine);
                assert_eq!(
                    result.get("status").and_then(Json::as_str),
                    Some("FAILED"),
                    "{engine}/{tier}/{call}: {result:?}"
                );
                assert_eq!(result.get("fault").and_then(Json::as_u64), Some(4));
                assert!(
                    result
                        .get("measurements")
                        .unwrap()
                        .as_arr()
                        .unwrap()
                        .is_empty()
                );
                assert_eq!(result.get("faults").unwrap().as_arr().unwrap().len(), 1);
            }
            std::fs::remove_file(path).unwrap();
        }
    }
}

#[test]
#[ignore = "published oracle or C ABI suite; mandatory in the gate"]
fn dynamic_region_alias_traps_interpreter() {
    alias("interpreter");
}
#[test]
#[ignore = "published oracle or C ABI suite; mandatory in the gate"]
fn dynamic_region_alias_traps_jit() {
    alias("jit");
}

#[test]
#[ignore = "published oracle or C ABI suite; mandatory in the gate"]
fn oracle_slots_trap_all_dynamic_alias_pairs_on_both_engines() {
    for engine in ["interpreter", "jit"] {
        for actuals in [
            "q q r s", "q r q s", "q r r s", "q r s q", "q r s r", "q r s s", "q q q q",
        ] {
            let text = support::source(&format!(
                "oracle @echo (v:wire[16] in) (t:wire[16]) (fr:wire[16]) (m:region[1,8] in) ()\n^entry():\nlift.retval v\nfn cc @g (a:qreg[16], b:qreg[16], c:qreg[16], d:qreg[16], tk:token) () () () ()\n^entry():\noracle_call @echo tk (a) b c (d) ()\nret\nfn cc @main (tk:token) () () () ()\n^entry():\nq = qreg.new any 16\nr = qreg.new any 16\ns = qreg.new any 16\ncall @g {actuals} tk\nv = measure q\nret"
            ));
            let path = std::env::temp_dir().join(format!(
                "oracle-alias09-{engine}-{}.bisa",
                std::process::id()
            ));
            std::fs::write(&path, text).unwrap();
            let result = outcome_with_args(
                Path::new(env!("CARGO_BIN_EXE_bennett-vm")),
                &path,
                engine,
                &["--arg", "token:3", "--token-aliases", "[0]"],
            );
            assert_eq!(
                result.get("fault").and_then(Json::as_u64),
                Some(4),
                "{engine}/{actuals}: {result:?}"
            );
            assert!(
                result
                    .get("measurements")
                    .unwrap()
                    .as_arr()
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(result.get("faults").unwrap().as_arr().unwrap().len(), 1);
            std::fs::remove_file(path).unwrap();
        }
    }
}

#[test]
#[ignore = "published oracle or C ABI suite; mandatory in the gate"]
fn action_entry_rejects_invalid_bindings_before_any_gate() {
    use bennett_vm::action::{QuantumState, RegionWires, execute_action_inner};
    let text = support::source(
        "action @a (t:wire[1] mut) (p:wire[1] in) (m:region[1,1] in) () ()\n^entry():\nx t",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let mem = RegionWires {
        cells: vec![vec![2], vec![3]],
        a: 1,
        w: 1,
    };
    for (muts, inputs, regions, controls) in [
        (vec![vec![0]], vec![vec![0]], vec![mem.clone()], vec![]),
        (vec![vec![0]], vec![vec![1]], vec![mem.clone()], vec![0]),
        (vec![vec![0, 1]], vec![vec![1]], vec![mem.clone()], vec![]),
        (vec![vec![0]], vec![vec![]], vec![mem.clone()], vec![]),
        (
            vec![vec![0]],
            vec![vec![1]],
            vec![RegionWires {
                cells: vec![vec![0], vec![3]],
                ..mem.clone()
            }],
            vec![],
        ),
        (
            vec![vec![0]],
            vec![vec![1]],
            vec![RegionWires {
                cells: vec![vec![2], vec![2]],
                ..mem.clone()
            }],
            vec![],
        ),
        (
            vec![vec![0]],
            vec![vec![1]],
            vec![RegionWires {
                cells: vec![vec![2, 3]],
                ..mem.clone()
            }],
            vec![],
        ),
        (vec![vec![0]], vec![vec![1]], vec![], vec![]),
    ] {
        let mut state = QuantumState::new();
        state.alloc_wires(4);
        let before = state.clone();
        let result = execute_action_inner(
            &obj,
            &mut state,
            "a",
            &muts,
            &inputs,
            &regions,
            &[],
            false,
            &controls,
        );
        assert!(
            result.is_err(),
            "accepted {muts:?}/{inputs:?}/{regions:?}/{controls:?}"
        );
        assert!(
            state.exact_equals(&before),
            "rejected binding changed the quantum state"
        );
    }
    // Effective controls may also be read-only inputs (including memory).
    let mut state = QuantumState::new();
    state.alloc_wires(4);
    state.prepare(&[1, 2], &bennett_vm::bigint::BigUint::from_u64(3));
    let before = state.clone();
    execute_action_inner(
        &obj,
        &mut state,
        "a",
        &[vec![0]],
        &[vec![1]],
        std::slice::from_ref(&mem),
        &[],
        false,
        &[1, 2],
    )
    .unwrap();
    assert!(!state.exact_equals(&before));
    execute_action_inner(
        &obj,
        &mut state,
        "a",
        &[vec![0]],
        &[vec![1]],
        &[mem],
        &[],
        true,
        &[1, 2],
    )
    .unwrap();
    assert!(state.exact_equals(&before));

    let text = support::source(
        "action @a (t:wire[1] mut) (p:wire[1] in) (m:region[1,1] mut) () ()\n^entry():\nx t",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let mem = RegionWires {
        cells: vec![vec![2], vec![3]],
        a: 1,
        w: 1,
    };
    assert!(
        execute_action_inner(
            &obj,
            &mut state,
            "a",
            &[vec![0]],
            &[vec![1]],
            &[mem],
            &[],
            false,
            &[2]
        )
        .is_err()
    );
    assert!(state.exact_equals(&before));
}
