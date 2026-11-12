//! Classical launch checkpoints must scale with the work executed on an installed target.
use bennett_vm::{
    json::{self, Json},
    launch::*,
};
use std::{
    ffi::CStr,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    ptr,
    time::{Duration, Instant},
};

const OBJECT: &[u8] = include_bytes!("fixtures/r3-stepping/launch.bennett");
const TARGET: &[u8] = include_bytes!("fixtures/r3-stepping/target.json");
const WORKER: &str = "BENNETT_STEPPING_WORKER";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/r3-stepping")
        .join(name)
}
fn expected() -> Json {
    json::parse(include_str!("fixtures/r3-stepping/reference.json")).unwrap()
}
fn checked(status: i32) {
    assert_eq!(
        status,
        0,
        "{}",
        unsafe { CStr::from_ptr(bennett_last_error()) }.to_string_lossy()
    );
}
struct Run {
    program: *mut bennett_program_t,
    id: u64,
}
impl Run {
    fn new() -> Self {
        Self::with_observation(false)
    }
    fn with_observation(observed: bool) -> Self {
        unsafe {
            let mut program = ptr::null_mut();
            checked(bennett_load(
                OBJECT.as_ptr(),
                OBJECT.len(),
                TARGET.as_ptr(),
                TARGET.len(),
                &mut program,
            ));
            let mut args = ptr::null_mut();
            checked(bennett_args_new(0, &mut args));
            let mut id = 0;
            let start = if observed {
                run_with_options(
                    program,
                    c"main".as_ptr(),
                    args,
                    ptr::null(),
                    0,
                    &mut id,
                    RunOptions::default().collect_trace(true),
                )
            } else {
                bennett_run(program, c"main".as_ptr(), args, ptr::null(), 0, &mut id)
            };
            checked(start);
            bennett_args_free(args);
            let (mut result, mut request) = (ptr::null_mut(), ptr::null_mut());
            checked(bennett_wait(program, id, &mut result, &mut request));
            assert!(result.is_null() && !request.is_null());
            assert_eq!(
                CStr::from_ptr(bennett_request_service(request)),
                c"bennett.argv"
            );
            let mut n = 0;
            let bytes = bennett_request_payload(request, &mut n);
            assert_eq!(std::slice::from_raw_parts(bytes, n), &[0; 20]);
            checked(bennett_reply(program, id, [0; 20].as_ptr(), 20));
            bennett_request_free(request);
            Self { program, id }
        }
    }
    fn prefix(&mut self, steps: usize) -> crate::execution_work::Snapshot {
        let probe = crate::execution_work::Probe::start();
        for _ in 0..steps {
            checked(unsafe { bennett_run_step(self.program, self.id) });
        }
        probe.snapshot()
    }
    fn complete(&mut self) {
        unsafe {
            let (mut result, mut request) = (ptr::null_mut(), ptr::null_mut());
            checked(bennett_wait(
                self.program,
                self.id,
                &mut result,
                &mut request,
            ));
            assert!(!result.is_null() && request.is_null());
            assert_eq!(bennett_result_nfaults(result), 0);
            assert_eq!(bennett_result_nvalues(result), 1);
            let (mut kind, mut width, mut bytes) = (0, 0, [0u8; 4]);
            checked(bennett_result_get(
                result,
                0,
                &mut kind,
                &mut width,
                bytes.as_mut_ptr(),
                bytes.len(),
            ));
            assert_eq!((kind, width, bytes), (0, 32, [0; 4]));
            bennett_result_free(result);
            let mut snapshot = ptr::null_mut();
            checked(bennett_run_snapshot(self.program, self.id, &mut snapshot));
            let mut n = 0;
            let bytes = bennett_snapshot_jcs(snapshot, &mut n);
            let report =
                json::parse(std::str::from_utf8(std::slice::from_raw_parts(bytes, n)).unwrap())
                    .unwrap();
            bennett_snapshot_free(snapshot);
            assert_eq!(report.get("state").unwrap().as_str(), Some("COMPLETED"));
            assert_eq!(report.get("fault"), Some(&Json::Null));
            let oracle = expected();
            assert_eq!(report.get("console_hex"), oracle.get("console_hex"));
            assert_eq!(report.get("history_length"), oracle.get("steps"));
            // Every executed classical row costs one unit in this published target.
            assert_eq!(
                report
                    .get("target_resources")
                    .unwrap()
                    .get("time")
                    .unwrap()
                    .as_str(),
                Some("225956")
            );
            checked(bennett_run_close(self.program, self.id));
            checked(bennett_unload(self.program));
            self.program = ptr::null_mut();
        }
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        if !self.program.is_null() {
            // A scaling prefix intentionally leaves a RUNNING run. Destroy its
            // owned test allocation without executing the rest of the program.
            unsafe {
                drop(Box::from_raw(self.program));
            }
        }
    }
}

