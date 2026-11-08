# VM quality evidence

The earlier review findings (segment cancellation, spectator measurement,
conditional adjoints, dynamic classical rows, allocation lifetimes and phase
signs) already have fixes at this branch's starting commit, 805f0ec.

Verification uses headless execution, a job-local TMPDIR and CARGO_TARGET_DIR,
`cargo -j3`, and `--test-threads=4`. The existing regression tests were retained.

Prior-fix verification:

- `timeout 300s cargo test -j3 --lib exact::tests:: -- --test-threads=4`: 5 passed.
- `timeout 240s cargo test -j3 --lib segment_cancellation -- --test-threads=4`: 2 passed.
- `timeout 300s cargo test -j3 --test measurement_exact09 --test allocation_bindings09 --test classical09 -- --test-threads=4 --skip incoming_tokens_with_equal_tuples_keep_distinct_identities`: allocation, dynamic-row and exact-measurement regressions passed. The skipped token-identity differential belongs to the pinned-oracle gate.

The review's original red-first evidence is preserved in the closed issues;
this round adds no replacement implementation for already-fixed findings.

Binary region input: `cargo test -j3 --test memory_files -- --test-threads=4`
reproduced 2 failures (`unknown option --mem-file`) before implementation,
then passed all 3 tests. Coverage includes both engines, both ISA versions,
step enumeration, reversal, duplicate/unknown/bad-size bindings and the last
byte of a 64 KiB image. Payload assertions now use the published JSON integer
convention (small integers are numbers), retaining exact hand-derived bytes.

Default test split: an initial consolidated run reached the 180 s timeout
inside exhaustive oracle-cleanup expansion. Published-oracle comparisons,
published conformance, C ABI rebuilds and exhaustive source expansion now use
`#[ignore]`; the second gate command runs **all** ignored tests. No assertion
or case was removed. Integration modules link into one `suite` binary.

`/usr/bin/time -p timeout 180s cargo test --offline -j3 -- --test-threads=4`
passed in **51.54 s** (warm test build, line tables only). All sibling fixtures,
specs and binaries resolve through `tests/support/pins.rs` to the full SHA in
`gate.toml`; a wrong checkout is an explicit failure reporting both heads.
The pin was materialised with the pinned gate tool's `verify` command against
the main checkout. That original measurement used fd626d3, which was not
reachable from origin/main at materialisation. The current published pin is
recorded in the verification below.

Coverage (default suite, before the structural refactors):
`timeout 600s cargo llvm-cov --offline -j3 --lcov --output-path .tmp/quality-vm/coverage.lcov -- --test-threads=4`
passed (44 unit + 310 integration; 72 gate-only ignored cases). Measured source
line coverage is **81.71% (23,850 / 29,189)**, summing LCOV LH/LF records for
`src/`. The resolver verified ISA pin fd626d37aba1f719c3b7b2059be46cbfaf462886.
This number excludes the mandatory slow gate, so it is a reproducible partial
measurement, not a full-contract coverage claim.

The five least-covered modules and reasons for the remaining gaps:

| module | covered / lines | reason |
| --- | --- | --- |
| sched_abi.rs | 0 / 226 | C consumers require a separate static-library build and run in the gate |
| ref_library/oracle_direct.rs | 42 / 139 | exhaustive row-carrier execution belongs to oracle_cleanup09 in the slow gate |
| ref_library/oracle_memory.rs | 113 / 264 | exhaustive memory-carrier reversal belongs to the same slow suite |
| qcli.rs | 159 / 275 | malformed CLI combinations and state display branches are incompletely covered; the larger differential suite adds command coverage |
| launch.rs | 568 / 961 | static-library C ABI consumers and oracle differentials run in the slow gate |

Test-quality pass: the three audit weak witnesses now check a negative large
region shape, the exact hand-derived recursive sum 1+3+7+11=22 through all
engines, and the captured external control/single nested scratch identities.
Focused `cargo test --offline -j3 --test suite -- <filter> --test-threads=4`
runs for `huge_regions`, `transformations_change_private_graph` and
`typed_templates_capture` passed (one test each). Binary-input tests retain
exact bytes, explicit success/failure and both engines/versions.

