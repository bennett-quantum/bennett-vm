//! Run ownership and token lifetimes through the published launch entry points.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::launch::*;
use std::{ffi::CString, ptr};

enum Arg {
    Int(u64),
    Register(u64),
    Token(u64),
}
unsafe fn load(functions: &str) -> *mut bennett_program_t {
    let source = support::source(functions);
    let mut program = ptr::null_mut();
    assert_eq!(
        unsafe { bennett_load(source.as_ptr(), source.len(), ptr::null(), 0, &mut program) },
        0
    );
    assert!(!program.is_null());
    program
}
unsafe fn start(program: *mut bennett_program_t, entry: &str, values: &[Arg]) -> Result<u64, i32> {
    let mut args = ptr::null_mut();
    assert_eq!(unsafe { bennett_args_new(values.len(), &mut args) }, 0);
    for (index, value) in values.iter().enumerate() {
        let status = unsafe {
            match value {
                Arg::Int(n) => bennett_args_set_int(args, index, 64, n.to_le_bytes().as_ptr()),
                Arg::Register(q) => bennett_args_set_qreg(args, index, *q),
                Arg::Token(t) => bennett_args_set_token(args, index, *t),
            }
        };
        assert_eq!(status, 0);
    }
    let mut run = 0;
    let entry = CString::new(entry).unwrap();
    let status = unsafe { bennett_run(program, entry.as_ptr(), args, ptr::null(), 0, &mut run) };
    unsafe {
        bennett_args_free(args);
    }
    if status == 0 { Ok(run) } else { Err(status) }
}
unsafe fn wait(program: *mut bennett_program_t, run: u64) -> (Vec<(i32, u64)>, Vec<u64>) {
    let mut result = ptr::null_mut();
    let mut request = ptr::null_mut();
    assert_eq!(
        unsafe { bennett_wait(program, run, &mut result, &mut request) },
        0
    );
    assert!(request.is_null() && !result.is_null());
    let mut values = vec![];
    for index in 0..unsafe { bennett_result_nvalues(result) } {
        let (mut kind, mut width, mut bytes) = (0, 0, [0u8; 8]);
        assert_eq!(
            unsafe {
                bennett_result_get(
                    result,
                    index,
                    &mut kind,
                    &mut width,
                    bytes.as_mut_ptr(),
                    bytes.len(),
                )
            },
            0
        );
        assert_eq!(width, 64);
        values.push((kind, u64::from_le_bytes(bytes)));
    }
    let faults = (0..unsafe { bennett_result_nfaults(result) })
        .map(|index| unsafe { bennett_result_fault(result, index) })
        .collect();
    unsafe {
        bennett_result_free(result);
    }
    (values, faults)
}

#[test]
fn from_id_ownership_is_acquired_before_effects_and_survives_reversal_until_close() {
    unsafe {
        let program = load(
            "fn cc @hold (q:qreg[*]) () () () ()\n^entry():\nret\nfn cc @acquire (id:i64) (w:i64) () () ()\n^entry():\nq = qreg.from_id id\nw = qreg.width q\nret w",
        );
        for n in [0, 1] {
            let mut q = 0;
            assert_eq!(bennett_qreg_new(program, c"any".as_ptr(), n, &mut q), 0);
            let owner = start(program, "hold", &[Arg::Register(q)]).unwrap();
            let denied = start(program, "acquire", &[Arg::Int(q)]).unwrap();
            assert_eq!(wait(program, denied), (vec![], vec![4]));
            assert_eq!(bennett_reverse(program, denied), 0);
            assert_eq!(bennett_run_close(program, denied), 0);
            assert_eq!(wait(program, owner), (vec![], vec![]));
            assert_eq!(bennett_run_close(program, owner), 0);

            let acquired = start(program, "acquire", &[Arg::Int(q)]).unwrap();
            assert_eq!(wait(program, acquired), (vec![(0, n as u64)], vec![]));
            for reverse in [false, true] {
                if reverse {
                    assert_eq!(bennett_reverse(program, acquired), 0);
                }
                let denied = start(program, "acquire", &[Arg::Int(q)]).unwrap();
                assert_eq!(wait(program, denied), (vec![], vec![4]));
                assert_eq!(bennett_run_close(program, denied), 0);
            }
            assert_eq!(bennett_run_close(program, acquired), 0);
            let last = start(program, "acquire", &[Arg::Int(q)]).unwrap();
            assert_eq!(wait(program, last), (vec![(0, n as u64)], vec![]));
            assert_eq!(bennett_run_close(program, last), 0);
            assert_eq!(bennett_qreg_free(program, q), 0);
        }
        assert_eq!(bennett_unload(program), 0);
    }
}

