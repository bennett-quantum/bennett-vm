//! Where the 0.8 text (`Bennett-ISA-Spec.md`) is silent, the reference interpreter is
//! the oracle (advisor ruling R1, 2026-11-02): each behaviour below is the reference's,
//! observed at 0.8 and 0.9 alike, and differs from the pre-vm-fixes VM (d9cb684) at
//! 0.8. The 0.9 sentence that states it is cited. The 0.8 text defect for draw ties is
//! b-isa-spec-0h4. Evidence: docs/reviews/2026-11-02-vm-fixes-review.md.
use bennett_vm::json::{self, Json};
use std::{
    path::Path,
    process::{Command, Output},
};

#[path = "support/source09.rs"]
#[allow(dead_code)]
mod source09;

const ENGINES: [&str; 2] = ["interpreter", "jit"];

fn run(args: &[String], engine: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bennett-vm"))
        .args(args)
        .env("BENNETT_VM_ENGINE", engine)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("BROWSER")
        .output()
        .unwrap()
}

fn object(label: &str, version: &str, functions: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".tmp")
        .join(format!(
            "isa08ref-{label}-{version}-{}.bisa",
            std::process::id()
        ));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, source09::source_at(version, functions)).unwrap();
    path.display().to_string()
}

fn argv(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

fn success(args: &[String], engine: &str) -> Json {
    let out = run(args, engine);
    assert!(
        out.status.success(),
        "{engine} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    json::parse(std::str::from_utf8(&out.stdout).unwrap().trim()).unwrap()
}

fn refused(args: &[String], engine: &str, rule: &str) {
    let out = run(args, engine);
    let said = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{engine} {args:?}: {said}");
    assert!(said.contains(rule), "{engine} {args:?}: {said}");
}

fn measured(args: &[String], engine: &str) -> String {
    let json = success(args, engine);
    let m = &json.get("measurements").unwrap().as_arr().unwrap()[0];
    m.get("bits").unwrap().as_str().unwrap().to_string()
}

const VERSIONS: [&str; 2] = ["0.8", "0.9"];

/// 0.9 §4.1 K17: an exact tie selects the next outcome. Reference at 0.8 and 0.9:
/// `h` then `--draws 0.5` measures 1 (d9cb684 measured 0 at 0.8).
#[test]
fn draw_at_an_exact_tie() {
    let functions = "action @hact (t:wire mut) () () () ()\n^entry():\nh t\nfn cc @main () (r:i1) () () ()\n^b():\nq = qreg.new any 1\naction_call @hact (q) () () ()\nr = measure q\nret r";
    for version in VERSIONS {
        let path = object("draw", version, functions);
        for engine in ENGINES {
            for command in ["run-cc", "run-program"] {
                let at = |draw| argv(&[command, &path, "main", "--draws", draw]);
                assert_eq!(measured(&at("0.5"), engine), "1", "{version} {engine}");
                assert_eq!(measured(&at("0.25"), engine), "0", "{version} {engine}");
                assert_eq!(measured(&at("0.75"), engine), "1", "{version} {engine}");
            }
        }
    }
}

/// 0.9 §3.4 K5: a block ends in its terminator. Reference at 0.8 and 0.9: "§3.4:
/// classical block requires final br/ret/trap" for a row after `trap` (d9cb684 ran it
/// to fault 4 at 0.8); a `ret` before the end was always refused.
#[test]
fn row_after_a_trap() {
    for version in VERSIONS {
        let functions = "fn cc @main () (r:i8) () () ()\n^b():\nr = const.i8 1\ntrap\nret r";
        let path = object("row-after-trap", version, functions);
        let functions =
            "fn cc @main () (r:i8) () () ()\n^b():\nr = const.i8 1\nret r\nr = const.i8 2\nret r";
        let ret = object("row-after-ret", version, functions);
        for engine in ENGINES {
            refused(&argv(&["check", &path]), engine, "§3.4");
            refused(&argv(&["run-cc", &path, "main"]), engine, "§3.4");
            refused(&argv(&["check", &ret]), engine, "§3.4");
        }
    }
}

/// 0.9 E16.9: a literal lies in [-2^(N-1), 2^N). Reference at 0.8 and 0.9: "§3.3:
/// const literal 3 is outside [-2^(N-1), 2^N) for N = 1" (d9cb684 accepted at 0.8).
#[test]
fn lifted_constant_spelling() {
    for (label, regs, row) in [
        ("const3", "b:reg[1]", "b = lift.const.i1 3"),
        ("const-3", "a:reg[2]", "a = lift.const.i2 -3"),
    ] {
        let functions = format!(
            "oracle @o (x:wire in) (t:wire) (f:wire[4]) () ()\nregs {{ {regs} }}\n^entry():\n{row}\nlift.retval x"
        );
        for version in VERSIONS {
            let path = object(label, version, &functions);
            for engine in ENGINES {
                refused(&argv(&["check", &path]), engine, "§3.3");
            }
        }
    }
    let functions = "oracle @o (x:wire in) (t:wire) (f:wire[4]) () ()\nregs { a:reg[2], b:reg[1] }\n^entry():\nb = lift.const.i1 1\na = lift.const.i2 -2\na = lift.const.i2 3\nlift.retval x";
    for version in VERSIONS {
        success(&argv(&["check", &object("const-edges", version, functions)]), "interpreter");
    }
}

/// 0.9 §5.1: a lifted call's classical argument is an immutable entry binding.
/// Reference at 0.8 and 0.9: "§3.3: region call operand type/width mismatch" for an
/// interface input or a location (d9cb684 accepted both at 0.8).
#[test]
fn lifted_call_classical_arguments() {
    let with = |regs: &str, actual: &str| {
        format!(
            "oracle @i (x:wire in) (t:wire) (f:wire[4]) () (n:i1)\n^entry():\nlift.retval x\noracle @o (x:wire in) (t:wire) (f:wire[4]) () (tk:token, m:i1)\nregs {{ r:reg[1], fr:reg[4]{regs} }}\n^entry():\nlift.oracle_call @i tk (x) r fr () ({actual})\nlift.retval r"
        )
    };
    for version in VERSIONS {
        for (label, regs, actual) in [
            ("input", "", "x"),
            ("location", ", k:reg[1]", "k"),
            ("wide", ", k:reg[2]", "k"),
        ] {
            let path = object(label, version, &with(regs, actual));
            for engine in ENGINES {
                refused(&argv(&["check", &path]), engine, "§3.3");
            }
        }
        let entry = object("entry", version, &with("", "m"));
        success(&argv(&["check", &entry]), "interpreter");
    }
}

/// 0.9 E17.7: a use without a current instance traps, in a region call as anywhere.
/// Reference at 0.8 and 0.9: FAILED with fault 4 (d9cb684 said "invalid action
/// argument" at 0.8).
#[test]
fn region_call_classical_operand_without_an_instance() {
    let functions = "action @n (t:wire mut) () () (k:i1) ()\n^entry():\nx t\nfn cc @main (b:i1) () () () ()\n^e():\nbr b ^l() ^r()\n^l():\nv = const.i1 0\nbr ^r()\n^r():\nq = qreg.new any 1\naction_call @n (q) () () (v)\nqreg.free q\nret";
    for version in VERSIONS {
        let path = object("arg-instance", version, functions);
        for engine in ENGINES {
            let json = success(&argv(&["run-cc", &path, "main", "--arg", "0"]), engine);
            assert_eq!(json.get("fault").unwrap().as_u64(), Some(4), "{version} {engine}");
        }
    }
}

/// §5.5: a call and its uncall match on all their operands, `tgt` and `fr'` included,
/// as in the reference at 0.8 and 0.9 ("no outstanding matching lift.oracle_call on
/// every path with unchanged ins, tgt, fr and supplied cells"): pairs on different
/// targets may cross, an unmatched call is retained, an uncall of no call is refused.
#[test]
fn crossing_uncall_pairs() {
    let with = |rows: &str| {
        format!(
            "oracle @i (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x\noracle @o (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs {{ r:reg[1], s:reg[1], fr:reg[4], fs:reg[4] }}\n^entry():\n{rows}\nlift.retval r"
        )
    };
    let admitted = [
        (
            "crossing",
            "lift.oracle_call @i tk (x) r fr () ()\nlift.oracle_call @i tk (x) s fs () ()\nlift.oracle_uncall @i tk (x) r fr () ()\nlift.oracle_uncall @i tk (x) s fs () ()",
            5,
        ),
        (
            "skipping",
            "lift.oracle_call @i tk (x) r fr () ()\nlift.oracle_call @i tk (x) s fs () ()\nlift.oracle_uncall @i tk (x) r fr () ()",
            4,
        ),
    ];
    for version in VERSIONS {
        for (label, rows, steps) in admitted {
            let path = object(label, version, &with(rows));
            for engine in ENGINES {
                let args = argv(&[
                    "run-oracle",
                    &path,
                    "o",
                    "--input",
                    "0",
                    "--arg",
                    "token:64",
                ]);
                assert_eq!(
                    success(&args, engine).get("steps").unwrap().as_u64(),
                    Some(steps),
                    "{label} {version} {engine}"
                );
            }
        }
        for (label, rows) in [
            (
                "unmatched",
                "lift.oracle_call @i tk (x) r fr () ()\nlift.oracle_uncall @i tk (x) s fr () ()",
            ),
            ("orphan", "lift.oracle_uncall @i tk (x) r fr () ()"),
        ] {
            let path = object(label, version, &with(rows));
            for engine in ENGINES {
                refused(&argv(&["check", &path]), engine, "§5.5");
            }
        }
    }
}
