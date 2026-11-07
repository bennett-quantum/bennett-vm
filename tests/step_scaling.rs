//! Classical launch checkpoints must scale with the work executed on an installed target.
use bennett_vm::{
    json::{self, Json},
    launch::*,
};
use std::{
    ffi::CStr,
    path::{Path, PathBuf},
    process::Command,
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
            checked(bennett_run(
                program,
                c"main".as_ptr(),
                args,
                ptr::null(),
                0,
                &mut id,
            ));
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
    fn prefix(&mut self, steps: usize) -> Duration {
        let start = Instant::now();
        for _ in 0..steps {
            checked(unsafe { bennett_run_step(self.program, self.id) });
        }
        start.elapsed()
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

#[test]
fn installed_target_prefix_cost_stays_within_twice_the_initial_cost() {
    let mut times = vec![];
    for steps in [500, 5000, 15000] {
        let mut run = Run::new();
        let elapsed = run.prefix(steps);
        eprintln!("{steps} cooperative steps: {elapsed:?}");
        times.push(elapsed.as_secs_f64() / steps as f64);
    }
    let ratio = times[2] / times[0];
    assert!(
        ratio <= 2.0,
        "per-step cost grew {ratio:.3}x; seconds/step {times:?}"
    );
}

#[test]
#[ignore = "published oracle differential; mandatory in the gate"]
fn full_object_matches_reference_output_within_ten_times_its_walltime() {
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
    let start = Instant::now();
    let output = Command::new("taskset")
        .args(["-c", cpu, "timeout", "180s"])
        .arg(&reference)
        .arg("run-program")
        .arg(fixture("launch.bennett"))
        .arg("main")
        .arg("--replies")
        .arg(fixture("replies.json"))
        .env("BENNETT_VM_ENGINE", "interpreter")
        .output()
        .unwrap();
    let reference_time = start.elapsed();
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
    let start = Instant::now();
    let output = Command::new("taskset")
        .args(["-c", cpu, "timeout"])
        .arg(format!("{:.6}s", reference_time.as_secs_f64() * 10.0))
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "step_scaling::full_object_matches_reference_output_within_ten_times_its_walltime",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(WORKER, "1")
        .env("BENNETT_VM_ENGINE", "interpreter")
        .output()
        .unwrap();
    let vm_time = start.elapsed();
    let ratio = vm_time.as_secs_f64() / reference_time.as_secs_f64();
    eprintln!("reference {reference_time:?}, VM {vm_time:?}, ratio {ratio:.3}");
    assert!(
        output.status.success(),
        "VM/reference wall ratio {ratio:.3}, exit {:?}: {}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(ratio < 10.0, "VM/reference wall ratio {ratio:.3}");
}

#[test]
fn classical_accounting_cost_stays_within_twice_the_initial_cost() {
    use bennett_vm::target::{Event, Rational, Target};
    use std::collections::BTreeMap;
    let target = Target::parse(std::str::from_utf8(TARGET).unwrap()).unwrap();
    let mut times = vec![];
    for count in [500u64, 5000, 15000] {
        // Hand-derived: disjoint unit intervals on no wires consume `count`
        // time units, zero gates, and zero wire occupancy.
        let events: Vec<_> = (0..count)
            .map(|i| Event {
                start: Rational::from(i),
                duration: Rational::from(1),
                wires: vec![],
                cost: BTreeMap::new(),
            })
            .collect();
        let start = Instant::now();
        let report = target.account(&events, &[]).unwrap();
        let elapsed = start.elapsed();
        eprintln!("{count} classical accounting events: {elapsed:?}");
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
        times.push(elapsed.as_secs_f64() / count as f64);
    }
    let ratio = times[2] / times[0];
    assert!(
        ratio <= 2.0,
        "per-event accounting cost grew {ratio:.3}x; seconds/event {times:?}"
    );
}
