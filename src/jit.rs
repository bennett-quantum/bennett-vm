//! ISA §5.7 private executor IR. Source rows, canonical labels, budgets and
//! activation records are never deleted. Native code threads runtime row
//! helpers; these own allocation, effects, history and arbitrary-width values.
use crate::{
    bigint::BigUint,
    object::{FnKind, Instr, Object},
    value,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Passes {
    pub inline: bool,
    pub structure: bool,
    pub reindex: bool,
    pub simplify: bool,
}
impl Passes {
    pub fn all() -> Self {
        Self {
            inline: true,
            structure: true,
            reindex: true,
            simplify: true,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub inlined_rows: usize,
    pub structured_edges: usize,
    pub reindexed_rows: usize,
    pub simplified_rows: usize,
}
#[derive(Clone)]
struct Node {
    source: usize,
    inlined: bool,
    ins: Instr,
    // (canonical successor, private node). A miss always dispatches canonically.
    edges: Vec<(usize, usize)>,
}
static NATIVE_ROWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NATIVE_ARITHMETIC: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NATIVE_RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn audit() {
    if std::env::var_os("BENNETT_JIT_AUDIT").is_some() {
        eprintln!(
            "JIT_AUDIT native_runs={} native_rows={} native_arithmetic={}",
            NATIVE_RUNS.load(std::sync::atomic::Ordering::Relaxed),
            NATIVE_ROWS.load(std::sync::atomic::Ordering::Relaxed),
            NATIVE_ARITHMETIC.load(std::sync::atomic::Ordering::Relaxed)
        );
    }
}
pub fn native_binop(op: &str, a: u64, b: u64, width: usize) -> Result<u64, String> {
    if !(1..=64).contains(&width) {
        return Err(
            "§5.7: machine integer kernel requires 1..64 bits; use bigint runtime helper".into(),
        );
    }
    NATIVE_ARITHMETIC.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    native::arithmetic(op, a, b, width)
}
pub struct Program {
    keys: BTreeMap<(String, usize, usize), usize>,
    nodes: Vec<Node>,
    stats: Stats,
    native: Option<native::Code>,
}

/// Explicit opt-in selection, inherited by CLI and C ABI. Invalid selections
/// fail before execution; there is no automatic interpreter fallback.
pub fn selection() -> Result<Option<(Passes, bool)>, String> {
    let mode = std::env::var("BENNETT_VM_ENGINE").unwrap_or_else(|_| "interpreter".into());
    Ok(match mode.as_str() {
        "interpreter" => None,
        "baseline" => Some((Passes::default(), false)),
        "inline" => Some((
            Passes {
                inline: true,
                ..Passes::default()
            },
            false,
        )),
        "structure" => Some((
            Passes {
                structure: true,
                ..Passes::default()
            },
            false,
        )),
        "reindex" => Some((
            Passes {
                reindex: true,
                ..Passes::default()
            },
            false,
        )),
        "simplify" => Some((
            Passes {
                simplify: true,
                ..Passes::default()
            },
            false,
        )),
        "optimised" => Some((Passes::all(), false)),
        "jit" => Some((Passes::all(), true)),
        _ => {
            return Err(format!(
                "unimplemented: execution engine {mode} (milestone 5)"
            ));
        }
    })
}
thread_local! {
    // One object per thread, bounded; key uses complete parsed representation,
    // including programmatically extended functions used by library consumers.
    static CACHE: RefCell<Option<(String, String, Arc<Program>)>> = const { RefCell::new(None) };
}
pub fn for_object(obj: &Object) -> Result<Option<Arc<Program>>, String> {
    let Some((passes, native)) = selection()? else {
        return Ok(None);
    };
    let key = format!("{obj:?}");
    let mode = format!("{passes:?}/{native}");
    CACHE.with(|cache| {
        if let Some((k, m, p)) = cache.borrow().as_ref()
            && *k == key
            && *m == mode
        {
            return Ok(Some(p.clone()));
        }
        let p = Arc::new(Program::compile(obj, passes, native)?);
        *cache.borrow_mut() = Some((key, mode, p.clone()));
        Ok(Some(p))
    })
}
impl Program {
    pub fn stats(&self) -> &Stats {
        &self.stats
    }
    pub fn row(&self, function: &str, block: usize, index: usize) -> Result<usize, String> {
        self.keys
            .get(&(function.into(), block, index))
            .copied()
            .ok_or_else(|| format!("§5.7: no compiled row @{function}/{block}/{index}"))
    }
    pub fn compile(obj: &Object, passes: Passes, host: bool) -> Result<Self, String> {
        let mut p = Self {
            keys: BTreeMap::new(),
            nodes: vec![],
            stats: Stats::default(),
            native: None,
        };
        let mut ranges = BTreeMap::new();
        for f in &obj.functions {
            if !matches!(
                f.kind,
                FnKind::Cc | FnKind::R | FnKind::Oracle | FnKind::Lproc
            ) {
                continue;
            }
            let start = p.nodes.len();
            for (b, block) in f.blocks.iter().enumerate() {
                for (i, ins) in block.instrs.iter().enumerate() {
                    let id = p.nodes.len();
                    p.keys.insert((f.name.clone(), b, i), id);
                    p.nodes.push(Node {
                        source: id,
                        inlined: false,
                        ins: ins.clone(),
                        edges: vec![],
                    });
                }
            }
            ranges.insert(f.name.clone(), start..p.nodes.len());
        }
        // Recover CFG continuations, including both arms and bounded/irreducible
        // cycles. Dynamic returns/jumpr retain the canonical dispatch escape.
        if passes.structure || passes.inline {
            for f in &obj.functions {
                if !ranges.contains_key(&f.name) {
                    continue;
                }
                for (b, block) in f.blocks.iter().enumerate() {
                    for (i, ins) in block.instrs.iter().enumerate() {
                        let id = p.row(&f.name, b, i)?;
                        let mut dests = vec![];
                        if i + 1 < block.instrs.len() {
                            dests.push(p.row(&f.name, b, i + 1)?);
                        }
                        if matches!(ins.op.as_str(), "br" | "qjump" | "qbranch") {
                            for operand in &ins.operands {
                                if let Some(label) = operand.strip_prefix('^') {
                                    let label = label.split('(').next().unwrap();
                                    let bi =
                                        f.blocks.iter().position(|b| b.name == label).ok_or_else(
                                            || format!("§5.7: unresolved label ^{label}"),
                                        )?;
                                    dests.push(p.row(&f.name, bi, 0)?);
                                }
                            }
                        }
                        if matches!(ins.op.as_str(), "call" | "lift.call")
                            && let Some(range) = ranges.get(ins.operands[0].trim_start_matches('@'))
                        {
                            dests.push(range.start);
                        }
                        if passes.structure {
                            p.stats.structured_edges += dests.len();
                        }
                        p.nodes[id].edges = dests.into_iter().map(|d| (d, d)).collect();
                    }
                }
            }
        }
        // Inline one level of procedure bodies at each call site. Runtime call/return
        // helpers remain to retain §3.7 limits, monotonic stack and §7.4 frames.
        // Calls inside a clone dispatch normally; recursion is never unrolled.
        if passes.inline {
            let original = p.nodes.len();
            for id in 0..original {
                let ins = &p.nodes[id].ins;
                if !matches!(ins.op.as_str(), "call" | "lift.call") {
                    continue;
                }
                let Some(range) = ranges.get(ins.operands[0].trim_start_matches('@')) else {
                    continue;
                };
                let offset = p.nodes.len();
                let mut clones = p.nodes[range.clone()].to_vec();
                for node in &mut clones {
                    node.inlined = true;
                    node.edges.retain(|(_, target)| range.contains(target));
                    for (_, target) in &mut node.edges {
                        *target = offset + *target - range.start;
                    }
                    if matches!(node.ins.op.as_str(), "ret" | "lift.ret") && id + 1 < original {
                        node.edges.push((id + 1, id + 1));
                    }
                }
                p.stats.inlined_rows += clones.len();
                p.nodes.extend(clones);
                p.nodes[id]
                    .edges
                    .retain(|(source, _)| *source != range.start);
                p.nodes[id].edges.push((range.start, offset));
            }
        }
        if passes.simplify {
            for f in &obj.functions {
                if !ranges.contains_key(&f.name) {
                    continue;
                }
                for (b, block) in f.blocks.iter().enumerate() {
                    let mut known: BTreeMap<String, (usize, BigUint)> = BTreeMap::new();
                    for (i, ins) in block.instrs.iter().enumerate() {
                        let base = ins.op.strip_prefix("lift.").unwrap_or(&ins.op);
                        let folded = if let Some(w) = base.strip_prefix("const.i") {
                            let w = w.parse::<usize>().map_err(|e| e.to_string())?;
                            Some((w, crate::oracle::parse_const(&ins.operands[0], w)?))
                        } else if matches!(
                            base,
                            "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr"
                        ) {
                            match (known.get(&ins.operands[0]), known.get(&ins.operands[1])) {
                                (Some((w, a)), Some((v, b))) if w == v => {
                                    Some((*w, value::binop(base, a, b, *w)))
                                }
                                _ => None,
                            }
                        } else {
                            None
                        };
                        // Calls can overwrite any lifted location via activation
                        // aliasing; parmov reads all old sources before writes.
                        if matches!(base, "call" | "oracle_call" | "oracle_uncall" | "perm_call") {
                            known.clear();
                        }
                        for name in &ins.lhs {
                            known.remove(name);
                        }
                        if ins.lhs.len() == 1
                            && let Some((w, value)) = folded
                        {
                            known.insert(ins.lhs[0].clone(), (w, value.clone()));
                            if !base.starts_with("const.") {
                                let source = p.row(&f.name, b, i)?;
                                for node in p.nodes.iter_mut().filter(|n| n.source == source) {
                                    node.ins.op = format!(
                                        "{}const.i{w}",
                                        if ins.op.starts_with("lift.") {
                                            "lift."
                                        } else {
                                            ""
                                        }
                                    );
                                    node.ins.operands = vec![value.to_decimal()];
                                }
                                p.stats.simplified_rows += 1;
                            }
                        }
                    }
                }
            }
        }
        if passes.reindex {
            // Private indices deliberately oppose canonical text order. Source
            // row ids (and every lift.label value) are unchanged.
            let n = p.nodes.len();
            p.nodes.reverse();
            for node in &mut p.nodes {
                for (_, target) in &mut node.edges {
                    *target = n - 1 - *target;
                }
            }
            p.stats.reindexed_rows = n;
        }
        if host {
            p.native = Some(native::Code::compile(&p.nodes, p.keys.len())?);
        }
        Ok(p)
    }
    /// Callback executes one reference-row boundary and returns its canonical
    /// successor, or None to yield/halt. Every error and panic is transported
    /// back across generated frames before propagation.
    pub fn run(
        &self,
        start: usize,
        mut step: impl FnMut(usize, &Instr) -> Result<Option<usize>, String>,
    ) -> Result<(), String> {
        if start >= self.keys.len() {
            return Err("§5.7: compiled entry outside dispatch".into());
        }
        if let Some(code) = &self.native {
            NATIVE_RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return code.run(start, &self.nodes, &mut |id, ins| {
                NATIVE_ROWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                step(id, ins)
            });
        }
        let mut defaults = vec![usize::MAX; self.keys.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            if !n.inlined {
                defaults[n.source] = i;
            }
        }
        let mut index = defaults[start];
        loop {
            let n = &self.nodes[index];
            let Some(next) = step(n.source, &n.ins)? else {
                return Ok(());
            };
            if next >= defaults.len() {
                return Err("§5.7: successor outside dispatch".into());
            }
            index = n
                .edges
                .iter()
                .find(|(source, _)| *source == next)
                .map(|(_, i)| *i)
                .unwrap_or(defaults[next]);
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
mod native {
    use super::*;
    use std::{
        ffi::c_void,
        panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    };
    unsafe extern "C" {
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            off: isize,
        ) -> *mut c_void;
        fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
        fn munmap(addr: *mut c_void, len: usize) -> i32;
    }
    pub struct Code {
        ptr: *mut c_void,
        len: usize,
    }
    // Mapping is immutable RX after construction; no pointer into a runtime is
    // embedded. All invocations own their context on the invoking thread.
    unsafe impl Send for Code {}
    unsafe impl Sync for Code {}
    impl Drop for Code {
        fn drop(&mut self) {
            assert_eq!(
                unsafe { munmap(self.ptr, self.len) },
                0,
                "JIT munmap failed"
            );
        }
    }
    struct Asm {
        bytes: Vec<u8>,
        labels: Vec<usize>,
        fixups: Vec<(usize, usize)>,
    }
    impl Asm {
        fn emit(&mut self, b: &[u8]) {
            self.bytes.extend_from_slice(b);
        }
        fn imm(&mut self, v: usize) {
            self.bytes.extend_from_slice(&(v as u64).to_le_bytes());
        }
        fn branch(&mut self, opcode: &[u8], target: usize) {
            self.emit(opcode);
            self.fixups.push((self.bytes.len(), target));
            self.emit(&[0; 4]);
        }
        fn label(&mut self, id: usize) {
            self.labels[id] = self.bytes.len();
        }
        fn finish(mut self) -> Result<Vec<u8>, String> {
            for (at, target) in self.fixups {
                let rel = i32::try_from(self.labels[target] as i64 - (at + 4) as i64)
                    .map_err(|_| "unimplemented: JIT code larger than rel32 (milestone 5)")?;
                self.bytes[at..at + 4].copy_from_slice(&rel.to_le_bytes());
            }
            Ok(self.bytes)
        }
    }
    thread_local! { static KERNELS: RefCell<BTreeMap<String, Code>> = const { RefCell::new(BTreeMap::new()) }; }
    pub fn arithmetic(op: &str, lhs: u64, rhs: u64, width: usize) -> Result<u64, String> {
        KERNELS.with(|kernels| {
            let mut kernels = kernels.borrow_mut();
            if !kernels.contains_key(op) {
                let mut a = Asm {
                    bytes: vec![],
                    labels: vec![0; 2],
                    fixups: vec![],
                };
                // SysV (a:rdi,b:rsi,width:rdx) -> rax; caller supplies masked values.
                a.emit(&[0xf3, 0x0f, 0x1e, 0xfa, 0x48, 0x89, 0xf8]);
                match op {
                    "add" => a.emit(&[0x48, 0x01, 0xf0]),
                    "sub" => a.emit(&[0x48, 0x29, 0xf0]),
                    "mul" => a.emit(&[0x48, 0x0f, 0xaf, 0xc6]),
                    "and" => a.emit(&[0x48, 0x21, 0xf0]),
                    "or" => a.emit(&[0x48, 0x09, 0xf0]),
                    "xor" => a.emit(&[0x48, 0x31, 0xf0]),
                    "shl" | "lshr" | "ashr" => {
                        a.emit(&[0x48, 0x39, 0xd6]); // cmp rsi,rdx
                        a.branch(&[0x0f, 0x83], 0); // jae zero, even for ashr
                        if op == "ashr" {
                            a.emit(&[0x48, 0xb9]);
                            a.imm(64);
                            a.emit(&[0x48, 0x29, 0xd1, 0x48, 0xd3, 0xe0, 0x48, 0xd3, 0xf8]);
                        }
                        a.emit(&[
                            0x48,
                            0x89,
                            0xf1,
                            0x48,
                            0xd3,
                            if op == "shl" {
                                0xe0
                            } else if op == "lshr" {
                                0xe8
                            } else {
                                0xf8
                            },
                        ]);
                    }
                    _ => {
                        return Err(format!(
                            "unimplemented: native integer kernel {op} (milestone 5)"
                        ));
                    }
                }
                a.emit(&[0x48, 0x83, 0xfa, 0x40]); // width == 64: avoid x86 masked shift
                a.branch(&[0x0f, 0x83], 1);
                a.emit(&[0x48, 0x89, 0xd1, 0x48, 0xba]);
                a.imm(1);
                a.emit(&[0x48, 0xd3, 0xe2, 0x48, 0xff, 0xca, 0x48, 0x21, 0xd0]);
                a.label(1);
                a.emit(&[0xc3]);
                a.label(0);
                a.emit(&[0x31, 0xc0, 0xc3]);
                kernels.insert(op.into(), Code::allocate(&a.finish()?)?);
            }
            let code = &kernels[op];
            let entry: unsafe extern "C" fn(u64, u64, usize) -> u64 =
                unsafe { std::mem::transmute(code.ptr) };
            Ok(unsafe { entry(lhs, rhs, width) })
        })
    }
    impl Code {
        fn allocate(bytes: &[u8]) -> Result<Self, String> {
            if bytes.is_empty() {
                return Err("§5.7: empty native code".into());
            }
            let len = bytes.len();
            // Linux anonymous mapping, RW during emission, RX before execution.
            let ptr = unsafe { mmap(std::ptr::null_mut(), len, 3, 0x22, -1, 0) };
            if ptr as isize == -1 {
                return Err(format!(
                    "§5.7: JIT mmap: {}",
                    std::io::Error::last_os_error()
                ));
            }
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast(), len);
            }
            if unsafe { mprotect(ptr, len, 5) } != 0 {
                let error = std::io::Error::last_os_error();
                unsafe {
                    munmap(ptr, len);
                }
                return Err(format!("§5.7: JIT mprotect RX: {error}"));
            }
            Ok(Self { ptr, len })
        }
        pub fn compile(nodes: &[Node], canonical_count: usize) -> Result<Self, String> {
            // SysV fn(context:rdi, helper:rsi, start:rdx). r12/r13 survive
            // helper calls; stack is 16-byte aligned at each call instruction.
            let dispatch = nodes.len();
            let exit = dispatch + 1;
            let mut a = Asm {
                bytes: vec![],
                labels: vec![0; nodes.len() + 2],
                fixups: vec![],
            };
            a.emit(&[
                0xf3, 0x0f, 0x1e, 0xfa, 0x41, 0x54, 0x41, 0x55, 0x48, 0x83, 0xec, 0x08, 0x49, 0x89,
                0xfc, 0x49, 0x89, 0xf5, 0x48, 0x89, 0xd0,
            ]);
            a.branch(&[0xe9], dispatch);
            for (id, node) in nodes.iter().enumerate() {
                a.label(id);
                a.emit(&[0x4c, 0x89, 0xe7, 0x48, 0xbe]);
                a.imm(id);
                a.emit(&[0x41, 0xff, 0xd5]); // call r13
                for (canonical, target) in &node.edges {
                    a.emit(&[0x48, 0xb9]);
                    a.imm(*canonical);
                    a.emit(&[0x48, 0x39, 0xc8]);
                    a.branch(&[0x0f, 0x84], *target);
                }
                a.branch(&[0xe9], dispatch);
            }
            a.label(dispatch);
            // Deterministic canonical dispatch. Linear comparisons are baseline
            // codegen; recovered edges bypass this even for cyclic control flow.
            let mut seen = BTreeSet::new();
            for (id, n) in nodes.iter().enumerate() {
                if n.inlined || !seen.insert(n.source) {
                    continue;
                }
                a.emit(&[0x48, 0xb9]);
                a.imm(n.source);
                a.emit(&[0x48, 0x39, 0xc8]);
                a.branch(&[0x0f, 0x84], id);
            }
            assert_eq!(seen.len(), canonical_count);
            a.label(exit);
            a.emit(&[0x48, 0x83, 0xc4, 0x08, 0x41, 0x5d, 0x41, 0x5c, 0xc3]);
            Self::allocate(&a.finish()?)
        }
        pub fn run(
            &self,
            start: usize,
            nodes: &[Node],
            step: &mut impl FnMut(usize, &Instr) -> Result<Option<usize>, String>,
        ) -> Result<(), String> {
            struct Context<'a, F> {
                nodes: &'a [Node],
                step: &'a mut F,
                error: Option<String>,
                panic: Option<Box<dyn std::any::Any + Send>>,
            }
            unsafe extern "C" fn helper<
                F: FnMut(usize, &Instr) -> Result<Option<usize>, String>,
            >(
                ctx: *mut c_void,
                id: usize,
            ) -> usize {
                // Only generated node immediates reach this helper.
                let ctx = unsafe { &mut *ctx.cast::<Context<'_, F>>() };
                let node = &ctx.nodes[id];
                match catch_unwind(AssertUnwindSafe(|| (ctx.step)(node.source, &node.ins))) {
                    Ok(Ok(Some(next))) if ctx.nodes.iter().any(|n| n.source == next) => next,
                    Ok(Ok(Some(_))) => {
                        ctx.error = Some("§5.7: successor outside dispatch".into());
                        usize::MAX
                    }
                    Ok(Ok(None)) => usize::MAX,
                    Ok(Err(e)) => {
                        ctx.error = Some(e);
                        usize::MAX
                    }
                    Err(panic) => {
                        ctx.panic = Some(panic);
                        usize::MAX
                    }
                }
            }
            fn call<F: FnMut(usize, &Instr) -> Result<Option<usize>, String>>(
                code: &Code,
                start: usize,
                nodes: &[Node],
                step: &mut F,
            ) -> Result<(), String> {
                let mut ctx = Context {
                    nodes,
                    step,
                    error: None,
                    panic: None,
                };
                type Entry = unsafe extern "C" fn(
                    *mut c_void,
                    unsafe extern "C" fn(*mut c_void, usize) -> usize,
                    usize,
                );
                let entry: Entry = unsafe { std::mem::transmute(code.ptr) };
                unsafe {
                    entry((&mut ctx as *mut Context<'_, F>).cast(), helper::<F>, start);
                }
                if let Some(panic) = ctx.panic {
                    resume_unwind(panic);
                }
                if let Some(error) = ctx.error {
                    return Err(error);
                }
                Ok(())
            }
            call(self, start, nodes, step)
        }
    }
}
#[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
mod native {
    use super::*;
    pub struct Code;
    pub fn arithmetic(_: &str, _: u64, _: u64, _: usize) -> Result<u64, String> {
        Err("unimplemented: JIT host arithmetic for this host (milestone 5)".into())
    }
    impl Code {
        pub fn compile(_: &[Node], _: usize) -> Result<Self, String> {
            Err("unimplemented: JIT host codegen for this host (milestone 5)".into())
        }
        pub fn run(
            &self,
            _: usize,
            _: &[Node],
            _: &mut impl FnMut(usize, &Instr) -> Result<Option<usize>, String>,
        ) -> Result<(), String> {
            unreachable!("unsupported host rejected at compilation")
        }
    }
}