Structural work: the red scan measured `walk` 602, oracle constructor `body`
501 and `exec_row` 430 lines. They are now grammar phases, storage versus row
construction, and locations versus effects respectively; every extracted
function is below 300 lines. `Instr.op` is a parsed `Op` enum, with widths
stored in sized variants and original spelling preserved for identities and
diagnostics. Oracle execution and exact-opcode validation dispatch on it.

`cargo test --offline -j3 -- --test-threads=4` passed 44 unit + 311 integration
cases after typed execution (72 ignored), and the focused coherent typing
suite passed 13 after enum validation. The explicitly ignored
`source_constructor_rejects_bad_pairs` test also passed. The opcode regression
first failed to compile because the typed representation did not exist.

Remaining function owners: next VM quality round, issue bennett-vm-bsj for
protected cc.rs `exec`/`command` (415/395; split after bjm-dense merges), and
bennett-vm-68j for qvalidate.rs `sequence`, ref_library/source.rs `instruction`
and typing.rs `classical` (334/359/352). The protected executors retain their
legacy spelling adapters; changing the parsed opcode's type required only
small constructor/trace conversions there.

Fail-fast/cache verification: scheduler lock-key reads now return named errors
for unreadable allocation, load/store, free, hostcall and effect operands;
launch propagates them. New-register tracking distinguishes deliberately
absent pure-row snapshots from a missing mutation snapshot. Sparse step cells
remain zero by the published machine model, an intentional value rather than
an error fallback. Two impossible action-state defaults are asserted.

Engine configuration (including errors) is read once through OnceLock.
Arithmetic propagates selection/kernel errors. The bounded thread-local JIT
cache uses the object hash plus a length-delimited execution SHA-256, since
oracle expansion and library callers can modify AST rows without changing
canonical object text. The new regression verifies cache reuse and recompiles
a changed constant whose object text/hash stays unchanged.

The proposed fallible resource API first failed its regression to compile
against the old tuple API. After implementation, the full default command
passed 46 unit + 313 integration tests (72 ignored); it includes absent-operand,
partial-snapshot, invalid-kernel and cache-mutation cases. Remaining legacy
opcode spelling adapters in dense-owned executors have owner/bead
bennett-vm-bai for integration after bjm-dense.

Source hygiene: the source grep for full internal issue IDs, shift/job labels
and numbered milestone references is now empty. User-facing refusals retain
published sections and concrete missing capabilities. Two old assertions
still depended on issue labels; they now check the corresponding expansion
library/qload refusal. Both focused tests pass. The ignored C11 scheduler
consumer also passed after updating its semantic diagnostic assertion.

Out-of-scope tracked litter is owned by the orchestrator under bennett-vm-dhx:
`.beads/hooks/pre-push.backup` and `docs/PROGRESS.md`, plus missing ignore
patterns. This job cannot edit those paths. README now documents the actual
pinned test layout and gate rather than the obsolete live-sibling overrides
or earlier release-gate procedure. The remaining tracked files are counted
as baseline exceptions, not described as removed.

The local ratchet is one small stdlib-only script, `scripts/quality.py`.
It scans Rust functions after blanking strings and nested comments, records
cumulative function-length buckets (>25, >100, >300) and maximum length per
file, and pairs them with a count of trivial call wrappers (at most three
nonblank body lines, a call, no control-flow keyword). `branch_mass` is the
repository's cognitive-complexity proxy: each if/else/for/while/loop/match,
logical conjunction/disjunction and match arm scores one plus its enclosing
brace depth beyond the outer scope. It is a structural estimate, not a claim
of an external analyser's score. Definitions stay fixed across measurements.

The one gate check runs the script's parser/mutation tests and rejects any
per-file increase, new wrapper, new litter path or increased source identifier
count. It compares only against this repository's recorded baseline. The
mutation tests independently add a branch, a trivial wrapper and hygiene
litter, and require each regression to be detected. Raw-string braces and
nested comments have parser tests. Intentional future baseline changes need
review alongside their source changes; `--record` writes the snapshot below.

Reproducible hygiene searches:

~~~sh
rg -n 'bennett-vm-|b-isa-spec-|milestone[ -][0-9]|shift [0-9]|quality-vm' src
git ls-files | rg '(__pycache__|[.]pyc$|[.]log$|LOCK$|[.]backup$|[.]darc$|PROGRESS|LAST-RUN)'
~~~

The first search has no matches; the second lists only the two scoped
exceptions above. Full machine-readable per-file distribution/mass follows.

