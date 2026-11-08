# VM slice 6a: evidence-first dispatch review

Reviewer: **gpt-6.1-sol**. UTC: **2026-11-08T11:26:37Z** (`date -u`).
Reviewed head: **c3d9a31daef7730adbd4d162c8ebfed29c345490**; commits
91623d125bf927c93091357185c8b2f3e5e530ce and c3d9a31; base
b64abfe6e4aeef2939730dcc57c3e6a6ef5d5d2e. Scope: `git diff b64abfe..c3d9a31`,
compiled dispatch and installed indices only; slice 6b excluded.

**Disposition: ACCEPT with P2 beads.** No P0/P1 finding. One proposed P2 bead
below; no `bd` writes. The orchestrator's full gate remains required on this
implementation head; this review does not claim that gate passed.

## Independent reproduction

Evidence is under `.tmp/s1-6a-review/`: `receipts/`, `comparison-initial.json`,
`comparison-paired.json`, `verification.json`, and `machine-profile.json`.
I read the author PROGRESS and receipts, then reused the resource author's
`guard.py` and `symbolicate.py` read-only, rebinding output paths and captured
working directories in `run.py`. Spiral's cited `timeouts.toml` moderate guard
is 4000 s, factor 1, with UNKNOWN/124 on a trip; no trip occurred. Every fresh
receipt records UTC, complete `uptime`, wall and wait4 CPU seconds. All commands
were headless/offline with owned TMPDIR/targets. Width **7**, replacing the
author's 10 in build/test argv, was derived from 64 available processors and
47.2 GiB available memory, reserving 16 threads/16 GiB and 4 GiB per worker.

Both revisions were extracted with `git archive` under owned scratch, so tests'
fixture writes stayed there. All 128 candidate source/test/manifest files were
byte-checked against c3d9a31. The existing gate-pinned ISA
`3b1090790d82a00ac95d30b8da6b42d1d7a9f5d1` was read-only; no interpreter changes.
The baseline release was built before applying the author's test-only
`red-reproduction.patch` in that archive. The candidate used its own fresh
`target/candidate`; immutable release binaries have SHA256
`90dc3de64c06416e2cb66ed6e276ec31c608f7e56c535af908fe8b6f57241baf` (base)
and `846c35a2dc5fcf736f7e42375a627bc691ab4439538eb8dc162c23f57692122e` (head).

The independently rerun commands were:

```sh
cargo test --offline -j7 --lib compiled_step_dispatch_is_independent_of_unused_rows -- --nocapture
BENNETT_VM_ENGINE=jit cargo test --offline -j7 --lib printf_dispatch_work -- --ignored --nocapture
cargo test --offline -j7 --test suite differential:: -- --include-ignored --nocapture --test-threads=7
cargo test --offline -j7 --test suite -- --test-threads=7
python3 -B scripts/quality.py
cargo fmt -- --check --config skip_children=true
git diff --check b64abfe..c3d9a31
```

The first command ran on both archives; printf work ran twice on each. The suite
commands ran on the unchanged candidate archive through the guarded wrapper.

| Evidence | Author | Independent review |
|---|---|---|
| Red portable initialization, 500 → 15000 unused rows | 505 → 15005 continuing; 2525 → 75025 cooperative | Exact match; FAIL/101, `per-step dispatch initialization depends on unused rows`, left 15005, right 505 |
| Red native probes, all passes | 526 → 15026 continuing; 2541 → 75041 cooperative | Exact match; default native also 5039 → 150039 / 5043 → 150043 |
| Green probes per five rows, every individual pass and composition, portable/native | 9 continuing; 5 cooperative; entry initialization 0 at both sizes | Exact match; PASS, Rwork = 1 |
| Printf probes / executed rows, both repeats | 554208551 / 225956 → 451911 / 225956 | Exact match: 2452.727747880 → 1.999995574 probes/row |
| Candidate compile initialization | 43952 visits once; zero at entry | Exact match; baseline compile visits were not instrumented |
| Differential, including ignored cases | 39 passed | 39 passed |
| Full default integration suite | 324 passed, 73 ignored | 324 passed, 73 ignored |
| Quality / formatting / whitespace | PASS | PASS |

