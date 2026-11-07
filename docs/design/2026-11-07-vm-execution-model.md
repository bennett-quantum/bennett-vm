# VM execution model: retain the semantics, reduce the work per row

## 1. Reference execution, reversal, replay and faults

This is a research proposal, not an implementation or a new benchmark result. VM
source citations below mean **d68eb76** unless another SHA is given. `REF` means
`bennett-isa-spec@3b1090790d82a00ac95d30b8da6b42d1d7a9f5d1/reference-interpreter/`;
`ISA` means the supplied `bennett-isa-spec/spec/Bennett-ISA-Spec-0.9.md` at
**62a0cf0**. The latter adds K94 after the VM's gate pin; its relevant CC/R memory,
history and launch rules are unchanged apart from line offsets. These versions
must remain explicit: `gate.toml:23-32` pins the oracle to 3b10907, not sibling
HEAD. The proposal is authorized by R37, recorded in
`bennett/.tmp/orchestrator/advisor-notes.md:442-456`.

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

Use the supplied measurements, without re-measuring them here. The fix job reports
prefix per-step growth of 1.314× from 500 to 15,000 rows and a final same-CPU
release pair: **225,956 steps**, reference **1.418830 s**, VM process
**6.948585 s** (4.897404×), VM execution **3.010474 s**, snapshot **2.891278 s**.
Execution/reference wall is about 2.12×, but its timing boundaries differ from the
process comparison. These are one supplied pair, not a distribution or a new
result from this document
(`bennett-vm-job-r3/.tmp/r3-vm-stepping/PROGRESS.md:13-20`, head **8a68da4**).

R37's “~100×” and “200 us vs 8 us” are advisor hypotheses, not mutually consistent
arithmetic: those two per-step numbers imply about **25×**. Do not use either as
the post-fix baseline. Its candidate list remains useful for investigation
(`bennett/.tmp/orchestrator/advisor-notes.md:442-456`). The earlier profiles show
event-vector/Rational copying and allocator work; the later debug profile includes
JSON subtree cloning. They precede the final shared-payload change and cannot rank
the residual costs at 8a68da4
(`bennett-vm-job-r3/.tmp/r3-vm-stepping/perf-report.txt:13-78`,
`full-debug-perf.txt:13-38`; **8a68da4**).

In particular the strike-force README's “no target” premise is refuted by the
reported original harness `tests/varargs.py:140-144` and the target-installed
fixture at **8a68da4**, `tests/step_scaling.rs:38-48`. The static reports disagree
on locks; the fix job says locks were below its 1% profile threshold and unchanged.
Treat all three reports as hypotheses rather than fresh measurements
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
(`src/jit.rs:389-425,613-625,643-658`). No paired JIT measurement was supplied with
the final 8a68da4 evidence; R37 requests one from the fix lane, and this research
job does not manufacture it (`advisor-notes.md:453-456`; fix-job `PROGRESS.md:10`).

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

### Ratios, timing boundaries and complexity

The final target is **VM/reference <=5× per executed row** on the unchanged
225,956-row, target-installed 8f3 object. Retain both an execution-only ratio and
the conservative whole-worker ratio including the requested final snapshot;
neither may hide work by omitting output. Reuse **8a68da4**'s same-permitted-CPU,
fresh-oracle-then-worker process method, but require matched release/release
profiles for the 5× comparison. Its checked-in test uses a debug oracle and a
10× timeout: merely changing its constant is neither a fair release comparison
nor an assured red test (`8a68da4:tests/step_scaling.rs:38-48,83-128,147-231`;
fix-job `PROGRESS.md:17-18`).

Define `Rscale = (T(15000)/15000)/(T(500)/500)` on equal-work rows, keeping the
5,000-row midpoint, and require `Rscale <=2` throughout. Also use 10× history
growth on a fixed-operation microfixture; growth of the *retained prefix* must
not raise ordinary-step bookkeeping work. Repeated same-CPU pairs and medians
reduce noise; output/type/target assertions run in every timed worker. For a
count ratio with a possibly empty denominator use `max(1, denominator)`.
These are proposed tests; none was executed for this note. The existing 5×
whole-worker target already passed once at 4.897404×, so a slice's red-first test
must expose its own cost rather than claiming that existing passing bound is red
(`8a68da4:tests/step_scaling.rs:119-128,147-231`; fix-job `PROGRESS.md:18`).

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

