# VM execution model: retain the semantics, reduce the work per row

## 1. Reference execution, reversal, replay and faults

This is a research proposal, not an implementation or a new benchmark result. VM
source citations below mean **d68eb76** unless another SHA is given. `REF` means
`bennett-isa-spec@3b1090790d82a00ac95d30b8da6b42d1d7a9f5d1/reference-interpreter/`;
`ISA` means the supplied `bennett-isa-spec/spec/Bennett-ISA-Spec-0.9.md` at
**62a0cf0**. The latter adds K94 after the VM's gate pin; its relevant CC/R memory,
history and launch rules are unchanged apart from line offsets. These versions
must remain explicit: `gate.toml:23-32` pins the oracle to 3b10907, not sibling
HEAD. The proposal is authorized by R37 and amended by R40: acceptance counts
deterministic work; elapsed time cannot make a performance test red
(`bennett/.tmp/orchestrator/advisor-notes.md:442-456,476-481`;
`bennett/.tmp/orchestrator/briefs/COMMON.md`, final bullet).

Evidence comments use `job/r3-vm-stepping/<file>` for supplied files under
`/home/tobias/Projects/bennett-workspace/bennett-vm-job-r3/.tmp/r3-vm-stepping/`.
Their head, host and date identify the reported measurement, not a measurement
performed by this research job. Source line numbers, ISA widths and review-round
policy counts are citations/semantic or policy units, not fitted performance
constants (`briefs/COMMON.md`, final bullet).

The reference's important distinction is **current execution state versus retained
undo records**. `ClassicalMachine` owns its current snapshot, a `Vec<HistoryFrame>`,
a `Vec<Segment>` and an initial snapshot. Each history frame contains an `Undo`
record and reversal/purity/irreversibility flags, rather than a pair of full
machine states. Undo records carry definitions, displaced bytes, allocation/free
changes, device changes, prior control and any resolved region invocation
(`REF/src/classical.rs:291-341,380-390`).

The pinned reference is more specific than the shorthand “flat memory + journal”:
its resident bytes are a **sorted contiguous sparse vector** of `(u128, u8)` behind
`Retained`, not an address-space-sized byte array. Lookup uses binary search;
insertion/removal shifts the vector. Zero bytes are omitted. `Retained<T>` holds
`Arc<Vec<T>>` plus a logical length; cloning shares a buffer, truncation changes the
length, and `prepare` copies a shared prefix before mutation. The allocation table
is a plain vector and validity scans its live entries. Thus these choices have low
constants on the measured object, but do **not** establish O(1) arbitrary sparse
writes, allocations, or snapshots (`REF/src/retained.rs:5-29,50-73,76-131`;
`REF/src/oracle.rs:133-151,268-303`).

Forward execution prepares capacity for one history record and executes the row
against the current state. Stores first save `(address, old byte)` for each
overwritten byte. Reversal visits those pairs backwards; allocation undo restores
the old heap position and table length; free undo restores its live bit.
Definitions pop their dynamic instance. Device undo stores a console length,
clock/entropy position, or old block sector with an explicit absence case; it also
pops the active effect-tape entry. Regions retain bindings and invoke the inverse
once, rather than rerunning forward execution to rediscover the record
(`REF/src/classical.rs:1200-1281,1322-1407,1484-1506,1583-1599`).

This is not a checkpoint-free reference. `transaction` shares/clones the current
snapshot, records history/segment lengths, and additionally clones history or
segments when an operation edits their earlier entries. It restores state by
assignment and restores/truncates the logs on a diagnostic. In particular,
`uncall` can copy history; writes to shared sparse/console/tape buffers can copy
resident payloads. Ordinary append does not clone the whole history. The dynamic
environment itself is a persistent AVL tree whose per-name value stacks share
immutable tails (`REF/src/classical.rs:721-751,1409-1448`;
`REF/src/instances.rs:1-22,67-88,142-149`).

Segment reversal checks the latest completed live segment, exact boundary values,
purity of its suffix and irreversibility of its contents, then undoes its range
in LIFO order. Already-reversed frames and uncall markers are skipped. Whole-run
reversal restores administrative entry control and checks equality with the
initial tuple; this costs work proportional to the history traversed, and the
reference's segment/suffix searches are linear, not a model of the desired index
(`REF/src/classical.rs:873-904,1274-1281,1745-1784`).

There are **two tapes**, with different lifetimes. The active typed device tape
is reversible. The CLI's `EffectTapeSession` is an executed-effects journal:
`expected`, `recorded`, and an output destination. Its observer checks
`dev/op/args/bytes` against the next expected entry and reserves journal capacity
inside the step transaction. Only a successful step publishes the journal entry;
end-of-run validation rejects unused input entries. Cancelling an effect by
`uncall` does not remove its execution-journal entry. Supplied entropy and checked
host reply queues remain separate inputs; replay does not authorize a new host
request to reconstruct history (`REF/src/classical.rs:76-92,764-787`;
`REF/src/main.rs:582-672,743-819,839-864`).

Exact faults are terminal records, distinct from executor/observer diagnostics.
The reference validates memory/device preconditions before mutations, maps only
the designated trap errors to code 4, preserves an explicit nonzero uint64 fault
code, restores the failing control point for ISA 0.9, and appends one impure frame.
Other errors escape through the transaction and leave its prefix retryable. Trace
JSON is optional observer work; the normal unobserved run need not construct it
(`REF/src/classical.rs:1304-1320,1786-1793,1878-1911`;
`REF/src/main.rs:833-860`).

## 2. What the VM does instead, and why

The present structures have identifiable origins. `git log -S` finds the first
`Frame`/`trace.push` in **ed55cb4**, `trace_enabled` and byte-persistent memory in
**87b1887**, and both `memory_locks` and `let checkpoint = self.machine.clone()`
in **6d96f5c**. The following explains the behavior supported by those diffs and
messages; it does not infer undocumented author intent.

| Structure now | Why it appeared; obligation served | Remaining cost or choice |
|---|---|---|
| Dynamic register instances and one retained frame per row | **ed55cb4**, “Implement oracle and CC/R execution with retained history and launch CLI”; frames retain definitions/control, purity, reversal and uncall marks. This implements dynamic SSA and LIFO reversal. | A second ordered map per name, keyed by `(history row, definition ordinal)`, adds lookup/update work. `src/cc.rs:15-19,64-81,478-492,750-771`. |
| `Data` before/after roots in launch frames | **a0c6029**, “Implement cooperative launch ABI and retained-history runtime adapter,” adds `Frame.after`, `stateful` and `undo_delta`. Restoring only the changed keys protects other runs' disjoint state. **6d96f5c** further fixes independent run resources and allocation undo. | Root pairs are an implementation of deltas, not a requirement to retain two `Data` values. Replacing them with a whole old global image would reintroduce the isolation bug. `src/cc.rs:569-580,1981-2048`. |
| Per-step whole-`Machine` rollback checkpoint | **6d96f5c** adds it around `RuntimeRun::step`, restoring the machine on `Err`; it also strengthens C launch entry checks. | Separate from the semantic per-row `Frame`: it protects diagnostics/retry. Root sharing is cheap only for shared fields; quantum amplitudes and some vectors still clone. `src/cc.rs:91-119,2426-2444`; `src/action.rs:43-60`. |
| Persistent AVL maps and sequences | **87b1887**, “Fix super-linear CC execution checkpoints and trace bookkeeping (fw8),” replaces full environment/history copies and whole-state trace scans with shared roots and touched-key work. Its `docs/PROGRESS.md:1-6` records the original register-read explosion and red/green evidence. | `Seq` is an AVL map of indices: append O(log H), index O(log H), index-based iteration O(H log H). At d68eb76 path copying also clones inline values. `src/persistent.rs:5-18,325-386`. |
| Allocation interval and exact-base indices | **87b1887** introduces indexed validity; **13f5a9e**, “Preserve zero-sized allocation locks with indexed lookups,” adds the exact-base fallback for empty ranges. | Keep the empty-allocation case and allocation order. These are useful indices, not incidental overhead to delete wholesale. `src/cc.rs:316-339,2370-2393`. |
| Trace with register/memory deltas and source row identity | **ed55cb4** adds diagnostic traces. **87b1887** makes program-only CLI trace-free and uses changed keys/NaN indexing for observed modes. | Launch still hard-codes trace on. Each executed row builds nested JSON and retains it even when only a final result is requested. `src/cc.rs:689-748,1768-1772,2188-2190`. |
| Run-local tape/host/measurement extraction from frames | **6d96f5c** removes shared tape-start offsets and reconstructs each run's records from its own non-reversed frame deltas. | Fixes cross-run contamination, but `records` scans history and uses a prefix-skipping sequence iterator. Repeated tape-bearing frames can cause repeated prefix traversal. `src/cc.rs:2195-2229`. |
| Saved execution journal separate from reversible device tape | **35bc619**, “save and consume deterministic effect journals (u8m),” adds strict entry validation, expected-entry checking and end-consumption checking. | JSON is retained internally, and replaying effects also takes a conditional machine checkpoint. Preserve this distinction even if internal payloads become typed. `src/cc.rs:120-190,643-659,1221-1263`. |
| Program-wide register and memory ownership | **a0c6029** establishes cooperative launch; **6d96f5c** replaces a single global memory owner with range/entry/device/allocator keys, permitting independent resource use. | A vector performs conflict and same-owner duplicate scans and lives until CLOSE. This is not the measured 8f3 cause, but grows with distinct lock keys. `src/launch.rs:113-118,172-237,690-708`; `src/cc.rs:2050-2063`. |
| Exact FAILED-run reversal and qreg validation | **7f8e35b** explicitly adds atomic overlap checks, `measure.iN`, FAILED reversal at 0.9 and exact measurement tie handling. | These are semantics to preserve, not costs that can be bypassed. `src/launch.rs:663-685`; commit **7f8e35b**. |

