# Execution model §7 Resources — review

Reviewer: **gpt-6.1-sol**. Date UTC: **2026-11-08T10:33:32Z**. Reviewed commit:
**72dab7779b1d55440b6feb8852568b33ab3adc7a**, including 549ab34.
`verify.py`: **PASS**, all eight checks. Independent live validation: **PASS**.
**Disposition: ACCEPT with P2 beads.** No P0/P1; two P2 findings, including one nit.

## Reproduction and measured comparison

`git diff origin/main..HEAD -- docs/design/2026-11-07-vm-execution-model.md`
adds only §7; §§1–6 are byte-identical to origin/main **4f49792**
(`.tmp/s1-review/design-diff.patch`, `scope.json`). No design/code/oracle edits.

Executed the author's unmodified `.tmp/s1-resources/verify.py` through
`python3 .tmp/s1-review/guard.py short author-verify python3 .tmp/s1-review/run_verify.py`.
The wrapper uses `runpy` and redirects only the final `verification.json` write
into the owned review directory; every original assertion executes. This checks
saved evidence, so it is supplemented by the live runs below.
Result: `.tmp/s1-review/verification.json`, `receipts/author-verify.*`.

Fresh `taskset -c 0,1,2,3,4,5,6,7,8,9 cargo build --release --offline -j 10`
passed under the copied author's **moderate** guard class: **21.710123 wall /
86.988594 CPU s**. Width was recalculated from **64** threads and
**61192528 KiB** MemAvailable: `min(nproc-16, floor((MemAvailable-16GiB)/4GiB))`;
CPU affinity enforces the ceiling. Workloads ran serially on CPU 0, with all four
display/browser variables unset and review-local TMPDIR/CARGO_TARGET_DIR.
The release CLI and rebuilt `phase_probe` are each byte-identical to the author's;
CLI SHA256 is `97bd21e79b201df47002e22d21fada31e9a3b8fdd248c96f0b7297fdc30dd1c2`.
Evidence: `.tmp/s1-review/machine-profile.json`, `binary-equality.json`,
`receipts/release-build.*`, `receipts/probe-build.*` and `validation.json`.

Repeated both largest profiles, plus both printf CLI profiles needed to check the
headline claims. For each engine, the command is the author's recorded argv with
only binary/probe and perf-output locations redirected into `.tmp/s1-review/`:
`taskset -c 0 perf record -e cpu-clock:u -F 499 -o <review>/<label>.data -- <command>`.
ABI command: `<review>/phase_probe <author>/corpus/r3 snapshot 0 false`.
CLI command: `<review>/target/release/bennett-vm run-program <author>/corpus/r3/launch.bennett main --replies <author>/corpus/r3/replies.json --target <author>/corpus/r3/target.json`.
`BENNETT_VM_ENGINE` selects the named engine. The initial profiling runs did not
enable `BENNETT_JIT_AUDIT`; the later live semantic runs did. That switch only
prints final diagnostic counters (`src/jit.rs:50-61`). Sampling and original
ELF-offset symbolication were reused, conserving all samples with zero lost
samples. Exact argv and results are in `.tmp/s1-review/receipts/<label>.*`;
raw reports, ELF maps, `*-functions.json` and `*-top20.md` are retained there.

| Profile | Author wall / CPU s | Reviewer wall / CPU s | Wall change | Samples author / reviewer |
|---|---:|---:|---:|---:|
| `abi-interpreter-perf` | 7.946328 / 7.888685 | 10.591422 / 10.587475 | +33.29% | 3315 / 3500 |
| `abi-jit-perf` | 9.050124 / 9.006911 | 8.686390 / 8.708854 | -4.02% | 3857 / 3703 |
| `r3-interpreter-perf` | 2.323535 / 2.268141 | 2.399561 / 2.339385 | +3.27% | 918 / 947 |
| `r3-jit-perf` | 3.025137 / 2.984499 | 3.364462 / 3.276278 | +11.22% | 1279 / 1400 |

Top-five order and exclusive shares, with the same all-user-sample denominator:

