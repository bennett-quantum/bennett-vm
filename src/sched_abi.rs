//! Implements the 15 declarations consumed from published bennett_sched.h.
//! Scheduler policy belongs to bennett-sched; no successful schedule is invented
//! when the mandatory default expansion contract cannot be implemented.
#![allow(
    non_camel_case_types,
    unsafe_op_in_unsafe_fn,
    clippy::missing_safety_doc
)]
use crate::{
    identity::Identities,
    json::{self, Json},
    launch::{bytes, error, get, get_mut, output, status, text},
    object::Object,
    target::{Rational, Target},
    value::Budget,
};
use std::{
    collections::BTreeMap,
    ffi::{CString, c_char},
    ptr,
};
#[derive(Default)]
pub struct bennett_sched_request_t {
    object: Option<(Object, Identities)>,
    target: Option<Target>,
    region: Option<(String, String, u32)>,
    budget: Option<Budget>,
    params: Option<Json>,
    accuracy: Option<Rational>,
    capacity: BTreeMap<String, u64>,
}
pub struct bennett_sched_response_t {
    outcome: i32,
    reason: CString,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_request_new(out: *mut *mut bennett_sched_request_t) -> i32 {
    status(|| output(out, bennett_sched_request_t::default()))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_request_set_object(
    r: *mut bennett_sched_request_t,
    p: *const u8,
    n: usize,
) -> i32 {
    status(|| {
        let r = get_mut(r)?;
        let s = std::str::from_utf8(bytes(p, n)?)
            .map_err(|_| error(11, "§16: invalid UTF-8 object"))?;
        let o = crate::load(s, false).map_err(|e| error(11, e))?;
        r.object = Some(o);
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_request_set_target(
    r: *mut bennett_sched_request_t,
    p: *const u8,
    n: usize,
) -> i32 {
    status(|| {
        let r = get_mut(r)?;
        let s =
            std::str::from_utf8(bytes(p, n)?).map_err(|_| error(5, "§8: invalid UTF-8 target"))?;
        let target = Target::parse(s).map_err(|e| error(5, e))?;
        r.target = Some(target);
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_request_set_region(
    r: *mut bennett_sched_request_t,
    closure: *const c_char,
    variant: *const c_char,
    controls: u32,
) -> i32 {
    status(|| {
        let r = get_mut(r)?;
        let hash = text(closure)?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(error(11, "§11: invalid closure_hash_hex"));
        }
        let v = text(variant)?;
        if v.is_empty() {
            return Err(error(11, "§11: empty variant"));
        }
        r.region = Some((hash.into(), v.into(), controls));
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_request_set_budget(
    r: *mut bennett_sched_request_t,
    steps: u64,
    heap: u64,
    stack: u64,
    depth: u64,
) -> i32 {
    status(|| {
        get_mut(r)?.budget = Some(Budget {
            steps,
            heap,
            stack,
            depth,
        });
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_request_set_params(
    r: *mut bennett_sched_request_t,
    p: *const u8,
    n: usize,
) -> i32 {
    status(|| {
        let r = get_mut(r)?;
        let s = std::str::from_utf8(bytes(p, n)?)
            .map_err(|_| error(11, "§11: invalid params UTF-8"))?;
        let j = json::parse(s).map_err(|e| error(11, e))?;
        if j.to_jcs() != s {
            return Err(error(11, "§11: params must be JCS"));
        }
        r.params = Some(j);
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_request_set_accuracy(
    r: *mut bennett_sched_request_t,
    e: *const c_char,
) -> i32 {
    status(|| {
        let r = get_mut(r)?;
        let e = text(e)?.parse::<Rational>().map_err(|e| error(11, e))?;
        if e.negative() || e > Rational::from(1) {
            return Err(error(11, "§11: epsilon outside [0,1]"));
        }
        r.accuracy = Some(e);
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_request_set_capacity(
    r: *mut bennett_sched_request_t,
    c: *const c_char,
    n: u64,
) -> i32 {
    status(|| {
        let r = get_mut(r)?;
        let c = text(c)?;
        if c.is_empty() || c == "any" {
            return Err(error(11, "§11: capacity requires concrete class"));
        }
        r.capacity.insert(c.into(), n);
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_request_free(r: *mut bennett_sched_request_t) {
    if !r.is_null() {
        drop(Box::from_raw(r));
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_schedule(
    r: *const bennett_sched_request_t,
    out: *mut *mut bennett_sched_response_t,
) -> i32 {
    status(|| {
        *get_mut(out)? = ptr::null_mut();
        let r = get(r)?;
        let (object, ids) = r
            .object
            .as_ref()
            .ok_or(error(11, "§11: request requires object"))?;
        let target = r
            .target
            .as_ref()
            .ok_or(error(11, "§11: request requires target"))?;
        let (hash, _, _) = r
            .region
            .as_ref()
            .ok_or(error(11, "§11: request requires region"))?;
        let name = ids
            .closure
            .iter()
            .find(|(_, h)| h.as_ref() == Some(hash))
            .map(|(name, _)| name)
            .ok_or(error(11, "§11: region closure not in object"))?;
        if !matches!(
            object.function(name).unwrap().kind,
            crate::object::FnKind::Oracle
                | crate::object::FnKind::Action
                | crate::object::FnKind::PermAction
        ) {
            return Err(error(11, "§11: scheduler region must be coherent"));
        }
        let budget = r
            .budget
            .as_ref()
            .ok_or(error(11, "§11: request requires budget"))?;
        crate::cliargs::validate_budget(object, budget).map_err(|e| error(6, e))?;
        r.params
            .as_ref()
            .ok_or(error(11, "§11: request requires params"))?;
        r.accuracy
            .as_ref()
            .ok_or(error(11, "§11: request requires accuracy"))?;
        if !target.supports_ptr(object.ptr_width) {
            return Err(error(2, "§8: unsupported ptr_width"));
        }
        for c in r.capacity.keys() {
            if !target.classes.contains_key(c) {
                return Err(error(5, format!("§11: unknown capacity class {c}")));
            }
        }
        let reason = crate::sched::require_library(None).unwrap_err();
        output(
            out,
            bennett_sched_response_t {
                outcome: 2,
                reason: CString::new(reason).unwrap(),
            },
        )
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_response_outcome(r: *const bennett_sched_response_t) -> i32 {
    get(r).expect("§11: null response").outcome
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_response_reason(
    r: *const bennett_sched_response_t,
) -> *const c_char {
    get(r).expect("§11: null response").reason.as_ptr()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_response_schedule(
    r: *const bennett_sched_response_t,
    n: *mut usize,
) -> *const u8 {
    get(r).expect("§11: null response");
    *get_mut(n).expect("§11: null output length") = 0;
    ptr::null()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_response_certificate(
    r: *const bennett_sched_response_t,
    n: *mut usize,
) -> *const u8 {
    get(r).expect("§11: null response");
    *get_mut(n).expect("§11: null output length") = 0;
    ptr::null()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_sched_response_free(r: *mut bennett_sched_response_t) {
    if !r.is_null() {
        drop(Box::from_raw(r));
    }
}