The measured successor **8a68da4** already repairs three costs; it is a prerequisite,
not work to repeat: **12cf12d** makes target histories persistent, **45f43a3**
pre-indexes nonempty-wire predecessors in accounting, and **8a68da4** stores
`Rc<V>` in persistent nodes so search-path copies share retained payloads.
The source still traverses wire-bearing predecessors and, with `max_parallel`,
the event list; this is not a general proof that all quantum accounting is linear
(`8a68da4:src/persistent.rs:5-18,104-150`;
`8a68da4:src/target.rs:915-943,963-978`).

Use the supplied measurements, without re-measuring them here. The unchanged
object executes **225,956 logical rows**. <!-- Measurement: job/r3-vm-stepping/PROGRESS.md:18, head 8a68da4; machine: this 12-core host; date: 2026-11-07. -->
One supplied same-CPU release pair has an **informational process-time ratio of
4.897404×**. <!-- Measurement: job/r3-vm-stepping/PROGRESS.md:18, head 8a68da4; machine: this 12-core host; date: 2026-11-07. -->
This is neither a work ratio nor a performance acceptance result under R40; it
cannot calibrate a work ceiling. Prefix timing, CPU timing and profile percentages
likewise do not count map operations, allocations or bytes. The earlier profiles show
event-vector/Rational copying and allocator work; the later debug profile includes
JSON subtree cloning. They precede the final shared-payload change and cannot rank
the residual costs at 8a68da4
(`bennett-vm-job-r3/.tmp/r3-vm-stepping/perf-report.txt:13-78`,
`full-debug-perf.txt:13-38`; **8a68da4**).

The deterministic retained-payload counter is relevant work evidence: appending
behind the retained root made **15 payload clones** before the fix and
**zero** afterward at the measured retained-prefix sizes.
<!-- Measurement: job/r3-vm-stepping/value-sharing-red.txt at head 4b67661 and value-fix.txt at head 8a68da4; machine: this 12-core host; date: 2026-11-07. -->
This proves the measured collection invariant, not a whole-VM/reference work bound
(`8a68da4:src/persistent.rs:474-495`; fix-job `PROGRESS.md:9,16`).

In particular the strike-force README's “no target” premise is refuted by the
reported original harness `tests/varargs.py:140-144` and the target-installed
fixture at **8a68da4**, `tests/step_scaling.rs:38-48`. The static reports disagree
on locks. Later diagnostic instrumentation at unchanged head **8a68da4** counts
**31,334,051 conflict-owner entry visits** and **1,625,740 duplicate-key entry
visits** over that same complete object; the candidate overlap function is never
called in the single-owner run.
<!-- Measurement: job/r3-vm-stepping/r37/locks.txt and PROGRESS.md:27-31, production head 8a68da4 with temporary diagnostic instrumentation; machine: this 12-core host; date: 2026-11-07. -->
Those counts are an actual work baseline. They refute a history-proportional lock
cardinality claim for this object; they do not measure multi-owner contention or
make the vector scan a dominant cost. Treat all three static reports as hypotheses
rather than fresh measurements
(`bennett/.tmp/orchestrator/vm-8f3/README.md:1-20`,
`step-engine.md:9-19`; fix-job `PROGRESS.md:7-10`).

## 3. ISA constraints versus VM choices

| Binding behavior | Constraint on a replacement | Representation left open |
|---|---|---|
| Sparse little-endian byte memory, complete live-allocation range validation, bump allocation, pinned globals, zero-size allocation and no reclamation | Preserve bytes, table entries/order, live/pinned bits, bump and fault-before-mutation behavior. | Per-byte AVL, sparse pages, radix indexing and buffers are choices. `ISA:400-408,1672-1691`. |
| One logical history frame per executed instruction and retained dynamic instances | Preserve source-order identity, instance order, displaced values, call/return boundaries and uncall marks. A diagnostic trace is not a substitute for history. | Checkpoints, deltas or deterministic replay are explicitly permitted. Reconstruction must not reissue effects/host requests, rerun regions for records, or copy Q state. `ISA:591-605,1761-1795`. |
| Segment uncall and whole-run reversal | Choose the most recent completed live segment, compare all boundary values, reject an impure suffix or irreversible contents, undo LIFO and skip markers/reversed frames exactly once. | Completed-segment stacks, prefix counts, sorted row indices or range indices may replace scans. Reversing K frames still costs O(K plus their deltas). `ISA:1797-1839`. |
| Atomic effects and active tape | Preserve the device-specific undo data, including absent versus present-zero sectors; failed effects append nothing. Tape order and unsigned little-endian argument encoding remain observable. | Typed internal events and direct frame-to-event indices may replace JSON and history scans. `ISA:1714-1759,2522-2525`. |
| Saved execution journals | ISA §7.3 explicitly delegates persistence/replay interchange and validation to a versioned tool profile. Preserve the VM's already supported strict saved-tape behavior and separately supplied entropy/replies. | Neither “not in the core ISA” nor trace-off permits deleting this interface. Internal encoding may change while current file bytes and validation stay compatible. `ISA:1714-1716`; **35bc619**; `src/cc.rs:120-190`; `tests/saved_tape.rs:135-200,232-290`. |
| Exact terminal faults | One exact uint64 cause/program point, no results or caller continuation, intact prefix and open activations, charged classical row, eligible FAILED reversal. | Preparation plus a typed fault frame can replace snapshot restoration. Ordinary diagnostics must retain their separate retry/rollback behavior. `ISA:1171-1212,2462-2465,2489-2502`; `src/cc.rs:2426-2444`. |
| §10 ownership through CLOSED | Preserve non-CLOSED ownership across RUNNING, SUSPENDED, HOST_REQUEST, COMPLETED, FAILED and REVERSED; preserve from-id touches and reverse eligibility. | An interval map may replace a vector. One currently RUNNING run does not imply no other owner. `ISA:2467-2487`; `src/launch.rs:690-708`. |
| Snapshot/report data | Preserve supported values/faults/measurements/tape and full semantic observations. Current snapshot fields include instances, frame marks, memory digest, devices, host requests, trace and target resources/error. | `bennett_run_snapshot` and its JSON layout are VM extensions, not listed §10 functions. Keep their supported projection; compute JSON lazily and expose trace collection explicitly. Digest encoding is VM-specific. `ISA:2489-2502,2527-2584`; `src/launch.rs:1029-1053`; `src/cc.rs:2475-2545,362-389`. |
| Target accounting and Q records | Preserve every leaf's charge, makespan, occupancy, peak, effective controls, resolved token identities and region inverse. | Compact classical accounting must remain lossless; do not remove events just because the object is classical. This proposal does not change the Q expansion or use classical history optimization as oracle pebbling. `ISA:1781,1905-1928,5772-5787`. |

T4 requires restoration of the active environment, memory, allocation table,
device state, register ledgers, token validity and active history. It permits
retained audit records and does not retract observed external events. Exact Q
restoration also depends on the stated inverse-realisation hypothesis; approximate
operations must retain their declared error semantics. “Byte-exact reversal” below
therefore means every classical byte and ledger field, plus exact Q cases under
T4's hypothesis, not a new exactness promise for approximate gates
(`ISA:4978-4991`).

No ISA clause requires an AVL node for every byte, `Data` pairs per frame, a
whole-`Machine` clone per attempted row, nested trace JSON per row, textual
operand/function lookup per step, or a vector of historical lock keys. The
permission to change representation comes from logical-history equivalence and
opaque launch objects, with all the obligations above still binding
(`ISA:1784-1795,5655-5656`; `src/cc.rs:35-118`; `src/launch.rs:113-118`).

## 4. What the JIT bypasses and what it shares

`BENNETT_VM_ENGINE=jit` selects all private passes plus native code; the default
is interpreter, other named modes use subsets/non-native dispatch, and invalid
selection errors rather than falling back. Compilation/cache lookup is per object,
keyed by published object identity and private executable identity
(`src/jit.rs:76-161`).

The JIT recovers CFG edges, clones one callee level without eliminating runtime
call/return bookkeeping, reindexes private nodes, and folds supported known
integer operations. Generated host code dispatches to a Rust row helper.
For integer binops of width <=64, `value::binop` can use a native arithmetic
kernel; wider values keep the runtime helper. Canonical source rows remain
available for history, faults and trace
(`src/jit.rs:220-310,312-383,585-629`; `src/value.rs:149-164`).

There are two materially different CC entry paths:

| Entry path | Already bypassed | Still paid |
|---|---|---|
| CLI `Machine::drive` | One `Program::run` can follow canonical successors and recovered edges over many rows. It also avoids the launch adapter's ownership/sync/export path, regardless of engine. | `step_row`/`step_row_inner` execute each logical row and retain its frame. Program-only CLI has trace off already. `src/cc.rs:609-641,661-771,1768-1779`. |
| Cooperative launch `Machine::step` | Native dispatch avoids rebuilding the non-native `defaults` vector; simplified effective instructions can reduce operation work. | The callback explicitly returns `None` after **one** row. Each next ABI step re-enters dispatch; `step_row_inner` still fetches/clones the original row for identity, and `effective.unwrap_or(...)` in `step_row` eagerly evaluates the original lookup. Locks, resource sync/export, checkpoint, memory, instances, history, effects, trace and snapshot paths are shared. `src/cc.rs:597-607,643-670,2188-2211,2426-2444`; `src/launch.rs:172-237`. |

Non-native compiled modes allocate and fill `defaults` on every `Program::run`,
O(program rows). Native generated canonical dispatch uses linear comparisons;
its helper validates a returned successor by scanning nodes. The latter scan is
on continuing `drive` calls, not the one-row callback returning `None`. Re-entering
native dispatch for every ABI row can still be expensive. These source paths do
not justify claiming that enabling JIT solves memory/history/launch overhead
(`src/jit.rs:389-425,613-625,643-658`). Later supplied paired runs in
`job/r3-vm-stepping/r37/engine-results.json`, production head **8a68da4**, on this
12-core host, dated **2026-11-07**, report matching completion, results, console,
row count and target charges for interpreter and JIT. Their elapsed timings are
informational; no JIT dispatch-operation or reference work ratio was measured.
Those missing counters remain in §6 (`advisor-notes.md:453-456`).