// R40's 2x bound and checkpoints use measured work on
// tobias-ThinkPad-X1-Carbon-Gen-11, 2026-11-07. Before indexing locks,
// scans/step were 11.148 / 27.022 / 51.288 at these checkpoints.
const CHECKPOINTS: [usize; 3] = [500, 5000, 15000];
const MAX_WORK_GROWTH: u128 = 2;
fn assert_flat(name: &str, initial: u64, initial_steps: usize, current: u64, steps: usize) {
    assert!(
        (current as u128) * (initial_steps as u128)
            <= MAX_WORK_GROWTH * (initial as u128) * (steps as u128),
        "{name}: {initial}/{initial_steps} -> {current}/{steps} operations"
    );
}
#[test]
fn installed_target_prefix_work_stays_within_twice_the_initial_work() {
    let mut initial = None;
    for steps in CHECKPOINTS {
        let mut run = Run::new();
        let work = run.prefix(steps);
        eprintln!("{steps} cooperative steps: {work:?}");
        assert_eq!(work.steps, steps as u64);
        // Measured 15 retained-value clones per append at 20k rows on the
        // same machine/date; immutable retained Frame/Json values now need zero.
        assert_eq!(work.payload_clones, 0, "retained Frame/Json payload copies");
        // Forward checkpoints only share the immutable event history.
        assert_eq!(
            work.target_history_copies, 0,
            "retained target event copies"
        );
        let first = *initial.get_or_insert(work);
        for (name, before, after) in [
            ("map nodes", first.map_nodes, work.map_nodes),
            ("path copies", first.path_copies, work.path_copies),
            (
                "target history copies",
                first.target_history_copies,
                work.target_history_copies,
            ),
            ("payload copies", first.payload_clones, work.payload_clones),
            (
                "accounting comparisons",
                first.accounting_comparisons,
                work.accounting_comparisons,
            ),
            ("lock scans", first.lock_scans, work.lock_scans),
        ] {
            assert_flat(name, before, CHECKPOINTS[0], after, steps);
        }
    }
}

