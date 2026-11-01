# Dense state-vector engine — bennett-vm-bjm

The standalone engine is implemented in `src/dense/`. The orchestrator
confirmed `bennett-vm-rhc` closed and merged here as `644a212` on
2026-11-01 at 22:39 UTC, clearing the original integration dependency.
Engine-selected action/CC execution remains incomplete. Its new red CLI
test fails in `src/jit.rs::selection` before reaching the action executor:
`unimplemented: execution engine dense (milestone 5)`. Finishing selection
and action-report provenance requires `src/jit.rs` and `src/qcli.rs`,
which are outside this job's current write scope. The integration-stage
note permits narrow wiring in `src/action.rs` / `src/cc.rs`; those files
have not been edited by this job. The full action-corpus differential
and actual numerical provenance remain integration acceptance items.

## Literature actually read

The predecessor read abstracts. This continuation read the following
full-text sections on 2026-11-01; it does not claim to have read every
page or the implementations of these packages.

| Paper | Sections read | Applicable observation |
|---|---|---|
| Smelyanskiy, Sawaya, Aspuru-Guzik, [qHiPSTER, arXiv:1601.07195](https://arxiv.org/pdf/1601.07195) | §3.1, §4.4, Fig. 4 | Apply successive original gates within a resident cache block, reducing main-memory transfers without constructing a larger dense gate. This is the low-wire fusion algorithm used here. |
| Jones et al., [QuEST, arXiv:1802.08032](https://arxiv.org/pdf/1802.08032), Scientific Reports 9, 10736 (2019) | §II.B, especially single-node subsection and Eq. 1 | Pair-index iteration updates a single vector in place. The paper distinguishes its own implementation from the cache-blocked comparator; we do not attribute cache blocking to QuEST. |
| Suzuki et al., [Qulacs, arXiv:2011.13524](https://arxiv.org/pdf/2011.13524), Quantum 5, 559 (2021) | §5.1–5.4 | Separate launch overhead, arithmetic and bandwidth limits. SIMD and thread thresholds address different regimes; dense matrix fusion trades extra arithmetic against memory traffic. |
| Häner and Steiger, [0.5 Petabyte Simulation of a 45-Qubit Quantum Circuit, arXiv:1704.01127](https://arxiv.org/pdf/1704.01127), SC'17 | §3.1–3.2 | In-place updates avoid a second state vector; gathered local vectors support scattered wires. Register blocking and vectorization matter as fused matrix dimension grows. |

These are algorithm references, not sources for local performance
claims. MPI, GPU execution, automatic code generation, and circuit
reordering are not implemented here. qsim and cuStateVec were not read.

## Layout choice by measurement

Wire q is bit q of the basis index, matching `QuantumState`.
A state consists of two `Vec<f64>` allocations (`re`, `im`), together
exactly `16 * 2^n` bytes. There is no second state-sized allocation in
any gate or measurement kernel.

The inherited scalar layout experiment was rebuilt and rerun, rather
than trusting its previous numbers:

```
rustc -O -C target-cpu=native .tmp/bjm/layout_proto.rs -o .tmp/bjm/target/layout_proto
.tmp/bjm/target/layout_proto 24 5
```

At 22:10 UTC on 2026-11-01, Intel i7-1365U, shared machine, end load
average `19.83 18.41 21.09`, gates/second:

| Target | Interleaved AoS / pair indices | SoA / contiguous runs |
|---:|---:|---:|
| 0 | 9.21 | 4.75 |
| 3 | 7.71 | 9.89 |
| 10 | 12.03 | 17.17 |
| 23 | 6.51 | 11.11 |

SoA was retained because it wins the larger-stride cases and admits
straight SIMD loads. The experiment compares layout plus loop order;
it does not isolate layout as the only variable. Qubit zero was a
measured weakness. The production kernels now also vectorize wires
zero and one using lane permutations; scalar code remains available.
The permanent harness retains an independent AoS baseline.

## Blocking, fusion and threading

For target t, stride s=2^t, the single-wire kernel processes disjoint
pairs in blocks of 2s amplitudes. Longer runs use contiguous slices;
controls below t require per-pair tests, while higher controls are
constant across a run. Diagonal gates multiply selected amplitudes
without pair traffic. Flip, swap, and XOR-oracle gates exchange index
pairs without complex arithmetic or a floating-point error charge.

`apply_fused` accepts consecutive one-wire, controlled, diagonal and
flip gates. With at most six involved wires:

- If every involved wire is below 10, the engine traverses aligned
  tiles of up to 1024 amplitudes (16 KiB). Every original gate runs
  on that tile before the traversal advances. Gate order is preserved;
  no larger matrix is constructed. Sixteen KiB is a conservative
  portable L1 working-set cap, not a runtime cache-size discovery.
- Otherwise, it gathers a vector of at most 64 amplitudes for each
  outside-bit setting, applies the original gates locally, and scatters
  once. Temporary vectors and spread indices have fixed bounds.
- Larger unions execute the same gates sequentially. This explicit
  fallback preserves capabilities and accounts for every gate.

The first implementation built a dense fused matrix; it was slow and
charged only for the final multiplication, omitting construction and
sequence-length error. The current algorithm avoids that arithmetic
and uses the same per-gate bound recurrence as sequential execution.

Workers use `std::thread::scope` with static contiguous partitions.
Pair kernels partition pairs; fused kernels partition complete tiles
or gathered blocks; permutations assign each involution pair to its
lower index. Each amplitude has a single writer. Measurement separates
source reads and source clearing into joined passes. These ownership
arguments are documented beside the raw-pointer operations.

Requested threads default to `available_parallelism()`. States below
2^14 amplitudes run on one worker; above that threshold the number of
work units caps workers. The decision uses state size, so changing a
work unit from amplitudes to cache tiles cannot accidentally disable
threading. Construction first-touches pages with the workers. Threads
are spawned per operation; no persistent pool or affinity policy is
claimed. Reduction order is fixed for a thread count.

## SIMD and scalar fallback

x86-64 kernels use `std::arch`, guarded by runtime detection of both
AVX2 and FMA. Wires zero/one use four-lane partner permutations; higher
wires load four pairs from contiguous runs. Eligible fused tile gates
use the same kernels. Controlled kernels, small cases, scattered
fusion and other architectures have scalar implementations.
`set_simd_enabled(false)` forces scalar execution in tests.

No AVX-512 implementation is shipped: the development CPU lacks that
feature, so it cannot be tested here. AVX2 is the implemented optional
accelerator; AVX-512 remains future optimization, not a correctness
requirement or a different supported gate set.

## Oracle enumeration, measurement and memory

`apply_xor_oracle` evaluates the supplied classical function once for
every input configuration and stores the output in one indexed memo
table. There is no redundant map. The input and target wire lists must
be disjoint and individually unique. XOR leaves input bits unchanged,
so each permutation is an involution and can use disjoint swaps.
Target and fault bits can be concatenated by the eventual caller;
controls, budgets and the existing interpreter memo context belong to
that integration, rather than being inferred by this kernel.

Measurement takes a caller-supplied draw in [0,1), enumerates outcomes
lexicographically in wire-list order, collapses, renormalizes and resets
measured wires to zero, matching the consumed-handle semantics of the
exact interpreter, including the empty-register outcome `(0, "")` on
zero-wire states. It never draws randomness: the caller owns ISA K17
ordering. Prefix reductions compute the next wire's two masses and
condition the draw on the chosen positive mass. Zero branches are never
selected, including when draw is the predecessor of 1 and the state
norm has drifted. Zero/nonfinite norm, invalid draws and duplicate
measurement wires fail explicitly.

A measurement over k wires uses O((k+1) * 2^n) work, O(workers) partials
and O(n) recursion stack per worker, without a 2^k outcome table. Each
leaf sums at most 32 terms; the remaining tree and partial merge are
balanced. `probabilities(wires)` is an explicit diagnostic which does
allocate its returned 2^k table; it is not used by measurement.

Memory is `16 * 2^n` bytes for the state, plus bounded local vectors
and index metadata and worker stacks/partials. The oracle memo table
is the explicit exception, O(2^input_wires), required for once-per-input
classical evaluation. Neither measurement nor fusion allocates a
second state vector. The benchmark also drops each state before
allocating the next, including its fused/sequential comparison.

## Floating-point bounds

`error_bound()` is a forward 2-norm state-error allowance B. Prepared
input is assumed normalized; `set_amplitude` is an input-preparation
operation and does not certify a different input or renormalize it.
Reference gate matrices are the binary64 entries handed to the engine.
Rounding of an ideal angle or symbolic matrix into those entries is a
separate caller obligation.

Let u=2^-53 and gamma(j)=j*u/(1-j*u). Let R bound the exact reference
state norm (initially 1), and A bound the gate operator norm. A is an
upward-padded row-sum bound on M†M followed by a square root for a
one-wire matrix, or the maximum diagonal modulus, and is at least one
to cover identity control sectors. This includes small norm amplification
from rounded matrix entries and also handles finite nonunitary test
matrices. Each arithmetic gate updates:

```
B' = A*B + gamma(j)*A*(R+B) + j*MIN_POSITIVE*sqrt(2^n)
R' = A*R
```

The accumulator rounds upward. j=32 for a pair matvec and j=8 for a
complex phase multiplication conservatively cover scalar association,
complex absolute-value sums and FMA variants. The absolute term covers
gradual underflow without relying only on relative error. Pure
permutations charge zero. Fusion applies this recurrence for every
original gate. Nonfinite inputs and overflowing gate-norm/error
calculations fail explicitly instead of emitting a misleading bound.

For measurement use eta=gamma(2*n+64), absolute probability allowance
alpha=4*2^n*MIN_POSITIVE, selected computed mass p, and
p_low=max(0,p-eta*p-alpha). For a supported reference outcome, the
normalized conditional-state distance is at most 2*B/sqrt(p_low),
plus reduction, square-root, division and scaling error. The engine
charges an additional 8*(eta+alpha/p). If p_low is zero it uses the
sum-of-vector-norms bound instead of claiming relative accuracy.
R resets to one for the normalized conditional reference state.

This bound is conditional on the returned outcome. No finite-precision
sampler promises the same outcome as an exact sampler for a draw
arbitrarily close to a cumulative-probability boundary. Evidence also
reports an accumulated measurement distribution allowance (total
variation, capped at one), using 2*B+B^2 plus conservative reduction and
sampling arithmetic terms. It applies to normalized unitary reference
states. A draw-order test chooses draws away from these uncertain
boundaries. The tests check norm distance against the stated bound,
with an explicitly separate allowance for the reference engine's f64
projection; there is no 1000x bound multiplier.

## Certificate labelling and integration contract

`Evidence.kind` is always `"numerical"`, including a fresh state or a
permutation-only run with B=0. It also contains the state error bound,
measurement distribution allowance, wire count and gate count. It is
simulation evidence and is never an `"exhaustive"` proof.

The published [ISA 0.9 §12.1](../../../bennett-isa-spec/spec/Bennett-ISA-Spec-0.9.md)
defines a formal numerical certificate as a checker-computed interval
operator-norm bound for every invocation context, with `subject`,
`metric: "operator_norm"`, `precision_bits` and decimal `bound`.
`src/certificate.rs` enforces the corresponding claim kinds. A single
state-vector error B or measurement total-variation estimate cannot
be substituted for this operator-norm certificate or its interval
checker. The standalone engine emits no certificate today.

The integration must propagate engine provenance and simulation
uncertainty to its report, prevent dense observations from generating
an exhaustive/epsilon-zero semantic claim, and retain the published
interval-checker path for formal numerical claims. Measurement-channel
claims additionally require the published channel checker; state-error
metadata is not a diamond-norm witness. No new certificate fields or
weakened checker semantics are introduced by this design.

Before that integration is complete, run the existing action corpus
with dense selection, compare exact and dense results within the
reported bound and explicitly test certificate kinds. Include CC
oracle-then-gate, controls, fault bits, K17 draws, allocations/releases
and cleanliness checks after merging rhc. Until then acceptance items
for selected execution and actual certificate output remain open.

## Verification and benchmark results

The kernel tests use independent out-of-place oracles: one with f64
arithmetic and one with the crate's exact scalar arithmetic (including
symbolic sqrt(1/2)). Their traversal is implemented in the test file,
independently of both the dense kernels and QuantumState's map engine.
`tests/dense_differential.rs` uses the exact interpreter, including a
128-gate fused/scalar sequence, wire-order-sensitive measurement, and
a two-wire Grover iteration. Tests cover SIMD/scalar agreement,
threaded collapse/reset, rare conditioning, loss of small positive
probability terms, near-one draws, invalid state/input failures,
scattered fusion and the actual seven-wire sequential fallback.

The harness reports gates/second and 16 bytes/amplitude for a naive AoS
baseline, scalar SoA and runtime SIMD SoA at one/all threads. Each
batch's norm is checked outside the timed interval; best of three
batches follows an untimed warmup (n=28 uses just one measured batch).
Printed GB/s is equivalent gate traffic, not physical DRAM bandwidth,
especially for fused batches that reuse cache. Per-row machine load
is printed. Only one benchmark runs at a time in this job.

Final kernels rebuilt with rustc 1.99.1 on an Intel i7-1365U (12
available threads, AVX2/FMA). Measurements began 2026-11-01 at 22:17
UTC (20/24) and 22:19 UTC (28); this results section was written
2026-11-01 22:23 UTC. Commands after the release build:

```
cargo build --release -j4 --offline --example dense_bench
timeout 240 .tmp/bjm/target/release/examples/dense_bench --qubits 20,24 --reps 10
timeout 240 .tmp/bjm/target/release/examples/dense_bench --qubits 28 --targets 27 --reps 1
```

Both runs exited successfully. There were three measured batches of ten
repetitions at 20/24, and one measured batch of one repetition at 28,
each after an untimed warmup. All norm and fused/sequential sample
checks passed. The 28-qubit run deliberately limits single-gate targets
to the highest wire; no unloaded-machine scaling claim is made.

High-target one-wire gates, gates/second:

| Qubits | Target | Naive AoS 1T | SoA scalar 1T | SoA AVX2 1T | SoA AVX2 12T | State MiB | Bytes/amplitude |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 20 | 19 | 171.03 | 233.06 | 322.06 | 353.70 | 16 | 16 |
| 24 | 23 | 8.63 | 19.31 | 22.40 | 58.44 | 256 | 16 |
| 28 | 27 | 0.53 | 0.98 | 0.67 | 1.41 | 4096 | 16 |

Low-wire AVX2 (target zero), gates/second:

| Qubits | Naive AoS 1T | AVX2 1T | AVX2 12T |
|---:|---:|---:|---:|
| 20 | 173.93 | 318.71 | 287.14 |
| 24 | 11.16 | 16.64 | 42.35 |

Eight gates on wires 0–3, counted as eight gates per group:

| Qubits | Fused 1T | Sequential 1T | Fused 12T | Sequential 12T |
|---:|---:|---:|---:|---:|
| 20 | 355.61 | 291.53 | 740.76 | 123.98 |
| 24 | 28.27 | 17.85 | 120.64 | 24.51 |
| 28 | 0.71 | 1.06 | 5.83 | 3.24 |

Recorded one-minute load (including start/end readings) ranged from
13.87 to 21.26 on 12 available threads. Start/end three-value load averages were:

| Run | Start | End |
|---|---|---|
| 20/24 | 16.43 15.86 18.86 | 17.47 16.13 18.66 |
| 28 | 21.26 17.14 18.92 | 19.56 17.99 18.96 |

For the 28-qubit high-target rows, one-minute load was 20.47 (naive),
19.98 (AVX2 1T), 18.91 (scalar), and 20.07 (AVX2 12T). The fused and
sequential rows were 20.08/18.74 (1T) and 19.48/19.56 (12T).

Interpretation and limits:

- The state allocation meets 16 bytes/amplitude at every measured size.
  Thread stacks, metadata and optional oracle memoization are separately
  bounded as described above; the figure is not total process RSS.
- Low-wire SIMD removes the scalar layout experiment's target-zero
  disadvantage in this fixture. Cache fusion is faster than sequential
  at 20 and 24 qubits, and at 28 qubits with 12 threads. It also avoids
  spawning workers separately for every constituent gate.
- Fusion is still slower at 28 qubits on one thread (0.71 vs 1.06).
  `bennett-vm-1x1` therefore remains open with narrower evidence; no
  universal fusion speedup is claimed. Tile-size and dispatch overhead
  tuning remain performance follow-up work, not missing gate semantics.
- At 28 qubits scalar is faster than single-thread AVX2 in this run.
  The measurements do not isolate bandwidth, frequency or contention
  well enough to assign that difference to one cause. Both paths are
  tested, and runtime SIMD can be disabled explicitly.
- Earlier predecessor tables were removed: they were not reproduced
  and included norm reduction in gate timings. Before the low-wire
  SIMD addition, this corrected harness measured n=24 fusion at
  9.93/24.43 gates/s (1T/12T) versus 9.98/16.96 sequential. Those
  preliminary rows had one-minute load 14–23. Final rows supersede them;
  shared-machine variation prevents a controlled before/after claim.

Standalone gate after the empty-register compatibility fix (before
adding the selected-engine integration regression):

```
timeout 300 cargo test -j4 --offline --test dense_kernels --test dense_differential -- --test-threads=4
# 26 kernel tests + 5 exact-interpreter differential tests passed; none failed.
timeout 300 cargo build -j4 --offline
# Whole crate build passed.
rustfmt --edition 2024 --check src/dense/mod.rs examples/dense_bench.rs tests/dense_kernels.rs tests/dense_differential.rs
# Passed.
```

The empty-register fix was made after the benchmark run; it changes
only measurement, which the benchmark does not call. The benchmarked
pair, SIMD and fusion kernels are unchanged. Full action selection and
certificate-output verification still await the integration boundary.

The next integration gate is now red in
`selected_dense_cli_executes_bell_and_labels_numerical_evidence`: dense
selection must execute the existing Bell fixture, preserve its amplitudes,
and emit a `simulation` object containing kind `numerical` and an error
bound. This report metadata is separate from formal ISA certificate
claims. The five existing exact-interpreter differential tests and all
26 kernel tests still pass; the whole crate builds. No test was skipped
or weakened to conceal the selector failure.