## 5. Proposed slices, in dependency order

Land only after the 8f3 successor **8a68da4** is integrated; it owns overlapping
execution work. Each slice is a **one-review-round prototype** with one sol review,
no council: the VM consumes the oracle, it is not the oracle. Estimates below are
planning estimates, not completed reviews. Preserve every supported path while
migrating; never convert an unimplemented optimization into a new product fault,
drop an obligation or weaken an existing check. Split a slice if its measurement
shows that one review cannot contain it (R37 `advisor-notes.md:446-456`;
`briefs/COMMON.md`, severity/prototype rules; **8a68da4**).

Acceptance throughout is **forward output = pinned reference; byte-exact eligible
reversal; deterministic replay**, plus existing VM API behavior. For every slice:

- Compare status, values, exact fault code/point, logical rows/instances, host
  transcript, active tape, devices and target time using the same object and inputs.
  Target time is the ISA's counted classical-step/scheduled-event charge, not
  measured host duration (`ISA:1905-1928`).
  Run both engines in fresh processes because selection is cached. Current
  differential checks compare every JSON field except implementation-specific
  digest encodings (`tests/differential.rs:479-527`; `src/jit.rs:78-80`).
- Check reversal against explicit bytes and allocation/ledger/token/device tuples,
  not just matching digests. Include nested/cancelled uncall, a segment starting at
  frame zero, open faulting activations, 0/max uint64 faults, zero allocations,
  block absence, qreg classes/views and rejection after measurement/unverified
  free (`ISA:1767-1839,2476-2487,4978-4991`; **7f8e35b**).
- Replay saved journals with separately fixed entropy and host replies; test
  cancelled effects, exhausted/extra/malformed/mismatched tape, fault-before-effect
  and diagnostic retry without cursor consumption. No external event is reissued
  during reversal or history reconstruction (`tests/saved_tape.rs:135-229,268-290`;
  `REF/src/main.rs:626-660`; `ISA:1784-1795`).

### Work ratios, evidence and complexity

The final target is a **bounded VM/reference work ratio per logical row** on the
unchanged target-installed object. This replaces the former elapsed-time ceiling;
it does not reinterpret an observed time multiple as measured operations. No
numerical VM/reference work ceiling is asserted before the missing paired
counter measurement in §6. The previous numerical factor cannot be asserted in
work units from timings alone. Existing timing tests at **8a68da4** are historical
evidence, not the proposed R40 acceptance mechanism
(`8a68da4:tests/step_scaling.rs:38-48,83-128,147-231`;
`advisor-notes.md:476-481`).

Define a vector `W` of named, separately counted work units: allocation requests,
allocated payload bytes, key/node comparisons, path-copy nodes, payload clone
calls and copied bytes, memory bytes read/written, journal entries, lock entries
visited, and trace nodes constructed. Count before each operation; include failed
preparation work. Publish each phase's counters separately: load, execution,
reversal/replay, snapshot/export and teardown. Do not hide a cost by moving it
from stepping to export, and do not sum unlike counters with invented weights
(`src/cc.rs:343-389,689-771,1984-2048,2426-2545`;
`REF/src/classical.rs:1200-1281,1409-1448`).

For metric `k`, define `Rwork[k] = (Wvm[k]/N)/(Wref[k]/N)` on the same object,
inputs and complete logical-row count `N`, with comparable event definitions in
both independently instrumented executors. VM tree probes/path copies must be
compared with the reference's sparse lookup/shift/COW and journal work, not called
equivalent merely because both expose a method named `write`. Keep the metric
vector visible. If the reference counter is absent or zero, report that metric
as VM-only excess work rather than dividing by an arbitrary epsilon. Cross-VM
lock ownership is such a VM-only metric; the reference has no shared launch lock
manager (`src/cc.rs:351-359`; `REF/src/retained.rs:76-131`;
`REF/src/classical.rs:380-390`). Independent reference instrumentation is a
follow-up measurement job, not a source edit authorized by this design job.

For retained-prefix scaling, define
`Rprefix[k] = (W[k, long]/rows(long))/(W[k, short]/rows(short))` using two runs in
the same process, fixed engine/settings and equal instruction footprints. The
supplied object's checkpoints are **500**, **5,000** and **15,000** post-reply
cooperative steps. <!-- Measurement: job/r3-vm-stepping/r37/locks.txt and PROGRESS.md:27-31, production head 8a68da4 with temporary diagnostic instrumentation; machine: this 12-core host; date: 2026-11-07. -->
Use them for same-object work observations, not an assumption that their different
row mixes cost equal work. For a flat-work assertion, use a repeated identical
operation after pre-seeding the same resident state and compare counters per
committed row; synthetic history, event and program-size parameters and nontrivial
ratio bounds need calibration in §6. Exact absence/equality assertions below are
design invariants, not fitted tolerance constants; their source-based red
candidates must still be demonstrated before implementing the slice.

Every numerical performance constant in a future test must carry a comment with
its evidence file under `job/r3-vm-stepping`, measured head, machine (**this
12-core host**) and measurement date. The same provenance is attached to retained
numbers in this note; policy review counts and ISA-defined units remain governed
by their cited policy/specification. Missing measurements stay in §6 without
guessed numerical values (`briefs/COMMON.md`, final bullet).

Wall time may only guard a hang: `guard >= 10 * measured_duration` for that phase,
input, instrumentation and engine, with the same provenance comment.
<!-- Policy multiplier: R40, advisor-notes.md:476-481 and COMMON.md final bullet, 2026-11-07; measured-duration source: job/r3-vm-stepping/PROGRESS.md:18, head 8a68da4, this 12-core host, 2026-11-07, only for its documented uninstrumented full-run phase. -->
A guard trip reports **UNKNOWN: incomplete work measurement**, never red, a
failed ratio, or implicit PASS. Counters and semantic comparisons from incomplete
runs are not acceptance evidence. Counter-instrumented/new fixtures need their
own duration measurement before setting a guard (§6); neither the old ratio-test
timeout nor a newly guessed duration is a permitted substitute. No build, test
or re-measurement was run for this amendment.

“O(1) amortised step” means ordinary fixed-width, fixed-footprint **execution and
append bookkeeping independent of retained H**, with amortised buffer growth.
Historical predecessor/range lookup is O(log H); output of H records costs O(H).
Undoing K records costs O(K + displaced payload). Memory allocation validation
and interval ownership have separate O(log A) and O(log L + overlaps) costs;
wide values and device transfers cost their bits/bytes. Do not call the entire
arbitrary-width, arbitrarily long `uncall` or lock acquisition O(1). This makes
the R37 target assessable without deleting the variable-work instructions
(`ISA:1672-1689,1805-1839,2472-2487`; `src/cc.rs:343-360,988-1015`).

### Slice 1 — explicit trace collection, lazy trace JSON

**Change/ownership:** `src/cc.rs`, `src/launch.rs`, `tests/step_scaling.rs`,
`tests/differential.rs`, `tests/suite.rs`. Introduce explicit per-run observation
configuration. Default production execution collects semantic history but does
not construct trace JSON. Keep observed CLI behavior and all current snapshot
fields; a legacy snapshot can project trace from compact row/typed deltas on
demand, while explicit trace mode retains the full diagnostic sequence. A
per-run builder setting is preferable; an explicitly documented process setting
is an initial option that preserves existing C signatures. If trace
reconstruction needs metadata not currently retained, retain that metadata rather
than return a misleading empty trace. Do not change object identity or the
existing ABI signatures to smuggle configuration through an object field
(`src/cc.rs:64-118,689-748,2188-2190,2475-2545`; **87b1887**).

**Red-first work test:** `trace_off_builds_no_step_json` runs the same object in
trace-off and trace-on modes in one process and reports
`Rtrace = step_trace_json_builds(off)/step_trace_json_builds(on)`, after asserting
that the observed-mode denominator is positive. The deterministic assertion is
`trace_builds_after_step(off) == trace_builds_before_step(off)`, with equal
logical rows and identical complete trace projection. The exact absence of
construction is the design invariant; no unmeasured tolerance is introduced.
Current launch always-on construction is the red candidate. Observe and cite its
counter measurement before production changes; additional allocation/byte ratios
await §6 calibration (`src/cc.rs:689-748,2190`).

**Differential/risk/rounds:** Apply all three common differential checks with trace
off/on, exact NaN/negative-zero trace encodings and hostcall reply deltas. Risk:
losing pre-values or original instruction identities during lazy reconstruction.
Estimate **1 round**, splitting projection mechanics first if needed. This slice
does not remove a frame, tape entry, fault or ownership check
(`src/cc.rs:697-717,737-748`; `REF/src/main.rs:786-817`).

### Slice 2 — append history and query it without prefix walks

**Dependency/change/ownership:** After slice 1; `src/cc.rs`, private
`src/history.rs` registered in `src/lib.rs`, `src/persistent.rs` only where directly needed,
`tests/step_scaling.rs`, `tests/saved_tape.rs`, `tests/segment_instances09.rs`,
`tests/suite.rs`. Use stable chunked append storage, separate reversal marks and
an explicit rollback edit journal for earlier marks. Keep its append-only chunk
directory under one run owner outside cheap checkpoint descriptors: a descriptor
records lengths/cursors, not a cloned shared `Vec` of all chunk pointers. Earlier
edits journal the old mark; failed publication truncates only its private tail.
Frozen records can be shared by result/snapshot owners without making the writer
copy a growing directory on each append. Store frame-local event references for direct tape/host/measurement
extraction. Preserve logical row/definition identities and existing snapshot
projection when rewinding a tail (`src/cc.rs:64-118,478-492,2213-2229,2446-2534`;
`8a68da4:src/persistent.rs:325-386`).