The regression can see its named defect: unreachable rows precede the executed
function, so its canonical IDs grow; the five-row order `[0, 2, 1, 2, 3]` uses an
interior-row backedge absent from recovered CFG edges. The old generated native
comparisons and membership visits are instrumented, rather than inferred from
elapsed time. Candidate counts are hand-explained by one entry plus two lookups
per continuation (1 + 4×2 = 9), or five independent entries (5). No time assertion
or sleep was added. Matching continuing/cooperative traces retain original raw
rows, and the separate JIT test checks line/raw metadata and missing entries.

| Fresh verification receipt | Wall / CPU s | Start load 1m / 5m / 15m (`uptime`) |
|---|---:|---|
| `baseline-build` | 28.927898 / 95.598151 | 11.20 / 6.79 / 5.16 |
| `candidate-fixed-build` | 30.353328 / 102.102733 | 10.91 / 9.38 / 6.67 |
| `red-dispatch` | 16.160978 / 23.944144 | 12.38 / 7.94 / 5.64 |
| `green-dispatch` | 19.494147 / 29.489525 | 10.97 / 10.09 / 7.20 |
| `baseline-work-1` | 19.629320 / 19.591982 | 10.96 / 8.30 / 5.90 |
| `baseline-work-2` | 19.962510 / 19.945948 | 11.41 / 8.67 / 6.10 |
| `candidate-work-1` | 10.858249 / 10.806547 | 13.39 / 10.66 / 7.45 |
| `candidate-work-2` | 10.496692 / 10.541823 | 14.28 / 10.95 / 7.58 |
| `differential` | 139.651121 / 321.236726 | 13.77 / 10.95 / 7.62 |
| `suite-fixed` | 33.424628 / 112.858870 | 13.74 / 12.97 / 9.20 |
| `quality` | 0.466702 / 0.434625 | 14.40 / 10.92 / 7.55 |
| `fmt` | 0.065272 / 0.064652 | 19.39 / 14.44 / 10.26 |
| `diff-check` | 0.008797 / 0.006175 | 19.39 / 14.44 / 10.26 |

## Printf JIT measurement

Fresh `cargo build --release --offline -j7` builds, CLI pinned to CPU 0; unchanged
resource corpus/replies/target. Each CLI run was followed by the author's
`perf record -e cpu-clock:u -F 499` and offline ELF/addr2line leaf attribution.
No test probe instrumentation was in either measured release binary. The first grouped
before/after comparison produced a smaller gain, so I additionally ran **ABBA**:
baseline-1, candidate-1, candidate-2, baseline-2, without builds/tests between them.
Both valid sets are retained; the first set is not discarded.

| Revision/repeat | Author wall / CPU s | Review initial wall / CPU s | Initial start load 1/5/15m | Review ABBA wall / CPU s | ABBA start load 1/5/15m |
|---|---:|---:|---|---:|---|
| baseline 1 | 3.043034 / 2.925576 | 6.384089 / 3.214422 | 11.39 / 8.57 / 6.04 | 6.833253 / 3.423379 | 13.55 / 13.06 / 9.37 |
| baseline 2 | 3.040827 / 2.982092 | 6.635228 / 3.266813 | 11.15 / 8.79 / 6.20 | 6.589754 / 3.265802 | 12.30 / 12.86 / 9.44 |
| candidate 1 | 2.372309 / 2.366184 | 5.581653 / 2.791893 | 11.95 / 9.75 / 6.88 | 5.293500 / 2.619182 | 13.81 / 13.14 / 9.46 |
| candidate 2 | 2.520123 / 2.402935 | 5.853399 / 2.859883 | 12.50 / 9.94 / 6.97 | 5.130302 / 2.559790 | 13.36 / 13.07 / 9.47 |

| Continuing-helper leaf samples | Author | Review initial | Review ABBA |
|---|---|---|---|
| baseline | 241/1228 (19.6254%); 277/1270 (21.8110%) | 272/1357 (20.0442%); 299/1390 (21.5108%) | 346/1549 (22.3370%); 336/1549 (21.6914%) |
| candidate | 2/961 (0.2081%); 4/1004 (0.3984%) | 5/1102 (0.4537%); 3/1122 (0.2674%) | 2/1028 (0.1946%); 4/1048 (0.3817%) |

