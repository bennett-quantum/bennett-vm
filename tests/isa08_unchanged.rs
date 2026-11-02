//! ISA 0.8 stays byte for byte where its text is silent (`Bennett-ISA-Spec.md`).
//! Each behaviour below is the pre-vm-fixes VM's at 0.8 (d9cb684, observed), with the
//! 0.9 sentence that changes it. The reference applies these 0.9 rules at 0.8 as well,
//! so at 0.8 they are divergences from it (see docs/reviews/2026-11-02-vm-fixes-review.md).
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
            "isa08-{label}-{version}-{}.bisa",
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

/// 0.9 §4.1 K17: an exact tie selects the next outcome. 0.8 has no draw rule, and
/// |1/sqrt(2)|^2 in binary64 is above 1/2.
#[test]
fn draw_at_an_exact_tie() {
    let functions = "action @hact (t:wire mut) () () () ()\n^entry():\nh t\nfn cc @main () (r:i1) () () ()\n^b():\nq = qreg.new any 1\naction_call @hact (q) () () ()\nr = measure q\nret r";
    for (version, tie) in [("0.8", "0"), ("0.9", "1")] {
        let path = object("draw", version, functions);
        for engine in ENGINES {
            for command in ["run-cc", "run-program"] {
                let at = |draw| argv(&[command, &path, "main", "--draws", draw]);
                assert_eq!(measured(&at("0.5"), engine), tie, "{version} {engine}");
                assert_eq!(measured(&at("0.25"), engine), "0", "{version} {engine}");
                assert_eq!(measured(&at("0.75"), engine), "1", "{version} {engine}");
            }
        }
    }
}

/// 0.9 §3.4 K5: a block ends in its terminator. 0.8's §3.4 is silent; a `ret` or a
/// `br` before the end stays refused, as it always was.
#[test]
fn row_after_a_trap() {
    let functions = "fn cc @main () (r:i8) () () ()\n^b():\nr = const.i8 1\ntrap\nret r";
    let eight = object("row-after-trap", "0.8", functions);
    let nine = object("row-after-trap", "0.9", functions);
    for engine in ENGINES {
        let json = success(&argv(&["run-cc", &eight, "main"]), engine);
        assert_eq!(json.get("fault").unwrap().as_u64(), Some(4), "{engine}");
        refused(&argv(&["check", &nine]), engine, "§3.4");
    }
    let functions =
        "fn cc @main () (r:i8) () () ()\n^b():\nr = const.i8 1\nret r\nr = const.i8 2\nret r";
    let eight = object("row-after-ret", "0.8", functions);
    refused(&argv(&["check", &eight]), "interpreter", "§3.4");
}

/// 0.9 E16.9: a literal lies in [-2^(N-1), 2^N). 0.8 states no range.
#[test]
fn lifted_constant_spelling() {
    for literal in ["3", "-3"] {
        let functions = format!(
            "oracle @o (x:wire in) (t:wire) (f:wire[4]) () ()\nregs {{ a:reg[2], b:reg[1] }}\n^entry():\na = lift.const.i2 {literal}\nlift.retval x"
        );
        let functions = if literal == "3" {
            functions.replace("a = lift.const.i2 3", "b = lift.const.i1 3")
        } else {
            functions
        };
        let eight = object(&format!("const{literal}"), "0.8", &functions);
        let nine = object(&format!("const{literal}"), "0.9", &functions);
        for engine in ENGINES {
            success(&argv(&["check", &eight]), engine);
            refused(&argv(&["check", &nine]), engine, "§3.3");
        }
    }
}

/// 0.9 §5.1: a lifted call's classical argument is an immutable entry binding. 0.8's
/// §3.3 asks for the callee's widths, which remain required.
#[test]
fn lifted_call_classical_arguments() {
    let with = |regs: &str, actual: &str| {
        format!(
            "oracle @i (x:wire in) (t:wire) (f:wire[4]) () (n:i1)\n^entry():\nlift.retval x\noracle @o (x:wire in) (t:wire) (f:wire[4]) () (tk:token, m:i1)\nregs {{ r:reg[1], fr:reg[4]{regs} }}\n^entry():\nlift.oracle_call @i tk (x) r fr () ({actual})\nlift.retval r"
        )
    };
    for (label, regs, actual) in [("input", "", "x"), ("location", ", k:reg[1]", "k")] {
        let eight = object(label, "0.8", &with(regs, actual));
        let nine = object(label, "0.9", &with(regs, actual));
        for engine in ENGINES {
            success(&argv(&["check", &eight]), engine);
            refused(&argv(&["check", &nine]), engine, "§3.3");
        }
    }
    let entry = object("entry", "0.8", &with("", "m"));
    success(&argv(&["check", &entry]), "interpreter");
    let wide = object("wide", "0.8", &with(", k:reg[2]", "k"));
    refused(&argv(&["check", &wide]), "interpreter", "§3.3");
}

/// 0.9 E17.7: a use without a current instance traps, in a region call as anywhere.
/// 0.8 has no such sentence: the classical operand of a region call is a diagnostic.
#[test]
fn region_call_classical_operand_without_an_instance() {
    let functions = "action @n (t:wire mut) () () (k:i1) ()\n^entry():\nx t\nfn cc @main (b:i1) () () () ()\n^e():\nbr b ^l() ^r()\n^l():\nv = const.i1 0\nbr ^r()\n^r():\nq = qreg.new any 1\naction_call @n (q) () () (v)\nqreg.free q\nret";
    let eight = object("arg-instance", "0.8", functions);
    let nine = object("arg-instance", "0.9", functions);
    for engine in ENGINES {
        refused(
            &argv(&["run-cc", &eight, "main", "--arg", "0"]),
            engine,
            "invalid action argument",
        );
        let json = success(&argv(&["run-cc", &nine, "main", "--arg", "0"]), engine);
        assert_eq!(json.get("fault").unwrap().as_u64(), Some(4), "{engine}");
    }
}

/// 0.9 E17.5: pairs match LIFO. 0.8's §5.5 pairs a call and an uncall by operands, so
/// pairs on different targets may cross; an uncall of no call is refused at both.
#[test]
fn crossing_uncall_pairs() {
    let with = |rows: &str| {
        format!(
            "oracle @i (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x\noracle @o (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs {{ r:reg[1], s:reg[1], fr:reg[4], fs:reg[4] }}\n^entry():\n{rows}\nlift.retval r"
        )
    };
    let crossing = with(
        "lift.oracle_call @i tk (x) r fr () ()\nlift.oracle_call @i tk (x) s fs () ()\nlift.oracle_uncall @i tk (x) r fr () ()\nlift.oracle_uncall @i tk (x) s fs () ()",
    );
    let eight = object("crossing", "0.8", &crossing);
    let nine = object("crossing", "0.9", &crossing);
    for engine in ENGINES {
        let args = argv(&[
            "run-oracle",
            &eight,
            "o",
            "--input",
            "0",
            "--arg",
            "token:64",
        ]);
        assert_eq!(
            success(&args, engine).get("steps").unwrap().as_u64(),
            Some(5),
            "{engine}"
        );
        refused(&argv(&["check", &nine]), engine, "§5.5");
    }
    let unmatched =
        with("lift.oracle_call @i tk (x) r fr () ()\nlift.oracle_uncall @i tk (x) s fr () ()");
    for version in ["0.8", "0.9"] {
        let path = object("unmatched", version, &unmatched);
        refused(&argv(&["check", &path]), "interpreter", "§5.5");
    }
}