| Profile | Author, ranks 1–5 | Reviewer, ranks 1–5 |
|---|---|---|
| `abi-interpreter-perf` | `malloc_consolidate` 7.81%; `unlink_chunk` 6.46%; `System::alloc_impl` 4.74%; `__GI___libc_free` 4.49%; `sha256` 4.22% | `malloc_consolidate` 7.71%; `unlink_chunk` 4.86%; `_int_malloc` 4.69%; `__GI___libc_free` 4.43%; `System::alloc_impl` 4.34% |
| `abi-jit-perf` | `malloc_consolidate` 8.06%; `unlink_chunk` 6.22%; `__GI___libc_free` 4.36%; `System::alloc_impl` 4.33%; `sha256` 3.71% | `malloc_consolidate` 7.80%; `unlink_chunk` 5.97%; `System::alloc_impl` 4.81%; `__GI___libc_free` 4.24%; `sha256` 3.83% |
| `r3-interpreter-perf` | `malloc_consolidate` 13.29%; `unlink_chunk` 11.55%; `__GI___libc_free` 7.19%; `__memcmp_avx2_movbe` 5.12%; `_int_malloc` 4.14% | `malloc_consolidate` 12.88%; `unlink_chunk` 11.62%; `__GI___libc_free` 7.71%; `__memcmp_avx2_movbe` 5.70%; `_int_malloc` 5.07% |
| `r3-jit-perf` | `native::Code::run::helper (drive)` 18.69%; `malloc_consolidate` 10.32%; `unlink_chunk` 8.52%; `__GI___libc_free` 5.55%; `__memcmp_avx2_movbe` 4.46% | `native::Code::run::helper (drive)` 20.71%; `malloc_consolidate` 10.07%; `unlink_chunk` 8.07%; `__GI___libc_free` 5.36%; `__memcmp_avx2_movbe` 4.14% |

Unprofiled full-ABI reruns were **7.863392 / 7.814012 wall/CPU s** (interpreter)
and **8.987073 / 9.029579 s** (JIT), beside the author's final-pair wall readings
**7.690843/7.587536** and **8.996545/8.945308**. Every phase's allocation,
reallocation, requested-byte and deallocation count agrees exactly. Execution
allocations remain **43455002 / 43652653**; snapshot allocations **25118243** in
either engine. All full snapshots remain **77536709 bytes**, SHA256
`213786b56204164c3bb5a1374a08a1cd39bf1192a9be6767504405fcd3681501`.
Evidence: `.tmp/s1-review/deterministic-equality.json`, `receipts/phase-*-snapshot.*`;
author's pairs in `.tmp/s1-resources/repeatability.json`.

Live forward/replay/reverse/corrupt-tape commands used the new release binary and
review-owned output tapes. Both engines match the independent printf fixture,
complete JSON and saved tape bytes; replay is identical; eligible reversal
restores memory/entry state and empties console/tape; corrupt tapes explicitly
exit 1 with `replay mismatch`. Trace off/on snapshots match; savings reproduce
exactly: **694279 allocations / 46738094 requested bytes** in either engine.
Interpreter trace CPU ratio is **1.3795×** versus author's **1.3765×**; JIT is
**1.1152×** versus **1.3579×**. Native compile CPU is **0.043274 s** versus
**0.030009 s**, with identical **309728 allocations / 46949819 bytes**.
Evidence: `.tmp/s1-review/validation.json`, `receipts/verify-*`, `receipts/trace-*`,
`receipts/compile.*`. No full repository or sibling gate was run.

Re-ran the author's CUDA query binary under its moderate class. Verbatim output
matches `.tmp/s1-resources/receipts/cuda-device-query.stdout`:

```text
driver_version=13000 runtime_version=12000 device_count=1
device=0 name=NVIDIA GeForce RTX 3090 SMs=82 global_memory_bytes=25769279488 compute_capability=8.6 memory_free_bytes=24436015104 memory_total_bytes=25769279488 unified_addressing=1 concurrent_kernels=1
```

Reviewer receipt: `.tmp/s1-review/receipts/cuda-device-query.*`, exit 0.
This confirms the inventory, including approximately 24 GiB; it supplies no
kernel-throughput result.

## Findings and proposed beads