Maintain append-only sorted row lists for impure/irreversible frames, prefix
counts where useful, and the stack of completed live segments. Sorted lists
append amortised O(1) and answer predecessor/range queries by binary search in
O(log H); uncall pays separately for frames it actually reverses. Index purity
over the same logical/audit suffix as today, not only active frames. Reversed
records and markers cannot disappear from the logical audit merely to satisfy
the performance test (`src/cc.rs:988-1015`; `ISA:1784-1839`).

**Red-first work tests:** `history_lookup_does_not_walk_its_prefix` uses identical
fixed-size uncall targets behind short and long pure prefixes in one process.
Assert `prefix_frame_visits_after_query == prefix_frame_visits_before_query`;
count index probes separately and report `Rprefix[index_probes]`. Actual undo
payload stays in its own counter rather than disappearing from total work.
`effect_export_work_is_linear` asserts that emitted event references are visited
exactly once and that visits to skipped tape-prefix entries do not increase;
report `visited_event_refs/published_event_refs` on the same-object runs. These
are exact traversal invariants. Prefix sizes and numerical index-probe/allocation
ceilings lack measurements and remain in §6. Current suffix scans and
`iter().skip(prefix)` provide source-based red candidates; record the failing
counters before changing production code
(`src/cc.rs:1002-1008,2213-2229`; `src/persistent.rs:374-375`).

**Differential/risk/rounds:** Common forward/reversal/replay checks, plus repeated
tail rewind/execute, nested markers and saved events cancelled from the active
tape. Risk: stale indices and audit marks or aliasing of a shared mutable tail.
Estimate **1 round**; keep JSON export in the same review as event indexing so
there is no semantic gap (`ISA:1805-1839`; **6d96f5c**, **35bc619**).

### Slice 3 — sparse pages and explicit displaced-byte journals

**Dependency/change/ownership:** After slice 2; `src/cc.rs`, private
`src/classical_memory.rs` and `src/lib.rs` registration, `tests/memory_address09.rs`,
`tests/memory_files.rs`, `tests/devices.rs`, `tests/step_scaling.rs`,
`tests/suite.rs`. Replace `Map<BigUint,u8>` with sparse page storage and a write
journal of original bytes/ranges. Resolve a page once per span; preserve zero
elision, zero-fill/allocation semantics and the exact validity indices. During
this migration old frame roots can reference shared page versions; they must
not force a whole resident-memory copy per write. Use radix lookup bounded by
address width or an expected-constant-time page directory with deterministic
sorted export; preserve the complete pointer range without narrowing to native
`usize` or `u64` (`src/cc.rs:35-53,316-389,1984-2024`;
`ISA:1672-1691`; `src/oracle.rs` remains outside this slice).

**Red-first work test:** `memory_journal_retention_tracks_changed_bytes` repeats
the same validated nonzero store footprint in short- and long-history runs in
one process. Assert `byte_key_tree_operations_after == before` and
`journaled_original_bytes == committed_write_span_bytes`: the page replacement
must eliminate per-byte tree operations while retaining the displaced bytes.
Report `Rprefix[memory_metadata_probes]`, `allocations/committed_write_bytes`
and `copied_bytes/committed_write_bytes`, also accounting for page creation and
retained memory. Compare the same byte/probe/allocation metrics with the pinned
reference on the unchanged full object. Page size, synthetic history sizes and
numerical allocation-ratio ceilings have no supplied measurement; they remain
in §6. The current per-byte write loop is the red candidate, not a measured
promise to halve allocations (`src/cc.rs:351-359`; **8a68da4**).

**Differential/risk/rounds:** Common checks, explicit readback of every changed
byte, i80/i128 unaligned cross-page stores, large sparse addresses, pinned/empty
allocations, block present-zero versus absent, faulted writes and other runs'
untouched pages. Risk: page aliasing, arithmetic overflow, memory amplification,
and confusing “old byte is zero” with absent allocation/sector. Estimate
**1 round**, subject to page-directory measurements in §6
(`ISA:1674-1691,1723-1757`; `src/cc.rs:327-389,1984-2048`).

### Slice 4 — prepared row commits instead of whole-machine checkpoints

**Dependency/change/ownership:** After slices 1–3; `src/cc.rs`, `src/object.rs`,
`src/value.rs` only for directly needed typed operands, `src/launch.rs`,
`src/action.rs` for target-log ownership,
`tests/differential.rs`, `tests/classical09_launch.rs`, `tests/step_scaling.rs`,
`tests/saved_tape.rs`. Intern names/rows once and keep mutable activation-local
instance stacks with stable retained instance records; push/pop O(1) rather than
insert another tree entry per definition. Split each row into preflight,
capacity preparation, and publication of a complete typed delta/frame. Record
control changes and lengths, not the whole growing machine. Move remaining
history-growing append streams (target events/live intervals, saved effects,
console, replies and draws) to the slice-2 owned log/cursor mechanism where they
still use AVL `Seq`; otherwise an ordinary target-installed row would retain
an O(log H) append term. Preserve complete event/liveness records and accounting,
not just a scalar time counter. Replace before/after
`Data` comparisons with allocation, ledger, token and device deltas that apply
only to that run's touched keys; retain the independence fixed by **a0c6029** and
**6d96f5c** (`src/cc.rs:35-53,91-125,478-492,661-688,1981-2048,2426-2444`;
**12cf12d**, **8a68da4**).

Keep ISA faults and executor diagnostics distinct. A valid terminal fault commits
exactly its fault record/accounting; diagnostic failure publishes neither a row
nor consumed replay/host/measurement input. Prepare all fallible capacities and
bindings before publishing effects. Region invocation rollback must keep the
existing proven transaction until its helper can meet this boundary; never
“rollback” approximate gates by applying an inverse or clone Q state into history.
If integrating all helpers cannot fit one round, split this slice by row family,
with explicit migration coverage and the existing semantic path retained for
each unmigrated family, not an automatic error fallback
(`ISA:1173-1212,1784-1795,4978-4991`; `src/cc.rs:643-659,1266-1268,2439-2444`).

**Red-first work tests:** `prepared_pure_steps_copy_no_machine_payload` asserts
that neither the machine-checkpoint counter nor the instance-tree-update counter
increases while committing ordinary pure rows; report each counter per committed
row. Both current operations are visible in the cited implementation.
`fixed_row_cost_is_independent_of_history` compares metadata-probe work per row
for identical pre-seeded footprints in short and long runs in one process and
asserts equality, with append growth/allocations reported separately. Preserve
the already measured **zero retained-payload clones** invariant.
<!-- Measurement: job/r3-vm-stepping/value-fix.txt, head 8a68da4, contrasted with value-sharing-red.txt at head 4b67661; machine: this 12-core host; date: 2026-11-07. -->
Counter calibration, synthetic history sizes and numerical aggregate
VM/reference work ceilings remain in §6. Full-object work reports must keep
snapshot, oracle outputs and ISA target charges intact
(`src/cc.rs:486-492,2439`; **8a68da4**).

**Differential/risk/rounds:** Common checks plus diagnostic injection at every
preparation stage, reply retry, token publication, terminal fault inside an open
callee and disjoint interleaved runs. Risk: partial commits and consuming a
nondeterministic input on failure; this is the highest-risk slice. Estimate
**1 round per measured row-family slice**, never one claimed round for an
unbounded rewrite (`ISA:1173-1212,2458-2487,2511-2518`; **6d96f5c**).

#### Integer-value family implementation (prototype)

The first family migrates constants, integer binary/division/comparison rows,
select and integer/pointer casts. Each computes a typed definition or terminal
fault, validates target accounting and reserves fallible history/definition
capacity before publication. A successful prepared row retains definitions and
prior control without Machine or Data checkpoint pairs. Faults retain the
exact cause and failing control point; diagnostic preparation publishes nothing.
Canonical value-row dispatch is indexed at executable installation; transformed
JIT rows use the same evaluator and publication boundary.

Per-name instance tails now push/pop without an instance AVL update. Segment
cancellation below a newer pure suffix shares the older prefix and rebuilds only
that suffix. Immutable append tails also replace AVL target-event, live-interval
and diagnostic-trace streams, preserving complete accounting and ordered export.
Checkpoints share tails; destruction of long unique tails is iterative. Ordered
export visits every retained record and allocates a temporary reference vector;
that export work has not disappeared.

Red-first evidence is in .tmp/vm-slice4/receipts/red-prepared-valid.* and
red-fixed.*: base **99f938c**, with test-only counters, Spiral. The paired
single-process fixture executes **500 identical rows** behind retained prefixes
of **500 / 15000** rows, with a target and an idle live qreg. Before migration,
each measured run performed **500 machine checkpoints / 500 instance-tree
updates**; map work was **32440 / 42500** and path copies **21452 / 29500**.
The completed candidate (work-final.*) performs **zero checkpoints, zero
instance-tree updates and zero retained Frame/Json or target-event clones**.
Map work is **4000 / 4000**, path copies **500 / 500**: both prefix ratios are
exactly **1**. Append work is separately **2500 nodes / 332000 inline entry
payload bytes** per measured run; that byte counter excludes allocator headers
and allocations owned by values. It is not a total allocation-byte measurement.
These are exact work invariants, not elapsed-time thresholds or an aggregate
VM/reference work bound.

Focused receipts cover each preparation-boundary diagnostic and retry, real
target-accounting rejection, arithmetic fault/reversal, explicit cross-page
bytes, preserved token/ledger state and exact Q-state equality. The pinned
reference differential covers integer operations, casts, boundary faults and
region reversal across private transformation/engine and observation modes.
The unchanged target-installed full object checks forward output against the
pinned reference; saved-journal replay/reset runs both engines and trace modes.
Target execution tests retain the forward-plus-inverse charge and snapshot
immutability obligations. The landing gate and independent review remain the
orchestrator's checks.