// The slowest observed debug full-object worker was 51.246s on
// tobias-ThinkPad-X1-Carbon-Gen-11, 2026-11-07 (orchestrator gate log).
// 600s is a hang guard greater than 10x that observation, never a speed verdict.
const HANG_GUARD: Duration = Duration::from_secs(600);
struct Completed {
    output: Output,
    wall: Duration,
}
enum Outcome {
    Completed(Completed),
    Unknown(String),
}
fn observe(command: &mut Command, name: &str) -> Outcome {
    let start = Instant::now();
    let output = command.output().unwrap();
    let wall = start.elapsed();
    if output.status.code() == Some(124) {
        Outcome::Unknown(format!(
            "UNKNOWN: {name} exceeded measured hang guard {HANG_GUARD:?}; wall {wall:?}; deterministic result not verified\n{}\n{}\n",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    } else {
        Outcome::Completed(Completed { output, wall })
    }
}
fn report_unknown(message: &str) {
    // Bypass libtest's success-output capture so UNKNOWN remains visible in a gate.
    std::io::stderr().write_all(message.as_bytes()).unwrap();
}

#[test]
#[ignore = "published oracle differential; mandatory in the gate"]
fn full_object_matches_reference_output() {
    if std::env::var_os(WORKER).is_some() {
        Run::new().complete();
        return;
    }
    let reference = crate::pins::isa().join("reference-interpreter/target/debug/bennett-isa-ref");
    // Compare on the same permitted CPU, including on hybrid-core hosts.
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let allowed = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .expect("allowed CPU list")
        .trim();
    let cpu = allowed
        .split([',', '-'])
        .next()
        .expect("first permitted CPU");
    let observation = observe(
        Command::new("taskset")
            .args(["-c", cpu, "timeout"])
            .arg(format!("{}s", HANG_GUARD.as_secs()))
            .arg(&reference)
            .arg("run-program")
            .arg(fixture("launch.bennett"))
            .arg("main")
            .arg("--replies")
            .arg(fixture("replies.json"))
            .env("BENNETT_VM_ENGINE", "interpreter"),
        "reference",
    );
    let reference_sample = match observation {
        Outcome::Completed(sample) => sample,
        Outcome::Unknown(message) => {
            report_unknown(&message);
            return;
        }
    };
    let output = reference_sample.output;
    assert!(
        output.status.success(),
        "reference: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let oracle = expected();
    for field in ["status", "steps", "console_hex", "results"] {
        assert_eq!(report.get(field), oracle.get(field), "reference {field}");
    }
    let observation = observe(
        Command::new("taskset")
            .args(["-c", cpu, "timeout"])
            .arg(format!("{}s", HANG_GUARD.as_secs()))
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "step_scaling::full_object_matches_reference_output",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(WORKER, "1")
            .env("BENNETT_VM_ENGINE", "interpreter"),
        "VM worker",
    );
    let vm_sample = match observation {
        Outcome::Completed(sample) => sample,
        Outcome::Unknown(message) => {
            report_unknown(&format!(
                "{message}reference wall {:?}\n",
                reference_sample.wall
            ));
            return;
        }
    };
    eprintln!(
        "INFORMATION ONLY: reference wall {:?}, VM wall {:?}, wall ratio {:.3}",
        reference_sample.wall,
        vm_sample.wall,
        vm_sample.wall.as_secs_f64() / reference_sample.wall.as_secs_f64()
    );
    assert!(
        vm_sample.output.status.success(),
        "VM worker diagnostic: {}\n{}",
        String::from_utf8_lossy(&vm_sample.output.stdout),
        String::from_utf8_lossy(&vm_sample.output.stderr)
    );
}

#[test]
fn classical_accounting_comparisons_stay_within_twice_the_initial_work() {
    use bennett_vm::target::{Event, Rational, Target};
    use std::collections::BTreeMap;
    let target = Target::parse(std::str::from_utf8(TARGET).unwrap()).unwrap();
    let mut initial = None;
    for count in CHECKPOINTS {
        // Hand-derived disjoint unit intervals consume count time units and
        // zero gates/occupancy. Old all-predecessor work is count*(count-1)/2.
        let events: Vec<_> = (0..count as u64)
            .map(|i| Event {
                start: Rational::from(i),
                duration: Rational::from(1),
                wires: vec![],
                cost: BTreeMap::new(),
            })
            .collect();
        let probe = crate::execution_work::Probe::start();
        let report = target.account(&events, &[]).unwrap();
        let work = probe.snapshot();
        eprintln!("{count} classical accounting events: {work:?}");
        assert_eq!(report.get("time"), Some(&Json::Str(count.to_string())));
        assert_eq!(report.get("gate_count").unwrap().as_str(), Some("0"));
        assert_eq!(
            report
                .get("qubit_second")
                .unwrap()
                .get("data")
                .unwrap()
                .as_str(),
            Some("0")
        );
        // Empty wire sets have no possible wire-dependency predecessor.
        assert_eq!(
            work.accounting_comparisons, 0,
            "wire-free predecessor comparisons"
        );
        let first = *initial.get_or_insert(work.accounting_comparisons);
        assert_flat(
            "accounting comparisons",
            first,
            CHECKPOINTS[0],
            work.accounting_comparisons,
            count,
        );
    }
}

// Counts complete diagnostic row constructors, separately from other JSON
// (effect journals, replies and snapshots are deliberately outside this unit).
thread_local! {
    static TRACE_BUILDS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
pub(crate) fn record_trace_json_build() {
    TRACE_BUILDS.with(|count| count.set(count.get() + 1));
}
pub(crate) fn trace_builds() -> u64 {
    TRACE_BUILDS.with(std::cell::Cell::get)
}

#[test]
fn measure_step_trace_json_builds() {
    for steps in CHECKPOINTS {
        let mut run = Run::new();
        let before = trace_builds();
        let start = Instant::now();
        let work = run.prefix(steps);
        let builds = trace_builds() - before;
        eprintln!(
            "trace measurement: {steps} rows, {builds} row JSON builds, {:.6} builds/row; wall {:?}",
            builds as f64 / work.steps as f64,
            start.elapsed()
        );
        assert_eq!(work.steps, steps as u64);
    }
}

impl Run {
    fn snapshot(&self) -> Json {
        unsafe {
            let mut snapshot = ptr::null_mut();
            checked(bennett_run_snapshot(self.program, self.id, &mut snapshot));
            let mut n = 0;
            let bytes = bennett_snapshot_jcs(snapshot, &mut n);
            let report =
                json::parse(std::str::from_utf8(std::slice::from_raw_parts(bytes, n)).unwrap())
                    .unwrap();
            bennett_snapshot_free(snapshot);
            report
        }
    }
}

#[test]
fn trace_off_builds_no_step_json() {
    for steps in CHECKPOINTS {
        let mut off = Run::new();
        let before_off = trace_builds();
        let off_work = off.prefix(steps);
        let after_off = trace_builds();
        let mut on = Run::with_observation(true);
        let before_on = trace_builds();
        let on_work = on.prefix(steps);
        let on_builds = trace_builds() - before_on;
        assert!(on_builds > 0, "observed denominator must be positive");
        assert_eq!(off_work.steps, on_work.steps);
        assert_eq!(off_work.steps, steps as u64);
        let before_export = trace_builds();
        let export_start = Instant::now();
        let off_snapshot = off.snapshot();
        let export_wall = export_start.elapsed();
        let export_builds = trace_builds() - before_export;
        let before_observed_export = trace_builds();
        let on_snapshot = on.snapshot();
        assert_eq!(
            trace_builds(),
            before_observed_export,
            "observed snapshot only clones its encoded rows"
        );
        assert_eq!(
            export_builds,
            off_snapshot
                .get("history_length")
                .unwrap()
                .as_u64()
                .unwrap()
        );
        eprintln!("lazy snapshot: {export_builds} row JSON builds; wall {export_wall:?}");
        assert_eq!(
            off_snapshot, on_snapshot,
            "complete legacy snapshot at {steps}"
        );
        assert!(
            !off_snapshot
                .get("trace")
                .unwrap()
                .as_arr()
                .unwrap()
                .is_empty()
        );
        eprintln!(
            "{steps} rows: off={} on={on_builds}, Rtrace={:.6}",
            after_off - before_off,
            (after_off - before_off) as f64 / on_builds as f64
        );
        assert_eq!(
            after_off, before_off,
            "trace-off execution constructed row JSON at {steps} rows"
        );
    }
}

fn history_run(functions: &str) -> crate::cc::RuntimeRun {
    let source = format!(
        "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=64\n\nsection symbols\n{{}}\n\nsection functions\n{functions}\n\nsection layout\n{{\"A_tab\":16,\"H\":16,\"S_cells\":0,\"globals\":{{}},\"heap_base\":16,\"heap_limit_max\":1024,\"interface_regions\":{{}},\"null_size\":1,\"ptr_width\":64}}\n"
    );
    let object = std::sync::Arc::new(crate::object::parse(&source).unwrap());
    let resources = crate::cc::Resources::new(&object).unwrap();
    crate::cc::RuntimeRun::new(object, &resources, "main", vec![], vec![]).unwrap()
}

#[test]
fn history_lookup_does_not_walk_its_prefix() {
    let mut samples = vec![];
    // Reuse the measured short/long fixture checkpoints on this 12-core host,
    // 2026-11-07 (see CHECKPOINTS); equality is an exact traversal invariant.
    for prefix in [CHECKPOINTS[0], CHECKPOINTS[2]] {
        let pure_prefix = "unused = const.i8 0\n".repeat(prefix);
        let mut run = history_run(&format!(
            "fn r @id () () () () ()\n^entry():\nret\nfn cc @main () () () () ()\n^entry():\n{pure_prefix}call @id\nuncall @id () ()\nret"
        ));
        for _ in 0..prefix + 2 {
            run.step().unwrap();
        }
        let probe = crate::history_work::Probe::start();
        probe.query_prefix(prefix);
        let before = probe.snapshot();
        run.step().unwrap();
        let after = probe.snapshot();
        assert_eq!(run.fault(), None);
        eprintln!("query prefix={prefix}: {after:?}");
        samples.push((before, after));
    }
    eprintln!(
        "Rprefix[index_probes]={}/{}; undo payload frames={}/{}",
        samples[1].1.index_probes,
        samples[0].1.index_probes,
        samples[1].1.undo_frames,
        samples[0].1.undo_frames
    );
    for (before, after) in samples {
        assert_eq!(
            after.undo_frames - before.undo_frames,
            2,
            "call and ret undo payload"
        );
        assert_eq!(
            after.prefix_frame_visits, before.prefix_frame_visits,
            "query walked retained prefix"
        );
    }
}

#[test]
fn effect_export_work_is_linear() {
    for count in [CHECKPOINTS[0], CHECKPOINTS[1]] {
        let effects = "effect clock.read p\n".repeat(count);
        let mut run = history_run(&format!(
            "fn cc @main () () () () ()\n^entry():\nn = const.i64 8\np = alloc n\n{effects}ret"
        ));
        while !run.terminal() {
            run.step().unwrap();
        }
        for _ in 0..2 {
            let probe = crate::history_work::Probe::start();
            let before = probe.snapshot();
            let events = run.tape();
            let after = probe.snapshot();
            assert_eq!(events.len(), count);
            eprintln!(
                "export count={count}: visited_event_refs/published_event_refs={}/{}",
                after.event_refs - before.event_refs,
                events.len(),
            );
            assert_eq!(
                after.event_refs - before.event_refs,
                events.len() as u64,
                "export visited entries beyond its published events"
            );
        }

        // Execute a reference export that deliberately walks every retained
        // prefix. Its output is still linear; the work oracle must reject it.
        let mut tape = crate::persistent::Seq::new();
        let mut prefixes = vec![];
        for row in 0..count {
            tape.push(Json::u64(row as u64));
            prefixes.push(tape.clone());
        }
        let probe = crate::history_work::Probe::start();
        let before = probe.snapshot();
        let mut events = vec![];
        for (prefix_len, prefix) in prefixes.iter().enumerate() {
            for (row, entry) in prefix.iter().enumerate() {
                if row >= prefix_len {
                    events.push(entry.clone());
                }
            }
        }
        let after = probe.snapshot();
        let expected: Vec<_> = (0..count).map(|row| Json::u64(row as u64)).collect();
        assert_eq!(events, expected);
        let visited = after.event_refs - before.event_refs;
        // Prefix lengths 1..=count sum to count*(count+1)/2 visits;
        // subtracting the count emitted entries leaves count*(count-1)/2.
        let expected_visits = (count * (count + 1) / 2) as u64;
        eprintln!(
            "prefix-walking control count={count}: visited={visited}; expected={expected_visits}"
        );
        assert_eq!(visited, expected_visits, "oracle missed prefix visits");
        assert_ne!(
            visited,
            events.len() as u64,
            "oracle accepted a prefix walk"
        );
        assert_eq!(
            visited - events.len() as u64,
            (count * (count - 1) / 2) as u64,
            "skipped prefix visits"
        );
    }
}

#[test]
fn tail_rewind_reexecution_preserves_row_instances_and_snapshot() {
    // This measured fixture prefix also crosses several history chunks.
    let prefix = CHECKPOINTS[0];
    let pure_prefix = "x = const.i8 1\n".repeat(prefix);
    let mut run = history_run(&format!(
        "fn cc @main () (v:i8) () () ()\n^entry():\n{pure_prefix}x = call @id\nsaved = const.i8 2\nx = const.i8 9\nuncall @id () (saved)\nret x\nfn r @id () (v:i8) () () ()\n^entry():\nv = const.i8 2\nret v"
    ));
    while !run.terminal() {
        run.step().unwrap();
    }
    let expected = run.snapshot("COMPLETED").unwrap();
    for _ in 0..3 {
        run.reverse_to(prefix).unwrap();
        assert_eq!(run.history_len(), prefix);
        while !run.terminal() {
            run.step().unwrap();
        }
        assert_eq!(run.fault(), None);
        assert_eq!(
            run.values(),
            &[crate::value::Value::int(
                8,
                crate::bigint::BigUint::from_u64(9)
            )]
        );
        assert_eq!(run.snapshot("COMPLETED").unwrap(), expected);
    }
    run.reverse_to(0).unwrap();
    assert!(
        run.snapshot("REVERSED")
            .unwrap()
            .get("instances")
            .unwrap()
            .as_arr()
            .unwrap()
            .is_empty()
    );
}

// Count canonical table slots/comparisons and table-construction node visits.
// Native comparison probes are emitted into the test-only machine code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DispatchWork {
    pub probes: u64,
    pub initialization_visits: u64,
}
thread_local! {
    static DISPATCH_WORK: std::cell::Cell<DispatchWork> = const {
        std::cell::Cell::new(DispatchWork { probes: 0, initialization_visits: 0 })
    };
}
pub(crate) fn dispatch_probe() {
    record_dispatch_probes(1);
}
pub(crate) fn record_dispatch_probes(count: u64) {
    DISPATCH_WORK.with(|work| {
        let mut next = work.get();
        next.probes += count;
        work.set(next);
    });
}
pub(crate) fn dispatch_initialization_visit() {
    DISPATCH_WORK.with(|work| {
        let mut next = work.get();
        next.initialization_visits += 1;
        work.set(next);
    });
}
fn reset_dispatch_work() {
    DISPATCH_WORK.with(|work| work.set(DispatchWork::default()));
}
fn dispatch_work() -> DispatchWork {
    DISPATCH_WORK.with(std::cell::Cell::get)
}

#[test]
fn compiled_step_dispatch_is_independent_of_unused_rows() {
    use crate::jit::{Passes, Program};
    // Existing calibrated program-size checkpoints; only unreachable rows vary.
    // The executed body is at high canonical IDs, with an interior-row backedge
    // deliberately absent from the recovered CFG.
    let mut comparisons = vec![];
    for passes in [
        Passes::default(),
        Passes {
            inline: true,
            ..Passes::default()
        },
        Passes {
            structure: true,
            ..Passes::default()
        },
        Passes {
            reindex: true,
            ..Passes::default()
        },
        Passes {
            simplify: true,
            ..Passes::default()
        },
        Passes::all(),
    ] {
        for native in [false, true] {
            let mut samples = vec![];
            for unused in [CHECKPOINTS[0], CHECKPOINTS[2]] {
                let dead = "unused = const.i8 0\n".repeat(unused);
                let functions = format!(
                    "fn cc @unused () () () () ()\n^entry():\n{dead}ret\nfn cc @main () () () () ()\n^entry():\na = const.i8 1\nb = const.i8 2\nc = add a b\nret"
                );
                let source = format!(
                    "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=64\n\nsection symbols\n{{}}\n\nsection functions\n{functions}\n"
                );
                let object = crate::object::parse(&source).unwrap();
                let program = Program::compile(&object, passes, native).unwrap();
                let rows = [0, 2, 1, 2, 3].map(|row| program.row("main", 0, row).unwrap());
                let mut traces = vec![];
                for cooperative in [false, true] {
                    reset_dispatch_work();
                    let mut trace = vec![];
                    if cooperative {
                        for row in rows {
                            program
                                .step(row, |ins| {
                                    trace.push((row - rows[0], ins.raw.clone()));
                                    Ok(())
                                })
                                .unwrap();
                        }
                    } else {
                        program
                            .run(rows[0], |id, ins| {
                                trace.push((id - rows[0], ins.raw.clone()));
                                Ok(rows.get(trace.len()).copied())
                            })
                            .unwrap();
                    }
                    let work = dispatch_work();
                    assert_eq!(trace.len(), rows.len());
                    traces.push(trace);
                    samples.push((cooperative, work));
                }
                assert_eq!(traces[0], traces[1]);
            }
            for mode in 0..2 {
                let short = samples[mode].1;
                let long = samples[mode + 2].1;
                eprintln!(
                    "dispatch native={native} passes={passes:?} cooperative={} short={short:?} long={long:?} probes/row={}/{} -> {}/{} Rwork={:.6}",
                    samples[mode].0,
                    short.probes,
                    5,
                    long.probes,
                    5,
                    long.probes as f64 / short.probes as f64
                );
            }
            for mode in 0..2 {
                comparisons.push((
                    native,
                    samples[mode].0,
                    samples[mode].1,
                    samples[mode + 2].1,
                ));
            }
        }
    }
    for (native, cooperative, short, long) in comparisons {
        assert_eq!(
            long.probes, short.probes,
            "dispatch probes per identical executed row depend on unused rows: native={native} cooperative={cooperative}"
        );
        assert_eq!(
            long.initialization_visits, short.initialization_visits,
            "per-step dispatch initialization depends on unused rows"
        );
        assert_eq!(
            short.probes,
            if cooperative { 5 } else { 9 },
            "one canonical entry, plus membership and successor lookups for each continuation"
        );
        assert_eq!(
            short.initialization_visits, 0,
            "compiled tables must be built before entry"
        );
    }
}

#[test]
#[ignore = "informational printf dispatch work receipt"]
fn printf_dispatch_work() {
    reset_dispatch_work();
    let execution = crate::execution_work::Probe::start();
    let args = [
        fixture("launch.bennett").to_str().unwrap().to_owned(),
        "main".into(),
        "--replies".into(),
        fixture("replies.json").to_str().unwrap().to_owned(),
        "--target".into(),
        fixture("target.json").to_str().unwrap().to_owned(),
    ];
    crate::cc::cmd_program(&args).unwrap();
    let work = dispatch_work();
    let rows = expected().get("steps").unwrap().as_u64().unwrap();
    assert_eq!(
        execution.snapshot().steps,
        rows,
        "published printf row count"
    );
    let compiled = crate::jit::selection().unwrap().is_some();
    assert_eq!(
        work.probes == 0,
        !compiled,
        "compiled dispatch must be observed exactly when selected"
    );
    eprintln!(
        "PRINTF_DISPATCH rows={rows} probes={} probes_per_row={:.9} initialization_visits={}",
        work.probes,
        work.probes as f64 / rows as f64,
        work.initialization_visits
    );
}

// Source-owned counters isolate byte-memory work from allocation validity,
// register trees, and the diagnostic address set retained by frames.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MemoryWork {
    pub byte_tree_nodes: u64,
    pub byte_tree_operations: u64,
    pub journal_bytes: u64,
    pub metadata_probes: u64,
    pub page_allocations: u64,
    pub copied_bytes: u64,
    pub resident_units: u64,
}
thread_local! {
    static MEMORY_WORK: std::cell::Cell<MemoryWork> = const {
        std::cell::Cell::new(MemoryWork {
            byte_tree_nodes: 0, byte_tree_operations: 0, journal_bytes: 0,
            metadata_probes: 0, page_allocations: 0, copied_bytes: 0, resident_units: 0,
        })
    };
}
pub(crate) fn record_memory_work(update: impl FnOnce(&mut MemoryWork)) {
    MEMORY_WORK.with(|cell| {
        let mut work = cell.get();
        update(&mut work);
        cell.set(work);
    });
}
fn memory_work() -> MemoryWork {
    MEMORY_WORK.with(std::cell::Cell::get)
}
fn reset_memory_work() {
    MEMORY_WORK.with(|cell| cell.set(MemoryWork::default()));
}
fn memory_object(functions: &str) -> crate::object::Object {
    let source = format!(
        "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=128\n\nsection symbols\n{{}}\n\nsection functions\n{functions}\n\nsection layout\n{{\"A_tab\":16,\"H\":128,\"S_cells\":0,\"globals\":{{}},\"heap_base\":16,\"heap_limit_max\":\"340282366920938463463374607431768211439\",\"interface_regions\":{{}},\"null_size\":1,\"ptr_width\":128}}\n"
    );
    crate::object::parse(&source).unwrap()
}
fn memory_run(functions: &str) -> crate::cc::RuntimeRun {
    let object = std::sync::Arc::new(memory_object(functions));
    let resources = crate::cc::Resources::new(&object).unwrap();
    crate::cc::RuntimeRun::new(object, &resources, "main", vec![], vec![]).unwrap()
}

#[test]
fn memory_journal_retention_tracks_changed_bytes() {
    let mut samples = vec![];
    for prefix in [CHECKPOINTS[0], CHECKPOINTS[2]] {
        let functions = format!(
            "fn cc @main () (v:i128) () () ()\n^entry():\nn = const.i128 16\np = alloc n\na = const.i128 22685491128062564230891640495451214097\nb = const.i128 45370982256125128461783280990902428194\nstore.i128 p a\n{}{}v = load.i128 p\nret v",
            "unused = const.i8 0\n".repeat(prefix),
            "store.i128 p b\nstore.i128 p a\n".repeat(CHECKPOINTS[0] / 2),
        );
        let mut run = memory_run(&functions);
        for _ in 0..prefix + 5 {
            run.step().unwrap();
        }
        reset_memory_work();
        let probe = crate::execution_work::Probe::start();
        for _ in 0..CHECKPOINTS[0] {
            run.step().unwrap();
        }
        let work = memory_work();
        assert_eq!(probe.snapshot().steps, CHECKPOINTS[0] as u64);
        drop(probe);
        assert_eq!(run.fault(), None);
        samples.push(work);
        eprintln!(
            "memory prefix={prefix}, committed_rows={}, committed_bytes={}: {work:?}; page_allocations/write_bytes={}/{}, copied_bytes/write_bytes={}/{}",
            CHECKPOINTS[0],
            CHECKPOINTS[0] * 16,
            work.page_allocations,
            CHECKPOINTS[0] * 16,
            work.copied_bytes,
            CHECKPOINTS[0] * 16
        );
        run.step().unwrap();
        run.step().unwrap();
        assert_eq!(
            run.values(),
            &[crate::value::Value::int(
                128,
                crate::bigint::BigUint::parse_decimal("22685491128062564230891640495451214097")
                    .unwrap()
            )]
        );
        run.reverse_to(prefix + 5).unwrap();
        while !run.terminal() {
            run.step().unwrap();
        }
        assert_eq!(run.fault(), None);
        assert_eq!(
            run.values(),
            &[crate::value::Value::int(
                128,
                crate::bigint::BigUint::parse_decimal("22685491128062564230891640495451214097")
                    .unwrap()
            )]
        );
    }
    eprintln!(
        "Rprefix[memory_metadata_probes]={}/{}",
        samples[1].metadata_probes, samples[0].metadata_probes
    );
    for work in &samples {
        assert_eq!(
            work.byte_tree_nodes, 0,
            "byte-memory AVL-node allocations per committed store must be zero"
        );
        assert_eq!(work.byte_tree_operations, 0, "byte-memory tree operations");
        assert_eq!(
            work.journal_bytes,
            (CHECKPOINTS[0] * 16) as u64,
            "original journal bytes must equal committed write bytes"
        );
    }
    assert_eq!(samples[0].metadata_probes, samples[1].metadata_probes);
}

#[test]
fn large_sparse_image_uses_only_touched_pages() {
    // A 2^25-byte allocation is zero-filled without resident byte storage.
    // Four nonzero 16-byte spans are widely separated within it.
    let mut run = memory_run(
        "fn cc @main () (a:i128, b:i128, c:i128, d:i128) () () ()\n^entry():\nn = const.i128 33554432\np = alloc n\nv = const.i128 22685491128062564230891640495451214097\np0 = const.i128 16\np1 = const.i128 10000\np2 = const.i128 16777216\np3 = const.i128 33554432\nstore.i128 p0 v\nstore.i128 p1 v\nstore.i128 p2 v\nstore.i128 p3 v\na = load.i128 p0\nb = load.i128 p1\nc = load.i128 p2\nd = load.i128 p3\nret a b c d",
    );
    reset_memory_work();
    while !run.terminal() {
        run.step().unwrap();
    }
    assert_eq!(run.fault(), None);
    let value = crate::value::Value::int(
        128,
        crate::bigint::BigUint::parse_decimal("22685491128062564230891640495451214097").unwrap(),
    );
    assert_eq!(
        run.values(),
        &[value.clone(), value.clone(), value.clone(), value]
    );
    let work = memory_work();
    eprintln!("sparse 2^25-byte image, four touched pages: {work:?}");
    assert_eq!(
        work.resident_units, 4,
        "resident memory units must be pages, independent of zero-filled image extent"
    );
    run.reverse_to(0).unwrap();
    assert_eq!(run.history_len(), 0);
}

#[test]
fn sparse_global_image_allocates_only_nonzero_pages() {
    use crate::{bigint::BigUint, object::Global};
    let size = 1 << 25;
    let mut image = vec![0; size];
    for offset in [0, 1 << 20, 1 << 24, size - 1] {
        image[offset] = 0x5a;
    }
    let mut object = memory_object(
        "fn cc @main () (v:i8) () () ()\n^entry():\np = const.i128 33554447\nv = load.i8 p\nret v",
    );
    object.layout.as_mut().unwrap().globals.insert(
        "image".into(),
        Global {
            base: BigUint::from_u64(16),
            size: BigUint::from_u64(size as u64),
            image: "image".into(),
        },
    );
    object
        .data
        .insert("image".into(), crate::sha256::to_hex(&image));
    let object = std::sync::Arc::new(object);
    reset_memory_work();
    let resources = crate::cc::Resources::new(&object).unwrap();
    let initialized = memory_work();
    assert_eq!(initialized.page_allocations, 4);
    assert_eq!(initialized.resident_units, 4);
    assert_eq!(
        initialized.journal_bytes, 0,
        "initial images are not row writes"
    );
    let mut run = crate::cc::RuntimeRun::new(object, &resources, "main", vec![], vec![]).unwrap();
    while !run.terminal() {
        run.step().unwrap();
    }
    assert_eq!(run.fault(), None);
    assert_eq!(
        run.values(),
        &[crate::value::Value::int(8, BigUint::from_u64(0x5a))]
    );
    eprintln!("2^25-byte global image, four nonzero pages: {initialized:?}");
}

#[test]
fn sparse_pages_preserve_wide_cross_page_versions_and_unrelated_bytes() {
    use crate::{
        bigint::BigUint,
        classical_memory::{Memory, PAGE_SIZE},
    };
    use std::collections::BTreeMap;
    let mut memory = Memory::default();
    let mut expected = BTreeMap::new();
    let base = BigUint::from_u64(1)
        .shl(96)
        .add(&BigUint::from_u64((PAGE_SIZE - 7) as u64));
    let mut snapshots = vec![];
    // i80/i128 footprints straddle page boundaries above u64::MAX. Vary
    // insertion order to exercise both children of compressed radix branches.
    for index in [31, 0, 12, 5, 22, 2, 15, 8] {
        let address = base.add(&BigUint::from_u64((index * PAGE_SIZE) as u64));
        for width in [10, 16] {
            let bytes: Vec<_> = (0..width).map(|i| (index + i + 1) as u8).collect();
            snapshots.push((memory.clone(), expected.clone()));
            memory.write(&address, &bytes);
            for (i, b) in bytes.into_iter().enumerate() {
                expected.insert(address.add(&BigUint::from_u64(i as u64)), b);
            }
            assert_eq!(
                memory.ordered_bytes(),
                expected
                    .iter()
                    .map(|(a, b)| (a.clone(), *b))
                    .collect::<Vec<_>>()
            );
        }
    }
    for (snapshot, expected) in snapshots {
        assert_eq!(
            snapshot.ordered_bytes(),
            expected.into_iter().collect::<Vec<_>>()
        );
    }
    let before = memory.clone();
    memory.write(&base, &[0; 16]);
    let after = memory.clone();
    // A disjoint write within the same page survives restoration of this row.
    let untouched = base.add(&BigUint::from_u64(100));
    memory.write(&untouched, &[0xa5]);
    memory.undo_delta(&before, &after);
    let mut bytes = [0; 16];
    memory.read(&base, &mut bytes);
    assert_eq!(
        bytes,
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
    assert_eq!(memory.get(&untouched), Some(&0xa5));
    memory.write(&base, &[0; 16]);
    assert!((0..16).all(|i| memory.get(&base.add(&BigUint::from_u64(i))).is_none()));
}

#[test]
fn overlapping_span_journal_restores_only_changed_row_bytes() {
    use crate::{bigint::BigUint, classical_memory::Memory};
    let address = BigUint::from_u64(4095);
    let mut memory = Memory::default();
    memory.write(&address, &[1, 2, 3]);
    let before = memory.clone();
    memory.write(&address, &[8, 9]);
    memory.write(&address, &[1]);
    let after = memory.clone();
    memory.write(&address, &[7]);
    memory.write(&address.add(&BigUint::from_u64(2)), &[99]);
    memory.undo_delta(&before, &after);
    let mut bytes = [0; 3];
    memory.read(&address, &mut bytes);
    assert_eq!(bytes, [7, 2, 99]);
}

#[test]
fn faulted_store_retains_no_displaced_bytes() {
    let mut run = memory_run(
        "fn cc @main () () () () ()\n^entry():\nn = const.i128 16\np = alloc n\nv = const.i128 22685491128062564230891640495451214097\nstore.i128 p v\nbad = const.i128 24\nstore.i128 bad v\nret",
    );
    for _ in 0..5 {
        run.step().unwrap();
    }
    let before = run.snapshot("RUNNING").unwrap();
    reset_memory_work();
    run.step().unwrap();
    assert_eq!(run.fault(), Some(7));
    let work = memory_work();
    assert_eq!(work.journal_bytes, 0);
    assert_eq!(work.page_allocations, 0);
    assert_eq!(
        before.get("memory_digest"),
        run.snapshot("FAILED").unwrap().get("memory_digest")
    );
    run.reverse_to(5).unwrap();
    run.step().unwrap();
    assert_eq!(run.fault(), Some(7));
}

#[test]
fn unchanged_writes_retain_original_bytes_and_release_long_journal() {
    use crate::{bigint::BigUint, classical_memory::Memory};
    let mut memory = Memory::default();
    let address = BigUint::from_u64(1);
    memory.write(&address, &[1]);
    reset_memory_work();
    for _ in 0..CHECKPOINTS[2] {
        memory.write(&address, &[1]);
    }
    let work = memory_work();
    assert_eq!(work.journal_bytes, CHECKPOINTS[2] as u64);
    assert_eq!(work.page_allocations, 0);
    assert_eq!(memory.get(&address), Some(&1));
    drop(memory);
    let mut zero = Memory::default();
    zero.write(&address, &[5]);
    zero.write(&address, &[0]);
    assert_eq!(zero, Memory::default());
    assert_eq!(
        memory_work().resident_units,
        0,
        "empty pages must be removed"
    );
}

#[test]
fn rewound_memory_releases_discarded_journal_suffix() {
    use crate::{bigint::BigUint, classical_memory::Memory};
    let address = BigUint::from_u64(1);
    let mut memory = Memory::default();
    memory.write(&address, &[1; 16]);
    let prefix = memory.retained_journal_bytes();
    for _ in 0..3 {
        let before = memory.clone();
        memory.write(&address, &[2; 16]);
        let after = memory.clone();
        memory.undo_delta(&before, &after);
        drop(after);
        assert_eq!(memory, before);
        assert_eq!(
            memory.retained_journal_bytes(),
            prefix,
            "rewound suffix retained"
        );
    }
}

#[test]
fn journal_forks_preserve_interleaved_bytes_when_uncall_is_reversed() {
    use crate::{bigint::BigUint, classical_memory::Memory};
    let a = BigUint::from_u64(1);
    let b = BigUint::from_u64(2);
    let c = BigUint::from_u64(3);
    let d = BigUint::from_u64(4);
    let mut memory = Memory::default();
    let before_a = memory.clone();
    memory.write(&a, &[11]);
    let after_a = memory.clone();
    memory.write(&b, &[22]); // Another run between two callee writes.
    let before_c = memory.clone();
    memory.write(&c, &[33]);
    let before_uncall = memory.clone();
    memory.undo_delta(&before_c, &before_uncall); // Exact tail, prune c.
    memory.undo_delta(&before_a, &after_a); // Keep b, undo a, fork journal.
    let after_uncall = memory.clone();
    assert_eq!(memory.ordered_bytes(), vec![(b.clone(), 22)]);
    memory.write(&d, &[44]); // Another run after uncall.
    memory.undo_delta(&before_uncall, &after_uncall);
    assert_eq!(
        memory.ordered_bytes(),
        vec![(a, 11), (b, 22), (c, 33), (d, 44)]
    );
}

// Equal operation footprints reuse the existing measured prefix sizes above.
// Exact zero/equality assertions are design invariants, not timing ceilings.
fn prepared_row_samples() -> Vec<crate::execution_work::Snapshot> {
    let mut samples = vec![];
    for prefix in [CHECKPOINTS[0], CHECKPOINTS[2]] {
        let functions = format!(
            "fn cc @main () (x:i64) () () ()\n^entry():\n{}ret x",
            "x = const.i64 37\n".repeat(prefix + CHECKPOINTS[0])
        );
        let object = std::sync::Arc::new(memory_object(&functions));
        let mut resources = crate::cc::Resources::new(&object).unwrap();
        resources.set_target(
            crate::target::Target::parse(std::str::from_utf8(TARGET).unwrap()).unwrap(),
        );
        // An idle live qreg makes an accidental checkpoint copy real Q payload.
        resources.new_reg(2, "data").unwrap();
        let mut run =
            crate::cc::RuntimeRun::new(object, &resources, "main", vec![], vec![]).unwrap();
        for _ in 0..prefix {
            run.step().unwrap();
        }
        let probe = crate::execution_work::Probe::start();
        for _ in 0..CHECKPOINTS[0] {
            run.step().unwrap();
        }
        let work = probe.snapshot();
        drop(probe);
        eprintln!(
            "prepared prefix={prefix} rows={}: {work:?}; checkpoints/row={}/{}; instance_tree_updates/row={}/{}",
            CHECKPOINTS[0],
            work.machine_checkpoints,
            work.steps,
            work.instance_tree_updates,
            work.steps
        );
        assert_eq!(work.steps, CHECKPOINTS[0] as u64);
        samples.push(work);
        run.step().unwrap();
        assert_eq!(
            run.values(),
            &[crate::value::Value::int(
                64,
                crate::bigint::BigUint::from_u64(37)
            )]
        );
        run.reverse_to(0).unwrap();
        assert_eq!(run.history_len(), 0);
    }
    samples
}

#[test]
fn prepared_pure_steps_copy_no_machine_payload() {
    for work in prepared_row_samples() {
        assert_eq!(work.machine_checkpoints, 0, "ordinary pure row checkpoint");
        assert_eq!(work.instance_tree_updates, 0, "per-instance tree update");
        assert_eq!(work.payload_clones, 0, "retained payload clones");
    }
}

#[test]
fn fixed_row_cost_is_independent_of_history() {
    let samples = prepared_row_samples();
    eprintln!(
        "Rprefix[map_nodes]={}/{}; Rprefix[path_copies]={}/{}",
        samples[1].map_nodes, samples[0].map_nodes, samples[1].path_copies, samples[0].path_copies
    );
    assert_eq!(samples[0].map_nodes, samples[1].map_nodes);
    assert_eq!(samples[0].path_copies, samples[1].path_copies);
    assert_eq!(samples[0].append_nodes, samples[1].append_nodes);
    assert_eq!(samples[0].append_payload_bytes, samples[1].append_payload_bytes);
}
