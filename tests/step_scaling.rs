//! Cooperative stepping must cost flat work per executed row, however long
//! the retained history already is, and the produced object that exposed the
//! growth must complete with the reference interpreter's output.
//!
//! Fixtures (tests/fixtures/r3-stepping/): `launch.bennett` is a produced
//! reversible-tier object (mixed-vararg musl printf shape, 12 functions,
//! sha256 5d1edcef…b302); `ideal-all-to-all.json` is the target document the
//! producing harness installed at load; `expected.json` records the pinned
//! reference interpreter's run of the same object (bennett-isa-spec@
//! 3b1090790d82 from gate.toml, `run-program … --budget 5000000,65536,8192,64`
//! with a 20 zero-byte `bennett.argv` reply, re-run and recorded 2026-11-07):
//! COMPLETED, 225956 steps, one i32 result 0, 54 console bytes.
//!
//! Work is counted, never timed: the unit is actual `target::Event` clones
//! (see `work::EVENT_CLONE_COUNT`). With a target installed every executed
//! row records one classical-step event in the run's accounting journal, so
//! journal-clone work per `bennett_run_step` is the per-step cost that must
//! not grow with history. Measured 2026-11-07 on an Intel i7-1365U (shared
//! 12-thread host), release build, via the launch ABI exactly as driven here:
//! before the fix, windows cost 106us/step at 500 steps and 3402us/step at
//! 5000 (clones/step ≈ 3 × history length; 15000 steps did not finish in
//! 90s); the same object with no target installed cost a flat ~13–22us/step.
//! The assertions below use clone counts only; the suite timeout is the hang
//! guard, and no seconds are asserted anywhere.
use bennett_vm::{json::Json, launch::*, work};
use std::{ffi::CStr, path::PathBuf, ptr};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/r3-stepping")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {}: {e}", path.display()))
}

/// A run of the fixture object, admitted with the fixture target and driven
/// through the `bennett.argv` host request with the harness's 20 zero-byte
/// reply. Returns (program, run, entry snapshot taken before any row ran),
/// positioned to execute the remaining rows.
unsafe fn admitted_run() -> (*mut bennett_program_t, u64, Json) {
    let object = fixture("launch.bennett");
    let target = fixture("ideal-all-to-all.json");
    let mut program = ptr::null_mut();
    assert_eq!(
        unsafe {
            bennett_load(
                object.as_ptr(),
                object.len(),
                target.as_ptr(),
                target.len(),
                &mut program,
            )
        },
        0
    );
    let mut args = ptr::null_mut();
    assert_eq!(unsafe { bennett_args_new(0, &mut args) }, 0);
    let mut run = 0u64;
    assert_eq!(
        unsafe { bennett_run(program, c"main".as_ptr(), args, ptr::null(), 0, &mut run) },
        0
    );
    unsafe { bennett_args_free(args) };
    let entry = unsafe { snapshot(program, run) };
    let (mut result, mut request) = (ptr::null_mut(), ptr::null_mut());
    assert_eq!(
        unsafe { bennett_wait(program, run, &mut result, &mut request) },
        0
    );
    assert!(result.is_null() && !request.is_null(), "expected HOST_REQUEST");
    let service = unsafe { CStr::from_ptr(bennett_request_service(request)) };
    assert_eq!(service.to_str().unwrap(), "bennett.argv");
    unsafe { bennett_request_free(request) };
    assert_eq!(unsafe { bennett_reply(program, run, [0u8; 20].as_ptr(), 20) }, 0);
    (program, run, entry)
}

unsafe fn snapshot(program: *mut bennett_program_t, run: u64) -> Json {
    let mut snap = ptr::null_mut();
    assert_eq!(unsafe { bennett_run_snapshot(program, run, &mut snap) }, 0);
    let mut len = 0usize;
    let bytes = unsafe { bennett_snapshot_jcs(snap, &mut len) };
    let text = String::from_utf8(unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec())
        .expect("snapshot JCS is UTF-8");
    unsafe { bennett_snapshot_free(snap) };
    bennett_vm::json::parse(&text).expect("snapshot JCS parses")
}

fn expected() -> Json {
    bennett_vm::json::parse(std::str::from_utf8(&fixture("expected.json")).unwrap())
        .expect("expected.json parses")
}