This is the design-authorized family split, **not completion of every migration
mechanic**. Branch/load, resource-changing, effect/host and region families keep
their existing semantic transactions. The outer environment remains a name-keyed
persistent map; activation slots/name interning, typed touched-resource deltas,
effective JIT-row metadata caching and the remaining console/effect/tape/reply/
draw/invocation append streams are tracked in **bennett-vm-6q3**. Ordinary
fixed-footprint work here is independent of retained instances in one activation;
no global O(1) claim is made across growing distinct activations or names. The
parked launch/sched 0.9 capability is unchanged.

### Slice 5 — interval ownership with exact resource domains

**Dependency/change/ownership:** After slice 4's publication boundary is in
place; `src/launch.rs`, `src/cc.rs`, `tests/launch_c.rs`,
`tests/classical09_launch.rs`, `tests/step_scaling.rs`, `tests/suite.rs`.
Use an ordered map of non-overlapping owned half-open memory intervals, merging
only overlapping/adjacent ranges of the same owner. Keep separate exact owner
maps for allocation entries and device names, an allocator owner, existing qreg
ownership, and per-run acquired-key lists for close. Check the complete touched
set before publishing acquisition; do not expand intervals across untouched
gaps or let one same-owner interval hide a different-owner conflict. A query
costs O(log L + overlaps), and close costs the run's owned keys
(`src/launch.rs:118,187-211,690-708`; `src/cc.rs:2050-2063,2315-2395`).

**Red-first work tests:** `lock_acquisition_is_sublinear_in_distinct_ranges`
counts comparisons for equal-size touches with short and long disjoint interval
sets in one process; assert that unrelated intervals outside the lookup search
path are never linearly visited. Report comparisons per touch and
`Rprefix[lock_comparisons]`; synthetic sizes and numerical scaling ceilings await
§6 measurements. Use separate allocations where needed to keep entry-key
behavior equivalent. `full_object_reduces_lock_scan_work` compares total
conflict-owner plus dedupe-entry visits with the measured baseline in §2;
assert `candidate_visits < measured_baseline_visits` and report
`candidate_visits/measured_baseline_visits`. The baseline is the exact sum of
the supplied separate scan counters, not a time-derived estimate.
<!-- Measurement: job/r3-vm-stepping/r37/locks.txt and PROGRESS.md:27-31, production head 8a68da4 with temporary diagnostic instrumentation; machine: this 12-core host; date: 2026-11-07. -->
Before changing the vector, repeat that instrumentation and record the red:
unchanged scan work does not satisfy strict reduction. No reduction is claimed
yet. This is a candidate scaling slice, not a claim that locks caused the 8f3
slowdown (`src/launch.rs:187-208`; fix-job `PROGRESS.md:27-31`).

**Differential/risk/rounds:** Common checks on each run plus shared-program
interleavings: overlap denied, disjoint use accepted, empty allocation touched,
allocator/device conflicts, qreg.from_id and held locks after COMPLETE/FAILED/
REVERSE until CLOSE. Reference single-run outputs are an oracle for each run;
published §10 rules and ABI assertions determine cross-run ownership, because the
reference classical machine has no shared launch lock manager. Risk: releasing
too early or losing zero-length entry locks. Estimate **1 round**
(`ISA:2472-2487`; **13f5a9e**; `REF/src/classical.rs:380-390`).

### Slice 6 — compiled dispatch through the same row transaction

**Dependency/change/ownership:** After slices 1–5; `src/jit.rs`, `src/cc.rs`,
`src/object.rs`, `tests/jit.rs`, `tests/differential.rs`,
`tests/step_scaling.rs`, `tests/suite.rs`. Precompute canonical dispatch maps,
function/block/operand indices, and constant-time successor membership. Give
the cooperative path a direct compiled row entry; let wait batch rows only while
crossing the same per-row preparation/commit and ownership checks. Yield on the
same host-request/suspend/fault/terminal boundaries. Preserve original source
metadata and logical frames even when private instructions are simplified
(`src/object.rs:262-264`; `src/cc.rs:597-670`; `src/jit.rs:204-208,389-425,613-658`).

**Red-first work test:** `compiled_step_dispatch_is_independent_of_unused_rows`
keeps the executed trace identical while varying unreachable program rows in
short- and long-program runs in one process. Assert equality of dispatch probes
per executed row for each engine; report their in-process work ratio. Also
assert that per-step dispatch-table initialization visits do not increase.
Exercise high canonical ids and interior rows so native linear dispatch is
observable. Program sizes and numerical nontrivial bounds await §6 calibration.
Replace the elapsed-time full-object test with
`full_object_reports_reference_work_ratios`: compare the metric-wise `Rwork`
vector with measured ceilings only after §6 supplies the common reference/VM
counter definitions and values. Source scans provide the slice's red candidate;
the full-object work bound is not claimed measured or red today
(`src/jit.rs:404-409,613-625`; **8a68da4:tests/step_scaling.rs:165-231**).

**Differential/risk/rounds:** Common checks, every existing transformation mode,
recursion, interior-row jumps, i80 runtime helpers, exact fault source points,
replies and per-row target time. Risk: batching past a host-visible boundary or
charging optimized private nodes instead of original rows. Estimate **1 round**;
native code generation and scheduler semantics do not otherwise change
(`src/jit.rs:279-310`; `src/cc.rs:661-772`; `ISA:1557-1562,1905-1928`).

## 6. Measurements needed before sizing each slice

These are implementation-lane measurement requests, not tests or profiles run by
this research job. **Missing measurements have no numerical thresholds here.**
The source-based exact invariants in §5 can define a red-first test, but their
actual failing counter transcripts must precede production changes. Beyond
logical-row/checkpoint counts, the supplied per-operation measurements used
numerically are the retained-payload and lock
counters cited in §2; historical timings cannot fill the remaining gaps.

1. **Same-object reference/VM work vector and ceiling:** instrument both
   executors independently on the unchanged complete object, preserving the
   published reference semantics. Measure allocations, allocated bytes, metadata
   comparisons/path copies, clone calls/copied bytes, memory accesses and journal
   entries by the same definitions. Record metric-wise `Rwork` for execution and
   separately for snapshot/export, replay/reversal and teardown. The replacement
   for the old elapsed-time ceiling is `Rwork[k] <= measured_work_ceiling[k]`;
   those ceilings are uncalibrated, so this note supplies no numeric factor.
   Publish the observations and choose the ceiling before implementing a slice,
   with evidence file, head, machine and date comments. Do not weight unlike
   counters into a synthetic total or turn an elapsed-time ratio into an operation
   multiplier (`src/cc.rs:343-389,2426-2545`;
   `REF/src/retained.rs:50-131`; `REF/src/classical.rs:1200-1281,1409-1448`).
2. **Trace demand and typed reconstruction:** count constructor calls,
   allocations and bytes for equal logical prefixes with observation off/on;
   preserve the positive observed-mode denominator and assert no trace
   constructors run during unobserved steps. Measure lazy full-snapshot work and
   retained pre-value metadata, including NaNs and host replies, before setting
   any allocation/retention ceiling. Source construction is known, its full-run
   count is not supplied (`src/cc.rs:689-748,2475-2545`).
3. **History/query calibration:** measure frame visits, index probes and emitted
   event-reference visits for append, suffix/segment lookup, export and repeated
   tail rewind. Calibrate short/long histories and event-set sizes using identical
   operation footprints in one process; derive numerical ratio ceilings only
   from those counts. Preserve exact no-prefix-scan and event-visit invariants.
   The full object alone does not exercise every nested-cancellation or repeated
   export case (`src/cc.rs:988-1015,2213-2229,2446-2469`; **887f49f**).
4. **Pages and displaced-byte journal:** measure resident/nonzero bytes, store
   spans, actual pointer widths, cross-page traffic, metadata probes, page
   allocations, allocator requests and copied/retained bytes. Calibrate page size,
   synthetic histories and allocation-ratio ceilings, comparing the same workload
   with the reference's journal and sparse-vector operations. No halving target
   or synthetic expansion factor is justified by the supplied evidence. Retain
   complete addresses and allocation/sector presence semantics
   (`src/cc.rs:35-53,343-389`; `REF/src/retained.rs:76-131`; **8a68da4**).
5. **Prepared transactions and quantum helpers:** count checkpoint field clones,
   instance tree updates, target-log append work, bytes and metadata visits with
   idle qregs and exact/approximate region calls; inject preparation failures and
   check publication/cursor invariants. The measured retained-value sharing fix
   is not a measurement of the remaining Machine fields. Calibrate row-family
   scope and any nontrivial prefix-work bound before removing their transaction
   paths (`src/action.rs:43-60`; `src/cc.rs:2426-2444`; **12cf12d**, **8a68da4**).
6. **Locks: retain supplied baseline; measure missing ownership shapes.** The
   supplied single-owner run retains **20**, **91**, **179** lock keys at the
   short, middle and long checkpoints defined in §5, and **398** at completion.
   <!-- Measurement: job/r3-vm-stepping/r37/locks.txt and PROGRESS.md:27-31, production head 8a68da4 with temporary diagnostic instrumentation; machine: this 12-core host; date: 2026-11-07. -->
   Its exact scan counts are already in §2; this incorporates the later evidence
   requested by review bead `bennett-vm-544`. Multi-owner conflicts, duplicate
   distributions, interval merging, streaming stores across allocations and
   close-time visits still need counts. Calibrate interval-set sizes and numerical
   scaling bounds from these measurements, preserving §10 acceptance of disjoint
   use. The supplied single-owner data does not justify sizing a lock rewrite as
   the main 8f3 fix (`src/launch.rs:187-208,690-708`).
