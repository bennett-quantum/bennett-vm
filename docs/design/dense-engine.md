# Dense state-vector engine — bennett-vm-bjm

`BENNETT_VM_ENGINE=dense` selects dense quantum execution for standalone
actions, CC programs and C ABI resources. The CC tier uses the interpreter;
other existing engine choices keep their previous behavior. Reports and
ABI snapshots include numerical simulation provenance. Exact reference
and certificate-replay states remain independent of engine selection.
The standalone kernels and the stable-wire runtime adapter are in
`src/dense/`; execution wiring in `action.rs`, `cc.rs`, `qcli.rs` and the
selector in `jit.rs` is deliberately small.

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

## Runtime integration and stable wires

`QuantumState::new()` remains the exact semantic reference, including for
certificate checking. `new_selected()` is used at execution entry points;
`new_dense()` explicitly constructs a dense state for Rust callers/tests.
The dense variant does not keep the exact map or binary64 sparse shadow.
`amplitudes()` materializes a diagnostic snapshot only when requested.
The legacy public `amps` cache belongs to exact states and stays empty
for a dense state. Standalone action JSON requests one such snapshot.

The runtime maps stable logical wire IDs onto packed dense coordinates.
A constant logical register is stored as basis metadata, so a 192-bit
basis value does not allocate 2^192 amplitudes. A gate that makes a wire
vary promotes that wire into the vector, extending the arrays in place.
Logical controls known false make the gate an identity; known true
controls are omitted from the packed kernel. Pure constant flips and
uniform remap outputs update basis metadata without complex arithmetic.
This preserves arbitrary logical bit widths while the dense vector still
has the usual exponential cost in the number of materialized wires.

The shared action/CC oracle helper from rhc retains projected-input and
memory memoization, fault XOR, controls and budgets. It calls the same
basis-remap interface as swaps and coherent memory operations. The dense
adapter finds a closed packed coordinate space, keeps uniform output bits
virtual, and moves amplitudes along permutation cycles. Cycle leaders use
one complex carry and bounded wire metadata, with no visited table or
second amplitude vector. The interpreter's involutions take linear work;
long general cycles can cost quadratic time to keep scratch bounded.
The adapter is tested on a four-cycle with 80 uniform output bits.

Discrete decisions are exact or fail loudly (beads yt9, ykb; review
`docs/reviews/2026-11-02-design-review.md`). A numerical state cannot
certify that a residue is zero, so constant-wire, §5.6 cleanliness and
§7.4 rewind answers are never decided within the error allowance. A wire
is constant only by one of three exact certificates:

- T0, metadata: the wire is basis metadata, exact by construction.
- T1, operation history: no live value-changing operation touches the wire
  since it was last an exact constant. The runtime logs logical operations
  and retires an inverse pair when the exact product of the two operands is
  the identity and every live operation between them commutes with them
  (operations commute when neither changes a wire the other involves). An
  operand counts as exact only when it cannot be a binary64 projection of a
  symbolic rotation: rx/ry projections have equal diagonal entries and rz
  projections with a unit first entry are the identity, so H, S, T, Z, their
  inverses, flips and phases cancel, while rotation-shaped matrices never do.
  This classification relies on `apply_exact` carrying only
  `Exact::rotation` matrices (true of both call sites in `action.rs`).
  A remap over a packed domain of at most 4096 keys is recorded as its
  key map, restricted to the support bitmap, which holds the exact support.
  Adjacent recorded remaps compose into one entry because support bits move
  with the map. The entry retires when the composition is the identity on
  that support. For example, XOR, swap, XOR in an adjoint `qinplace` is
  the identity on the support but not on the packed domain. This covers oracle compute and
  uncompute, as in fixture `r9-codex-oracle`. Remaps over larger domains
  are unrecorded and not replayable (bennett-vm-k8w).
  Measurement, preparation, input states and remaps are barriers; measured and prepared wires are re-anchored to their new exact
  value. The log is bounded (old entries merge into one barrier).
- T2, support: a bitmap, one bit per amplitude, marks amplitudes that may be
  nonzero in the exact or computed state. Kernels update it lane-parallel
  (a general gate ORs the inputs with nonzero exact coefficients,
  permutations and remaps move bits, diagonals keep them). A wire whose
  other branch has no set bit is exactly constant. This covers Bennett
  compute/use/uncompute with non-commuting uses in between.

