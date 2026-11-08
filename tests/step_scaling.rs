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
fn trace_builds() -> u64 {
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
        assert_eq!(trace_builds(), before_observed_export, "observed snapshot only clones its encoded rows");
        assert_eq!(export_builds, off_snapshot.get("history_length").unwrap().as_u64().unwrap());
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
    let source = format!("bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=64\n\nsection symbols\n{{}}\n\nsection functions\n{functions}\n\nsection layout\n{{\"A_tab\":16,\"H\":16,\"S_cells\":0,\"globals\":{{}},\"heap_base\":16,\"heap_limit_max\":1024,\"interface_regions\":{{}},\"null_size\":1,\"ptr_width\":64}}\n");
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
        let mut run = history_run(&format!("fn r @id () () () () ()\n^entry():\nret\nfn cc @main () () () () ()\n^entry():\n{pure_prefix}call @id\nuncall @id () ()\nret"));
        for _ in 0..prefix + 2 { run.step().unwrap(); }
        let probe = crate::history_work::Probe::start();
        probe.query_prefix(prefix);
        let before = probe.snapshot();
        run.step().unwrap();
        let after = probe.snapshot();
        assert_eq!(run.fault(), None);
        eprintln!("query prefix={prefix}: {after:?}");
        samples.push((before, after));
    }
    eprintln!("Rprefix[index_probes]={}/{}; undo payload frames={}/{}", samples[1].1.index_probes, samples[0].1.index_probes, samples[1].1.undo_frames, samples[0].1.undo_frames);
    for (before, after) in samples {
        assert_eq!(after.undo_frames - before.undo_frames, 2, "call and ret undo payload");
        assert_eq!(after.prefix_frame_visits, before.prefix_frame_visits, "query walked retained prefix");
    }
}

#[test]
fn effect_export_work_is_linear() {
    for count in [CHECKPOINTS[0], CHECKPOINTS[1]] {
        let effects = "effect clock.read p\n".repeat(count);
        let mut run = history_run(&format!("fn cc @main () () () () ()\n^entry():\nn = const.i64 8\np = alloc n\n{effects}ret"));
        while !run.terminal() { run.step().unwrap(); }
        for _ in 0..2 {
            let probe = crate::history_work::Probe::start();
            let before = probe.snapshot();
            let events = run.tape();
            let after = probe.snapshot();
            assert_eq!(events.len(), count);
            eprintln!("export count={count}: visited_event_refs/published_event_refs={}/{}; skipped={}", after.event_refs - before.event_refs, events.len(), after.skipped_tape_prefix - before.skipped_tape_prefix);
            assert_eq!(after.event_refs - before.event_refs, events.len() as u64);
            assert_eq!(after.skipped_tape_prefix, before.skipped_tape_prefix, "export walked tape prefix");
        }
    }
}