7. **Compiled execution work:** the supplied
   `job/r3-vm-stepping/r37/engine-results.json` at production head **8a68da4**
   records equivalent interpreter/JIT outputs on this 12-core host, dated
   **2026-11-07**. Still missing: compile-time probes, per-step dispatch setup,
   canonical comparisons, successor checks, helper calls and native arithmetic
   versus runtime-helper work. Count one-row cooperative and continuing-drive
   paths separately, then calibrate short/long unused-program shapes and any
   numerical work bounds (`src/jit.rs:47-67,389-425,613-658`;
   `src/cc.rs:597-641`; `src/value.rs:149-164`).
8. **Guards and reproducibility:** obtain a duration measurement for each new
   counter-instrumented fixture/phase before choosing its hang guard by the R40
   rule in §5. Do not reuse an uninstrumented release duration for debug,
   instrumentation or a different phase. Record counter definitions and whether
   instrumentation was enabled in every evidence file, together with head,
   this host and the actual measurement date. Repeated deterministic counts must
   agree; load-sensitive elapsed differences do not change the work verdict.
   An interrupted measurement is UNKNOWN and leaves the work obligation open;
   exact semantic mismatches and completed counter-assertion failures remain red
   (`advisor-notes.md:476-481`; `briefs/COMMON.md`, final bullet).

## 7. Resources

This section adds measurements on **Spiral**, VM production head **4f49792**,
using the unchanged release build, separately from the historical evidence in
§§1–6. Machine, binary hash and actual UTC are in
`.tmp/s1-resources/provenance.json`; corpus hashes and origins are in
`.tmp/s1-resources/corpus-manifest.json`. These are informational host costs,
not deterministic work ceilings or rung acceptance. All evidence paths below
are relative to this worktree and remain local research artifacts.

The retained rung-3 conformance evidence stops before generating launch objects.
The available printf corpus is the VM's unchanged `r3-stepping` fixture, whose
published fixture oracle is checked below; it is not the whole formatted rung-3
acceptance corpus. The second corpus uses the retained LLVM18 rung-2 basic/grow
objects at input `1000`, with the gate's explicit budget. Object discovery and
the archived gate source are retained as
`.tmp/s1-resources/receipts/objects.stdout`,
`.tmp/s1-resources/gate.py.source.txt` and
`.tmp/s1-resources/rung2-report.json`.

### Release command observations

Let `J=.tmp/s1-resources`, `VM=$J/target/release/bennett-vm`, and
`O=$J/corpus/<corpus>/launch.bennett`. Commands use `taskset -c 0`, fresh
processes and `BENNETT_VM_ENGINE=interpreter|jit`. For printf, run
`$VM run-program $O main --replies $J/corpus/r3/replies.json --target $J/corpus/r3/target.json`.
For hashtables, run
`$VM run-program $O ht_demo_basic|ht_demo_grow --args 1000 --budget 10000000,1048576,65536,256`.
Exact argv, wall/CPU and wait4 high-water are in each named receipt below.

| Corpus | Engine | Wall s | CPU s | Peak RSS KiB | Receipt under `.tmp/s1-resources/receipts/` |
|---|---|---:|---:|---:|---|
| r3 | interpreter | 2.372132 | 2.342005 | 600672 | `r3-interpreter-cli-1.json` |
| r3 | jit | 3.074482 | 3.070585 | 620316 | `r3-jit-cli-1.json` |
| ht_basic | interpreter | 1.770919 | 1.749278 | 696000 | `ht_basic-interpreter-cli-correct-1.json` |
| ht_basic | jit | 2.172113 | 2.149915 | 696504 | `ht_basic-jit-cli-correct-1.json` |
| ht_grow | interpreter | 4.480394 | 4.459307 | 1820780 | `ht_grow-interpreter-cli-correct-1.json` |
| ht_grow | jit | 5.534526 | 5.490579 | 1822352 | `ht_grow-jit-cli-correct-1.json` |

### GPU inventory and policy boundary

The compiled `.tmp/s1-resources/device_query.cu` successfully initialized a CUDA
context. Verbatim `.tmp/s1-resources/receipts/cuda-device-query.stdout`:

```text
driver_version=13000 runtime_version=12000 device_count=1
device=0 name=NVIDIA GeForce RTX 3090 SMs=82 global_memory_bytes=25769279488 compute_capability=8.6 memory_free_bytes=24436015104 memory_total_bytes=25769279488 unified_addressing=1 concurrent_kernels=1
```

`.tmp/s1-resources/gpu-environment.json` records `/dev/dxg`, loadable WSL CUDA,
the nvcc path and absent `nvidia-smi`. Build/run argv and errors are retained in
`receipts/cuda-build.*` and `receipts/cuda-device-query.*` under the same job
directory. This inventory measures availability, not GPU throughput.

`std::thread` needs no std-only policy exception. A CUDA/HIP/wgpu backend needs
FFI or a crate and therefore a **maintainer-ratified std-only policy exception**.
The `bennett-vm-bjm` CPU plan, captured verbatim in
`.tmp/s1-resources/receipts/dense-plan.stdout`, leaves GPU implementation for a
follow-up; inventory does not authorize that exception.


### Measurement setup and scope

All further evidence files in this section are under `.tmp/s1-resources/` unless
an absolute source path is given. `guard.py` and `timeouts.toml` implement the
census model's process-tree guards and wait4 wall/CPU/RSS receipts. The table
cites Spiral measurements, then this job's own command receipts, uses machine
factor **1.0**, and treats a trip as UNKNOWN/124. Bootstrap envelopes are labelled;
guards are not performance assertions. No full VM or sibling gate ran in this
child: T3CHILD assigns the landing gate to the orchestrator. No ISA reference
source was changed, and no change or parallelization of that interpreter is proposed.

`machine-profile.json` derives **11** build jobs and CPU affinity **0–10** from
measured `nproc` and `MemAvailable`, reserving at least **16** threads and **16 GiB**
for neighbors. Workloads run serially on CPU **0**. Host readings are retained in
`meminfo.initial.txt` and `meminfo.during.txt`. `receipts/release-build.json` records
`cargo build --release --offline -j 11` under that affinity: **20.934893 wall s**,
**88.316226 CPU s**, **1098672 KiB** high-water. This is the normal release build,
without production instrumentation or new crate dependencies.

The external `phase_probe.rs` links the unchanged release rlib. A `System`
allocator wrapper counts allocation/zero-allocation requests, reallocations,
requested bytes and deallocations separately, before formatting phase receipts.
Requested bytes include the new size on realloc; they are neither live nor
copied bytes. CPU uses the Linux process CPU clock. Phase high-water is cumulative
VmHWM; command RSS uses wait4's child-tree maximum, which can differ slightly on
WSL. CLI allocation counts stop when the command API returns, before thread-local
JIT-cache teardown. These VM-only counts do not supply reference work ratios.

### Sampled hot functions

Record with `perf record -e cpu-clock:u -F 499 -o $J/<label>.data -- <command>`;
report with `perf report --stdio --no-children --percent-limit 0 --sort dso,symbol --show-nr-samples`.
Exact argv, wall/CPU, RSS and output are in `receipts/<label>.*`. Hardware and
software event probes both completed; software events make this study repeatable
on WSL. The host perf lacks libelf (`receipts/perf-build-options.stdout`).
`symbolicate.py` translates recorded **file offsets through ELF PT_LOAD segments**
before `addr2line -f -C`; direct use as virtual addresses would misname Rust
functions. Raw reports, ELF maps and symbolication receipts are retained.

`*-functions.json` and `*-top20.md` hold complete per-function top twenties for
all three CLI objects on both engines and for both ABI profiles. Percentages
are exclusive leaf samples divided by all user samples in that profile;
seconds are sample-count/recording-frequency estimates. They are not inclusive
time, wall time, deterministic work or an attainable optimization gain. Sampling
and inlining limit attribution; no allocation call-site shares are inferred.

Generated JIT mappings have no function map: unassigned native-code leaves total
**0.31% / 0.33% / 0.55%** in printf/basic/grow CLI profiles and **10.11%** in the
ABI JIT profile (`jit-symbol-coverage.json`). Their raw IPs are retained and stay
in each percentage denominator; named-function tables do not assign that work
to a helper. Resolving those mappings is an open profiling measurement, and the
ABI dispatch-versus-arithmetic split cannot be inferred from them.

Printf CLI top twenty: full names in `r3-interpreter-perf-functions.json` and
`r3-jit-perf-functions.json`; display aliases in `profile-aliases.json`.