A wire varies only when both branch masses are certainly nonzero: the
computed branch norm, rounded down, exceeds that branch's error bound (T3).
Per packed wire p and value v the runtime keeps a bound on the error
restricted to bit p = v. A kernel whose target is not p (or a diagonal one)
acts within each branch and maps it like B; a kernel on p mixes the two
branch bounds by the moduli of its entries; flips move them; a promoted
wire starts at [B, 0]. Rounding terms stay global (gamma*A*(R+B)), so a
branch bound is never below the per-gate rounding floor, but it is not
amplified by unrelated history. When T0-T3 cannot settle a
`constant_value` question, the runtime falls back to an exact answer. It
replays the live operation history through the exact interpreter
(`QuantumState::new()`), making the same calls the program made. Retired
pairs are exact identities, so only live operations are replayed. Replay
is possible only when every live operation has exact operands: no
rotation-shaped matrix, no unrecorded remap and no merged history. Every
replayed measurement (recorded with its draw) must also take the outcome
the dense engine took. This is how algebraic cancellations are decided,
for example a controlled -I that acts as Z between two H gates
(conformance `interference`), which no structural certificate sees.
Replay is checked against every certificate that did decide. It costs as
much as the exact interpreter on the live history and runs only for
undecided questions. `release` uses the same replay for released wires the
certificates leave undecided, and re-anchors their history certificate.
This keeps a residue that is exactly zero from staying materialized on a
released wire, where a projected report would otherwise show it. Every
other case is undecidable: `constant_value`,
and so `is_clean`, `qfree`, region release and rewind, panics with
"undecidable" and asks for the exact engine. Compaction before
measurement and remaps, and release, factor only certified constants, so
they never delete amplitude the exact state has. Dropping the computed
residue of a certified constant is a contraction (the exact state has no
mass there) and is not charged to B; `numerical_projections` counts such
residue drops. A scaled norm still guards against squaring underflow.
Measured wires are reset by collapse and their coordinates reclaimed
without consuming draws; constant measured bits merge into the result in
wire-list order. A randomized test checks thousands of decisions against
the interpreter (no disagreement; about 4% undecidable on circuits that mix
rotation-shaped matrices). Remaining coverage gaps are filed: rotation
inverse pairs need exact operands from `action.rs` (bennett-vm-k8w).

`exact_equals` never accepts a dense state as an exact proof. The dense
reversal decision is `Runtime::equals`: true by identical live operation
history from shared ancestry (sequence numbers are globally unique, so an
actual inverse of exact operands certifies equality although B > 0) or by
two error-free equal vectors. It is false when the distance, rounded
down, exceeds the total or a per-wire-branch error bound. Otherwise it
compares the two exact replays when both histories are replayable, and
fails with a loud "undecidable" panic when they are not. `QuantumState::numerically_equals` in `action.rs`
still compares against B_a + B_b and must delegate to it (bennett-vm-5vj,
test `d1b` ignored on that bead). Snapshot/transaction cloning at VM API
boundaries retains its existing semantics and can copy a state; the
one-vector memory statement applies to an individual state and its kernels
(plus the 1/128-size support bitmap), not arbitrary caller-requested
snapshots or VM transactions.

### Mixed fat objects (bennett-4ht)

The decision machinery assumes nothing about the whole program. It lives
in one `QuantumState` (one device state) and sees only logical wire IDs,
gate operands and remap closures; it decides no entry point, argv/envp,
handle representation or host-memory layout. Oracle remaps that read host
memory by reference stay ordinary `map_basis` closures, and a remap that is
the identity on the state's domain is not logged, whatever memory it read.
Several device states in one host process are independent: the only
shared item is a process-wide monotonic sequence counter, which keeps
history certificates unique across states and threads. A history equality
certificate needs shared ancestry inside one process; a state serialized
across a host/device boundary has no history and falls back to numerical
refutation or a loud fault, never a guess.

One decision here would need a change for a mixed fat object: an
undecidable cleanliness or equality answer is currently a Rust panic,
because `QuantumState::constant_value`/`is_clean`/`numerically_equals`
(src/action.rs) return `Option`/`bool`. Inside a native host binary that
panic would abort the host process. The exact change is to return a
`Result` from those `QuantumState` methods, so the launch ABI reports a
device fault (a §5.6 undecidable diagnostic) to the host. That belongs
with the action.rs plumbing in bennett-vm-5vj and bennett-vm-k8w. Engine
selection through the process-wide `BENNETT_VM_ENGINE` predates this fix.
A fat object would need a per-launch selection in the launch ABI.

## Floating-point bounds

