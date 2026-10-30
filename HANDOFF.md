Milestone 5 implementation is in progress (bead bennett-vm-npe).
Read docs/PROGRESS.md top section for current JIT evidence and modes.
Private graph passes and x86-64 Linux codegen are implemented; initial existing
1,076-invocation corpus agrees for every pass, composition and JIT (zero diverge).
Published conformance in JIT mode:445/0/46. Full stateful/native gate pending.
M4 blockers below remain unchanged. No push.

Milestone 4 independent target work VERIFIED 2026-10-30; parent v23 remains BLOCKED.
Read docs/PROGRESS.md top section for current evidence; older sections are historical.

Commits this resumed session start with dbdfe60 (recovered structured work); the
subsequent implementation/docs commit contains native QRAM and final accounting.
Child v23.4 is complete: native target leaves for bundles, qif/loops/while flags,
local image creation/actual inverse image gates; explicit designated dirty-scheme
and qborrow pools; ordered per-logical-gate request batches with achieved-error
charging and exact permutation enforcement; native qram load/store/exchange;
actual event/live-interval accounting including classical/effect and inverse rows.
CLI target_resources/target_error and immutable launch snapshots expose the
accumulated program ledger at the observed boundary, not per-run estimates/certs.

Gates: 72 Rust tests pass, 160 launch C checks, all 15 scheduler exports,
21,888 one-row inverse checks. 1,076/1,076 CLI differential comparisons agree
(1,050 JSON / 26 diagnostics), zero divergences. Raw logs .tmp/resume-tests.log,
.tmp/resume-clippy.log, .tmp/resume-conformance-final.log.
Standalone published conformance: 445 PASS / 0 FAIL / 46 UNAVAILABLE, exit 2,
491 cases. Per-suite pass/fail/unavailable:
A21/0/0 B35/0/0 C339/0/0 D7/0/5 E0/0/16 F33/0/2 G10/0/13 H0/0/10.
The concurrent ISA agent fixed E fixtures and semantic staging-refusal probes;
this implementer only read their published artifacts. E/H still report hard-coded
UNAVAILABLE; no fabricated promotion. New E-epsilon-zero-exact increases total.

Blocking 47t: §11 ref expansion document/hash/ordinals/scratch/removable subset
is unpublished until accepted ISA 0.9. Full lifted/oracle arithmetic and dispatch,
comparators/multiplexers/inrange scans, qinplace oracle calls and non-native qram
therefore fail with explicit milestone-4 item/47t. Verbatim schedules, scheduled
shares/defaults/timing/accounting/reproducibility wait too. Do not guess a private
ref library. cxg: eps_decade index remains undefined. The fixture-gap clh has
published repairs; 47i partial-sector clarification remains tracked.
Parent v23 stays BLOCKED, not closed. Next complete milestone 4 after publication,
then milestone 5 JIT; estimates/rigorous certificates/coverage still name 5.

Gotchas from red-green recovery: invalid caller idle designation was silently
accepted; fixed before allocation. Inherited while-adjoint witness contradicted
§5.3's explicit adjoint order; implementation was right. qload allocated unclassed
wires; fixed for native qram. Borrow frames omitted dirty ownership replacement;
zero-time allocations omitted peak; qfree adjoint lost original class; all fixed
with hand-derived regressions. Resource fixture initially used an unsupported
16-bit target width because parsing treated the spec's example list as an enum;
fixed to honor pointer parameters. A previous byte-equality target test needed to
compare all execution fields plus separately hand-derived new ledger fields.

Headless/offline, TMPDIR .tmp, absolute paths, std-only; no foreign repo edits,
no push, no Dolt remote sync. Keep commits path-scoped and retry index.lock.
