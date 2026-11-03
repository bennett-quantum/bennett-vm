//! ISA0.9 classical rows use runtime widths and reversible resource identities.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::json::{self, Json};
use std::{process::Command, sync::atomic::{AtomicUsize, Ordering}};

fn execute(engine: &str, functions: &str, command: &str, args: &[&str]) -> Json {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("classical09-{engine}-{}-{}.bisa", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, support::source(functions)).unwrap();
    let mut run = Command::new(env!("CARGO_BIN_EXE_bennett-vm"));
    run.arg(command).arg(&path).arg("main").env("BENNETT_VM_ENGINE", engine);
    for arg in args { run.args(["--arg", arg]); }
    let output = run.output().unwrap();
    assert!(output.status.success(), "{engine}, {command}, {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    let result = json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    result
}
fn completed(result: &Json, expected: &[&str]) {
    assert_eq!(result.get("fault"), Some(&Json::Null));
    assert_eq!(result.get("status").and_then(Json::as_str), Some("COMPLETED"));
    assert_eq!(result.get("results").unwrap().as_arr().unwrap().iter().map(|v| v.get("value").unwrap().as_str().unwrap()).collect::<Vec<_>>(), expected);
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
        for command in ["run-cc", "run-program"] { completed(&execute(engine, functions, command, &[n]), &[n, "1"]); }
        let reversed = execute(engine, functions, "reverse", &[n]);
        assert_eq!(reversed.get("equals_entry"), Some(&Json::Bool(true)));
        assert_eq!(reversed.get("first_difference"), Some(&Json::Null));
    }
    let concat = "fn cc @main () (w:i64, old:i64) () () ()\n^entry():\nz = const.i64 0\nn = const.i64 3\na = qreg.new any z\nb = qreg.new any n\nold = qreg.width a\nq = qreg.concat a b\nw = qreg.width q\nqreg.free q\nret w old";
    completed(&execute(engine, concat, "run-program", &[]), &["3", "0"]);
}
#[test] fn dynamic_register_rows_interpreter() { register_rows("interpreter"); }
#[test] fn dynamic_register_rows_jit() { register_rows("jit"); }

fn transfer_rows(engine: &str) {
    let call = "fn r @fixed (q:qreg[2]) (r:qreg[2]) () () ()\n^entry():\nret q\nfn cc @main (n:i64) (w:i64) () () ()\n^entry():\nq = qreg.new any n\nr = call @fixed q\nw = qreg.width r\nqreg.free r\nret w";
    let ret = "fn r @fixed (q:qreg[*]) (r:qreg[2]) () () ()\n^entry():\nret q\nfn cc @main (n:i64) (w:i64) () () ()\n^entry():\nq = qreg.new any n\nr = call @fixed q\nw = qreg.width r\nqreg.free r\nret w";
    let branch = "fn cc @main (n:i64) (w:i64) () () ()\n^entry():\nq = qreg.new any n\nbr ^next(q)\n^next(r:qreg[2]):\nw = qreg.width r\nqreg.free r\nret w";
    for functions in [call, ret, branch] {
        completed(&execute(engine, functions, "run-program", &["2"]), &["2"]);
        for n in ["0", "1", "3"] { failed(&execute(engine, functions, "run-program", &[n]), 4); }
    }
    let slice = "action @flip (t:wire mut) () () () ()\n^entry():\nx t\nfn cc @main (n:i64, lo:i64, len:i64) (w:i64) () () ()\n^entry():\nq = qreg.new any n\nv = qreg.slice q lo len\naction_call @flip (v) () () ()\nv2 = qreg.slice q lo len\naction_call @flip (v2) () () ()\nw = qreg.width q\nqreg.free q\nret w";
    for lo in ["0", "2"] { completed(&execute(engine, slice, "run-program", &["3", lo, "1"]), &["3"]); }
    for args in [["3", "3", "1"], ["3", "0", "2"], ["3", "0", "0"], ["3", "18446744073709551615", "1"]] {
        failed(&execute(engine, slice, "run-program", &args), 4);
    }
}
#[test] fn dynamic_transfers_interpreter() { transfer_rows("interpreter"); }
#[test] fn dynamic_transfers_jit() { transfer_rows("jit"); }

fn terminal_faults(engine: &str) {
    for code in [0, 4, 42, u64::MAX] {
        let functions = format!("fn r @stop () () () () ()\n^entry():\nfault {code}\nfn cc @main () (r:i8) () () ()\n^entry():\nbefore = const.i8 7\ncall @stop\nr = const.i8 9\nret r");
        for command in ["run-cc", "run-program"] { failed(&execute(engine, &functions, command, &[]), if code == 0 { 4 } else { code }); }
        let reversed = execute(engine, &functions, "reverse", &[]);
        assert_eq!(reversed.get("equals_entry"), Some(&Json::Bool(true)));
        assert_eq!(reversed.get("first_difference"), Some(&Json::Null));
    }
}
#[test] fn terminal_fault_records_interpreter() { terminal_faults("interpreter"); }
#[test] fn terminal_fault_records_jit() { terminal_faults("jit"); }

const ORACLE: &str = "oracle @o (x:wire in) (t:wire) (f:wire[4]) () ()\nregs { l:reg[1] }\n^entry():\nl = lift.const.i1 0\nlift.retval l\n";
fn minted_budgets(engine: &str) {
    let functions = format!("{ORACLE}fn cc @main (s:i64, h:i64, st:i64, d:i64) (tk:token) () () ()\n^entry():\ntk = budget.mint @o s h st d\nret tk");
    let result = execute(engine, &functions, "run-program", &["32", "0", "0", "0"]);
    assert_eq!(result.get("fault"), Some(&Json::Null));
    let token = &result.get("results").unwrap().as_arr().unwrap()[0];
    assert_eq!(token.get("type").and_then(Json::as_str), Some("token"));
    assert_eq!(token.get("value"), Some(&json::parse(r#"{"steps":32,"heap":0,"stack":0,"depth":0}"#).unwrap()));
    for args in [["32", "1", "0", "0"], ["32", "0", "1", "0"], ["32", "0", "0", "1"]] {
        failed(&execute(engine, &functions, "run-program", &args), 4);
    }
    let revoked = format!("{ORACLE}fn r @make (s:i64, z:i64) (tk:token) () () ()\n^entry():\ntk = budget.mint @o s z z z\nret tk\nfn cc @main () () () () ()\n^entry():\ns = const.i64 32\nz = const.i64 0\ntk = call @make s z\nbr ^next(tk)\n^next(alias:token):\nuncall @make (s, z) (alias)\nx = qreg.new any 1\nt = qreg.new any 1\nf = qreg.new any 4\noracle_call @o alias (x) t f () ()\nret");
    failed(&execute(engine, &revoked, "run-program", &[]), 4);
}
#[test] fn minted_budget_rows_interpreter() { minted_budgets("interpreter"); }
#[test] fn minted_budget_rows_jit() { minted_budgets("jit"); }