Repository HANDOFF.md is retired; README points to the workspace handoff.
The red presence/pointer check now passes. Dated review and quality evidence
remain available; transient job progress stays outside tracked files.

Verification before the ISA pin update (8bc9d78):

- Headless job-local environment; ISA fixture root set to the verified cache checkout for fd626d37aba1f719c3b7b2059be46cbfaf462886.
- /usr/bin/time -p timeout 180s cargo test --offline -j3 -- --test-threads=4: **46 unit + 313 integration passed**, 72 mandatory gate cases ignored, **52.72 s** (warm build).
- timeout 600s cargo llvm-cov --offline -j3 --lcov --output-path .tmp/quality-vm/coverage-final.lcov -- --test-threads=4: same default suite passed; refreshed source line coverage **81.98% (24,057 / 29,346)**.
- Least-covered module counts remain sched_abi 0/226, oracle_direct 42/139, oracle_memory 113/264, qcli 159/275, launch 568/963; the reasons above still apply.
- timeout 180s cargo clippy --offline -j3 --all-targets -- -D warnings: passed.
- Focused ignored C11 launch and scheduler consumers: one test each passed. Full slow/push gate is deliberately left to the orchestrator.
- python3 scripts/quality.py: **five parser/mutation tests passed**, no baseline regression. A quote inside a block comment first reproduced a false function boundary; advancing lexical search past the complete comment fixed it without changing this source baseline.
- git diff --check: passed. Obsolete fixture-root bindings were deleted after pin conversion.

Before the contract adoption, the function distribution was 1,124 functions: 792 at most 25 lines,
254 at 26–100, 73 at 101–300, five over 300 with owners listed above.
The round's explicit follow-ups are bennett-vm-bsj, bennett-vm-68j,
bennett-vm-bai and bennett-vm-dhx. Internal process labels in source remain
zero. These measurements do not claim a full gate, fresh review or merge.

Published ISA pin verification at the pin-only checkpoint (b0a842c):

- `gate.toml` and all ISA commit fields in `SPEC_VERSIONS.toml` now name
  `3b1090790d82a00ac95d30b8da6b42d1d7a9f5d1`. The ISA 0.8 spec,
  ISA 0.9 spec, frame-domain erratum and catalog were checked with
  `git show 3b10907:<path>`. The catalog envelope digest remains
  `5de4e4995350313e50217880ba1b95de39cd043283b726337d4894efb005d197`.
- The pinned gate tool's `verify --repo <main-checkout> --sha 58080e3`
  materialised and built both published binaries and reported `pushed=True`.
- Each focused invocation of `cargo test --offline -j3 --test suite <test>
  -- --ignored --exact --test-threads=4` passed for
  `classical09::incoming_tokens_with_equal_tuples_keep_distinct_identities`,
  `differential::region_call_overlaps_trap_before_any_effect` and
  `differential::admitted_classical09_rows_differential`.
  The last witness uses `qreg[*]`, explicitly allowed by ISA 0.9 §16.2;
  no VM parser change or expectation adjustment was necessary.
- `python3 scripts/quality.py` passed all five parser/mutation tests and
  reported no baseline regression.
- `cargo fmt --all -- --check` failed on inherited formatting in source and
  tests. At this initial checkpoint the default suite, full ignored suite
  and Clippy were not rerun. Formatting is not a `gate.toml` check and remains
  a separate follow-up; the contract-adoption verification supersedes this
  initial stop.

K91/K84 contract adoption (source 64415e8):

- K91 tuple-only Q requests validate canonical partition labels, arity and
  equal-label tuple consistency before execution. Actions have no primary
  identity slot; oracles prepend theirs. Source value bindings and captured
  oracle contexts retain identities separately from catalog JSON and tuples.
  Same-identity oracle calls retain their matched cleanup carrier through
  forwarding and adjoint traversal; distinct equal-tuple tokens do not match.
  Schedule consumption compares partitions as well as parameter hashes.
- K84 load checks reject duplicate resolved domain components and missing
  expected relations. Certificate object hashes must match the loaded source.
  Malformed control counts fail explicitly, and failed partition binding
  leaves its argument identities unchanged.
- Generated CC wrappers follow the source object's ABI (§9.1), rebind the
  changed object hash (§9.2/§12.3), and provide explicit tuple partitions
  (§12.3 K91). The newly published unresolved-import fixture remains in every
  corpus and must diagnose §9.3 on execution. Published case commands and
  expectations were not changed or omitted; all 10 D and 16 G CLI cases run.