Author mean reductions: **19.58% wall / 19.27% CPU**. Review initial means:
**12.17% wall / 12.80% CPU**. Review ABBA means: **22.34% wall / 22.58% CPU**,
6.711504 → 5.211901 wall s, 3.344591 → 2.589486 CPU s. ABBA within-revision
wall spreads (range/mean) are 3.63% baseline and 3.13% candidate; its observed
cross-pair reduction envelope is **19.67–24.92%**. The initial envelope was
8.31–15.88%; across all four repeats per revision it is 8.31–24.92%.
These are observed ranges, not statistical confidence intervals. CPU 0 wall time
was approximately twice CPU time under shared load. The roughly 19.6% headline
benefit reproduces in magnitude, including a balanced comparison whose minimum
observed gain exceeds it; the exact point estimate is load-sensitive. Helper
sampling corroborates removal of the measured dispatch term, not a causal
speedup bound. No irreproducible-headline P1 is warranted by these receipts.

I compared **all eight complete CLI outputs and all eight perf-run outputs**
byte-for-byte: identical SHA256
`e71d8c7ec46f148e7707008f59a6898819bcd889a5ce6917546d69beaad039f1`.
Against the independent fixture oracle I also checked completed status,
225956 rows, exact 54 console bytes, and i32 result 0; target time is 225956,
charged to original rows. The printf object retains its i80 load/store rows;
the arbitrary-width runtime helpers are unchanged by this diff.

Two review-harness failures were diagnosed, corrected within owned scratch, and
kept out of valid comparisons: initial `candidate-build` reused the baseline
artifact because preserved archive mtimes shared a Cargo target (0.033 s build,
identical binary hash); receipts `candidate-1/2` are invalid. The first full suite
returned 225 passed/99 failed/73 ignored with catalog ENOENT because
`Catalog::published` uses `../bennett-isa-spec`; a scratch symlink to the same
existing gate pin restored it, after which the unchanged full suite passed.
Neither is attributed to the reviewed implementation. See `receipt-note.txt`
and the original failed receipts.

## Semantics and structural review

`canonical_dispatch` is constructed after private transformations/reindexing;
`successor` bounds-checks canonical membership before using a recovered edge or
the canonical entry. Invalid entries/successors return explicit errors. Native
entry addresses come from immutable compiled labels, retaining stack alignment,
error/panic transport and ENDBR64 targets. There is no interpreter fallback;
the differential harness requires nonzero native execution for eligible JIT runs.

`Machine::step` still invokes `step_row` exactly once, via `Program::step`;
the native callback returns None immediately. Launch checks resource ownership
and host requests before that row, then exports resources and terminal state
after it. No wait-loop batching or publication-boundary change was introduced.
`step_row_inner` still resolves the original instruction for metadata/history,
charges its classical target event once, and preserves frame, rollback, reply
and reversal bookkeeping. Activation names remain alongside numeric indices.
The installed object is immutable, uniqueness is validated, and index creation
uses that installed object; operand tokenization is equivalent to the old scan.
The 39-case differential run covers all transformation modes, recursion,
canonical jumps/fault source points, replies, prefix reversal and replay; the
full suite also passed native panic/invalid-entry controls and terminal faults.
No wrong result, silent fallback, omitted capability or 6b scope expansion found.

The new dispatch uses typed indices/maps, replaces the old scans, and adds no
process identifiers, new time literals, dead compatibility wrappers or crates.
The remaining size issue is recorded below, including its inherited baseline.

## Numbered findings / proposed beads

1. **P2 — Split the two touched classical-control monoliths.** Evidence:
   `src/cc.rs:808` `Machine::exec` is **415 lines** (base 411), and
   `src/cc.rs:1559` `command` is **397 lines** (base 393), measured with the
   repository's `scripts/quality.py:functions` parser on both revisions.
   COMMON explicitly makes semantics-bearing functions over 300 lines a review
   finding; a passing structural ratchet does not erase that plan target.
   The new branch/call/return indices and installation logic remain embedded in
   these large functions. This is inherited structural debt, not evidence of a
   wrong result. **Proposed bead:** “Split CC row execution and command setup
   into named phases below 300 lines.” Fix by extracting operation-family
   handlers and argument/binding/installation/report phases, with focused unit
   witnesses and byte-identical fault, target, trace, host-reply and reversal
   outputs; preserve one-row transaction boundaries. A follow-up bead suffices;
   no repair round requested.

Suggestions: keep isolated per-revision Cargo targets, immutable binary hashes,
and an interleaved timing order in future performance receipts. No nit bead.
