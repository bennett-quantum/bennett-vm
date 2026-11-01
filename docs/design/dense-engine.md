# Dense state-vector engine — design brief (bead bennett-vm-bjm)

Status: implemented in `src/dense/`, standalone. Integration through
`BENNETT_VM_ENGINE=dense` into `src/action.rs` / `src/cc.rs` is NOT
part of this stream: it waits for bead `rhc` to land on `main` and a
later orchestrator stage (see "Integration boundary" below).

## Literature actually read

Only abstracts / arXiv landing pages were reachable and read in this
session (2026-11-01); full texts were not read. Claims below rely on
the abstracts plus first-principles derivation and local measurement.

| Source | What was read | What is taken from it (abstract-level) |
|---|---|---|
| Smelyanskiy, Sawaya, Aspuru-Guzik, qHiPSTER, [arXiv:1601.07195](https://arxiv.org/abs/1601.07195) | abstract | vectorization + multi-threading + cache blocking; performance bounded by sustainable memory bandwidth |
| Jones, Brown, Bush, Benjamin, QuEST, [arXiv:1802.08032](https://arxiv.org/abs/1802.08032), Sci Rep 9, 10736 (2019) | abstract (via search result + landing page) | hybrid threaded/distributed/GPU exact simulation toolkit; comparison point only |
| Suzuki et al., Qulacs, [arXiv:2011.13524](https://arxiv.org/abs/2011.13524), Quantum 5, 559 (2021) | abstract | "numerical techniques to speed-up simulation" — no specific technique is attributed to Qulacs here beyond that |
| Häner & Steiger, 0.5 Petabyte Simulation of a 45-Qubit Quantum Circuit, [arXiv:1704.01127](https://arxiv.org/abs/1704.01127), SC'17 | abstract | automatic kernel code generation + a circuit scheduling algorithm that reduces communication; the single-node analogue used here is gate fusion / reordering into blocked passes |

Not read: full texts of the above; qsim and cuStateVec (named in the
bead as successors). Nothing below cites their internals.

## State representation and layout choice (by measurement)

State: `2^n` complex amplitudes, binary64, qubit `q` = bit `q` of the
basis index (same convention as `src/action.rs`: bit `i` of a basis
key is wire `i`). Exactly one state vector is held: 16 bytes per
amplitude (8 re + 8 im) plus O(1) scalars and O(threads) reduction
scratch. No second state copy exists in any kernel; every kernel is
in place.

Layout candidates measured before choosing (`.tmp/bjm/layout_proto.rs`,
`rustc -O -C target-cpu=native`, scalar only, fresh state per variant,
unitary 2x2, i7-1365U, machine shared, load average ≈ 20 at
measurement, 2026-11-01):

| n | target qubit | AoS interleaved `[re,im]` | SoA blocked (`re[]`, `im[]`) |
|---|---|---:|---:|
| 24 | 0 | 11.88 gates/s | 6.93 gates/s |
| 24 | 3 | 9.52 gates/s | 11.01 gates/s |
| 24 | 10 | 13.19 gates/s | 18.14 gates/s |
| 24 | 23 | 10.35 gates/s | 17.37 gates/s |
| 20 | 0 | 122.8 gates/s | 86.0 gates/s |
| 20 | 19 | 202.6 gates/s | 263.8 gates/s |

Choice: **SoA** (`re: Vec<f64>, im: Vec<f64>`). Reasons, in order:

1. In the memory-bound regime this engine exists for (n ≥ 24, state
   ≫ cache), SoA-blocked is faster for every target except qubit 0
   (up to 1.7x at n = 24 above).
2. SoA is the layout AVX2/FMA can consume without deinterleave
   shuffles: a 2x2 complex matvec over a contiguous run is pure
   broadcast-mul/FMA streams over four arrays' slices.
3. The qubit-0 (stride-1) weakness is exactly the case gate fusion
   hides: low qubits are applied in fused block passes (below), not
   as isolated stride-1 passes, in the workloads that matter.

The benchmark harness (`examples/dense_bench.rs`) keeps an
interleaved naive baseline so this choice stays re-checkable.

## Blocking (qHiPSTER-style)

A 1-qubit gate on qubit `t` (stride `s = 2^t`) touches amplitude
pairs `(i, i+s)` where bit `t` of `i` is 0. The state is processed in
blocks of `2s` consecutive amplitudes; inside a block the lower half
and upper half are each contiguous runs of length `s`. Kernels
iterate (block, offset-in-run): for `s ≥ 4` the inner run is a
unit-stride stream over `re`/`im` slices (AVX2, §SIMD); for `s < 4`
the pair is processed scalarly. Each amplitude is read and written
exactly once per gate (or once per fused group). There is no
separate cache-block parameter to tune for a single gate: the pair
structure itself is the blocking. Cache reuse across gates is bought
by fusion, not by single-gate tiling.

## Gate fusion

`apply_fused(&[Gate])` takes consecutive 1-qubit / controlled /
diagonal / flip gates whose involved-wire union (targets + controls)
has size `k ≤ 6`. It builds the group's `2^k × 2^k` unitary once
(O(gates · 4^k · 2^k) scalar work, at most 64×64) by applying the
gates to the identity, then applies it to the state in ONE pass: for
each setting of the outside bits, gather the `2^k` amplitudes whose
indices vary over the fused wires, multiply, scatter back. Fused
wires are expected to be low (the gather set then lies in one or a
few cache lines / one L1 block); correctness does not depend on it.
Measured reality on this machine: fusion currently does not beat
sequential application (see Results); it bounds memory passes at
one per group, which is the property the later integration relies
on, and its speed is a filed follow-up. If the union exceeds 6 wires the group is
applied sequentially — a documented fallback, identical result, more
passes. Fusion is the single-node form of Häner–Steiger scheduling:
reorder/group work so data already in cache is reused before any new
memory traffic.

## Kernel taxonomy

- General / controlled 1-qubit: pair matvec, control mask tested per
  pair from the pair index (controls are all-ones polarity, matching
  `action::QuantumState::apply1`; arbitrary polarity is built by the
  caller with X conjugation, as the exact engine does).
- Diagonal (`z, s, t, rz, cz, phase_if_all`, controlled phase):
  in-place complex multiply of selected amplitudes; no pair traffic.
- Permutations (`x, cx, ccx, mcx` = `flip`; `swap`; XOR-oracle
  remap): pure index remaps, no complex arithmetic. `flip` and
  `swap` are involutions, applied as disjoint pair swaps. The oracle
  branch enumeration (`apply_xor_oracle`) enumerates basis indices,
  projects the input wires, evaluates a caller-supplied classical
  function once per distinct projected input (memoised in a map
  local to the call), and XORs the result into the target wires.
  Because the map `|x>|y> -> |x>|y XOR f(x)>` is an involution
  composed of pair swaps between indices that agree on input-wire
  bits, it is applied by swapping index pairs `(i, i')` with
  `i < i'` — O(1) scratch besides the memo table (the memo table is
  the one structure whose size is O(distinct inputs), as in the
  exact engine's cache; it is dropped after the call).
- Measurement: Born probabilities by parallel chunked reduction
  (per-thread partial sums, combined in fixed chunk order —
  deterministic); outcome chosen by a caller-supplied `draw ∈ [0,1)`
  in the same order as `QuantumState::measure` (outcomes ordered
  lexicographically in wire order, wire 0 of the wire list first);
  collapse zeroes other outcomes, renormalises by `1/sqrt(p)`, and
  resets measured wires to 0 in the remaining state, mirroring the
  exact engine's handle-consumed semantics. K17 draw order is a
  caller (cc) concern; this engine consumes draws in the order they
  are handed to it and never draws randomness itself.

## Threading

`std::thread` only (crate is std-only; no new dependencies). Work is
statically partitioned by contiguous ranges of pair indices (or
index ranges for diagonal/permutation/reduction kernels): pair
ranges are disjoint by construction, so no locks and no atomics are
needed on amplitudes; threads write through a raw pointer to
disjoint pairs (safety argument documented at the pointer type).
Static partition is chosen over work-stealing: per-pair cost is
uniform, so stealing buys nothing. Thread count: explicit
`with_threads`, defaulting to `available_parallelism()`. First-touch:
when threads > 1, construction re-writes the zero pages in the same
static partition the kernels use, so pages are faulted in by the
threads that will own them. All reductions combine per-thread
partials in fixed order, so results are deterministic for a fixed
thread count (and the error bound below covers variation across
thread counts). Multithreading is skipped for small states
(`2^n < 2^15` amplitudes) where spawn cost dominates.

## SIMD

x86-64 AVX2+FMA via `std::arch`, selected at runtime with
`is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma")`
inside a `#[target_feature]` function. Applies to the contiguous-run
case (stride ≥ 4) of the uncontrolled 1-qubit kernel: 4 amplitude
pairs per iteration, `_mm256` loads over the `re`/`im` runs,
broadcast scalar matrix entries, FMA. The scalar path (also used for
stride < 4, controlled gates, remainders, and non-x86 targets) is
always compiled and is tested to agree with the AVX2 path within the
stated bound (in practice bit-for-bit up to FMA contraction). Tests
can force the scalar path (`set_simd_enabled(false)`) so equivalence
is checked on AVX2 machines too. AVX-512: the development machine
(i7-1365U) has none; no untestable AVX-512 code is shipped. (Bead
says "AVX2/AVX-512 only via std::arch with runtime detection and a
tested scalar fallback" — detection + fallback exist; AVX-512 is left
out rather than shipped untested, flagged here as a deferral, not a
scope cut.)

## Memory bound

`bytes = 16 · 2^n + O(1)` (struct scalars) + O(threads) f64 partials
during reductions + O(4^k) for one fused matrix (k ≤ 6, ≤ 64 KiB) +
the oracle memo table during `apply_xor_oracle` only. Construction
fails loud (panics with context, per crate rule 1) when `2^n` would
overflow `usize` or the requested thread count is 0. `DenseState::
memory_bytes()` reports the exact state allocation.

## Floating-point error bound and certificate labels

Binary64, unit roundoff `u = 2^-53 ≈ 1.11e-16`. Each kernel is a
normwise-backward-stable unitary application; the engine accumulates
a running bound `B` (in 2-norm, initial state assumed normalised):

- 1-qubit / controlled gate: output = sum of 2 complex products.
  Standard rounding analysis (Higham, *Accuracy and Stability*,
  γ-notation applied componentwise; counted conservatively) gives
  per-gate forward error ≤ `8u·‖ψ‖ + O(u²)`; engine charges
  `B += 8u·(1+B)` (the `(1+B)` factor keeps it a valid bound under
  composition with unitaries, which preserve the 2-norm up to prior
  error).
- Fused k-wire gate: output = sum of `2^k` complex products; charged
  `B += 8·2^k·u·(1+B)`.
- Diagonal gate: one complex product per amplitude; charged
  `B += 4u·(1+B)`.
- Permutation (`flip`, `swap`, XOR oracle): exact index moves;
  charged 0.
- Measurement collapse/renormalise: scaling by `1/sqrt(p)` after a
  parallel sum of squares over ≤ `2^n` terms; charged
  `B += (n+8)·u·(1+B)` (pairwise-by-chunk summation keeps the term
  logarithmic, not linear, in the number of amplitudes).

`error_bound()` returns `B`. This is an honest a priori bound for
exactly the operations performed, assuming gate matrices are the
binary64 values handed in (matrix-entry rounding vs the ideal real
gate is the caller's charge, exactly as `target` accuracy accounting
already handles it).

Certificate rule: `evidence()` returns kind `"numerical"` with the
current bound, qubit count and gate count. The dense engine NEVER
produces kind `"exhaustive"`: any `ε = 0` claim can only come from
the exact engine (`src/exact.rs`, `src/action.rs`). This matches
`src/certificate.rs`, where an error-bound claim requires the
corresponding numerical/channel kind. Differential tests compare
dense output to the exact engine and to an independent slow oracle
in the test file, with tolerance derived from `error_bound()` — the
bound is thereby itself tested, not just asserted.

## Integration boundary (later stage, NOT this stream)

Deferred verbatim from the bead, postponed mechanics only: engine
selection through `BENNETT_VM_ENGINE=dense` in the Q-action path,
branch enumeration wiring into CC `oracle_call`, K17 draw plumbing,
and "every existing action test passes on the dense engine". Those
edit `src/action.rs` / `src/cc.rs`, which a concurrent job (rhc) is
changing; the brief forbids touching them here. The standalone API
mirrors `QuantumState::{apply1, flip, phase_if_all, swap, measure}`
signatures and semantics so the later wiring is mechanical.

## Verification plan (as executed)

Red-first tests in `tests/dense_kernels.rs` (independent slow oracle:
full pair loop over an interleaved `Vec<(f64,f64)>`, written inside
the test) and `tests/dense_differential.rs` (the exact
`action::QuantumState` engine as oracle). Benchmarks in
`examples/dense_bench.rs`: naive interleaved single-thread baseline
vs dense engine, 1 thread and all threads, 20/24/28 qubits, gates/s
and bytes/amplitude; numbers recorded in this file's Results section
(and `git log` / progress file) with the machine load at measurement.

## Results (measured, this machine)

Filled in after the benchmark runs in this stream; see the commit
that touches this section. Command shape:

```
cargo run --release -j4 --example dense_bench -- --qubits 20,24,28
```

<!-- RESULTS:BEGIN -->

Measured 2026-11-01 on the development machine (Intel i7-1365U,
12 hardware threads, AVX2+FMA, no AVX-512; rustc 1.99.1,
`cargo run --release -j4 --offline --example dense_bench`). The
machine was heavily shared throughout (~15 concurrent jobs; load
average 20–40 during every run, printed by the harness), so absolute
rates are depressed and multithreaded scaling is understated; treat
ratios between rows measured in the same run as the signal. All rows
report 16 bytes/amplitude and norm2 = 1.000000000000 after every
batch (asserted by the harness). Full raw outputs were kept in the
job directory (`.tmp/bjm/bench24.txt`, `bench28.txt`).

Single-qubit gate, high target (`t = n-1` unless noted), gates/s:

| n | naive AoS 1T | dense scalar 1T | dense AVX2 1T | dense AVX2 12T |
|---:|---:|---:|---:|---:|
| 20 | 136.7 | 315.0 | 438.3 | 228.0 |
| 24 | 7.78 | 13.69 | 12.59 (t=12: 21.00) | 18.98 (t=12: 22.16) |
| 28 | 0.34 | 0.65 | 0.62 (t=14: 0.58) | 1.10 (t=14: 1.13) |

Fused group of 8 gates on qubits 0–3 (counted as 8 gates) vs the
same gates sequentially, gates/s:

| n | fused 1T | seq 1T | fused 12T | seq 12T |
|---:|---:|---:|---:|---:|
| 20 | 91.0 | 147.8 | 133.0 | 122.9 |
| 24 | 4.46 | 9.29 | 11.59 | 11.78 |
| 28 | 0.27 | 0.46 | 0.87 | 0.91 |

Readings (honest, including the negative ones):

- Memory is exactly 16 bytes/amplitude at every size (256 MiB at
  n = 24, 4096 MiB at n = 28), as required.
- AVX2 wins where the state is cache-resident or semi-resident
  (n = 20: 1.4x over the run-scalar kernel, 3.2x over naive). At
  n = 28 the kernel is purely bandwidth-bound under load and
  scalar-run ≈ AVX2 single-threaded (0.65 vs 0.62); SIMD is not the
  bottleneck there, memory is — consistent with qHiPSTER's
  bandwidth-bound abstract claim.
- Threading helps once a gate's work dwarfs spawn cost (n = 28:
  1.8x; n = 24 high target: 1.5x) and hurts at n = 20 under this
  load. With load ≈ 30 on 12 threads there were rarely free cores;
  unloaded scaling remains unmeasured.
- Kernel iteration driven by these benchmarks: replacing the
  per-pair index arithmetic scalar kernel with contiguous-run slice
  loops took n = 20 high-target scalar from 111 to 315 gates/s.
- Fusion does NOT yet pay on this machine: the fused 16×16 matvec
  is scalar-arithmetic-bound, while the sequential low-qubit gates
  use the AVX2/run kernels, so fused ≤ sequential at every size
  (closest at 12 threads). Fusion is implemented and tested
  correct; making it fast needs a contraction-order or SIMD fused
  kernel. Filed as follow-up bead `bennett-vm-1x1`; the design
  claim is narrowed accordingly: fusion currently bounds memory
  passes, it does not yet win time.

<!-- RESULTS:END -->