- The additive estimate and scheduler symbols validate alias-aware requests.
  Complete resource-estimate realisation/certificate construction remains at
  its existing explicit implementation boundary. The loader negotiation
  explicitly left undefined by the pinned §10.1 is not invented here.
- `timeout 300s cargo test --offline -j3 -- --test-threads=4`: **46 unit +
  323 integration passed**, 72 mandatory cases reserved for the ignored run.
- `timeout 1200s cargo test --offline -j3 -- --ignored --test-threads=4`:
  **72 passed, zero failed**, 424.54 s test runtime. This is the entire ignored
  set, including both published conformance checks and every differential and
  source-expansion test. No published case was re-expected or skipped.
- `timeout 300s cargo clippy --offline -j3 --all-targets -- -D warnings`:
  passed. `timeout 60s python3 scripts/quality.py`: five parser/mutation tests
  passed, no baseline regression. `git diff --check`: passed.
- The pinned gate tool's `verify --repo <main-checkout> --sha 64415e8`
  passed and reported `pushed=True`. These individual check invocations do not
  claim a child-run aggregate `gate_run run`; that remains merge-time work.
- Ten added contract tests pass. The initial CLI/metadata/checker classes,
  source carrier, stale source binding, malformed control count and atomic
  binding checks each reproduced a failure before their corresponding fixes.

The K91/K84 contract adoption intentionally updates the baseline alongside its
source changes, for review under the baseline policy above. New partition and
certificate-metadata modules have maximum function lengths of 38 and 64 lines.
The existing source-lowering module now has seven functions over 100 lines
(previously eight); its maximum and count over 300 lines are unchanged. Added
branches implement partition validation, capture and schedule matching,
alias-aware request validation, stale certificate rejection and complete
control reversal. The quality script and its five mutation/parser tests are
unchanged. Source process identifiers remain zero and no litter exception was
added. This snapshot does not claim the deferred resource-estimation mechanics
or the loader negotiation that the pinned ISA explicitly leaves undefined.

