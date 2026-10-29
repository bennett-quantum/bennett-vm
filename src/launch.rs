//! §10 launch 0.8 C ABI. Calls on one program must be externally synchronized.
//! Cooperative execution: wait drives RUNNING to the next observable boundary.
#![allow(
    non_camel_case_types,
    unsafe_op_in_unsafe_fn,
    clippy::missing_safety_doc
)]
use crate::{
    bigint::BigUint,
    cc::{ResourceKey, Resources, RuntimeRun},
    json::Json,
    object::{FnKind, Object, Type},
    value::{Budget, Value},
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::{CStr, CString, c_char},
    ptr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
const ARG: i32 = 11;
const INVALID: i32 = 8;
const NOT_REV: i32 = 9;
type Fallible<T> = Result<T, (i32, String)>;
fn error(code: i32, msg: impl Into<String>) -> (i32, String) {
    (code, msg.into())
}
thread_local! { static LAST:RefCell<CString>=RefCell::new(CString::new("").unwrap()); }
fn status(f: impl FnOnce() -> Fallible<()>) -> i32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(())) => 0,
        Ok(Err((code, msg))) => {
            LAST.with(|v| *v.borrow_mut() = CString::new(msg.replace('\0', "\\0")).unwrap());
            code
        }
        Err(_) => {
            LAST.with(|v| {
                *v.borrow_mut() = CString::new("internal panic at launch ABI boundary").unwrap()
            });
            INVALID
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn bennett_last_error() -> *const c_char {
    LAST.with(|v| v.borrow().as_ptr())
}
unsafe fn bytes<'a>(p: *const u8, n: usize) -> Fallible<&'a [u8]> {
    if n == 0 {
        return Ok(&[]);
    }
    if p.is_null() || n > isize::MAX as usize {
        return Err(error(ARG, "§10: E_ARG: null/oversized buffer"));
    }
    Ok(std::slice::from_raw_parts(p, n))
}
unsafe fn text<'a>(p: *const c_char) -> Fallible<&'a str> {
    if p.is_null() {
        return Err(error(ARG, "§10: E_ARG: null name"));
    }
    CStr::from_ptr(p)
        .to_str()
        .map_err(|_| error(ARG, "§10: E_ARG: name is not UTF-8"))
}
unsafe fn get<'a, T>(p: *const T) -> Fallible<&'a T> {
    p.as_ref()
        .ok_or_else(|| error(ARG, "§10: E_ARG: null object"))
}
unsafe fn get_mut<'a, T>(p: *mut T) -> Fallible<&'a mut T> {
    p.as_mut()
        .ok_or_else(|| error(ARG, "§10: E_ARG: null object"))
}
unsafe fn output<T>(out: *mut *mut T, v: T) -> Fallible<()> {
    *get_mut(out)? = Box::into_raw(Box::new(v));
    Ok(())
}
fn id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
macro_rules! free {
    ($name:ident,$t:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(p: *mut $t) {
            if !p.is_null() {
                drop(Box::from_raw(p));
            }
        }
    };
}
#[derive(Clone)]
enum Arg {
    Int(usize, BigUint),
    F64(f64),
    Qreg(u64),
    Token(u64),
}
pub struct bennett_args_t {
    values: Vec<Option<Arg>>,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Running,
    Suspended,
    HostRequest,
    Completed,
    Failed,
    Reversed,
    Closed,
}
struct Run {
    state: State,
    runtime: RuntimeRun,
    request: Option<(String, Vec<u8>)>,
}
pub struct bennett_program_t {
    obj: Arc<Object>,
    resources: Resources,
    tokens: BTreeMap<u64, (String, Budget)>,
    runs: BTreeMap<u64, Run>,
    locks: BTreeMap<u64, u64>,
    memory_locks: Vec<(ResourceKey, u64)>,
}
struct Encoded {
    kind: i32,
    width: u32,
    bytes: Vec<u8>,
}
pub struct bennett_result_t {
    values: Vec<Encoded>,
    faults: Vec<u64>,
    measurements: Vec<(u64, Vec<u8>)>,
    tape_length: u64,
}
pub struct bennett_request_t {
    service: CString,
    payload: Vec<u8>,
}
pub struct bennett_tape_iter_t {
    entries: Vec<(CString, CString, Vec<u8>)>,
    next: usize,
}
pub struct bennett_estimate_t {
    _private: (),
}
pub struct bennett_coverage_t {
    _private: (),
}
pub struct bennett_snapshot_t {
    jcs: Vec<u8>,
}
impl bennett_program_t {
    fn run_ref(&self, r: u64) -> Fallible<&Run> {
        self.runs
            .get(&r)
            .filter(|v| v.state != State::Closed)
            .ok_or_else(|| error(INVALID, "§10: E_INVALID_STATE: unknown or CLOSED run"))
    }
    fn run_mut(&mut self, r: u64) -> Fallible<&mut Run> {
        self.runs
            .get_mut(&r)
            .filter(|v| v.state != State::Closed)
            .ok_or_else(|| error(INVALID, "§10: E_INVALID_STATE: unknown or CLOSED run"))
    }
    fn unlocked(&self, q: u64) -> Fallible<()> {
        if self.locks.contains_key(&q) {
            Err(error(
                INVALID,
                "§10: run owns touched register exclusively until CLOSED",
            ))
        } else {
            Ok(())
        }
    }
    fn step(&mut self, r: u64) -> Fallible<()> {
        let run = self
            .runs
            .get_mut(&r)
            .filter(|v| v.state != State::Closed)
            .ok_or_else(|| error(INVALID, "§10: unknown/CLOSED run"))?;
        if run.state != State::Running {
            return Err(error(INVALID, "§10: step requires RUNNING"));
        }
        run.runtime.sync(&self.resources);
        let (memory, regs) = self.runs[&r].runtime.touches();
        if memory.iter().any(|key| {
            self.memory_locks
                .iter()
                .any(|(other, owner)| *owner != r && key.overlaps(other))
        }) || regs
            .iter()
            .any(|q| self.locks.get(q).is_some_and(|owner| *owner != r))
        {
            return Err(error(
                INVALID,
                "§10: resources owned exclusively by another non-CLOSED run",
            ));
        }
        for key in memory {
            if !self
                .memory_locks
                .iter()
                .any(|(k, owner)| *owner == r && *k == key)
            {
                self.memory_locks.push((key, r));
            }
        }
        for q in regs {
            self.locks.insert(q, r);
        }
        let run = self.runs.get_mut(&r).unwrap();
        if let Some(request) = run.runtime.request() {
            run.request = Some(request);
            run.state = State::HostRequest;
            return Ok(());
        }
        run.runtime.step().map_err(|s| error(INVALID, s))?;
        run.runtime.export(&mut self.resources);
        if run.runtime.terminal() {
            run.state = if run.runtime.fault().is_some() {
                State::Failed
            } else {
                State::Completed
            };
        }
        for q in run.runtime.created_regs() {
            self.locks.insert(q, r);
        }
        // New registers created by a run also remain exclusively owned.
        for v in run.runtime.values() {
            if let Value::Qreg(q) = v {
                self.locks.insert(*q, r);
            }
        }
        Ok(())
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_load(
    object: *const u8,
    len: usize,
    target: *const u8,
    tlen: usize,
    out: *mut *mut bennett_program_t,
) -> i32 {
    status(|| {
        *get_mut(out)? = ptr::null_mut();
        let raw = std::str::from_utf8(bytes(object, len)?)
            .map_err(|_| error(ARG, "§16: invalid UTF-8 object"))?;
        bytes(target, tlen)?;
        let (obj, _) = crate::load(raw, false).map_err(|s| {
            let code = if s.contains("version") {
                1
            } else if s.contains("ptr_width") {
                2
            } else if s.contains("device") || s.contains("service") {
                3
            } else {
                ARG
            };
            error(code, s)
        })?;
        if tlen != 0 {
            return Err(error(
                5,
                "unimplemented: target (§8 launch lowering) (milestone 4)",
            ));
        }
        if obj.has_schedule {
            return Err(error(
                10,
                "unimplemented: schedule (§11 verbatim consumption) (milestone 4)",
            ));
        }
        let resources = Resources::new(&obj).map_err(|s| error(ARG, s))?;
        output(
            out,
            bennett_program_t {
                obj: Arc::new(obj),
                resources,
                tokens: BTreeMap::new(),
                runs: BTreeMap::new(),
                locks: BTreeMap::new(),
                memory_locks: vec![],
            },
        )
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_unload(p: *mut bennett_program_t) -> i32 {
    status(|| {
        if get(p)?.runs.values().any(|r| r.state != State::Closed) {
            return Err(error(INVALID, "§10: bennett_unload with a run not CLOSED"));
        }
        drop(Box::from_raw(p));
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_budget(
    p: *mut bennett_program_t,
    region: *const c_char,
    steps: u64,
    heap: u64,
    stack: u64,
    depth: u64,
    out: *mut u64,
) -> i32 {
    status(|| {
        let p = get_mut(p)?;
        let name = text(region)?.trim_start_matches('@');
        get_mut(out)?;
        let f = p
            .obj
            .function(name)
            .ok_or_else(|| error(ARG, "§10: unknown budget region"))?;
        if !matches!(f.kind, FnKind::Oracle | FnKind::Action) {
            return Err(error(ARG, "§10: budget requires an oracle/action region"));
        }
        let b = Budget {
            steps,
            heap,
            stack,
            depth,
        };
        crate::cliargs::validate_budget(&p.obj, &b).map_err(|s| error(6, s))?;
        let token = id();
        p.tokens.insert(token, (name.into(), b));
        *out = token;
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_qreg_new(
    p: *mut bennett_program_t,
    cls: *const c_char,
    n: u32,
    out: *mut u64,
) -> i32 {
    status(|| {
        let p = get_mut(p)?;
        let cls = text(cls)?;
        get_mut(out)?;
        if cls != "any" {
            return Err(error(
                5,
                "unimplemented: concrete target register classes (milestone 4)",
            ));
        }
        if n == 0 {
            return Err(error(ARG, "§3.3: empty register"));
        }
        *out = p.resources.new_reg(n as usize);
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_qreg_prepare(
    p: *mut bennett_program_t,
    q: u64,
    bits: *const u8,
    nbits: usize,
) -> i32 {
    status(|| {
        let p = get_mut(p)?;
        p.unlocked(q)?;
        let bs = bytes(bits, nbits)?;
        if p.resources.reg_width(q) != Some(nbits) || bs.iter().any(|b| *b > 1) {
            return Err(error(ARG, "§10: register width/bits mismatch"));
        }
        if !p.resources.clean(q) {
            return Err(error(7, "§10: E_QREG_NOT_CLEAN"));
        }
        p.resources.prepare(q, bs);
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_qreg_free(p: *mut bennett_program_t, q: u64) -> i32 {
    status(|| {
        let p = get_mut(p)?;
        p.unlocked(q)?;
        if p.resources.reg_width(q).is_none() {
            return Err(error(ARG, "§10: invalid register"));
        }
        if !p.resources.clean(q) {
            return Err(error(7, "§10: E_QREG_NOT_CLEAN"));
        }
        p.resources.free(q);
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_args_new(n: usize, out: *mut *mut bennett_args_t) -> i32 {
    status(|| {
        get_mut(out)?;
        let mut values = vec![];
        values
            .try_reserve_exact(n)
            .map_err(|_| error(12, "§10: E_NOMEM"))?;
        values.resize(n, None);
        output(out, bennett_args_t { values })
    })
}
unsafe fn set_arg(p: *mut bennett_args_t, i: usize, arg: Arg) -> Fallible<()> {
    let slot = get_mut(p)?
        .values
        .get_mut(i)
        .ok_or_else(|| error(ARG, "§10: argument index out of range"))?;
    *slot = Some(arg);
    Ok(())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_args_set_int(
    p: *mut bennett_args_t,
    i: usize,
    width: u32,
    le: *const u8,
) -> i32 {
    status(|| {
        if width == 0 {
            return Err(error(ARG, "§10: zero integer width"));
        }
        let mut v = BigUint::zero();
        for (i, b) in bytes(le, (width as usize).div_ceil(8))?.iter().enumerate() {
            v = v.bitor(&BigUint::from_u64(*b as u64).shl(8 * i));
        }
        set_arg(p, i, Arg::Int(width as usize, v.mask(width as usize)))
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_args_set_f64(p: *mut bennett_args_t, i: usize, v: f64) -> i32 {
    status(|| set_arg(p, i, Arg::F64(v)))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_args_set_qreg(p: *mut bennett_args_t, i: usize, q: u64) -> i32 {
    status(|| set_arg(p, i, Arg::Qreg(q)))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_args_set_token(p: *mut bennett_args_t, i: usize, t: u64) -> i32 {
    status(|| set_arg(p, i, Arg::Token(t)))
}
free!(bennett_args_free, bennett_args_t);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_run(
    p: *mut bennett_program_t,
    entry: *const c_char,
    args: *const bennett_args_t,
    entropy: *const u8,
    entropy_len: usize,
    out: *mut u64,
) -> i32 {
    status(|| {
        let p = get_mut(p)?;
        let name = text(entry)?.trim_start_matches('@');
        let args = get(args)?;
        get_mut(out)?;
        let f = p
            .obj
            .function(name)
            .filter(|f| f.kind == FnKind::Cc)
            .ok_or_else(|| error(ARG, "§10: run requires CC entry"))?;
        if f.lists[0].len() != args.values.len() {
            return Err(error(ARG, "§10: positional argument count differs"));
        }
        let mut values = vec![];
        let mut regs = vec![];
        for (param, arg) in f.lists[0].iter().zip(&args.values) {
            values.push(match (&param.ty, arg) {
                (Type::I(n), Some(Arg::Int(w, b))) if n == w => Value::int(*n, b.clone()),
                (Type::F64, Some(Arg::F64(v))) => Value::F64(*v),
                (Type::Qreg(n), Some(Arg::Qreg(q))) if p.resources.reg_width(*q) == Some(*n) => {
                    p.unlocked(*q)?;
                    regs.push(*q);
                    Value::Qreg(*q)
                }
                (Type::Token, Some(Arg::Token(t))) => Value::TokenId(
                    *t,
                    p.tokens
                        .get(t)
                        .ok_or_else(|| error(ARG, "§10: token not scoped to this program"))?
                        .1,
                ),
                _ => return Err(error(ARG, "§10: positional argument type/width differs")),
            });
        }
        if regs.iter().collect::<std::collections::BTreeSet<_>>().len() != regs.len() {
            return Err(error(ARG, "§3.3: aliased register operands"));
        }
        let entropy = bytes(entropy, entropy_len)?.to_vec();
        let r = id();
        let runtime = RuntimeRun::new(p.obj.clone(), &p.resources, name, values, entropy);
        for q in regs {
            p.locks.insert(q, r);
        }
        p.runs.insert(
            r,
            Run {
                state: State::Running,
                runtime,
                request: None,
            },
        );
        *out = r;
        Ok(())
    })
}
impl bennett_result_t {
    fn from_run(p: &mut bennett_program_t, r: u64) -> Self {
        let run = &p.runs[&r];
        let mut values = vec![];
        if run.state == State::Completed {
            for v in run.runtime.values() {
                let (kind, width, bytes) = match v {
                    Value::Int { width, bits } => (
                        0,
                        *width as u32,
                        (0..width.div_ceil(8))
                            .map(|i| bits.shr(i * 8).mask(8).to_u64().unwrap() as u8)
                            .collect(),
                    ),
                    Value::Ptr(bits) => (
                        0,
                        p.obj.ptr_width as u32,
                        (0..p.obj.ptr_width.div_ceil(8))
                            .map(|i| bits.shr(i * 8).mask(8).to_u64().unwrap() as u8)
                            .collect(),
                    ),
                    Value::F64(v) => (1, 64, v.to_le_bytes().to_vec()),
                    Value::Qreg(q) => (2, 64, q.to_le_bytes().to_vec()),
                    Value::TokenId(t, _) => (3, 64, t.to_le_bytes().to_vec()),
                    Value::Token(b) => {
                        let t = p
                            .tokens
                            .iter()
                            .find(|(_, (_, v))| v == b)
                            .map(|(k, _)| *k)
                            .unwrap_or_else(id);
                        (3, 64, t.to_le_bytes().to_vec())
                    }
                };
                values.push(Encoded { kind, width, bytes });
            }
        }
        let measurements = run
            .runtime
            .measurements()
            .iter()
            .map(|v| {
                (
                    v.get("reg").unwrap().as_u64().unwrap(),
                    v.get("bits")
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .bytes()
                        .map(|b| b - b'0')
                        .collect(),
                )
            })
            .collect();
        Self {
            values,
            faults: run.runtime.fault().into_iter().collect(),
            measurements,
            tape_length: run.runtime.tape().len() as u64,
        }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_wait(
    p: *mut bennett_program_t,
    r: u64,
    result_out: *mut *mut bennett_result_t,
    request_out: *mut *mut bennett_request_t,
) -> i32 {
    status(|| {
        get_mut(result_out)?;
        get_mut(request_out)?;
        let p = get_mut(p)?;
        if !matches!(
            p.run_ref(r)?.state,
            State::Running | State::HostRequest | State::Completed | State::Failed
        ) {
            return Err(error(INVALID, "§10: wait on SUSPENDED/REVERSED/CLOSED"));
        }
        while p.runs[&r].state == State::Running {
            p.step(r)?;
        }
        *result_out = ptr::null_mut();
        *request_out = ptr::null_mut();
        if let Some((s, b)) = &p.runs[&r].request {
            output(
                request_out,
                bennett_request_t {
                    service: CString::new(s.as_str()).unwrap(),
                    payload: b.clone(),
                },
            )
        } else {
            let result = bennett_result_t::from_run(p, r);
            output(result_out, result)
        }
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_reply(
    p: *mut bennett_program_t,
    r: u64,
    payload: *const u8,
    len: usize,
) -> i32 {
    status(|| {
        let bytes = bytes(payload, len)?;
        let run = get_mut(p)?.run_mut(r)?;
        if run.state != State::HostRequest {
            return Err(error(INVALID, "§10: reply requires HOST_REQUEST"));
        }
        run.runtime.reply(bytes).map_err(|s| error(INVALID, s))?;
        run.request = None;
        run.state = State::Running;
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_suspend(p: *mut bennett_program_t, r: u64) -> i32 {
    status(|| {
        let run = get_mut(p)?.run_mut(r)?;
        if run.state != State::Running {
            return Err(error(INVALID, "§10: suspend requires RUNNING"));
        }
        run.state = State::Suspended;
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_resume(p: *mut bennett_program_t, r: u64) -> i32 {
    status(|| {
        let run = get_mut(p)?.run_mut(r)?;
        if run.state != State::Suspended {
            return Err(error(INVALID, "§10: resume requires SUSPENDED"));
        }
        run.state = State::Running;
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_reverse(p: *mut bennett_program_t, r: u64) -> i32 {
    status(|| {
        let p = get_mut(p)?;
        let resources = &mut p.resources;
        let run = p
            .runs
            .get_mut(&r)
            .ok_or_else(|| error(INVALID, "§10: unknown run"))?;
        if run.state != State::Completed {
            return Err(error(INVALID, "§10: reverse requires COMPLETED"));
        }
        if run.runtime.measured() {
            return Err(error(
                NOT_REV,
                "§10: E_NOT_REVERSIBLE: run contains measure",
            ));
        }
        run.runtime.sync(resources);
        run.runtime.reverse_to(0).map_err(|s| error(NOT_REV, s))?;
        run.runtime.export(resources);
        run.state = State::Reversed;
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_run_close(p: *mut bennett_program_t, r: u64) -> i32 {
    status(|| {
        let p = get_mut(p)?;
        let run = p.run_mut(r)?;
        if !matches!(
            run.state,
            State::Completed | State::Failed | State::Reversed
        ) {
            return Err(error(
                INVALID,
                "§10: close requires COMPLETED/FAILED/REVERSED",
            ));
        }
        run.state = State::Closed;
        p.runs.remove(&r);
        p.locks.retain(|_, owner| *owner != r);
        p.memory_locks.retain(|(_, owner)| *owner != r);
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_result_nvalues(p: *const bennett_result_t) -> usize {
    assert!(!p.is_null(), "§10: null result");
    (*p).values.len()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_result_nfaults(p: *const bennett_result_t) -> usize {
    assert!(!p.is_null(), "§10: null result");
    (*p).faults.len()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_result_fault(p: *const bennett_result_t, i: usize) -> u64 {
    assert!(!p.is_null(), "§10: null result");
    (&(*p).faults)[i]
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_result_nmeasurements(p: *const bennett_result_t) -> usize {
    assert!(!p.is_null(), "§10: null result");
    (*p).measurements.len()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_result_tape_length(p: *const bennett_result_t) -> u64 {
    assert!(!p.is_null(), "§10: null result");
    (*p).tape_length
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_result_get(
    p: *const bennett_result_t,
    i: usize,
    kind: *mut i32,
    width: *mut u32,
    out: *mut u8,
    cap: usize,
) -> i32 {
    status(|| {
        let v = get(p)?
            .values
            .get(i)
            .ok_or_else(|| error(ARG, "§10: result index out of range"))?;
        get_mut(kind)?;
        get_mut(width)?;
        if cap < v.bytes.len() || out.is_null() {
            return Err(error(ARG, "§10: result capacity too small/null"));
        }
        ptr::copy_nonoverlapping(v.bytes.as_ptr(), out, v.bytes.len());
        *kind = v.kind;
        *width = v.width;
        Ok(())
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_result_measurement(
    p: *const bennett_result_t,
    i: usize,
    reg: *mut u64,
    out: *mut u8,
    cap: usize,
    nbits: *mut usize,
) -> i32 {
    status(|| {
        let (q, b) = get(p)?
            .measurements
            .get(i)
            .ok_or_else(|| error(ARG, "§10: measurement index out of range"))?;
        get_mut(reg)?;
        get_mut(nbits)?;
        if cap < b.len() || out.is_null() {
            return Err(error(ARG, "§10: measurement capacity too small/null"));
        }
        ptr::copy_nonoverlapping(b.as_ptr(), out, b.len());
        *reg = *q;
        *nbits = b.len();
        Ok(())
    })
}
free!(bennett_result_free, bennett_result_t);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_request_service(p: *const bennett_request_t) -> *const c_char {
    assert!(!p.is_null(), "§10: null request");
    (*p).service.as_ptr()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_request_payload(
    p: *const bennett_request_t,
    len: *mut usize,
) -> *const u8 {
    assert!(!p.is_null() && !len.is_null(), "§10: null request/length");
    *len = (*p).payload.len();
    (*p).payload.as_ptr()
}
free!(bennett_request_free, bennett_request_t);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_tape_open(
    p: *mut bennett_program_t,
    r: u64,
    out: *mut *mut bennett_tape_iter_t,
) -> i32 {
    status(|| {
        let run = get(p)?.run_ref(r)?;
        let entries = run
            .runtime
            .tape()
            .iter()
            .map(|v| {
                let dev = CString::new(v.get("dev").unwrap().as_str().unwrap()).unwrap();
                let op = CString::new(v.get("op").unwrap().as_str().unwrap()).unwrap();
                let Json::Arr(args) = v.get("args").unwrap() else {
                    unreachable!()
                };
                let bytes = args
                    .iter()
                    .flat_map(|v| v.as_u64().unwrap().to_le_bytes())
                    .collect();
                (dev, op, bytes)
            })
            .collect();
        output(out, bennett_tape_iter_t { entries, next: 0 })
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_tape_next(
    p: *mut bennett_tape_iter_t,
    dev: *mut *const c_char,
    op: *mut *const c_char,
    args: *mut *const u8,
    alen: *mut usize,
) -> i32 {
    status(|| {
        let p = get_mut(p)?;
        get_mut(dev)?;
        get_mut(op)?;
        get_mut(args)?;
        get_mut(alen)?;
        let (d, o, b) = p.entries.get(p.next).ok_or_else(|| {
            error(
                INVALID,
                "§10: end of effect tape (ABI defines no end sentinel; E_INVALID_STATE)",
            )
        })?;
        *dev = d.as_ptr();
        *op = o.as_ptr();
        *args = b.as_ptr();
        *alen = b.len();
        p.next += 1;
        Ok(())
    })
}
free!(bennett_tape_free, bennett_tape_iter_t);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_estimate_run(
    p: *mut bennett_program_t,
    region: *const c_char,
    token: u64,
    out: *mut *mut bennett_estimate_t,
) -> i32 {
    status(|| {
        let p = get(p)?;
        text(region)?;
        *get_mut(out)? = ptr::null_mut();
        if !p.tokens.contains_key(&token) {
            return Err(error(ARG, "§10: unknown token"));
        }
        Err(error(
            5,
            "unimplemented: estimates/certificates (§8/§12) (milestone 4–5)",
        ))
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_estimate_nresources(_p: *const bennett_estimate_t) -> usize {
    eprintln!("unimplemented: estimates (milestone 4–5)");
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_estimate_get(
    _p: *const bennett_estimate_t,
    _i: usize,
    _name: *mut *const c_char,
    _value: *mut *const c_char,
) -> i32 {
    status(|| Err(error(5, "unimplemented: estimates (milestone 4–5)")))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_estimate_certificate(
    _p: *const bennett_estimate_t,
    len: *mut usize,
) -> *const u8 {
    if !len.is_null() {
        *len = 0
    }
    eprintln!("unimplemented: estimate certificates (milestone 4–5)");
    ptr::null()
}
free!(bennett_estimate_free, bennett_estimate_t);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_coverage_get(
    p: *mut bennett_program_t,
    out: *mut *mut bennett_coverage_t,
) -> i32 {
    status(|| {
        get(p)?;
        *get_mut(out)? = ptr::null_mut();
        Err(error(5, "unimplemented: coverage (§13) (milestone 4–5)"))
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_coverage_jcs(
    _p: *const bennett_coverage_t,
    len: *mut usize,
) -> *const u8 {
    if !len.is_null() {
        *len = 0
    }
    eprintln!("unimplemented: coverage (milestone 4–5)");
    ptr::null()
}
free!(bennett_coverage_free, bennett_coverage_t);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_run_step(p: *mut bennett_program_t, r: u64) -> i32 {
    status(|| get_mut(p)?.step(r))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_run_snapshot(
    p: *mut bennett_program_t,
    r: u64,
    out: *mut *mut bennett_snapshot_t,
) -> i32 {
    status(|| {
        let run = get(p)?.run_ref(r)?;
        output(
            out,
            bennett_snapshot_t {
                jcs: run
                    .runtime
                    .snapshot(&format!("{:?}", run.state))
                    .to_jcs()
                    .into_bytes(),
            },
        )
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_snapshot_jcs(
    p: *const bennett_snapshot_t,
    len: *mut usize,
) -> *const u8 {
    assert!(!p.is_null() && !len.is_null(), "null snapshot/length");
    *len = (*p).jcs.len();
    (*p).jcs.as_ptr()
}
free!(bennett_snapshot_free, bennett_snapshot_t);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bennett_run_reverse_to(p: *mut bennett_program_t, r: u64, n: u64) -> i32 {
    status(|| {
        let p = get_mut(p)?;
        let resources = &mut p.resources;
        let run = p
            .runs
            .get_mut(&r)
            .ok_or_else(|| error(INVALID, "unknown run"))?;
        if !matches!(
            run.state,
            State::Suspended | State::Completed | State::Failed
        ) {
            return Err(error(
                INVALID,
                "reverse_to requires SUSPENDED/COMPLETED/FAILED",
            ));
        }
        let n = usize::try_from(n).map_err(|_| error(ARG, "history index overflow"))?;
        run.runtime.sync(resources);
        run.runtime.reverse_to(n).map_err(|s| error(NOT_REV, s))?;
        run.runtime.export(resources);
        run.request = None;
        run.state = State::Suspended;
        Ok(())
    })
}
pub fn cmd_run_program(args: &[String]) -> Result<(), String> {
    crate::cc::cmd_program(args)
}
