# Dense state-vector engine — bennett-vm-bjm

The standalone engine is implemented in `src/dense/`. Q-action and CC
selection through `BENNETT_VM_ENGINE=dense` is the last deliverable,
pending the orchestrator merging closed `bennett-vm-rhc` from main into
this branch and notifying this job. `src/action.rs` and `src/cc.rs` have
not been edited by this job. The full action-corpus differential and
certificate output remain integration acceptance items.

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
exact interpreter. It never draws randomness: the caller owns ISA K17
ordering. Prefix reductions compute the next wire's two masses and
condition the draw on the chosen positive mass. Zero branches are never
selected, including when draw is the predecessor of 1 and the state
norm has drifted. Zero/nonfinite norm, invalid draws and duplicate
measurement wires fail explicitly.

A measurement over k wires uses O(k * 2^n) work, O(workers) partials
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

The kernel tests use an independent out-of-place slow oracle.
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

Final measured tables will be replaced after rebuilding the final
kernels. Earlier predecessor tables have been removed because they
were not reproduced and timed norm reductions as part of gate work.