<!-- quality-baseline -->
```json
{
  "files": {
    "src/action.rs": {
      "branch_mass": 1220,
      "max_lines": 202,
      "over_100": 5,
      "over_25": 27,
      "over_300": 0,
      "trivial_wrappers": 9
    },
    "src/action/allocation_bindings.rs": {
      "branch_mass": 200,
      "max_lines": 91,
      "over_100": 0,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/bigint.rs": {
      "branch_mass": 148,
      "max_lines": 23,
      "over_100": 0,
      "over_25": 0,
      "over_300": 0,
      "trivial_wrappers": 11
    },
    "src/call_matching.rs": {
      "branch_mass": 69,
      "max_lines": 71,
      "over_100": 0,
      "over_25": 3,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/cc.rs": {
      "branch_mass": 1345,
      "max_lines": 415,
      "over_100": 5,
      "over_25": 25,
      "over_300": 2,
      "trivial_wrappers": 28
    },
    "src/cc/frame_work.rs": {
      "branch_mass": 1,
      "max_lines": 20,
      "over_100": 0,
      "over_25": 0,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/cc/trace.rs": {
      "branch_mass": 96,
      "max_lines": 56,
      "over_100": 0,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/certificate.rs": {
      "branch_mass": 485,
      "max_lines": 164,
      "over_100": 3,
      "over_25": 13,
      "over_300": 0,
      "trivial_wrappers": 7
    },
    "src/certificate/metadata.rs": {
      "branch_mass": 69,
      "max_lines": 64,
      "over_100": 0,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/cleanup_discipline.rs": {
      "branch_mass": 449,
      "max_lines": 151,
      "over_100": 1,
      "over_25": 9,
      "over_300": 0,
      "trivial_wrappers": 7
    },
    "src/cliargs.rs": {
      "branch_mass": 152,
      "max_lines": 33,
      "over_100": 0,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 4
    },
    "src/coherent_typing.rs": {
      "branch_mass": 367,
      "max_lines": 211,
      "over_100": 1,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/device.rs": {
      "branch_mass": 4,
      "max_lines": 12,
      "over_100": 0,
      "over_25": 0,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/estimate.rs": {
      "branch_mass": 19,
      "max_lines": 36,
      "over_100": 0,
      "over_25": 1,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/exact.rs": {
      "branch_mass": 236,
      "max_lines": 76,
      "over_100": 0,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 11
    },
    "src/execution_work.rs": {
      "branch_mass": 62,
      "max_lines": 19,
      "over_100": 0,
      "over_25": 0,
      "over_300": 0,
      "trivial_wrappers": 5
    },
    "src/history.rs": {
      "branch_mass": 48,
      "max_lines": 28,
      "over_100": 0,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 6
    },
    "src/history_work.rs": {
      "branch_mass": 23,
      "max_lines": 17,
      "over_100": 0,
      "over_25": 0,
      "over_300": 0,
      "trivial_wrappers": 4
    },
    "src/identity.rs": {
      "branch_mass": 330,
      "max_lines": 88,
      "over_100": 0,
      "over_25": 12,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/instances.rs": {
      "branch_mass": 393,
      "max_lines": 99,
      "over_100": 0,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/interval.rs": {
      "branch_mass": 107,
      "max_lines": 66,
      "over_100": 0,
      "over_25": 7,
      "over_300": 0,
      "trivial_wrappers": 4
    },
    "src/invocation.rs": {
      "branch_mass": 52,
      "max_lines": 42,
      "over_100": 0,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/jit.rs": {
      "branch_mass": 481,
      "max_lines": 176,
      "over_100": 1,
      "over_25": 8,
      "over_300": 0,
      "trivial_wrappers": 7
    },
    "src/json.rs": {
      "branch_mass": 481,
      "max_lines": 76,
      "over_100": 0,
      "over_25": 5,
      "over_300": 0,
      "trivial_wrappers": 5
    },
    "src/launch.rs": {
      "branch_mass": 368,
      "max_lines": 71,
      "over_100": 0,
      "over_25": 13,
      "over_300": 0,
      "trivial_wrappers": 18
    },
    "src/launch_locks.rs": {
      "branch_mass": 88,
      "max_lines": 62,
      "over_100": 0,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 5
    },
    "src/lib.rs": {
      "branch_mass": 12,
      "max_lines": 21,
      "over_100": 0,
      "over_25": 0,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/lifted_typing.rs": {
      "branch_mass": 259,
      "max_lines": 128,
      "over_100": 1,
      "over_25": 3,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/link.rs": {
      "branch_mass": 151,
      "max_lines": 125,
      "over_100": 2,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/main.rs": {
      "branch_mass": 31,
      "max_lines": 33,
      "over_100": 0,
      "over_25": 1,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/object.rs": {
      "branch_mass": 680,
      "max_lines": 170,
      "over_100": 3,
      "over_25": 13,
      "over_300": 0,
      "trivial_wrappers": 6
    },
    "src/opcode.rs": {
      "branch_mass": 228,
      "max_lines": 33,
      "over_100": 0,
      "over_25": 1,
      "over_300": 0,
      "trivial_wrappers": 7
    },
    "src/oracle.rs": {
      "branch_mass": 890,
      "max_lines": 263,
      "over_100": 5,
      "over_25": 13,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/persistent.rs": {
      "branch_mass": 257,
      "max_lines": 68,
      "over_100": 0,
      "over_25": 6,
      "over_300": 0,
      "trivial_wrappers": 30
    },
    "src/persistent/node_work.rs": {
      "branch_mass": 1,
      "max_lines": 11,
      "over_100": 0,
      "over_25": 0,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/qcli.rs": {
      "branch_mass": 133,
      "max_lines": 202,
      "over_100": 1,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/qreg_typing.rs": {
      "branch_mass": 154,
      "max_lines": 72,
      "over_100": 0,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/qvalidate.rs": {
      "branch_mass": 974,
      "max_lines": 334,
      "over_100": 4,
      "over_25": 13,
      "over_300": 1,
      "trivial_wrappers": 1
    },
    "src/rational.rs": {
      "branch_mass": 157,
      "max_lines": 33,
      "over_100": 0,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 7
    },
    "src/ref_library.rs": {
      "branch_mass": 1238,
      "max_lines": 223,
      "over_100": 8,
      "over_25": 15,
      "over_300": 0,
      "trivial_wrappers": 10
    },
    "src/ref_library/action_support.rs": {
      "branch_mass": 53,
      "max_lines": 55,
      "over_100": 0,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/ref_library/address.rs": {
      "branch_mass": 27,
      "max_lines": 33,
      "over_100": 0,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/ref_library/call_domains_tests.rs": {
      "branch_mass": 18,
      "max_lines": 175,
      "over_100": 1,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 2
    },
    "src/ref_library/calls.rs": {
      "branch_mass": 13,
      "max_lines": 51,
      "over_100": 0,
      "over_25": 1,
      "over_300": 0,
      "trivial_wrappers": 2
    },
    "src/ref_library/cleanup.rs": {
      "branch_mass": 66,
      "max_lines": 108,
      "over_100": 2,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/ref_library/named_interface.rs": {
      "branch_mass": 197,
      "max_lines": 132,
      "over_100": 1,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/ref_library/named_interface_tests.rs": {
      "branch_mass": 31,
      "max_lines": 36,
      "over_100": 0,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/ref_library/oracle_bindings_tests.rs": {
      "branch_mass": 85,
      "max_lines": 56,
      "over_100": 0,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 6
    },
    "src/ref_library/oracle_cleanup.rs": {
      "branch_mass": 146,
      "max_lines": 124,
      "over_100": 1,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/ref_library/oracle_direct.rs": {
      "branch_mass": 25,
      "max_lines": 150,
      "over_100": 1,
      "over_25": 1,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/ref_library/oracle_memory.rs": {
      "branch_mass": 107,
      "max_lines": 138,
      "over_100": 2,
      "over_25": 3,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/ref_library/oracle_paths.rs": {
      "branch_mass": 211,
      "max_lines": 227,
      "over_100": 1,
      "over_25": 1,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/ref_library/oracle_procedures.rs": {
      "branch_mass": 197,
      "max_lines": 136,
      "over_100": 2,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/ref_library/oracle_source.rs": {
      "branch_mass": 356,
      "max_lines": 273,
      "over_100": 2,
      "over_25": 4,
      "over_300": 0,
      "trivial_wrappers": 8
    },
    "src/ref_library/source.rs": {
      "branch_mass": 2069,
      "max_lines": 359,
      "over_100": 7,
      "over_25": 20,
      "over_300": 1,
      "trivial_wrappers": 6
    },
    "src/ref_library/source/partition.rs": {
      "branch_mass": 254,
      "max_lines": 166,
      "over_100": 1,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/sched.rs": {
      "branch_mass": 381,
      "max_lines": 97,
      "over_100": 0,
      "over_25": 9,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/sched_abi.rs": {
      "branch_mass": 52,
      "max_lines": 74,
      "over_100": 0,
      "over_25": 1,
      "over_300": 0,
      "trivial_wrappers": 5
    },
    "src/sha256.rs": {
      "branch_mass": 21,
      "max_lines": 15,
      "over_100": 0,
      "over_25": 0,
      "over_300": 0,
      "trivial_wrappers": 1
    },
    "src/step.rs": {
      "branch_mass": 699,
      "max_lines": 197,
      "over_100": 4,
      "over_25": 8,
      "over_300": 0,
      "trivial_wrappers": 9
    },
    "src/stepcli.rs": {
      "branch_mass": 70,
      "max_lines": 130,
      "over_100": 1,
      "over_25": 3,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/target.rs": {
      "branch_mass": 730,
      "max_lines": 185,
      "over_100": 3,
      "over_25": 12,
      "over_300": 0,
      "trivial_wrappers": 4
    },
    "src/token_aliases.rs": {
      "branch_mass": 44,
      "max_lines": 38,
      "over_100": 0,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 0
    },
    "src/typing.rs": {
      "branch_mass": 640,
      "max_lines": 352,
      "over_100": 1,
      "over_25": 1,
      "over_300": 1,
      "trivial_wrappers": 2
    },
    "src/validate.rs": {
      "branch_mass": 1136,
      "max_lines": 260,
      "over_100": 7,
      "over_25": 21,
      "over_300": 0,
      "trivial_wrappers": 7
    },
    "src/value.rs": {
      "branch_mass": 224,
      "max_lines": 55,
      "over_100": 0,
      "over_25": 2,
      "over_300": 0,
      "trivial_wrappers": 0
    }
  },
  "litter": [
    ".beads/hooks/pre-push.backup",
    "docs/PROGRESS.md"
  ],
  "process_identifiers": {}
}
```
