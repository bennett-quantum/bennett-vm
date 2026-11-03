//! ISA0.9 classical rows use runtime widths and reversible resource identities.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::json::{self, Json};
use std::{
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

fn run(
    binary: &std::path::Path,
    engine: &str,
    functions: &str,
    command: &str,
    args: &[&str],
    aliases: Option<&str>,
) -> std::process::Output {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "classical09-{engine}-{}-{}.bisa",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, support::source(functions)).unwrap();
    let mut run = Command::new(binary);
    run.arg(command)
        .arg(&path)
        .arg("main")
        .env("BENNETT_VM_ENGINE", engine);
    for arg in args {
        run.args(["--arg", arg]);
    }
    if let Some(aliases) = aliases {
        run.args(["--token-aliases", aliases]);
    }
    let output = run.output().unwrap();
    std::fs::remove_file(path).unwrap();
    output
}
fn execute(engine: &str, functions: &str, command: &str, args: &[&str]) -> Json {
    let output = run(
        std::path::Path::new(env!("CARGO_BIN_EXE_bennett-vm")),
        engine,
        functions,
        command,
        args,
        None,
    );
    assert!(
        output.status.success(),
        "{engine}, {command}, {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap()
}
fn completed(result: &Json, expected: &[&str]) {
    assert_eq!(result.get("fault"), Some(&Json::Null));
    assert_eq!(
        result.get("status").and_then(Json::as_str),
        Some("COMPLETED")
    );
    assert_eq!(
        result
            .get("results")
            .unwrap()
            .as_arr()
            .unwrap()
            .iter()
            .map(|v| v.get("value").unwrap().as_str().unwrap())
            .collect::<Vec<_>>(),
        expected
    );
}
fn failed(result: &Json, code: u64) {
    assert_eq!(result.get("status").and_then(Json::as_str), Some("FAILED"));
    assert_eq!(result.get("fault").and_then(Json::as_u64), Some(code));
    assert_eq!(result.get("faults").unwrap().as_arr().unwrap().len(), 1);
    assert_eq!(result.get("results").unwrap().as_arr().unwrap().len(), 0);
}

fn register_rows(engine: &str) {
    let functions = "fn cc @main (n:i64) (w:i64, same:i1) () () ()\n^entry():\nq = qreg.new any n\nid = qreg.id q\nalias = qreg.from_id id\nw = qreg.width alias\nagain = qreg.id alias\nsame = icmp.eq id again\nqreg.free alias\nret w same";
    for n in ["0", "1", "3"] {
        for command in ["run-cc", "run-program"] {
            completed(&execute(engine, functions, command, &[n]), &[n, "1"]);
        }
        let reversed = execute(engine, functions, "reverse", &[n]);
        assert_eq!(reversed.get("equals_entry"), Some(&Json::Bool(true)));
        assert_eq!(reversed.get("first_difference"), Some(&Json::Null));
    }
    let concat = "fn cc @main () (w:i64, old:i64) () () ()\n^entry():\nz = const.i64 0\nn = const.i64 3\na = qreg.new any z\nb = qreg.new any n\nold = qreg.width a\nq = qreg.concat a b\nw = qreg.width q\nqreg.free q\nret w old";
    completed(&execute(engine, concat, "run-program", &[]), &["3", "0"]);
}
#[test]
fn dynamic_register_rows_interpreter() {
    register_rows("interpreter");
}
#[test]
fn dynamic_register_rows_jit() {
    register_rows("jit");
}

#[test]
fn unallocatable_dynamic_register_is_a_host_limit_diagnostic() {
    let functions = "fn cc @main (n:i64) () () () ()\n^entry():\nq = qreg.new any n\nret";
    for engine in ["interpreter", "jit"] {
        let output = run(
            std::path::Path::new(env!("CARGO_BIN_EXE_bennett-vm")),
            engine,
            functions,
            "run-program",
            &["18446744073709551615"],
            None,
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("host limit") && !error.contains("panicked"),
            "{engine}: {error}"
        );
    }
}

fn identities_and_aliases(engine: &str) {
    let prefix = "fn r @make () (q:qreg[1]) () () ()\n^entry():\nq = qreg.new any 1\nret q\n";
    let fresh = format!(
        "{prefix}fn cc @main () (same:i1) () () ()\n^entry():\nq = call @make\nold = qreg.id q\nuncall @make () (q)\nnewq = qreg.new any 1\nnewid = qreg.id newq\nsame = icmp.eq old newid\nqreg.free newq\nret same"
    );
    completed(&execute(engine, &fresh, "run-program", &[]), &["0"]);
    let stale = format!(
        "{prefix}fn cc @main () () () () ()\n^entry():\nq = call @make\nold = qreg.id q\nuncall @make () (q)\nnewq = qreg.new any 1\nalias = qreg.from_id old\nret"
    );
    failed(&execute(engine, &stale, "run-program", &[]), 4);
    let stale_alias = format!(
        "{prefix}fn cc @main () (w:i64) () () ()\n^entry():\nq = call @make\nid = qreg.id q\nalias = qreg.from_id id\nuncall @make () (q)\nw = qreg.width alias\nret w"
    );
    failed(&execute(engine, &stale_alias, "run-program", &[]), 4);
    let aliased_concat = "fn cc @main () () () () ()\n^entry():\nq = qreg.new any 1\nid = qreg.id q\nalias = qreg.from_id id\nr = qreg.concat q alias\nret";
    failed(&execute(engine, aliased_concat, "run-program", &[]), 4);
    let viewed_id = "action @flip (t:wire mut) () () () ()\n^entry():\nx t\nfn cc @main () () () () ()\n^entry():\nq = qreg.new any 1\nid = qreg.id q\nv = qreg.slice q 0 1\nalias = qreg.from_id id\naction_call @flip (v) () () ()\nret";
    failed(&execute(engine, viewed_id, "run-program", &[]), 4);
    for id in ["0", "18446744073709551615"] {
        let invalid = format!(
            "fn cc @main () () () () ()\n^entry():\nid = const.i64 {id}\nq = qreg.from_id id\nret"
        );
        failed(&execute(engine, &invalid, "run-program", &[]), 4);
    }
    for query in ["qreg.width", "qreg.id"] {
        let consumed = format!(
            "fn cc @main () (w:i64) () () ()\n^entry():\nq = qreg.new any 1\nid = qreg.id q\nalias = qreg.from_id id\nqreg.free alias\nw = {query} q\nret w"
        );
        failed(&execute(engine, &consumed, "run-program", &[]), 4);
        let viewed = format!(
            "action @flip (t:wire mut) () () () ()\n^entry():\nx t\nfn cc @main () (w:i64) () () ()\n^entry():\nq = qreg.new any 1\nid = qreg.id q\nalias = qreg.from_id id\nv = qreg.slice q 0 1\nw = {query} alias\naction_call @flip (v) () () ()\nret w"
        );
        failed(&execute(engine, &viewed, "run-program", &[]), 4);
        let reversed = execute(engine, &viewed, "reverse", &[]);
        assert_eq!(reversed.get("equals_entry"), Some(&Json::Bool(true)));
        assert_eq!(reversed.get("first_difference"), Some(&Json::Null));
    }
    let entry = "fn cc @main (q:qreg[*]) (w:i64) () () ()\n^entry():\nw = qreg.width q\nret w";
    for (bits, n) in [("qreg:", "0"), ("qreg:010", "3")] {
        completed(&execute(engine, entry, "run-program", &[bits]), &[n]);
    }
}
#[test]
fn register_id_lifetimes_interpreter() {
    identities_and_aliases("interpreter");
}
#[test]
fn register_id_lifetimes_jit() {
    identities_and_aliases("jit");
}

fn transfer_rows(engine: &str) {
    let call = "fn r @fixed (q:qreg[2]) (r:qreg[2]) () () ()\n^entry():\nret q\nfn cc @main (n:i64) (w:i64) () () ()\n^entry():\nq = qreg.new any n\nr = call @fixed q\nw = qreg.width r\nqreg.free r\nret w";
    let ret = "fn r @fixed (q:qreg[*]) (r:qreg[2]) () () ()\n^entry():\nret q\nfn cc @main (n:i64) (w:i64) () () ()\n^entry():\nq = qreg.new any n\nr = call @fixed q\nw = qreg.width r\nqreg.free r\nret w";
    let branch = "fn cc @main (n:i64) (w:i64) () () ()\n^entry():\nq = qreg.new any n\nbr ^next(q)\n^next(r:qreg[2]):\nw = qreg.width r\nqreg.free r\nret w";
    for functions in [call, ret, branch] {
        completed(&execute(engine, functions, "run-program", &["2"]), &["2"]);
        for n in ["0", "1", "3"] {
            failed(&execute(engine, functions, "run-program", &[n]), 4);
        }
    }
    let slice = "action @flip (t:wire mut) () () () ()\n^entry():\nx t\nfn cc @main (n:i64, lo:i64, len:i64) (w:i64) () () ()\n^entry():\nq = qreg.new any n\nv = qreg.slice q lo len\naction_call @flip (v) () () ()\nv2 = qreg.slice q lo len\naction_call @flip (v2) () () ()\nw = qreg.width q\nqreg.free q\nret w";
    for lo in ["0", "2"] {
        completed(
            &execute(engine, slice, "run-program", &["3", lo, "1"]),
            &["3"],
        );
    }
    for args in [
        ["3", "3", "1"],
        ["3", "0", "2"],
        ["3", "0", "0"],
        ["3", "18446744073709551615", "1"],
    ] {
        failed(&execute(engine, slice, "run-program", &args), 4);
    }
}
#[test]
fn dynamic_transfers_interpreter() {
    transfer_rows("interpreter");
}
#[test]
fn dynamic_transfers_jit() {
    transfer_rows("jit");
}

fn terminal_faults(engine: &str) {
    for code in [0, 4, 42, u64::MAX] {
        let functions = format!(
            "fn r @stop () () () () ()\n^entry():\nfault {code}\nfn cc @main () (r:i8) () () ()\n^entry():\nbefore = const.i8 7\ncall @stop\nr = const.i8 9\nret r"
        );
        for command in ["run-cc", "run-program"] {
            failed(
                &execute(engine, &functions, command, &[]),
                if code == 0 { 4 } else { code },
            );
        }
        let reversed = execute(engine, &functions, "reverse", &[]);
        assert_eq!(reversed.get("equals_entry"), Some(&Json::Bool(true)));
        assert_eq!(reversed.get("first_difference"), Some(&Json::Null));
    }
}
#[test]
fn terminal_fault_records_interpreter() {
    terminal_faults("interpreter");
}
#[test]
fn terminal_fault_records_jit() {
    terminal_faults("jit");
}

const ORACLE: &str = "oracle @o (x:wire in) (t:wire) (f:wire[4]) () ()\nregs { l:reg[1] }\n^entry():\nl = lift.const.i1 0\nlift.retval l\n";
fn minted_budgets(engine: &str) {
    let functions = format!(
        "{ORACLE}fn cc @main (s:i64, h:i64, st:i64, d:i64) (tk:token) () () ()\n^entry():\ntk = budget.mint @o s h st d\nret tk"
    );
    let result = execute(engine, &functions, "run-program", &["32", "0", "0", "0"]);
    assert_eq!(result.get("fault"), Some(&Json::Null));
    let token = &result.get("results").unwrap().as_arr().unwrap()[0];
    assert_eq!(token.get("type").and_then(Json::as_str), Some("token"));
    assert_eq!(
        token.get("value"),
        Some(&json::parse(r#"{"steps":32,"heap":0,"stack":0,"depth":0}"#).unwrap())
    );
    for args in [
        ["32", "1", "0", "0"],
        ["32", "0", "1", "0"],
        ["32", "0", "0", "1"],
    ] {
        failed(&execute(engine, &functions, "run-program", &args), 4);
    }
    let revoked = format!(
        "{ORACLE}fn r @make (s:i64, z:i64) (tk:token) () () ()\n^entry():\ntk = budget.mint @o s z z z\nret tk\nfn cc @main () () () () ()\n^entry():\ns = const.i64 32\nz = const.i64 0\ntk = call @make s z\nbr ^next(tk)\n^next(alias:token):\nuncall @make (s, z) (alias)\nx = qreg.new any 1\nt = qreg.new any 1\nf = qreg.new any 4\noracle_call @o alias (x) t f () ()\nret"
    );
    failed(&execute(engine, &revoked, "run-program", &[]), 4);
}
#[test]
fn minted_budget_rows_interpreter() {
    minted_budgets("interpreter");
}
#[test]
fn minted_budget_rows_jit() {
    minted_budgets("jit");
}

#[test]
fn incoming_tokens_with_equal_tuples_keep_distinct_identities() {
    let distinct = "fn r @echo (t:token) (r:token) () () ()\n^entry():\nret t\nfn cc @main (a:token, b:token) () () () ()\n^entry():\nr = call @echo a\nuncall @echo (b) (r)\nret";
    let same = "fn r @echo (t:token) (r:token) () () ()\n^entry():\nret t\nfn cc @main (a:token) () () () ()\n^entry():\nr = call @echo a\nuncall @echo (a) (r)\nret";
    let distinct = format!("{ORACLE}{distinct}");
    let same = format!("{ORACLE}{same}");
    let reference = std::env::var_os("BENNETT_ISA_REF")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref")
        });
    for binary in [
        reference.as_path(),
        std::path::Path::new(env!("CARGO_BIN_EXE_bennett-vm")),
    ] {
        for engine in ["interpreter", "jit"] {
            for (functions, args, aliases, fault) in [
                (
                    distinct.as_str(),
                    vec!["token:32", "token:32"],
                    "[0,1]",
                    true,
                ),
                (
                    distinct.as_str(),
                    vec!["token:32", "token:32"],
                    "[0,0]",
                    false,
                ),
                (same.as_str(), vec!["token:32"], "[0]", false),
            ] {
                let output = run(
                    binary,
                    engine,
                    functions,
                    "run-program",
                    &args,
                    Some(aliases),
                );
                assert!(
                    output.status.success(),
                    "{} {engine}: {}",
                    binary.display(),
                    String::from_utf8_lossy(&output.stderr)
                );
                let result = json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
                if fault {
                    failed(&result, 4);
                } else {
                    completed(&result, &[]);
                }
            }
        }
    }
}

#[test]
fn incoming_token_partitions_are_validated_before_execution() {
    let functions =
        format!("{ORACLE}fn cc @main (a:token, b:token) () () () ()\n^entry():\nfault 42");
    for engine in ["interpreter", "jit"] {
        for aliases in [
            None,
            Some("[]"),
            Some("[0]"),
            Some("[0,1,2]"),
            Some("[1,1]"),
            Some("[0,2]"),
            Some("[0,-1]"),
            Some("[0,0.5]"),
            Some("[0,\"1\"]"),
            Some("{}"),
            Some("invalid"),
        ] {
            let output = run(
                std::path::Path::new(env!("CARGO_BIN_EXE_bennett-vm")),
                engine,
                &functions,
                "run-program",
                &["token:32", "token:32"],
                aliases,
            );
            assert!(!output.status.success(), "{engine}/{aliases:?}");
            assert!(output.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("E_ARG"),
                "{output:?}"
            );
        }
        for (args, aliases, valid) in [
            (["token:32", "token:33"], "[0,0]", false),
            (["token:32", "token:33"], "[0,1]", true),
            (["token:32", "token:32"], "[0,0]", true),
            (["token:32", "token:32"], "[0,1]", true),
        ] {
            let output = run(
                std::path::Path::new(env!("CARGO_BIN_EXE_bennett-vm")),
                engine,
                &functions,
                "run-program",
                &args,
                Some(aliases),
            );
            assert_eq!(
                output.status.success(),
                valid,
                "{engine}/{args:?}/{aliases}: {output:?}"
            );
            if valid {
                failed(
                    &json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap(),
                    42,
                );
            } else {
                assert!(output.stdout.is_empty());
                assert!(String::from_utf8_lossy(&output.stderr).contains("E_ARG"));
            }
        }
    }
}