#[test]
fn minted_tokens_revoke_on_reverse_and_close_but_host_tokens_remain_program_scoped() {
    unsafe {
        let functions = "oracle @o (x:wire in) (t:wire) (f:wire[4]) () ()\nregs { l:reg[1] }\n^entry():\nl = lift.const.i1 0\nlift.retval l\nfn cc @mint () (tk:token) () () ()\n^entry():\ns = const.i64 32\nz = const.i64 0\ntk = budget.mint @o s z z z\nret tk\nfn cc @hold (tk:token) () () () ()\n^entry():\nret\nfn cc @invoke (tk:token) () () () ()\n^entry():\nx = qreg.new any 1\nt = qreg.new any 1\nf = qreg.new any 4\noracle_call @o tk (x) t f () ()\nqreg.free f\nqreg.free t\nqreg.free x\nret";
        let functions = format!(
            "{functions}\naction @flip (t:wire mut) () () (tk:token) ()\n^entry():\nx t\nfn cc @act (q:qreg[*], tk:token) () () () ()\n^entry():\naction_call @flip (q) () () (tk)\nret"
        );
        let program = load(&functions);
        let producer = start(program, "mint", &[]).unwrap();
        let (values, faults) = wait(program, producer);
        assert!(faults.is_empty());
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0, 3);
        let token = values[0].1;
        let invocation = start(program, "invoke", &[Arg::Token(token)]).unwrap();
        assert_eq!(wait(program, invocation), (vec![], vec![]));
        assert_eq!(bennett_reverse(program, invocation), 0);
        assert_eq!(bennett_run_close(program, invocation), 0);

        let pending = start(program, "invoke", &[Arg::Token(token)]).unwrap();
        let mut clean = 0;
        assert_eq!(bennett_qreg_new(program, c"any".as_ptr(), 1, &mut clean), 0);
        let pending_action =
            start(program, "act", &[Arg::Register(clean), Arg::Token(token)]).unwrap();
        assert_eq!(bennett_reverse(program, producer), 0);
        assert_eq!(start(program, "hold", &[Arg::Token(token)]), Err(11));
        assert_eq!(wait(program, pending_action), (vec![], vec![4]));
        assert_eq!(bennett_run_close(program, pending_action), 0);
        // The rejected token must precede the action's X effect.
        assert_eq!(bennett_qreg_free(program, clean), 0);
        assert_eq!(wait(program, pending), (vec![], vec![4]));
        assert_eq!(bennett_reverse(program, pending), 0);
        assert_eq!(bennett_run_close(program, pending), 0);
        assert_eq!(bennett_run_close(program, producer), 0);

        let producer = start(program, "mint", &[]).unwrap();
        let (values, faults) = wait(program, producer);
        assert!(faults.is_empty());
        assert_ne!(values[0].1, token);
        let second = values[0].1;
        assert_eq!(bennett_run_close(program, producer), 0);
        assert_eq!(start(program, "hold", &[Arg::Token(second)]), Err(11));

        let mut host_token = 0;
        assert_eq!(
            bennett_budget(program, c"o".as_ptr(), 32, 0, 0, 0, &mut host_token),
            0
        );
        for _ in 0..2 {
            let run = start(program, "invoke", &[Arg::Token(host_token)]).unwrap();
            assert_eq!(wait(program, run), (vec![], vec![]));
            assert_eq!(bennett_run_close(program, run), 0);
        }
        assert_eq!(bennett_unload(program), 0);
    }
}