`error_bound()` is a forward 2-norm state-error allowance B. Prepared
input is assumed normalized; `set_amplitude` is an input-preparation
operation and does not certify a different input or renormalize it.
Reference gate matrices are the binary64 entries handed to the engine.
Rounding of an ideal angle or symbolic matrix into those entries is a
separate caller obligation. The runtime charges an additional
32*u*(R+B) for the bounded symbolic-to-binary64 projection used by the
ISA gate alphabet; arbitrary external symbolic matrices require their
own coefficient-error accounting. Proof checking still uses exact
symbolic states rather than this numerical allowance.

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
normalized conditional-state distance is at most 2*B_P/sqrt(p_low), where
B_P = min(B, min over measured wires of the branch bound of the chosen
value) bounds the error of the projected vector (the projection onto the
outcome lies inside each measured wire's chosen branch),
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
checker. The VM verifies supplied formal certificates; it does not
turn an observed state vector into a new formal unitary/channel proof.

Dense action/CC reports and ABI snapshots emit a separate `simulation`
object. Its `kind` is always `numerical`, including permutation-only
runs with B=0; dense observations never emit an exhaustive proof. Fields:

| Field | Meaning |
|---|---|
| `engine`, `kind`, `metric` | `dense`, `numerical`, `state_2_norm` |
| `error_bound` | Current conditional state 2-norm allowance B |
| `measurement_error_bound` | Accumulated measurement-distribution allowance |
| `qubits` | Logical wire-ID extent, including released stable IDs |
| `packed_qubits`, `state_bytes` | Materialized coordinates and their vector allocation |
| `gates` | Gate/remap count, including virtual constant operations |
| `numerical_projections` | Factorings of exactly certified constant wires that dropped a nonzero computed residue (not charged to B) |

Hardware target realization error remains a separate `target_error`.
Simulation metadata does not replace an ISA operator-norm certificate,
channel diamond-norm witness, or their interval checkers. No published
certificate fields or admission semantics were changed. Exact proof
states are constructed explicitly through the default exact constructor,
even while an execution in the same process selects dense.

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

Integration validation on 2026-11-02 adds a dense comparison to every
existing differential-harness invocation. The original byte-identical
reference and JIT comparisons remain unchanged. Dense diagnostics and
discrete outputs must agree exactly. Complex state distance must fit the
reported B plus a separate allowance for the reference's binary64 display;
missing sparse zero entries are handled within that distance bound.
Numerical provenance is required for successful selected execution.

The complete existing coherent fixture corpus passed with 552 dense
command comparisons, including 126 `run-action` calls and their reversal
cases (600-second debug gate, 518.10 seconds). CC oracle-then-gate,
budget/control/fault regressions also passed with the added dense check.
Runtime tests cover wide virtual constants, allocation/cleanup, measured
wire reset, general permutation cycles, scaled support/equality checks,
exact-proof separation and actual CLI/ABI numerical labels. The initial combined release gate found the inverse-circuit draw-zero
bug described above (20/21 general differential tests passed). After the
fix, the identical complete test set passed in the final debug gate. The
release build had taken 12m25 on the shared machine at load ~52; no test
or fixture was omitted or weakened when checking the corrected source.

Final integration gates (2026-11-02 07:52 UTC):

```
timeout 1200 cargo test -j4 --offline --test differential --test dense_kernels --test dense_differential --lib -- --test-threads=4
# 22 library + 12 dense differential/runtime + 26 kernel + 21 general differential tests passed.
BENNETT_VM_ENGINE=dense CARGO_BUILD_JOBS=4 timeout 600 cargo test -j4 --offline --test launch_c -- --test-threads=1
# Both published C11 launch and scheduling consumers passed (93.91 s).
timeout 300 cargo build -j4 --offline
# Whole crate build passed.
```

The 21-test final differential suite completed in 230.94 seconds.
Formatting checks passed for all changed Rust files. The north-star
executor plan entry was read back from the orchestrator's plan and is
present. No integration acceptance work remains; 28-qubit single-thread
fusion tuning is the separate open performance bead `bennett-vm-1x1`.

### Exact-decision fix gates (opus-bjm-fix, beads yt9 and ykb, 2026-11-02)

The review probes are ported as `tests/dense_decisions.rs`. At 701dae3,
6 of these tests failed as expected. The rest are regression guards for
decisions that must stay exact: inverse pairs, Bennett uncompute,
`interference`, bell adjoint reverse, oracle and `qinplace` remaps,
replay through measurement, and a randomized property test. The property
test produced about 23,100 dense decisions identical to the interpreter's,
about 900 loud "undecidable" faults, and no disagreement. d1b, reversal
equality through `action.rs`, is ignored on bennett-vm-5vj. The full gate
first ran at 1ec2f7b. It found three corpus programs that the
certificates alone could not decide (`interference`, `r9-codex-oracle`,
the `r9-sol-qinplace` reverse). Those led to the exact replay fallback,
recorded remaps and the release replay.

```
cargo test --offline -j4 -- --test-threads=2   # 09:03-09:41Z, exit 0
# all suites pass: lib 27, dense_decisions 17 (+1 ignored on 5vj),
# dense_differential 12, dense_kernels 26, differential 21 (dense compared
# on every fixture command), launch_c 2, and the rest of the crate.
cargo build --offline -j4                      # whole crate, passed
```