**Red-first ratio test:** `trace_off_builds_no_step_json`: assert
`step_trace_json_builds(off)/max(1, step_trace_json_builds(on)) ==0` with equal
logical steps and identical full trace projection. Current launch always-on
construction should expose the red. Record the timing ratio `T(off)/T(on)` and
keep `Rscale <=2`; the work-count ratio is the deterministic required perf
assertion, not a guessed wall-time speedup (`src/cc.rs:689-748,2190`).

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

**Red-first ratio tests:** `history_lookup_does_not_walk_its_prefix` uses identical
fixed-size uncall targets behind H and 10H pure frames and asserts lookup work
`W(10H)/W(H) <=2`; exclude the actual undo payload from that counter.
`effect_export_work_is_linear` requires `T(10E)/T(E) <=15` on E and 10E effect
entries and `visited_entries/output_entries <=2`. Current suffix scans and
`iter().skip(prefix)` provide source-based red candidates; the implementer must
record the actual failing run before changing production code
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

**Red-first ratio test:** `memory_journal_retention_tracks_changed_bytes`: on
repeated eight-byte writes across a fixed resident page set, assert retained
memory metadata allocations per step relative to the byte-node implementation
`A(new)/A(8a68da4) <=0.5`; capture the pre-change counter baseline and observe
red before the new representation. Also require normalized late/early
same-footprint step time <=2 across 10× history. Count allocator calls and
retained bytes separately so larger pages cannot hide a memory regression
(`src/cc.rs:351-359`; **8a68da4**).

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

**Red-first ratio tests:** `prepared_pure_steps_copy_no_machine_payload` asserts
`machine_checkpoints/committed_pure_rows ==0` and
`instance_tree_updates/defined_instances ==0`; both current operations are
visible in the cited implementation. `fixed_row_cost_is_independent_of_history`
requires `work_per_row(10H)/work_per_row(H) <=1.1` for metadata visits/copies
and timing `Rscale <=2`. The full paired object remains <=5×, with complete
snapshot, oracle outputs and target charges checked
(`src/cc.rs:486-492,2439`; **8a68da4**).

**Differential/risk/rounds:** Common checks plus diagnostic injection at every
preparation stage, reply retry, token publication, terminal fault inside an open
callee and disjoint interleaved runs. Risk: partial commits and consuming a
nondeterministic input on failure; this is the highest-risk slice. Estimate
**1 round per measured row-family slice**, never one claimed round for an
unbounded rewrite (`ISA:1173-1212,2458-2487,2511-2518`; **6d96f5c**).

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

**Red-first ratio test:** `lock_acquisition_is_sublinear_in_distinct_ranges`:
execute equal-size touches with L and 10L retained disjoint ownership intervals;
assert overlap/dedupe comparisons per touch `C(10L)/C(L) <=2`, then normalized
timing `Tstep(10L)/Tstep(L) <=2`. Use separate allocations where needed to keep
entry-key behavior equivalent. The vector scan is the red candidate even though
it was not the measured 8f3 bottleneck (`src/launch.rs:187-208`; fix-job
`PROGRESS.md:10`).

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

**Red-first ratio test:** `compiled_step_dispatch_is_independent_of_unused_rows`
keeps the executed trace identical while growing unreachable program rows P to
10P; assert dispatch probes per executed row `C(10P)/C(P) <=2` for both native
and non-native compiled modes and timing ratio <=2. Exercise high canonical ids
as well as the first row, so native linear dispatch is observable. Retain
`paired_full_object_within_five_times_reference` for **both engines** with
matched release profiles and same-CPU pairing. The former isolates the red;
the latter is the final acceptance target, not asserted to fail today
(`src/jit.rs:404-409,613-625`; **8a68da4:tests/step_scaling.rs:165-231**).