/// Cumulative journal-clone work per cooperative step at history checkpoints.
/// Flat means the 5000- and 15000-step averages stay within 2× the 500-step
/// average. Before the fix each step deep-copied the whole event journal
/// three times (sync, rollback checkpoint, export), so clones/step grew
/// linearly (≈3 × history) and the 5000 checkpoint already failed by ~10×.
#[test]
fn cooperative_step_journal_work_is_flat_in_history() {
    unsafe {
        let (program, run, _) = admitted_run();
        work::EVENT_CLONE_COUNT.with(|n| n.set(0));
        // Assert at each checkpoint as it is reached, so a growing journal
        // fails at 5000 steps instead of first running to 15000.
        let mut base: Option<(u64, u64)> = None; // (steps, clones) at 500
        for step in 1..=15_000u64 {
            assert_eq!(bennett_run_step(program, run), 0, "step {step}");
            if ![500, 5_000, 15_000].contains(&step) {
                continue;
            }
            let clones = work::EVENT_CLONE_COUNT.with(|n| n.get());
            // The journal really is being written: one classical event per row.
            assert!(clones > 0, "no target events cloned; counter or journal inert");
            match base {
                None => base = Some((step, clones)),
                Some((base_steps, base_clones)) => assert!(
                    // clones/step <= 2 * base_clones/base_steps, cross-multiplied.
                    clones * base_steps <= 2 * base_clones * step,
                    "journal clones per step grew with history: {base_clones} clones / {base_steps} steps, then {clones} clones / {step} steps"
                ),
            }
        }
    }
}

/// End to end on the produced object: completion, exact console bytes and
/// row count from the reference interpreter (expected.json), then reversal
/// back to an empty console and tape. Ignored in the default set only for
/// debug-build duration; mandatory in the gate's ignored set.
#[test]
#[ignore = "produced-object end-to-end (226k rows); mandatory in the gate"]
fn produced_object_completes_with_reference_output_and_reverses() {
    let want = expected();
    let want_console = want.get("console_hex").unwrap().as_str().unwrap();
    let want_steps = want.get("steps").unwrap().as_u64().unwrap();
    unsafe {
        let (program, run, _) = admitted_run();
        let (mut result, mut request) = (ptr::null_mut(), ptr::null_mut());
        assert_eq!(bennett_wait(program, run, &mut result, &mut request), 0);
        assert!(request.is_null() && !result.is_null(), "expected completion");
        assert_eq!(bennett_result_nfaults(result), 0);
        assert_eq!(bennett_result_nvalues(result), 1);
        let (mut kind, mut width, mut bytes) = (0i32, 0u32, [0u8; 4]);
        assert_eq!(
            bennett_result_get(result, 0, &mut kind, &mut width, bytes.as_mut_ptr(), 4),
            0
        );
        bennett_result_free(result);
        assert_eq!((kind, width, bytes), (0, 32, [0; 4]), "i32 result 0");
        let forward = snapshot(program, run);
        assert_eq!(forward.get("state").unwrap().as_str().unwrap(), "COMPLETED");
        assert_eq!(forward.get("console_hex").unwrap().as_str().unwrap(), want_console);
        assert_eq!(
            forward.get("history_length").unwrap().as_u64().unwrap(),
            want_steps,
            "executed rows must equal the reference step count"
        );
        assert_eq!(forward.get("fault").unwrap(), &Json::Null);
        assert_eq!(bennett_reverse(program, run), 0);
        let reversed = snapshot(program, run);
        assert_eq!(reversed.get("console_hex").unwrap().as_str().unwrap(), "");
        // No entry/reversed memory_digest equality here: undo canonicalises
        // allocation-table order (Data::table_sorted), so the digest of the
        // identical memory differs between a fresh entry snapshot and a
        // reversed one. Probed 2026-11-07 on this fixture: the reversed
        // digest equals a second fresh run's entry digest on the same
        // program (state restored; serialization order differs).
        assert_eq!(reversed.get("tape").unwrap().as_arr().unwrap().len(), 0);
        assert_eq!(bennett_run_close(program, run), 0);
        assert_eq!(bennett_unload(program), 0);
    }
}