| Rank | Interpreter leaf function | Self % | User s estimate | JIT leaf function | Self % | User s estimate |
|---|---|---:|---:|---|---:|---:|
| 1 | `malloc_consolidate` | 13.29 | 0.2445 | `native::Code::run::helper (drive)` | 18.69 | 0.4790 |
| 2 | `unlink_chunk` | 11.55 | 0.2124 | `malloc_consolidate` | 10.32 | 0.2645 |
| 3 | `__GI___libc_free` | 7.19 | 0.1323 | `unlink_chunk` | 8.52 | 0.2184 |
| 4 | `__memcmp_avx2_movbe` | 5.12 | 0.0942 | `__GI___libc_free` | 5.55 | 0.1423 |
| 5 | `_int_malloc` | 4.14 | 0.0762 | `__memcmp_avx2_movbe` | 4.46 | 0.1142 |
| 6 | `_int_free` | 4.03 | 0.0741 | `_int_malloc` | 3.44 | 0.0882 |
| 7 | `Seq<Event>::iter/fold (target export)` | 3.59 | 0.0661 | `__memcpy_avx_unaligned_erms` | 3.21 | 0.0822 |
| 8 | `__memcpy_avx_unaligned_erms` | 3.49 | 0.0641 | `_int_free` | 2.97 | 0.0762 |
| 9 | `Rc<Node<Frame>>::drop_slow` | 3.05 | 0.0561 | `Seq<Event>::iter/fold (target export)` | 2.27 | 0.0581 |
| 10 | `__GI___libc_malloc` | 2.61 | 0.0481 | `Rc<Node<Frame>>::drop_slow` | 2.27 | 0.0581 |
| 11 | `Rc<Node<Event>>::drop_slow` | 2.07 | 0.0381 | `Rc<Node<Event>>::drop_slow` | 1.95 | 0.0501 |
| 12 | `Rc<Node<Activation>>::drop_slow` | 1.74 | 0.0321 | `__GI___libc_malloc` | 1.80 | 0.0461 |
| 13 | `Rc<Node<Frame>>::make_mut` | 1.74 | 0.0321 | `Rc<Node<Frame>>::make_mut` | 1.49 | 0.0381 |
| 14 | `Rc<Node<Event>>::make_mut` | 1.63 | 0.0301 | `Machine::exec` | 1.41 | 0.0361 |
| 15 | `persistent::balance<Event>` | 1.63 | 0.0301 | `Rc<Node<Event>>::make_mut` | 1.17 | 0.0301 |
| 16 | `Rc<Node<Value>>::drop_slow` | 1.42 | 0.0261 | `persistent::balance<Event>` | 1.02 | 0.0261 |
| 17 | `Machine::note_register` | 1.31 | 0.0240 | `drop_glue<Frame>` | 1.02 | 0.0261 |
| 18 | `Machine::step_row` | 1.20 | 0.0220 | `Machine::read` | 0.94 | 0.0240 |
| 19 | `Machine::read` | 1.09 | 0.0200 | `persistent::balance<Frame>` | 0.94 | 0.0240 |
| 20 | `persistent::balance<Frame>` | 0.98 | 0.0180 | `Machine::step_row` | 0.94 | 0.0240 |

Other profiles' leading leaves follow. Each label names its receipt, `.data`,
`-functions.json` and `-top20.md` under this job directory.

| Profile label / command variant | Wall s | CPU s | Peak RSS KiB | Leading three leaf functions (self %) |
|---|---:|---:|---:|---|
| `ht_basic-interpreter-perf` | 2.226477 | 2.152507 | 695516 | `__GI___libc_free` 6.53%; `Rc<Node<Value>>::drop_slow` 6.25%; `__memcmp_avx2_movbe` 6.25% |
| `ht_basic-jit-perf` | 2.377178 | 2.285730 | 696016 | `native::Code::run::helper (drive)` 14.07%; `__memcmp_avx2_movbe` 6.54%; `Rc<Node<Value>>::drop_slow` 5.45% |
| `ht_grow-interpreter-perf` | 6.293803 | 6.202671 | 1820936 | `__GI___libc_free` 7.34%; `Rc<Node<Value>>::drop_slow` 6.47%; `__memcmp_avx2_movbe` 6.23% |
| `ht_grow-jit-perf` | 6.492873 | 6.475552 | 1822332 | `native::Code::run::helper (drive)` 15.90%; `__memcmp_avx2_movbe` 7.76%; `__GI___libc_free` 5.25% |
| `abi-interpreter-perf` | 7.946328 | 7.888685 | 1834300 | `malloc_consolidate` 7.81%; `unlink_chunk` 6.46%; `<std::alloc::System>::alloc_impl` 4.74% |
| `abi-jit-perf` | 9.050124 | 9.006911 | 1854524 | `malloc_consolidate` 8.06%; `unlink_chunk` 6.22%; `__GI___libc_free` 4.36% |

CLI profile commands are the corpus commands above. ABI commands are
`$J/phase_probe $J/corpus/r3 snapshot 0 false` under the selected engine; zero
means complete execution. ABI profiles include allocator instrumentation,
full snapshot, verification hash and teardown. Their allocator atomics/System
wrapper and SHA256 verification are harness work. The two printf CLI profiles
have **918 / 1279** samples (`r3-*-perf-functions.json`). The JIT continuing helper
has **18.69%** self samples; `src/jit.rs:653` performs a linear successor-membership
scan on continuing `drive` calls. This supports a dispatch investigation;
samples do not count its comparisons.

### Compile, execute, observe and release

For `BENNETT_VM_ENGINE=<engine> taskset -c 0 $J/phase_probe $J/corpus/r3 snapshot 0 false`,
reading 2 from `receipts/phase-<engine>-snapshot-0-false-2.stderr` is parsed into
`phases.json`. The ABI run includes target accounting, the host reply, cooperative
row transactions, retained history and the full legacy snapshot.

| Phase | Interpreter wall / CPU s | JIT wall / CPU s | Interpreter allocations | JIT allocations | Interpreter / JIT requested bytes |
|---|---:|---:|---:|---:|---:|
| load | 0.025722 / 0.025723 | 0.025065 / 0.025066 | 196150 | 196150 | 20723714 / 20723714 |
| start | 0.001398 / 0.001399 | 0.036800 / 0.036793 | 19272 | 352978 | 956175 / 53623057 |
| host_reply | 0.004596 / 0.004599 | 0.003041 / 0.003043 | 58254 | 58257 | 2869157 / 2869169 |
| execute | 2.605340 / 2.604767 | 3.725317 / 3.710785 | 43455002 | 43652653 | 4057356072 / 4057701516 |
| snapshot | 2.977791 / 2.976574 | 3.041948 / 3.040245 | 25118243 | 25118243 | 1636887478 / 1636887478 |
| snapshot_hash | 0.280666 / 0.280552 | 0.285883 / 0.285591 | 1 | 1 | 232610127 / 232610127 |
| snapshot_free | 0.008414 / 0.008416 | 0.007165 / 0.007167 | 0 | 0 | 0 / 0 |
| teardown | 1.134229 / 1.132878 | 1.125882 / 1.124942 | 0 | 0 | 0 / 0 |

Snapshots are **77536709 bytes**, with identical SHA256 across both engines and
repeats (`receipts/phase-*-snapshot-0-false-{2,3}.stdout`). ABI high-water rises
from **643444 / 663316 KiB** after interpreter/JIT execution to
**1834800 / 1854480 KiB** during snapshot (`phases.json`). Deferring trace encoding
retains a substantial export obligation; do not count moving work into export
as removing it. Teardown is separately measured.

The independent native compile probe
`$J/phase_probe $J/corpus/r3 compile 0 false` records **0.030007 wall / 0.030009 CPU s**,
**309728 allocations** and **46949819 requested bytes**, after parse/validation
(`receipts/phase-interpreter-compile-0-false-1.stderr`). A JIT ABI start additionally
hashes/looks up the object and constructs run state. CLI audit records
**225956 native rows / 28644 native arithmetic operations**
(`receipts/r3-jit-cli-1.stderr`). Compilation is too small on this object to
explain its CLI slowdown; continuing dispatch and shared runtime work remain candidates.

For the CLI allocation view, the harness invokes the same API as the release
CLI. Values come from `receipts/<corpus>-<engine>-allocations.stderr` and `phases.json`:

| Corpus | Interpreter allocations / requested bytes | JIT allocations / requested bytes |
|---|---:|---:|
| r3 | 19972118 / 890891117 | 20503478 / 943903455 |
| ht_basic | 15480630 / 1116016499 | 15809037 / 1121995022 |
| ht_grow | 38954545 / 2803154788 | 39743053 / 2814557398 |

Request totals plus allocator/persistent-node samples motivate reducing allocation
frequency, history turnover and environment updates before adding workers to one
classical run. They do not identify each allocation's owner or promise that
pages/journals remove all allocator time. CLI and C ABI differ in observation
and ownership work; their time ratio does not isolate one mechanism.

### Parallel decompositions and dense resource phases

**Interpreter.** One row's preflight, budget/fault decision, SSA/activation update,
memory/device publication, history append and replay/reply cursor advancement
depend on source order. Commit each row atomically; reversal remains LIFO.
Dividing these small steps adds synchronization to measured metadata work.
Independent launches/objects, independent differential cases and read-only
analysis are coarse parallel units when ownership is disjoint. Construct a
program/runtime inside each worker: persistent `Rc` structures do not become
thread-safe through an `Arc` wrapper. A frozen snapshot could encode disjoint
row ranges and join in source order, preserving deterministic JSON; first
measure encoding/allocator contention. The ISA reference remains single-threaded
and unchanged.

**JIT.** Separate immutable analysis/compilation from running row transactions.
Independent objects can compile concurrently; per-function compilation needs
stable canonical IDs and deterministic joining before publication. Sharing
compiled programs also requires valid cache identity and suitable ownership;
today's cache is thread-local. The small measured compile phase favors dispatch
and shared runtime work over parallel compilation for this corpus. Across runs,
workers still obey memory/device ownership. Inside one run, native code crosses
the same row transaction and host-visible yield/fault/reply boundaries. A
dispatch-only change can retain those boundaries and be measured independently
of prepared transactions.

