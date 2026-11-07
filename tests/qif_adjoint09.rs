//! The adjoint reverses unconditional lifetimes across the whole qif.
use crate::source09 as support;
use bennett_vm::{
    action::{QuantumState, execute_action_inner},
    bigint::BigUint,
    exact::Exact,
    json::{self, Json},
};
use std::{path::Path, process::Command};

const ACTIONS: &str = "action @cross (t:wire mut) (p:wire in) () () ()\n^entry():\nqif p {\nd = qalloc any 1\n} {\nqfree d\n}\naction @arms (t:wire mut) (p:wire in) () () ()\n^entry():\nqif p {\nd = qalloc any 2\nh t\ns t\nx d[0]\nx d[0]\n} {\nt t\nh t\nqfree d\n}\naction @nested (t:wire mut) (p:wire in, q:wire in) () () ()\n^entry():\nqif p {\nd = qalloc any 1\nqif q {\ne = qalloc any 1\ns t\n} {\nh t\nqfree e\n}\n} {\nqfree d\nt t\n}";

#[test]
fn complete_qif_adjoint_restores_basis_and_coherent_states() {
    let actions = format!(
        "{ACTIONS}\naction @wrapped (t:wire mut) (p:wire in) () () ()\n^entry():\nadj action_call @arms (t) (p) () ()"
    );
    let (object, _) = bennett_vm::load(&support::source(&actions), false).unwrap();
    for name in ["cross", "arms", "nested", "wrapped"] {
        for basis in 0..=16 {
            let mut state = QuantumState::new();
            state.alloc_wires(4);
            if basis == 16 {
                let h = Exact::sqrt_half();
                for wire in 0..4 {
                    state.apply_exact(wire, &[], [[h.clone(), h.clone()], [h.clone(), h.neg()]]);
                }
            } else {
                state.prepare(&[0, 1, 2, 3], &BigUint::from_u64(basis));
            }
            let inputs = if name == "nested" {
                vec![vec![1], vec![2]]
            } else {
                vec![vec![1]]
            };
            for inherited in [vec![], vec![3]] {
                for adjoint_first in [false, true] {
                    let mut actual = state.clone();
                    for adjoint in [adjoint_first, !adjoint_first] {
                        execute_action_inner(&object, &mut actual, name, &[vec![0]], &inputs, &[], &[], adjoint, &inherited)
                            .unwrap_or_else(|e| panic!("{name}, basis={basis}, inherited={inherited:?}, adjoint={adjoint}: {e}"));
                    }
                    assert!(
                        actual.exact_equals(&state),
                        "{name}, basis={basis}, inherited={inherited:?}, adjoint_first={adjoint_first}"
                    );
                }
            }
        }
    }
}

fn run(path: &Path, engine: &str, arguments: &[&str]) -> Json {
    let output = Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
        .arg(arguments[0])
        .arg(path)
        .args(&arguments[1..])
        .env("BENNETT_VM_ENGINE", engine)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{engine} {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap()
}

#[test]
fn cross_arm_identity_runs_in_both_cli_directions_and_engines() {
    let path = std::env::temp_dir().join(format!("qif-adjoint-action-{}.bisa", std::process::id()));
    std::fs::write(&path, support::source(ACTIONS)).unwrap();
    for engine in ["interpreter", "jit"] {
        for t in ["0", "1"] {
            for p in ["0", "1"] {
                for adjoint in [false, true] {
                    let mut args = vec!["run-action", "cross", "--mut", t, "--in", p];
                    if adjoint {
                        args.push("--adjoint");
                    }
                    let result = run(&path, engine, &args);
                    assert_eq!(
                        result.get("mut"),
                        Some(&Json::Arr(vec![Json::Num(t.into())]))
                    );
                    assert_eq!(
                        result.get("in"),
                        Some(&Json::Arr(vec![Json::Num(p.into())]))
                    );
                }
            }
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn classical_reversal_inverts_cross_arm_actions_in_cc_and_r_frames() {
    for kind in ["cc", "r"] {
        let functions = format!(
            "{ACTIONS}\nfn {kind} @inner (q:qreg[1], p:qreg[1]) () () () ()\n^entry():\naction_call @cross (q) (p) () ()\nret\nfn cc @main () () () () ()\n^entry():\nq = qreg.new any 1\np = qreg.new any 1\ncall @inner q p\nret"
        );
        let path = std::env::temp_dir().join(format!(
            "qif-adjoint-frame-{kind}-{}.bisa",
            std::process::id()
        ));
        std::fs::write(&path, support::source(&functions)).unwrap();
        for engine in ["interpreter", "jit"] {
            for command in ["run-cc", "run-program", "reverse"] {
                let result = run(&path, engine, &[command, "main"]);
                assert_eq!(
                    result.get("status").and_then(Json::as_str),
                    Some(if command == "reverse" {
                        "REVERSED"
                    } else {
                        "COMPLETED"
                    })
                );
                assert_eq!(result.get("fault"), Some(&Json::Null));
                if command == "reverse" {
                    assert_eq!(result.get("equals_entry"), Some(&Json::Bool(true)));
                    assert_eq!(result.get("first_difference"), Some(&Json::Null));
                }
            }
        }
        std::fs::remove_file(path).unwrap();
    }
}