#[test]
fn nominal_class_allocations_accept_literal_and_runtime_widths_on_an_installed_target() {
    use bennett_vm::json::{self, Json};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../bennett-isa-spec/conformance/cases/E/E-matching-first-entry/target.json");
    let mut target = json::parse(std::fs::read_to_string(path).unwrap().trim_end()).unwrap();
    let Json::Obj(fields) = &mut target else {
        panic!("target object");
    };
    fields.insert(
        "classes".into(),
        json::parse(r#"{"data":{"capacity":2,"dirty_rate":1,"qubit_second":1},"fast":{"capacity":2,"dirty_rate":1,"qubit_second":1}}"#).unwrap(),
    );
    let target = target.to_jcs();
    unsafe {
        for width in [0, 1, 2, 3] {
            for operand in [width.to_string(), "n".into()] {
                let source = support::source(&format!(
                    "fn cc @main (n:i64) (w:i64) () () ()\n^entry():\nq = qreg.new data {operand}\nw = qreg.width q\nqreg.free q\nret w"
                ));
                let mut program = ptr::null_mut();
                let status = bennett_load(
                    source.as_ptr(),
                    source.len(),
                    target.as_ptr(),
                    target.len(),
                    &mut program,
                );
                // §3.3 admits zero only through an unsigned i64 value.
                if width == 0 && operand == "0" {
                    assert_eq!(status, 11);
                    assert!(program.is_null());
                    assert!(
                        std::ffi::CStr::from_ptr(bennett_last_error())
                            .to_string_lossy()
                            .contains("literal qreg length must be positive")
                    );
                    continue;
                }
                assert_eq!(
                    status,
                    0,
                    "width {width}/{operand}: {}",
                    std::ffi::CStr::from_ptr(bennett_last_error()).to_string_lossy()
                );
                let run = start(program, "main", &[Arg::Int(width)]).unwrap();
                assert_eq!(
                    wait(program, run),
                    if width <= 2 {
                        (vec![(0, width)], vec![])
                    } else {
                        (vec![], vec![2])
                    }
                );
                assert_eq!(bennett_reverse(program, run), 0);
                assert_eq!(bennett_run_close(program, run), 0);
                let mut full = 0;
                assert_eq!(bennett_qreg_new(program, c"data".as_ptr(), 2, &mut full), 0);
                assert_eq!(bennett_qreg_free(program, full), 0);
                assert_eq!(bennett_unload(program), 0);
            }
        }
    }
}

#[test]
fn failed_register_capacity_is_a_terminal_record_without_allocation_or_ownership() {
    use bennett_vm::json::{self, Json};
    unsafe {
        let source =
            support::source("fn cc @main (n:i64) () () () ()\n^entry():\nq = qreg.new any n\nret");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../bennett-isa-spec/conformance/cases/E/E-matching-first-entry/target.json");
        let mut target = json::parse(std::fs::read_to_string(path).unwrap().trim_end()).unwrap();
        let Json::Obj(fields) = &mut target else {
            panic!("target object");
        };
        fields.insert(
            "classes".into(),
            json::parse(r#"{"fast":{"capacity":1,"dirty_rate":1,"qubit_second":1}}"#).unwrap(),
        );
        let target = target.to_jcs();
        let mut program = ptr::null_mut();
        assert_eq!(
            bennett_load(
                source.as_ptr(),
                source.len(),
                target.as_ptr(),
                target.len(),
                &mut program
            ),
            0,
            "{}",
            std::ffi::CStr::from_ptr(bennett_last_error()).to_string_lossy()
        );
        let mut occupied = 0;
        assert_eq!(
            bennett_qreg_new(program, c"fast".as_ptr(), 1, &mut occupied),
            0
        );
        let failed = start(program, "main", &[Arg::Int(1)]).unwrap();
        assert_eq!(wait(program, failed), (vec![], vec![2]));
        // A failed allocation does not acquire the unrelated existing handle.
        assert_eq!(bennett_qreg_free(program, occupied), 0);
        let mut replacement = 0;
        assert_eq!(
            bennett_qreg_new(program, c"fast".as_ptr(), 1, &mut replacement),
            0
        );
        assert_ne!(replacement, occupied);
        assert_eq!(bennett_reverse(program, failed), 0);
        assert_eq!(bennett_run_close(program, failed), 0);
        assert_eq!(bennett_qreg_free(program, replacement), 0);
        assert_eq!(bennett_unload(program), 0);
    }
}