**Dense CPU plan.** `bennett-vm-bjm` specifies a binary64 state vector,
cache-blocked in-place kernels, fusion and branch-enumerated classical oracles
(`receipts/dense-plan.stdout`). The primary paper supports the bandwidth/cache
rationale, not a Spiral speedup claim:
[qHiPSTER, Smelyanskiy, Sawaya and Aspuru-Guzik](https://arxiv.org/abs/1601.07195v2).
The reading's scope is recorded in `literature.txt`. These classical objects
exercise no dense amplitude kernels: dense phase times and hot-function ranking
remain **unmeasured**.

`dense-cost-model.json` contains this **analytical** byte model from the bead's
binary64 layout. Write `A=2^n`, `S=16A` bytes. A full read/write traversal moves
`2S` bytes before cache reuse; a lower bound is `2S / sustainable_bandwidth`,
whose denominator this study has not measured.

| Dense phase | Work / traffic model | Candidate decomposition and constraints |
|---|---|---|
| Allocate/initialize | One state vector, `S` payload; initialization writes `S`; metadata/small worker accumulators separately counted | Static amplitude blocks with `std::thread::scope`; workers first-touch their pages. Avoid a second amplitude-sized buffer. |
| Generic/fused gate group | Read/write `2S` per full traversal; fusion amortizes traversals with small local gate scratch | Partition pairs/tiles, not arbitrary amplitude halves: high target bits couple blocks. Each pair has one writer; synchronize dependent groups. Choose block size from target stride and measured cache behavior. |
| Permutation/diagonal | Index remap or phase update; control-dependent touched fraction | In-place pair ownership for swaps; disjoint selected amplitudes for phases. Preserve bijectivity and fault-bit semantics. |
| Lifted oracle | Project distinct inputs, run the existing classical oracle per cache miss, then XOR-remap amplitudes | Worker-local oracle state; deterministic memo-result coordination. Measure hit rate and classical rows separately from amplitude traversal. |
| Born probabilities | Read `S`, bounded partial reductions | Fixed partition/reduction order; finalize the draw decision serially in published ISA order. |
| Collapse/renormalize | A further read/write pass, `2S` | Disjoint blocks after selecting one outcome; synchronize before the next action and retain explicit numerical error bounds. |
| Export/certificate | Requested projection and error metadata; full vector export costs `S` before encoding | Preserve complete requested observations; certificates remain numerical with a bound, never exact/exhaustive. |

| Qubits | State GiB (`S`) | Full read/write pass GiB (`2S`) |
|---:|---:|---:|
| 20 | 0.015625 | 0.03125 |
| 24 | 0.25 | 0.5 |
| 28 | 4 | 8 |
| 30 | 16 | 32 |
| 31 | 32 | 64 |

`numa-cache.json` reports one WSL-visible NUMA node; this host cannot establish
cross-node scaling. Retain first-touch/block affinity for multi-node hosts.
Use the smaller of available parallelism and a fresh memory/neighbor budget;
this build's worker count is not a dense bandwidth optimum. Sysfs cache sizes
are inventory, not calibrated tile sizes.

**GPU follow-up.** Keep amplitudes device-resident across fused groups. Measure
context startup, host/device transfer, kernel time, synchronization, reduction
and requested readback separately. Transferring a full vector moves `S` each
way; CPU kernel costs omit that traffic. Avoid round trips after every gate.
Measured free memory fits the analytical **30-qubit / 16-GiB** vector but not
**31 qubits / 32 GiB**, before backend/context/scratch demands (`dense-cost-model.json`).
That is capacity evidence, not a supported size or backend benchmark. Requery
availability when running. Backend implementation requires the explicitly named
maintainer-ratified std-only exception above; CPU std-thread work needs none.

### Slice priority per unit of implementation work

Trace slice **1** and indexed-ownership slice **5** already landed at **4f49792**
(`receipts/production-log.stdout`, `receipts/current-design-paths.stdout`). The
current lock code preserves empty-range semantics and **does not merge adjacency**;
do not apply §5's older adjacency suggestion over those semantics.

Residual order: **1 and 5 retained as landed → 2 → 6a dispatch only → 3 → 4 → 6b
cooperative integration**. Extract 6a as a review-sized dispatch change that
retains the row transaction; 6b keeps the publication-boundary dependencies.
This changes priority, not semantic obligations.

Implementation effort means §5's estimated review-sized round, not measured
engineer hours. Only the trace-mode comparison measures a realized throughput
difference. For unimplemented changes, measurements rank **opportunity per round**;
candidate builds and counter deltas must precede any claimed measured gain/work.

| Slice / placement | Measurement justifying placement | Gain or limit per estimated round |
|---|---|---|
| 1, retain landed | `trace-comparison.json`: same-prefix deferred/eager CPU 0.145843/0.200758 s (interpreter), 0.189718/0.257619 s (JIT), identical complete snapshots | Realized prefix stepping ratios **1.3765× / 1.3579×** for the landed mechanism; **694279** fewer execution allocation requests and **46738094** fewer requested bytes in either engine. Not a whole-object speedup or zero-retention claim. |
| 2, first residual | Full ABI snapshot makes **25118243** allocation requests; records traversal is sampled in `abi-*-perf-functions.json`; history/target turnover appears in both CLI profiles | Shared log/event-index/export work addresses both engines. Exact removable share and gain per round remain open. |
| 6a, promote before 3/4 | Continuing helper is **18.69%** of printf JIT user samples, while compile CPU is **0.030009 s**; JIT CLI is slower on every measured object | A direct successor/dispatch lookup targets a measured term with a contained change, keeping transaction checks. No gain claimed without a candidate; native arithmetic already runs. |
| 3, after contained dispatch work | Byte-memory COW is visible in grow; allocation totals are high (`phases.json`, `ht_grow-*-perf-functions.json`) | Retain page/journal work before slice 4 as its dependency. Store-span/call-site attribution is missing; no measured gain supports a broad page rewrite ahead of localized dispatch. |
| 4, after 2/3 | ABI execution makes **43455002 / 43652653** allocation requests and retains checkpoints (`receipts/current-design-paths.stdout`) | Counts include more than checkpoints, so cannot price their removal. Preserve diagnostic/quantum rollback and measure row families before sizing rounds. |
| 5, retain landed | Indexed locks are in this measured tree and absent from leading leaf samples | No repeat rewrite on a stale baseline. Sampling absence is not a zero-cost proof; multi-owner work counts remain open. |
| 6b, last | ABI JIT audit re-enters native dispatch per row (`receipts/phase-jit-snapshot-0-false-2.stderr`); CLI uses one continuing run | Batch only after publication/ownership prerequisites. ABI/CLI times mix mechanisms and cannot be called batching gain. |

### Repeatability, verification and §6 obligations

The two largest workload commands are full interpreter/JIT ABI execution plus
snapshot. Final-harness readings 2 and 3 use the same binary and inputs.
`repeatability.json` defines relative difference as `abs(second-first)/first`.

| Command | Wall readings 2 / 3 s | CPU readings 2 / 3 s | Wall difference | CPU difference |
|---|---:|---:|---:|---:|
| full ABI interpreter | 7.690843 / 7.587536 | 7.641501 / 7.568916 | 1.343% | 0.950% |
| full ABI jit | 8.996545 / 8.945308 | 8.976201 / 8.883670 | 0.570% | 1.031% |

The observed final-pair envelope is **1.4% wall / 1.1% CPU**. Every per-phase
allocation/reallocation/requested-byte/deallocation count and full snapshot hash
agrees exactly. These are observed variation, not acceptance limits or statistical
confidence intervals. Pilot `phase-*-snapshot-0-false-1.*` is retained: interpreter
wall **9.297237 s** versus final-pair **7.690843 / 7.587536 s**. That wider spread
is not hidden; startup/cache/load conditions and pilot harness layout differed.
The tight final pair is not a general host-variance bound.

The largest construction command was repeated from a clean **owned** release
target: **20.934893 / 21.339878 wall s**, **88.316226 / 87.434635 CPU s**, within
**2.0% wall / 1.1% CPU** (`receipts/release-build{,-2}.json`). The binary was
byte-identical (`rebuild-binary-equality.json`). Grow CLI repeats are retained
in `receipts/ht_grow-*-cli-correct-{1,2}.json`; their wider load-sensitive spread
does not alter output/work verdicts.

`python3 .tmp/s1-resources/guard.py short focused-verification python3 .tmp/s1-resources/verify.py`
completed with PASS (`verification.json`, `receipts/focused-verification.*`).
It checks corpus hashes, full engine JSON equality, the independent printf oracle
and retained native hashtable answers, complete saved-tape replay equality,
eligible reversal including memory and empty console/tape, corrupted-tape
rejection, repeated counts/snapshot hashes, sample conservation, cited guard
margins and preserved §§1–6. Negative replay probes explicitly reject mismatches;
they are expected negative controls. This is not a full landing-gate claim.

| §6 request | Closed here | Still open |
|---|---|---|
| Same-object work vector | VM allocation requests/bytes, wall/CPU/RSS and self samples on both engines and corpora | Comparable independently obtained reference work counts, operation/path-copy/copied-byte definitions and numeric work ceilings. No oracle source changes are authorized or proposed. |
| Trace demand | Eager/deferred prefix allocations, bytes, CPU and identical snapshots; complete deferred export cost | Constructor counts, broader trace families, retained pre-values and fitted retention bounds. |
| History/query | History/target node and records/export samples; export/teardown priced separately | Exact frame/index/event visits, repeated export/rewind/cancellation and fixed-footprint scaling. |
| Pages/journal | Real-object request totals and memory high-water | Resident/nonzero/store spans, pointer widths, cross-page/copy/metadata counts and calibrated page size. |
| Prepared transactions/Q helpers | Classical ABI execution allocation baseline | Checkpoint-field/target-log work by row family, Q-state costs and failure-injection/publication invariants. Classical corpora do not price dense gates. |
| Locks | Current index identified; stale vector rewrite removed from priority | Multi-owner, point/adjacency, allocator/device and close-time deterministic work shapes. |
| Compiled execution | Compile/run split, native audits and both entry-path profiles | Dispatch/setup/successor counts, unused-program scaling and measured candidate gain/work. |
| Guards/reproducibility | Cited Spiral guards, headless isolated targets, derived concurrency, command receipts, clean rebuild and repeated largest workloads | New instrumentation/fixtures/phases need their own calibration; timing is never a work ceiling. |

Remaining VM measurements are **bennett-vm-3ol** (`receipts/followup-bead.stdout`);
dense implementation remains **bennett-vm-bjm**. Official whole formatted rung-3
objects and both-major profiles await conformance artifacts; the precise request
is in `cross-repo-beads.md`. GPU inventory is closed; dense CPU/GPU bandwidth,
transfer/kernel phases, scaling, numerical error and actual optimization gain
per unit of implementation work remain measurements to perform.
