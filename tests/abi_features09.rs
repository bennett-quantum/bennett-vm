//! ISA 0.9 §10.1: immutable per-role provider advertisements.
use bennett_vm::abi_features::{bennett_launch_features, bennett_sched_features};
use std::{ptr, slice};
type Query = unsafe extern "C" fn(*mut *const u8, *mut usize) -> i32;
const QUERIES: [(Query, &[u8]); 2] = [
    (
        bennett_launch_features,
        br#"{"abi":"launch/0.9","features":[],"schema":"bennett.abi-features/1"}"#,
    ),
    (
        bennett_sched_features,
        br#"{"abi":"sched/0.9","features":[],"schema":"bennett.abi-features/1"}"#,
    ),
];
#[test]
fn role_queries_are_canonical_stable_and_safe_before_handle_creation() {
    for (query, expected) in QUERIES {
        unsafe {
            let mut bytes = ptr::null();
            let mut len = usize::MAX;
            assert_eq!(query(&mut bytes, &mut len), 0);
            assert_eq!(slice::from_raw_parts(bytes, len), expected);
            for _ in 0..4 {
                let mut next = ptr::null();
                let mut n = 0;
                assert_eq!(query(&mut next, &mut n), 0);
                assert_eq!(next, bytes);
                assert_eq!(n, len);
            }
        }
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(move || unsafe {
                    let mut bytes = ptr::null();
                    let mut len = 0;
                    assert_eq!(query(&mut bytes, &mut len), 0);
                    assert_eq!(slice::from_raw_parts(bytes, len), expected);
                });
            }
        });
    }
}
#[test]
fn null_output_queries_do_not_write_either_output() {
    for (query, _) in QUERIES {
        unsafe {
            let sentinel = ptr::dangling();
            let mut bytes = sentinel;
            let mut len = usize::MAX;
            assert_eq!(query(&mut bytes, ptr::null_mut()), 11);
            assert_eq!(bytes, sentinel);
            assert_eq!(query(ptr::null_mut(), &mut len), 11);
            assert_eq!(len, usize::MAX);
            assert_eq!(query(ptr::null_mut(), ptr::null_mut()), 11);
        }
    }
}

#[test]
fn queries_remain_valid_after_program_and_request_free() {
    use bennett_vm::{launch::*, sched_abi::*};
    let source = crate::source09::source("perm_action @p (t:wire mut) () () () ()\n^entry():\nx t");
    for (query, expected) in QUERIES {
        unsafe {
            let mut bytes = ptr::null();
            let mut len = 0;
            assert_eq!(query(&mut bytes, &mut len), 0);
            let mut program = ptr::null_mut();
            assert_eq!(
                bennett_load(source.as_ptr(), source.len(), ptr::null(), 0, &mut program),
                0
            );
            let mut request = ptr::null_mut();
            assert_eq!(bennett_sched_request_new(&mut request), 0);
            assert_eq!(bennett_unload(program), 0);
            bennett_sched_request_free(request);
            assert_eq!(slice::from_raw_parts(bytes, len), expected);
        }
    }
}