1. **P2 — Rung 3, musl hello-world: profile ranking and share stability need their own repeated evidence.**
   At design lines 779–811 and 954–960, the original figures are accurately copied
   from the original samples, but both largest ABI top threes change on a
   byte-identical rerun: interpreter rank 3 becomes `_int_malloc` instead of
   `System::alloc_impl`; JIT rank 3 becomes `System::alloc_impl` instead of
   `__GI___libc_free`. Interpreter ABI `unlink_chunk` falls **6.46→4.86%**
   (**−1.60 percentage points**); CLI JIT helper rises **18.69→20.71%**
   (**+2.03 points**), exceeding even a 1.4-point interpretation of the requested
   variance comparison. Exact differences are in `.tmp/s1-review/validation.json`.
   The **1.4% wall / 1.1% CPU** envelope in the document applies to one unprofiled
   pair, explicitly not a statistical or host-wide bound; it cannot bound leaf
   shares. The first reviewer ABI interpreter profile is also **33.29%** slower,
   while its deterministic counts and snapshot remain identical. This is a
   profiling-reproducibility limitation, not an incorrect retained-history result
   or a false claim that the original receipts had different values; no P0/P1
   rung-path defect is demonstrated.

   **Proposed P2 bead:** “Measure leaf-profile repeatability before pricing VM
   slice priorities.” Retain these recordings, repeat samples/count attribution
   with a stated uncertainty method, and report rank/share ranges separately
   from wall variation. Keep the order an opportunity hypothesis until candidate
   dispatch/log counters and measured gain per implementation unit exist. Do not
   turn this timing discrepancy into a failing performance gate. Related existing
   open measurements: `bennett-vm-3ol`.

2. **P2 nit — Rung 3 resources documentation: process identifiers escape the open-items list.**
   Design lines **706** and **886** use `bennett-vm-bjm` in the policy/dense
   narrative, and **718–719** explain child/orchestrator assignment through
   `T3CHILD`. The brief permits bead IDs only in the open-items list; COMMON
   forbids process identifiers in product documentation. Evidence paths needed
   to find measurements are not the concern.

   **Proposed P2 nit bead:** “Remove orchestration identifiers from Resources
   prose.” Refer to the dense CPU plan by its evidence filename, retain bead IDs
   in the final open-items list, and state verification scope without child
   protocol names. This is the only nit.

## Reasoning and semantic assessment

The order **1/5 landed → 2 → 6a dispatch → 3 → 4 → 6b** is defensible as a
provisional investigation order. Each table row names evidence: trace off/on
counts for 1; snapshot allocations plus records/target/history samples for 2;
continuing helper plus small compile phase for 6a; grow/COW and request totals
for 3; ABI execution/checkpoint baseline for 4; current indexed-lock source for
5; per-row ABI native-entry audit for 6b. The fresh results retain the broad
allocation/export and continuing-dispatch signal. None isolates removable cost
or establishes quantitative throughput gain per round; the text explicitly
acknowledges that. Promoting dispatch-only 6a preserves the same row transaction;
6b retains publication/ownership dependencies. Sampling absence cannot prove
locks free, and the text correctly keeps multi-owner work open.

The decompositions respect §§1–4: ordered atomic row publication, LIFO reversal,
separate saved journal versus active reversible tape, no effect/reply reissuance
for reconstruction, and ownership across non-CLOSED states remain binding.
Independent runtimes are worker-local; `Rc` is not shared by wrapping it in
`Arc`. Frozen snapshot output rejoins in source order. JIT workers preserve
canonical identities and host-visible boundaries. Dense tiles assign each
amplitude pair one writer, synchronize dependent groups, retain oracle/fault
mapping, and serialize the final draw in ISA order. No retained-history
counterexample was found on the rung-2/3 paths; these are candidate designs,
not verified implementations.

The std-only policy boundary is explicit and correctly scoped at lines 704–708:
`std::thread` needs no exception; CUDA/HIP/wgpu FFI/crates need maintainer
ratification. §7 repeatedly requires the ISA reference interpreter to remain
single-threaded and unchanged, and author/reviewer production diffs contain no
oracle edits. Comparable reference work remains missing; this review authorizes
no reference-source instrumentation. Dense times/hot functions are explicitly
unmeasured; `dense-cost-model.json` is an analytical **16·2^n** state-byte and
**32·2^n** full-pass model, whose table was recalculated independently.

The displayed CLI/top-20/other-profile numbers match their named author receipts
and profiles (`.tmp/s1-review/document-number-audit.json`); phase and trace
counts, snapshot bytes, build/provenance, GPU, repeated pairs and analytical
capacity numbers have named local sources. No unattributed fitted performance
constant was found in §7. Historical numbers are not upgraded to acceptance
bounds, and retained fixture coverage is not called whole-rung acceptance.

Suggestion: retain the already-open JIT function-map measurement before assigning
the unmapped ABI native-code share to arithmetic or dispatch; no new semantic
change or oracle work is implied.

**Disposition: ACCEPT with P2 beads.** The orchestrator must still obtain its
single landing gate on the review head. Proposed beads above were not filed;
this review performed no `bd` writes.