**Differential/risk/rounds:** Common checks, every existing transformation mode,
recursion, interior-row jumps, i80 runtime helpers, exact fault source points,
replies and per-row target time. Risk: batching past a host-visible boundary or
charging optimized private nodes instead of original rows. Estimate **1 round**;
native code generation and scheduler semantics do not otherwise change
(`src/jit.rs:279-310`; `src/cc.rs:661-772`; `ISA:1557-1562,1905-1928`).

## 6. Measurements needed before sizing each slice

These are implementation-lane measurements to request, not claims that this
research job ran a build, test or profile. Existing supplied evidence is the
8a68da4 fix-job report and profiles cited in §2.

1. **Residual cost at 8a68da4:** repeat matched release/release pairs, separate
   execution, result extraction, JSON construction/serialization, parsing and
   teardown, then profile each. The supplied 4.897404× whole-process ratio is
   close to 5× and one pair is insufficient to size savings. Snapshot cost is
   already comparable to execution cost (`PROGRESS.md:17-18`, **8a68da4**).
2. **Observation demand:** count actual trace/snapshot consumers and measure
   trace off/on, retained metadata bytes per row and full reconstruction time.
   Which pre-values cannot be recovered without new typed metadata? This sizes
   slice 1 while protecting the current snapshot contract
   (`src/cc.rs:689-748,2475-2545`).
3. **History query distribution:** measure append/index/suffix/tape-export work
   separately, with many effects, frequent uncall, nested cancellation and
   repeated prefix reversal. The 8f3 fixture does not by itself stress all these
   scans; their source existence is not their measured contribution
   (`src/cc.rs:988-1015,2213-2229,2446-2469`; **887f49f**).
4. **Page and journal shape:** resident/nonzero bytes, touched spans, address
   widths, cross-page transfers, write density and allocator counts versus
   retained bytes. Compare candidate page sizes and directory types before
   promising slice 3's allocation ratio; reject a page scheme that silently
   narrows BigUint addresses or explodes sparse memory
   (`src/cc.rs:35-53,343-389`; **8a68da4**).
5. **Transactions with quantum state:** profile Machine checkpoint fields with
   idle live qregs, exact/approximate region calls and failures; inventory every
   fallible operation and measurement-input publication point. The target-history
   fix does not make amplitudes, emitted leaves and every other quantum field
   persistent. This determines the number of slice-4 row-family jobs and the
   cost of migrating target append streams to meet the H-independent bound
   (`src/action.rs:43-60`; `src/cc.rs:2426-2444`; **12cf12d**, **8a68da4**).
6. **Lock cardinality and ABI contention:** measure distinct ranges per owner,
   duplicate frequency and merging opportunities on streaming stores and multiple
   open runs. The supplied report does not establish locks as an 8f3 cost; the
   synthetic ratio test sizes the actual interval work and close-time cost
   (`src/launch.rs:187-208,690-708`; fix-job `PROGRESS.md:10`).
7. **JIT pair and program-size sensitivity:** obtain the R37-requested JIT
   measurement on the same installed target, object, CPU and release profile;
   separately measure compilation, one-row ABI dispatch and multi-row CLI drive.
   A native-kernel audit counter alone cannot demonstrate a speedup
   (`advisor-notes.md:453-456`; `src/jit.rs:47-67,389-425`; `src/cc.rs:597-641`).
8. **Acceptance margin:** record multiple paired ratios and RSS/retained bytes
   across the full object and equal-work scaling fixtures before setting a tighter
   improvement objective. Keep <=5×, `Rscale <=2`, exact outputs and the structural
   ratio tests as the stated targets; do not invent an unmeasured 2× or 3× promise,
   relax ratios after seeing a red, or count a mixed-profile comparison as matched
   release evidence (`8a68da4:tests/step_scaling.rs:147-231`; fix-job
   `PROGRESS.md:17-18`).
