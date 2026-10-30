# Published unavailable metadata re-baseline — 2026-10-30

Initial published ISA checkout `0b913da`; final tested checkout `e332108`;
VM starting HEAD `16caaca`. Only published
spec/fixture data and black-box binaries were used across repository boundaries.
No ISA files were changed, no dependency added, no push or Dolt sync performed.

Removed exact unavailable-count pins. Exact per-suite PASS pins and FAIL=0
remain, including all=445; reported totals and unavailable lines must agree.
Every UNAVAILABLE id resolves to its published case.json with a non-empty
unavailable.reason and a bead or staged deliverable (8–11). Duplicate outcomes
fail. The test prints every unavailable id. Exit 2 remains required while any
case is unavailable; the unchanged harness still fails silently accepted probes.
No previously runnable case was demoted.

Observed rows in both interpreter and JIT (PASS / FAIL / UNAVAILABLE / TOTAL):

| Suite | Before (recorded baseline) | After (actual published run) |
|---|---|---|
| A | 21 / 0 / 0 / 21 | 21 / 0 / 1 / 22 |
| B | 35 / 0 / 0 / 35 | 35 / 0 / 0 / 35 |
| C | 339 / 0 / 0 / 339 | 339 / 0 / 1 / 340 |
| D | 7 / 0 / 6 / 13 | 7 / 0 / 6 / 13 |
| E | 0 / 0 / 16 / 16 | 0 / 0 / 20 / 20 |
| F | 33 / 0 / 2 / 35 | 33 / 0 / 4 / 37 |
| G | 10 / 0 / 14 / 24 | 10 / 0 / 14 / 24 |
| H | 0 / 0 / 10 / 10 | 0 / 0 / 10 / 10 |
| all | 445 / 0 / 48 / 493 | 445 / 0 / 56 / 501 |

Initially `0b913da` yielded 445/0/54/499 (E 0/0/18/18). During the gate,
upstream advanced to `5663477`, adding `E-ref-09-copy-native` with a published
reason naming b-isa-spec-8r4 / b-isa-spec-cbk / b-isa-spec-8sc. Both engine
reports accepted the addition without any code/count-pin change and now show
445/0/55/500. A second concurrent addition, `E-ref-09-ripple` at `e332108`,
also names b-isa-spec-8r4 / b-isa-spec-cbk and remains pending 0.9. Final
focused conformance (including scratch corruption) passes in both engines at
445/0/56/501 (`.tmp/rebaseline-final-published.log`). Neither draft catalog
checkpoint is the complete ref library.

The six originally added pending-0.9 ids are `A-measure-draw-order-09`,
`C-multiblock-09`, `E-ref-09-qxor-witness`, `E-topology-grid-09`,
`F-frame-records-09`, `F-ratified-confirmations-09`. Their published reasons
name b-isa-spec-2c9, b-isa-spec-8r4, b-isa-spec-wr7 and the specific ratifications;
no 0.9 executor support is claimed by an ISA 0.8 pass.

RED: the original count-pinned test fails on the published suite
(`.tmp/rebaseline-red.log`). A new scratch-suite regression initially failed
because the consumer accepted an emptied E-topology-grid-09 unavailable.reason
(`.tmp/rebaseline-reason-red.log`). GREEN: the completed consumer check rejects
that whitespace-only reason in a full scratch copy via harness --cases in both
engine modes, while the unchanged published binary itself still exits 2.
The published suite passes in both modes (`.tmp/rebaseline-focused.log`);
the corruption regression remains part of the gate. All scratch copies live
under this repo's .tmp and successful regression runs clean them up.

## Full gate

All pass: fmt --check, cargo clippy --offline --all-targets -- -D warnings,
cargo test --offline in interpreter and BENNETT_VM_ENGINE=jit modes, and
scripts/pre-push. **108 tests pass per full run, zero failures/ignored.**
All commands run headless/offline with project-local TMPDIR. Logs:
.tmp/rebaseline-{fmt,clippy,interpreter,jit,pre-push}.log and the gate exit
summary .tmp/rebaseline-gates.json (all five exits 0). The full gate passed
before the final concurrent ripple addition. Focused conformance was re-run for that published addition: latest raw reports
.tmp/conformance-{interpreter,jit}.log both show 445/0/56/501, exit 2.
The first strict Clippy run flagged nested pattern matching in the scratch
fixture setup; explicit destructuring fixed it before all final gates passed.

## Unavailable ids (56 observed; no total pin)

- `A-measure-draw-order-09`
- `C-multiblock-09`
- `D-channel-diamond-witness`
- `D-channel-nonpositive-rejected`
- `D-eps-scheme-realisation`
- `D-expected-oracle-claim`
- `D-numerical-operator-norm`
- `D-sampled-relation-invocation`
- `E-aggregation`
- `E-ast-division-by-zero`
- `E-ast-eps-out-of-range`
- `E-ast-log2inv`
- `E-ast-unbound-variable`
- `E-epsilon-zero-entry`
- `E-epsilon-zero-exact`
- `E-imports`
- `E-jcs-target-hash`
- `E-matching-first-entry`
- `E-mbu-must-be-false`
- `E-ref-09-copy-native`
- `E-ref-09-qxor-witness`
- `E-ref-09-ripple`
- `E-scheme-correctness-negative`
- `E-scheme-expansion`
- `E-table-index`
- `E-target-staged-probe`
- `E-timing-overlap`
- `E-topology-grid-09`
- `F-frame-live-ranges`
- `F-frame-records-09`
- `F-link-conflict`
- `F-ratified-confirmations-09`
- `G-accessor-lifetimes`
- `G-argument-kinds`
- `G-copied-arguments`
- `G-estimate-and-coverage`
- `G-estimate-resolved-class-decimal`
- `G-invocation-tree-postorder`
- `G-qreg-prepare-unclean`
- `G-register-exclusive`
- `G-result-encodings`
- `G-result-get-small-cap`
- `G-suspend-resume`
- `G-tape-64bit-fields`
- `G-unload-unclosed-run`
- `G-wait-on-suspended`
- `H-capacity-peak`
- `H-defaults-uncovered`
- `H-mbu-realisation-invalid`
- `H-path-not-in-object`
- `H-reproducibility`
- `H-schedule-staged-probe`
- `H-specificity-incomparable`
- `H-timing-violation`
- `H-unknown-expansion-library`
- `H-unsupported`

# JCS binary64 number canonicalisation — 2026-10-30 (bennett-vm-ciz)

Implemented RFC 8785 §3.2.2.3 / ES2019 §7.1.12.1 without dependencies.
All JSON numbers, including integer tokens, now pass through binary64 for
canonical emission. Shortest digits come from std LowerExp; our layout follows
ES steps 6–10: decimal for 1e-6 <= abs(value) < 1e21, otherwise lowercase e
with an explicit positive exponent sign. Both zero signs emit `0`. Parsing
retains original decimal text for exact §8 rational evaluation; canonical-byte
checks reject spellings that change after binary64 round-trip. §9.1's large
exact integers remain decimal strings.

Rust's shortest formatting alone is insufficient: RFC Appendix B's bits
`43143ff3c1cb0959` are exactly 1424953923781206.25, but Rust emits `.3` while
ES Note 2 requires `.2`. The formatter checks adjacent even significands via
exact integer/dyadic midpoint equality (2*v = (s+candidate)*10^q), cancels
powers of two, and compares odd integers using checked powers of five. A
candidate must also round-trip before replacement. No floating distance or
approximate epsilon decides the tie. External malformed/nonfinite numeric
input returns a parse error; attempts to emit a manually constructed nonfinite
number fail explicitly rather than outputting NaN/Infinity.

Sources: [RFC 8785](https://www.rfc-editor.org/rfc/rfc8785.html#section-3.2.2.3),
[ES2019 NumberToString](https://262.ecma-international.org/10.0/#sec-tostring-applied-to-the-number-type).
No sibling implementation source was read or copied.

RED: six new integration tests failed against the original implementation
(`.tmp/jcs-red.log`). GREEN focused: all six pass, covering every requested
boundary, both signs, -0, integer rounding, malformed grammar/leading zeros/+,
nonfinite input/emission, noncanonical E/trailing zeros/exponents, RFC Appendix B
bit-pattern answers, and target acceptance/hash/exact-rational cost checks
(`.tmp/jcs-focused.log`).

Independent ECMAScript check: **132,238 finite binary64 inputs agree with
Node v24.11.1 JSON.stringify, zero differences**. Corpus includes fixed-seed
(8785) random bit patterns, neighbors at every binary exponent in both signs,
and 10,000 decimal halfway constructions in both signs. This exercises both
layout and digit/tie selection, not a comparison with Rust's own formatter.
Report: `.tmp/jcs-ecmascript-comparison.json`.

## Targets witness replay

The exact 1,980-byte grid projection
`../bennett-targets/.tmp/vm-jcs-exponent-witness.json` matches the validator's
`canon` response byte for byte (black-box binary invocation). It contains
`1e-8`, `2e-8` and `2e-7`. Before: VM exit 1, “§8.1: target must be in
RFC 8785 canonical form”. After, in **both interpreter and JIT**:

```sh
/home/tobias/Projects/bennett-workspace/bennett-vm/target/debug/bennett-vm run-program /home/tobias/Projects/bennett-workspace/bennett-isa-spec/conformance/cases/objects/registers.bisa views --target /home/tobias/Projects/bennett-workspace/bennett-targets/.tmp/vm-jcs-exponent-witness.json
```

Exit **0**, status COMPLETED, run_state CLOSED, 7 steps, no fault,
`target_error="0"`. Resources: gate_count 2, peak buffer 2/data 0,
qubit_second buffer **0.00000018**/data 0, time **0.00000011**.
The minimized `docs/verification.json` case
`../bennett-targets/.tmp/verify/canonical-small-exponent.json` likewise gets
past canonical validation in both modes; its cx-only alphabet then correctly
reports “§8.3: unsupported gate x at c=0” for this program. That minimized
validation fixture is not a complete grid execution target. Raw CLI commands,
stdout/stderr and exit codes: `.tmp/jcs-targets-witness.json`; validator response:
`.tmp/jcs-grid-validator.json`. No sibling file was edited.

## Full gate

All green before closing ciz: fmt --check; cargo clippy --offline --all-targets
-- -D warnings; cargo test --offline in interpreter and BENNETT_VM_ENGINE=jit
modes (**107 passed per mode, zero failures/ignored**); scripts/pre-push
(**107 passed**). The published harness remains **445 PASS / 0 FAIL /
48 UNAVAILABLE / 493 total**, unchanged by the JCS fix in either mode.
All commands run headless with TMPDIR under this repository's .tmp. Logs:
`.tmp/jcs-{clippy,interpreter,jit,pre-push}.log`. No dependency was added,
no scope was narrowed, no push or Dolt remote sync occurred. Existing ISA 0.9
47t/hsd/clw/cxg blockers remain; no new maintainer blocker was discovered.

# Published conformance baseline refresh — 2026-10-30 (bennett-vm-w0x)

Inspected `bennett-isa-spec` commit `91aa124` against its parent, restricted
to the published `conformance/` fixtures and documentation. Exactly two cases
were added, no executable case changed. Both explicitly say unavailable pending
ISA 0.9, have no CLI probe, and witness omissions in accepted ISA 0.8:

- `D-sampled-relation-invocation`: §12.1 sampled payload has no expected
  comparison relation; §12.3 has no standalone resolved arguments/budget or
  tuple interchange. Existing **bennett-vm-clw / b-isa-spec-9uy** tracks this.
- `G-estimate-resolved-class-decimal`: §10 estimate_run has no argument field;
  estimate_get's decimal scalar has no per-class encoding (§8.2) or rounding
  for permitted rational costs such as 1/3 (§8.1). Existing
  **bennett-vm-hsd / b-isa-spec-c15** tracks this.

No passable ISA 0.8 case was relabelled, and no new FAIL was found. Existing
47t (unpublished ref expansion) and cxg (eps_decade) blockers remain in scope.
Changed pins (PASS / FAIL / UNAVAILABLE / TOTAL), identical in both engines:

| Suite | Before | After |
|---|---|---|
| D | 7 / 0 / 5 / 12 | 7 / 0 / 6 / 13 |
| G | 10 / 0 / 13 / 23 | 10 / 0 / 14 / 24 |
| all | 445 / 0 / 46 / 491 | 445 / 0 / 48 / 493 |

All other rows stay A 21/0/0/21, B 35/0/0/35, C 339/0/0/339,
E 0/0/16/16, F 33/0/2/35, H 0/0/10/10. The published harness remains
unchanged and exits 2 (unavailable), not an implementation pass for those cases.
RED: `cargo test --offline --test conformance published_conformance_report`
failed on the old pins (`.tmp/baseline-red.log`).

GREEN: fmt --check, offline all-target Clippy with -D warnings, offline cargo
test in interpreter and BENNETT_VM_ENGINE=jit modes (**101 passed, zero failed
or ignored per mode**), and scripts/pre-push (**101 passed**) all succeed.
Runs unset DISPLAY/WAYLAND_DISPLAY/DBUS_SESSION_BUS_ADDRESS/BROWSER and use
repository-local TMPDIR=.tmp. Logs: `.tmp/baseline-{clippy,interpreter,jit,pre-push}.log`;
raw harness `.tmp/conformance-{interpreter,jit}.log` shows 445/0/48/493.

# Certificate and launch-report follow-up — 2026-10-30

Available implementation: **bennett-vm-abx.1**. Parent **bennett-vm-abx** remains
**BLOCKED** on 47t (b-isa-spec-8r4), hsd and clw. This is not a claim that full
scheduled certificates or resource estimates are implemented. No source outside
this repository was edited, no reference implementation source was read/copied/
linked, no dependency was added, no push or Dolt remote synchronization occurred.
Commits `ea640cf` (replay/algebra/coverage), `8925d45` (executor/reference checks),
then the enclosure and final-evidence follow-up.

## Implemented claims and exact obligations

- `certificate::verify_claim` independently replays exhaustive table/program
  comparisons. Full ABI tuples include inputs, arbitrary initial targets/faults,
  dirty bundles and every interface cell; zero workspace and exact phase are
  checked, not merely displayed binary64 basis values. Oracle replay uses the
  literal bounded machine, commits XOR payload/code, then reverses all B frames.
  §12.1: “re-execute every domain element … compare full tuples”.
- Only exhaustive D gives `Proof`; partial domains and sampled replay give
  `Evidence`, with exact integer counts/fractions. xoshiro256** uses §12.1's
  rejection sampling, least-significant word first, declared component order,
  inclusive bounds and zero words for singleton ranges. Sampled coverage counts
  distinct sampled elements; sampling never becomes proof even if it covers D.
  §12.2: “A proof label requires the domain to equal D”.
- Structural translation witnesses name actual §9.2 point tuples. The checker
  resolves the complete frame, V/W lists, reverse V/repeated W, dirty wires,
  effective predicates and every §5.6 side condition. The existing independent
  per-lifetime full-domain fallback remains; no later cancellation can repair an
  earlier bad borrow frame. §5.6: “P_F = P′ ⊗ I_B”.
- `adjoint_table` checks the admitted §5.3 instructions and already-validated
  type/bound/lifetime/control conditions. It proves the adjoint transformation,
  not an expected oracle permutation. `inverse_pair_library` resolves actual
  closure hashes and replays the library claim, with cycle detection; no producer
  proof label is trusted. Verification indices, subclaim cycles and obligation
  kinds are checked: an adjoint proof cannot discharge `verification.oracle`,
  and evidence cannot discharge a proof obligation.
- `inverse_pair` and `borrow_frames` are §12.3 verification references, **not**
  invented claim kinds. Before qinplace allocates workspace, independently
  enumerate both oracle compositions and require FAULT_NONE on every input under
  tk. §4.5: “g∘ginv = ginv∘g = id”, fault-free “on every input”.
- Embedded certificates are no longer ignored at Q/oracle execution: bindings
  and claims are checked before mutation. Executable numerical/channel and
  expected-oracle claims require an actual matching root schedule hash and
  refuse unpublished ref@0.8, naming **b-isa-spec-8r4 / bennett-vm-47t**.
  §12.1: “the claim holds iff it holds for every context”; §11: a consumer not
  implementing the bound library rejects E_SCHEDULE_INFEASIBLE. No template
  fixture matrix or unscheduled simulator run is substituted for a realisation.

## Rigorous algebra, distinct from executable-claim acceptance

`interval` implements exact-rational endpoints rounded outward to the requested
significant binary precision after interval operations. It computes V†U−I and
compares the Frobenius upper bound (squared, equivalently, against bound²),
preserving global phase. The channel dual uses unnormalised input-first Choi
ordering, partial trace over output, and complex Hermitian LDL†; every pivot's
lower bound must be strictly positive, including the mu-I partial-trace check.
Zero/inconclusive pivots reject; diag(0,−1) rejects, as §12.1 explicitly requires.

π is enclosed using Machin's formula and an alternating-series tail bound;
argument reduction uses that outward enclosure and an integral turn. Taylor
sin/cos stops at the first omitted-term Lagrange bound below 2^−(p+2), per §17(8).
rx/ry/rz matrices use the enclosed θ/2 values. Tests derive exact zero matrices,
the Y=I4/16, mu=3/16 dual, zero pivots, complex pivots, four-bit rounding and
independent rational sin/cos brackets. These helpers accept **checker-built**
matrices as Rust inputs; they are not a certificate interchange accepting producer
matrices. Building realised/reference matrices at **all scheduled contexts** and
multiplicity composition remain blocked on the expansion library. No numerical
or channel executable certificate is currently accepted.

## Launch estimate and coverage behavior

`bennett_coverage_get/jcs/free` returns an owned, canonical copy of the object's
§13 report. Entry classifications/schema/counts and actual §9.2 point identities
are validated; data symbols and nonexistent blocks/ordinals reject. Accessor
bytes survive program unload until coverage_free (§10 ownership sentence).
Missing source coverage explicitly names **bennett-driver**, instead of inventing
an empty/lowered report. Empty producer coverage is supported and tested.

Estimate request checks program, region, program-scoped token and matching
region, then target. Accessor/null ownership behavior is implemented. **No
successful estimate or estimate certificate is issued** while required request
contracts are absent. The error names **bennett-estimate**; padded oracle paths
also name **b-isa-spec-8r4**. No T-count/depth, summed class peak, timing or other
quantity absent from the target is fabricated. Existing §8 exact target-table /
leaf-event accounting remains separately tested, not mislabeled an estimate.

New gaps filed here with label **isa-0.9**:

- **bennett-vm-hsd:** §10 declares estimate_run(program, region, token, out),
  without classical arguments; estimate_get exposes name/value_decimal while
  §8.2 requires occupancy/peak **per class**. No class encoding or rounding of
  a permitted rational such as 1/3 is specified. Needs argument/default binding,
  resource-vector encoding and decimal precision/rounding in the contract.
- **bennett-vm-clw:** §12.1's sampled payload lists domain/sampler but no expected
  relation/default; non-scheduled claim argument/budget binding and table tuple
  encoding are also absent. The independent Rust replay adapter requires an
  explicit comparison and resolved args/budget, rather than claiming a new
  canonical interchange. Missing sampled comparison explicitly names clw and
  bennett-driver. Scheduled requests additionally require ref@0.8.

NorthStar §5 certificates and @cost, and §8 padded SQLite/kernel estimates, remain
in scope. These are blockers to mechanics, not reductions of the north star.

## Verification and corpus

**101 Rust tests, zero failures/ignored**, headless/offline in interpreter and
JIT-selected modes. Includes 10 certificate, five interval, four report tests,
the added differential group and all-ten-H refusal group. Retains **160 launch C
checks**, all **15 scheduler exports**, and **21,888 row inverse checks**.
Formatting, strict all-target Clippy and diff checks pass. Final logs:
`.tmp/cert-release-interpreter.log`, `.tmp/cert-release-jit.log`; focused final
obligations in `.tmp/cert-focused-{interpreter,jit}.log` and enclosure logs.

**1,519 / 1,519 reference comparisons agree per mode, zero divergences**:
1,493 JSON and 26 diagnostics; **77 distinct object byte images**, including all
12 fixtures and both independently extracted spec examples. Every request also
compares four individual passes, composition and JIT against the interpreter:
**9,114** transformation comparisons. **1,002** JIT invocations on **70** images
actually enter native segments (1,002 agree, zero diverge); the remaining 517
are identity/debug/pure/pre-execution paths. Existing opaque memory fingerprint
exclusion and phase-sensitive binary64 display tolerance remain unchanged.

| Command | Comparisons |
|---|---:|
| check | 72 |
| hash | 16 |
| run-oracle | 383 |
| run-cc | 120 |
| reverse | 415 |
| run-action | 357 |
| step-check | 64 |
| run-program | 92 |

The new 284 requests are 24 inverse-pair/budget oracle calls, four in-place
permutations and every one of 256 input/target/fault tuples for oracle_tuple.
The reference serves these requests through existing CLI commands; it serves
**no certificate verdict, estimate or coverage request CLI**, so no invented
command/differential verdict is counted. Direct API tests carry those obligations.

Published harness runs unchanged, in both engine modes, exit **2**:

| Suite | Pass | Fail | Unavailable | Total |
|---|---:|---:|---:|---:|
| A | 21 | 0 | 0 | 21 |
| B | 35 | 0 | 0 | 35 |
| C | 339 | 0 | 0 | 339 |
| D | 7 | 0 | 5 | 12 |
| E | 0 | 0 | 16 | 16 |
| F | 33 | 0 | 2 | 35 |
| G | 10 | 0 | 13 | 23 |
| H | 0 | 0 | 10 | 10 |
| **all** | **445** | **0** | **46** | **491** |

Raw reports: `.tmp/conformance-interpreter.log`, `.tmp/conformance-jit.log`.
The read-only runner returns UNAVAILABLE for no-command scenarios and retains
that classification even when a probe's precise refusal succeeds. Direct H tests
retain all ten cases; available selectors/defaults/paths/capacity/timing are
covered by existing independent witnesses, and unsupported requests/schedules
fail with §11 and E_SCHEDULE_INFEASIBLE. They are not ten promoted H passes.
D scenario JSON are templates (missing bound subject hashes, and a symbolic Y
string), not executable claims to accept verbatim. Local matrix witnesses check
their mathematical derivations; actual schedule-dependent claims fail loudly.

**Remaining reference divergences: none.** New blockers above cite the deciding
sentences rather than changing expected values. Initial red logs:
`.tmp/cert-red.log`, `.tmp/reports-red.log`, `.tmp/oracle-cert-red.log`,
`.tmp/cert-references-red.log`, `.tmp/adjoint-red.log`, `.tmp/transcendental-red.log`.
The interval module-export red was reproduced after the first algebra draft;
that is not claimed as a pristine pre-implementation failure. A new differential
initially used comma-splitting --args for a token and both binaries refused;
the harness correctly rejected agreement-on-refusal, and --arg now reaches
actual execution. No published fixture, case or expected value was edited.

Observed ISA checkout HEAD `738b233d17f4d19351fde6fcb6e454f900d5b450`.
Separate observed executable SHA-256 fingerprints (external binaries are never
claimed rebuilt from checkout HEAD): reference
`17ec0463391db3503ad61e334d71066a6457d59bec80becb2d2fd002b9651e90`;
conformance `267079641cfae28432a82a0deddf2b03b2f09372fd3215ff1aa3704464e41189`;
VM `f0e6320f2cd7fdd6b36567938f5372c2e5dcbaa13487208d0c1443fa4d066427`.

# Milestone 5 JIT verified — 2026-10-30

Milestone `bennett-vm-npe`: ISA **0.8** §5.7 private executor passes and
native codegen for the current **x86-64 Linux** host. All previous North-Star
capabilities remain in scope. M4's 47t/cxg blockers are unchanged; this JIT does
not invent a target expansion library or implement scheduler/pebbling policy.
No other repository edited, no reference implementation source read/copied/linked,
no dependencies added, no push or Dolt remote synchronization.

## Passes and their differential obligations

- **Inlining:** finite one-level call-site clones of CC/R/lproc bodies, including
  recursive callees. Calls inside clones continue by ordinary dispatch, not
  recursive code expansion. Logical call/return rows and their runtime helpers
  retain §3.7 depth/monotonic-stack limits, argument capture, activation effects,
  §7.4 dynamic instances and uncall eligibility. A cloned return is linked to
  its call-site continuation only if the runtime successor matches it.
- **Structure recovery:** recover straight-line continuations, both explicit
  branch edges and cyclic CFG continuations as guarded direct-threaded regions.
  Generated forward/back edges bypass generic dispatch. Indirect jumps, dynamic
  returns and other unmatched successors use the canonical dispatch map. No
  assumption of reducibility, input values or a static trip count is needed;
  the runtime row helper still enforces each reference-row budget boundary.
- **Dispatch re-indexing:** reverse private node layout and relocate every edge.
  Dispatch retains an independent canonical-source-to-private-node map. Neither
  the object nor §3.6 label/index computation changes: §3.6 requires “every label
  value observable through `lift.label` keeps its canonical numeric value”.
- **Simplification:** block-local constant propagation/folding of the nine
  wrapping integer operations, with exact widths and kills for every destination
  and call boundary. Dataflow resets at every block entry; parallel moves read
  old sources and invalidate written facts. Folded rows remain individual rows,
  with original source op/operands/line, step charging and history frames.

Deciding §5.7 sentence: “Inlining, structure recovery, dispatch re-indexing and
simplification MUST produce on every x ∈ D the same final interface tuple as
§5.2's construction on the reference body (T6).” §14.1: “the step counter counts
executed rows”. The private graph never removes a logical row or call boundary.
For unchanged instructions the proof obligation reduces to guarded continuation
relocation and identical runtime helpers; folded values follow block-local
constant facts. Differential/exhaustive tests below check those obligations on
the recorded domains; they are not a mechanized universal proof for all objects.

## Host codegen and execution boundaries

`src/jit.rs` emits native System V x86-64 direct-threaded row code, calls the
runtime row helper with its compiled source row, tests the canonical successor
and branches to private continuations or dispatch. CC/R CLI runs and sequential
oracle F_B evaluations execute whole segments through this code. Stateful C ABI
single stepping enters the compiled row and yields at the ordinary instruction
boundary, preserving HOST_REQUEST, suspension, transactional failures and saved
snapshots. The nine 1–64-bit integer kernels emit actual host arithmetic/shifts;
**7,434** independent exhaustive-small-width and boundary pairs compare them
with interpreter arithmetic, including shift >= width and negative ashr.
Arbitrary-width values (including 257-bit fixtures), division, comparisons,
memory, effects, Q simulation and history use the VM's runtime helpers. These
are deliberate generated calls, not an automatic failed-compilation fallback.
Other hosts and unknown engine names fail explicitly as milestone 5.

Mappings are RW while emitting, then immutable RX, never RWX; callee-saved
registers and stack alignment follow the host ABI. Errors and caught panics
return through generated frames before propagation. Code is shared immutably;
each invocation owns its callback context. The bounded per-thread cache keys
all parsed object content, including programmatically extended functions.
Object hashes, reference bodies and stored identities are untouched.

Select `BENNETT_VM_ENGINE=interpreter|inline|structure|reindex|simplify|optimised|jit`.
Interpreter remains default; each individual pass and their composition can be
run without codegen. `BENNETT_JIT_AUDIT=1` writes native-entry, callback and kernel
counts to stderr. Literal §3.6 diagnostic forward/inverse stepping and pure Q
gates retain their independent runtime implementations; JIT mode compiles their
CC/oracle segments. Identity/validation and pure/debug-only commands are not
claimed to execute a native segment. Kernel counts can include compile-time
constant folding; callbacks can include a budget-boundary check, so neither is
reported as a reference-row count. No performance claim is made by these gates.

## Red-green evidence and final gates

Initial red: absent JIT module (`.tmp/m5-red.log`); then absent native integer
kernels (`.tmp/m5-arithmetic-red.log`). Green: seven focused JIT integration
checks, covering graph changes, unchanged hashes, folding with retained source
boundaries, call/block fact barriers, native errors/panics/invalid dispatch,
integer kernels and explicit unknown-engine rejection. The initial inlining
witness exposed an unnecessary leaf-only restriction; finite one-level cloning
now also exercises the recursive sum fixture without unrolling recursion.

The common invocation in `tests/differential.rs` runs every original case in
interpreter mode, four individual pass modes, combined-pass mode, and JIT mode.
Each transformed mode must match interpreter stdout, exit status and diagnostic
text exactly (only the audit line is excluded). The original comparison against
the CLI reference oracle remains, with its established opaque-memory-digest
exception and phase-sensitive display tolerance. No reference expected value or
published fixture was changed. Successful CC/oracle/launch/step-check comparisons
also require evidence that native entry actually ran.

**80 Rust tests pass, zero failed/ignored**; **160 launch C checks**, all **15
scheduler exports**, and **21,888 one-row inverse checks** retained. Full test
suites are run headlessly/offline in interpreter-default and JIT-selected modes.
Formatting, strict all-target Clippy and `git diff --check` pass.
Logs: `.tmp/m5-final-tests.log`, `.tmp/m5-final-all-jit.log`,
`.tmp/m5-final-clippy.log`; machine-readable counts `.tmp/m5-counts.json`.

**1,235 / 1,235 comparisons agree per mode, zero diverge**, across **75 distinct
object byte images**, including all 12 fixtures and both spec examples.
1,209 successful JSON comparisons and 26 diagnostics. Six modes give **7,410**
transformation/interpreter obligations, plus **1,235** reference comparisons.
Of the JIT-selected invocations, **718** on **68** object images actually enter
native segments (**718 agree / 0 diverge**); the other 517 compare identity,
validation, pure actions, literal debug stepping or pre-execution diagnostics.
Audit totals in this corpus: 2,255 native entries, 10,632 callbacks and 4,698
integer kernel calls, with the counting limits described above.

| Command | Comparisons |
|---|---:|
| check | 72 |
| hash | 16 |
| run-oracle | 359 |
| run-cc | 120 |
| reverse | 415 |
| run-action | 97 |
| step-check | 64 |
| run-program | 92 |

New reference witnesses exercise wrapping folds, canonical label observations,
indirect-jump faults, a branch diamond, loop joins, simultaneous parallel moves,
all listed step-budget cutoffs and every prefix of folded CC history. Five new
oracle bodies also run exhaustive 8-bit step-check domains. Existing recursive
activation, heap/stack/depth failures, memory, nested oracle XORs, arbitrary
initial target/fault registers, predication, exact phase/borrow and device tests
remain in the same unfiltered harness.

Published conformance is invoked unchanged for **both interpreter and JIT modes**:

| Suite | Pass | Fail | Unavailable | Total |
|---|---:|---:|---:|---:|
| A | 21 | 0 | 0 | 21 |
| B | 35 | 0 | 0 | 35 |
| C | 339 | 0 | 0 | 339 |
| D | 7 | 0 | 5 | 12 |
| E | 0 | 0 | 16 | 16 |
| F | 33 | 0 | 2 | 35 |
| G | 10 | 0 | 13 | 23 |
| H | 0 | 0 | 10 | 10 |
| **all** | **445** | **0** | **46** | **491** |

Both exit **2**, retaining every unavailable classification. Raw logs:
`.tmp/conformance-interpreter.log`, `.tmp/conformance-jit.log`.
**Every remaining divergence: none.** No new ISA gap was found. Existing gaps
remain filed with label isa-0.9 and their deciding sentences in the M4 evidence.

Observed ISA checkout: `92c26af045b816f5ef1e283a7b734aa244d8f320`.
Executable SHA-256 fingerprints (external binaries are observed, not rebuilt):
VM `1f01e2c04901a5bc89a51bb7bb0ba478c1f580b66101760d1dfa4e5a865bfbfe`;
reference `ceec144ccb47c05ed9c8f2d9372f2bfa690736a73b423c4f7b8727092667f744`;
conformance `267079641cfae28432a82a0deddf2b03b2f09372fd3215ff1aa3704464e41189`.

## Remaining scope

M5 as PLAN defines it is implemented. M4 remains blocked on the unpublished §11
reference expansion library (`47t`) and undefined `eps_decade` indexing (`cxg`):
full lifted/oracle target decomposition, non-native QRAM comparator expansion,
verbatim scheduled execution, scheduled accounting and reproducibility retain
explicit `unimplemented: <item> (milestone 4)` errors. Estimates, rigorous
certificate and coverage APIs retain their explicit milestone-5 errors; they
are not a claim delivered by native codegen. No North-Star capability is removed.

# Historical milestone-4 evidence

# Milestone 4 independent target work verified — 2026-10-30

Child `bennett-vm-v23.4` completed. Parent `bennett-vm-v23` remains BLOCKED
on `47t` (unpublished §11 reference expansion library) and `cxg` (undefined
`eps_decade` indexing). No private library, hash, scratch identities or ordinals
were invented. This section supersedes the historical verification below.

## Delivered and deciding contract sentences

- Explicit gates, bundle copy/XOR/exchange, qif toggles, bounded loops, while
  flags and local-image setup/cleanup now execute real target native leaves.
  §4.5: “`cx s[i] d[i]` under the predicate”; §4.6 lists qif's toggles and
  interleaves forward while bodies with flag computation. §5.3 explicitly
  specifies the adjoint's different order: “flags computed forward ... adj A
  for i = n..1 under c_i; flags uncomputed”. The initial inherited adjoint
  expected order contradicted that sentence; the implementation was retained.
- §4.6: a local region is “created at function entry (unconditional allocation,
  X gates from the data image ...)”, then “image gates reversed, cells released
  clean”. All its cells are allocated before image gates and remain live until
  all reversed image gates finish; cleanup order is deterministic. An explicit
  allocation's declared class also survives the §5.3 qalloc/qfree adjoint swap.
- Native `qram` load/store/exchange evaluate costs with exactly `a,w,c`, execute
  the §4.6 selected-cell permutation coherently and record one event per native
  access. **768** independent basis cases cover all cell contents, targets,
  addresses and inherited predicate values for all three access kinds. A separate
  address-superposition witness preserves relative phase. When `qram` is null,
  comparator decomposition still names 47t and milestone 4, before action entry;
  direct Rust calls cannot silently bypass this target guard.
- Designated dirty scheme scratch and qborrow pools preserve caller wire IDs,
  enforce class/uniqueness/liveness/interface disjointness and consume no extra
  physical capacity. §9.4: “callees `qborrow` only from the caller's designated
  idle set”. Entangled scratch is tested against an external reference. §8.2:
  “borrowed wires to the borrower at `dirty_rate`, replacing the owner's charge”.
  Explicit borrow frames and dirty schemes both replace ownership charges.
- Resolved `GateRequest` batches select per-logical-gate realisations and exact
  rational epsilon shares in the known structured expansion order; they are a
  diagnostic Rust API, **not** §11 schedules or library ordinals. Missing/unused
  requests fail transactionally. §8.1: “the error charged ... is the entry's
  evaluated `epsilon`”; two requested shares 0.01/0 with achieved 0.001/0 charge
  0.001. §4.4: “Oracles and permutation actions require ε = 0 for every gate”;
  permutation actions reject nonzero shares, including nested permutation calls.
  Ordinary unscheduled target calls retain exact shares. Scheme correctness
  remains the target author's obligation, not an automatic proof claim (§8.3).
- Integrated actual accounting records native leaves, classical rows and effects
  once, includes inverse execution and slices wire ownership over their durations.
  §8.2: “Classical instructions and effects take `classical_step_time`”; an
  eight-row device entry costs 8 forward / 16 forward+reverse. Seven classical
  rows plus eight H/S scheme leaves cost 15, with occupancy 26 and peak 2.
  Native-image example: 22 X gates + two 10-time loads + eight 2-time CXs = 58;
  occupancy = 25*58 + 8*36 = 1738, peak 33. §8.4 peak includes allocations even
  when every event has zero duration. Template gates obey §3.3 disjointness and
  effective predicate depth is checked against target capabilities.
- Target-backed CLI output and immutable launch snapshots expose
  `target_resources` and `target_error`. These are actual accumulated simulator
  ledger values at the observed boundary; stateful execution carries the program
  ledger between runs, not a per-run estimate or certificate. Untargeted output
  is unchanged. The full §10 estimate/certificate/coverage API still explicitly
  names milestone 5. A Rust consumer of the exported C ABI checks snapshots,
  reversal accounting and saved-pointer immutability in addition to the existing
  C11 consumers. Target pointer support now follows the object's parameter range
  instead of treating §8.3's illustrative `[32,64]` array as an enum; the published
  16-bit classical fixture exercises this through real target execution.

## Final gates and corpus

Headless/offline, std-only, sole implementer; display variables unset and TMPDIR
under `.tmp`. **72 Rust tests pass, zero failed/ignored** (16 new target-execution
integration groups), **160 launch C checks**, all **15 scheduler exports** and the
existing **21,888 one-row inverse checks**. `cargo fmt --check`, strict all-target
Clippy and `git diff --check` pass. Raw test log `.tmp/resume-tests.log`.

**1,076 / 1,076 CLI differential comparisons agree, zero divergence:** 1,050 JSON
and 26 diagnostics. Counts and comparison limits remain those of the prior
milestone-4 gate below (including the opaque memory-fingerprint exception).
Target-only fields are separately hand-derived and do not alter untargeted
reference comparisons. No reference implementation code was read/copied/linked.

The other agent's published fixture/probe repairs were observed read-only.
`tests/targets.rs` now consumes the corrected canonical hash vector, negative
zero-epsilon case and separate valid `E-epsilon-zero-exact`; it still independently
mutates a trailing LF and forbidden eps reference. The report test gates every
suite in the unchanged 491-case harness, with zero allowed failures. No ISA repo
file or published expected value was edited by this implementer.

Final standalone conformance (exit **2**, unavailable cases remain unavailable):

| Suite | Pass | Fail | Unavailable | Total |
|---|---:|---:|---:|---:|
| A | 21 | 0 | 0 | 21 |
| B | 35 | 0 | 0 | 35 |
| C | 339 | 0 | 0 | 339 |
| D | 7 | 0 | 5 | 12 |
| E | 0 | 0 | 16 | 16 |
| F | 33 | 0 | 2 | 35 |
| G | 10 | 0 | 13 | 23 |
| H | 0 | 0 | 10 | 10 |
| **all** | **445** | **0** | **46** | **491** |

Raw standalone verdict: `.tmp/resume-conformance-final.log`; the integration
invocation separately writes `.tmp/conformance.log`. E/H are still hard-coded
UNAVAILABLE by the published runner, including successfully refused semantic
probes; local native-target tests do not promote those results.

Observed ISA checkout HEAD `92c26af045b816f5ef1e283a7b734aa244d8f320`.
Observed executable SHA-256 (separate fingerprints, no claim about source builds):
VM `49c9b7b432e8122134c5383ffd2af98d75272b1a4061b0a0a2f343445c41828e`;
reference `679de84f8c016ac2626a2eca2c262129d204ae9f7723cec00a92cead0f4beff5`;
conformance `267079641cfae28432a82a0deddf2b03b2f09372fd3215ff1aa3704464e41189`.

## What remains blocked, without reducing scope

§11: the expansion document must supply row decompositions, generated ordinals,
scratch identities and the removable subset; it is unpublished until accepted
ISA 0.9 (`47t`). Lifted-row/dispatch arithmetic, comparators, multiplexers,
`inrange` scans, oracle compute/commit/cleanup lowering, qinplace's oracle calls,
and non-native quantum-memory comparator expansion therefore remain explicit
`unimplemented: <item> (milestone 4)` paths. Verbatim schedules, schedule-derived
accuracy defaults/shares, start times, integrated **scheduled** accounting and
reproducibility also wait on that library. `cxg` still blocks eps_decade tables.
No successful schedule or certificate is claimed. Child v23.4 closes; parent v23
stays BLOCKED. Finish milestone 4 after publication, then **milestone 5 JIT**.
No push or Dolt remote synchronization.

# Historical milestone-4 baseline

# Milestone 4 partial implementation — 2026-10-29

Milestone `bennett-vm-v23` is **not complete**. Gate-only target launch execution,
exact target evaluation and accounting, schedule validation utilities and all
15 published scheduler C exports are implemented. The mandatory `ref` expansion
document is absent (47t), so no schedule is accepted or emitted; default requests
return UNSUPPORTED with the exact library/version/reason. Full oracle/structured
row lowering, designated dirty scheme pools, approximate scheduled realisations,
and schedule-driven execution/accounting remain milestone 4. Branch enumeration
and all four reversible devices remain implemented from milestones 1–3.

Target tests cite §8; scheduler tests cite §11; no foreign crate code is consumed.
Target input is exactly the §8 JSON document, canonical bytes required. Gate/scheme
costs use arbitrary precision exact rationals; no binary64 cost arithmetic.
Classes resolve in UTF-16 JCS order. Expanded native leaves are actually simulated;
schemes have no own cost. The target author owes template correctness (§8.3).
Class metadata is retained/restored with classical history, separate from Q state.
Static known-zero divisors are rejected while loading; context-dependent errors
are rejected when lowering. Event accounting is an explicit library operation on
leaf events and live intervals; it is **not** an end-to-end resource estimate.

The published harness cannot promote staged scenarios: its runner returns
UNAVAILABLE unconditionally when `case.unavailable` exists. Its raw current verdict
is **445 pass / 2 fail / 43 unavailable** (exit 1). The two failures are obsolete
staging-prose probes, not changed semantic expected values: E requires the string
"staged deliverable 9" instead of the actual §8.1 noncanonical-input error; H
requires "staged deliverable 10" instead of the actual §11 unavailable-ref-library
error. The integration test gates all 445 available cases and asserts the exact
identity/reason of those two remaining probe failures, preserving the raw report.

ISA gaps: `47t` missing ref expansion; `cxg` undefined eps_decade table indexing;
`clh` contradictory E fixture data (epsilon-zero file has no forbidden reference;
JCS hash file has a trailing newline, and hashes the newline). No conformance
fixture/expected value was changed. All remaining staged APIs name milestone 5
for rigorous certificates/estimates/coverage; milestone 4 target paths name their
specific expansion/pool/index item. Full verification counts follow below.


## Final gates and counts

Headless `cargo test --offline`: **56 passed, 0 failed, 0 ignored**. This includes
6 unit tests, 10 coherent groups, 2 raw-conformance/metadata groups, 2 device
regressions, 16 differential groups, 2 linked C11 consumers, 7 schedule groups,
10 target groups and 1 eight-case validation group. The launch C consumer still
passes **160 checks**; the scheduler consumer calls all **15** published exports.
Strict all-target Clippy, `cargo fmt --check` and `git diff --check` pass.
Raw test output: `.tmp/m4-final-tests.log`; raw conformance: `.tmp/conformance.log`.

**1,076 differential comparisons: 1,076 agree / 0 diverge** (1,050 successful
JSON comparisons, 26 diagnostics). Added all 7 runnable D and all 10 runnable G
CLI cases, plus five block-image boundary comparisons. All existing fixture,
phase-sensitive amplitude and opaque memory-fingerprint comparison limits from
milestone 3 remain unchanged. The expected-invalid G budget case must compare
diagnostics rather than require a successful coherent execution.

| Command | JSON | Diagnostic |
|---|---:|---:|
| check | 66 | 0 |
| hash | 12 | 4 |
| run-oracle | 218 | 1 |
| run-cc | 116 | 3 |
| reverse | 406 | 3 |
| run-action | 97 | 0 |
| step-check | 59 | 0 |
| run-program | 76 | 15 |

Unchanged read-only published conformance (standalone exit **1**):

| Suite | Pass | Fail | Unavailable | Total |
|---|---:|---:|---:|---:|
| A | 21 | 0 | 0 | 21 |
| B | 35 | 0 | 0 | 35 |
| C | 339 | 0 | 0 | 339 |
| D | 7 | 0 | 5 | 12 |
| E | 0 | 1 | 14 | 15 |
| F | 33 | 0 | 2 | 35 |
| G | 10 | 0 | 13 | 23 |
| H | 0 | 1 | 9 | 10 |
| **all** | **445** | **2** | **43** | **490** |

These 43 unavailable results are not claimed promoted by local unit tests. D's
five rigorous-certificate cases and F's linking/frame cases remain unavailable;
G's library scenarios are checked by the C consumer except estimates/coverage,
which explicitly fail as milestone 5. E/H direct witnesses exercise the supported
AST, hashes, matching, leaves, accounting, selectors, defaults, timing, paths,
bindings and unsupported library responses. Successful emitted schedules and
their reproducibility are **not** tested or claimed while 47t is unresolved.

## Every gap/divergence and its deciding text

- **47t, blocking:** §11: “bennett-isa-spec publishes the reference library `ref`
  (§17 item 7).” The same paragraph requires ordinals, scratch identities and
  the removable subset in a versioned document; no such document/hash is
  published. §11: “A consumer that does not implement the bound library rejects
  the schedule (`E_SCHEDULE_INFEASIBLE`).” Default bindings also reject (ref@0.8).
- **cxg, blocking:** §8.1 includes `"index": "k" | "eps_decade"` but defines no
  `eps_decade` rounding/offset/domain rule. It fails explicitly as milestone 4.
- **clh, fixture contradictions:** §8.1: “A target is a JSON document in RFC 8785
  canonical form”; and “an entry with `epsilon` 0 MUST NOT reference `eps` or
  `log2inv_eps`.” The hash fixture includes a newline; the epsilon-zero fixture
  has no forbidden reference. Tests reject the former and accept the latter;
  a separately mutated forbidden-reference witness rejects. The old E launch
  probe FAILs on changed diagnostic prose, as documented above. Nothing is
  changed to match an erroneous staged expected value.
- **47i, clarification:** §7.3: “sector map from the `block_image` (else empty),
  512-byte sectors; absent sectors read as zero.” The final partial-sector
  encoding is not explicit. The VM follows the reference's strict full-sector
  initialization rule. Red exposed a short read leaving byte 1 at 0xff; green
  rejects incomplete images before execution and tests full-sector reads and
  reversal. An initial attempt to reject during `check` differed from the
  reference; rejection now belongs to device initialization, so `check` and
  execution diagnostics both agree. No permanent CLI divergence remains.
- **H probe FAIL:** §11 requires explicit unavailable-library rejection, while
  the published probe expects the old literal “staged deliverable 10”. The
  actual refusal names `ref@0.8`, E_SCHEDULE_INFEASIBLE and milestone 4.

Completed beads: `v23.1` target-backed gate execution/class capacity, `v23.2`
scheduler validation/request-response ABI, `v23.3` block-image hardening.
Parent **v23 is BLOCKED**, depending on 47t and cxg; it is not closed.
Next: finish **milestone 4**, including full row lowering, target dirty pools,
approximate realisations, schedule execution and integrated resource accounting,
then milestone 5 JIT. Estimates/rigorous certificates/coverage still name 5.

Observed final test executable hashes (not claims about external source builds):
VM `4d249369634f42115bd51539ebef962d9464ce0f93e5c3c9dd3842406441d872`;
reference `679de84f8c016ac2626a2eca2c262129d204ae9f7723cec00a92cead0f4beff5`;
conformance `1bd5aab7f20690c7959edfd217d5380cfd8ea4a4f7f7bc17bea4b86561e2bb92`.
No other repository changed; no push or Dolt remote sync; std-only throughout.

# Milestone 3 verification — 2026-10-29

Accepted ISA/object/launch **0.8**, std-only, sole implementer. No reference
interpreter source copied, linked or read; its executable is a CLI oracle.
No other repository edited, no push, no Dolt remote sync.

## Delivered and verified

- `step`: independent literal §3.6 basis machine, canonical index space,
  pc/pc-prime exchanges, fault/flag/successor uncomputation, derived counter,
  cell and fixed frame-slot widths; sparse cells, allocation entries and
  monotonic activation records. Assignments retain displaced values, not
  snapshots. §3.7 call capture and return delivery have explicit inverse
  sequences. Nested oracle XORs are involutions; permutation calls reverse
  through the callee's actual adjoint.
- `step-check`: exhaustive inputs plus every unfixed interface cell, or seeded
  xoshiro256** samples; sequential/literal F_B comparison and complete B-step
  reversal. Exhaustive width is a user guard, not a fixed ISA restriction.
  Oracle `reverse` implements every prefix k <= B, including B=0 and HALT padding.
- `exact`: finite symbolic sums of dyadic coefficients times sqrt(2), i and
  exp(i * dyadic angle). Binary64 input amplitudes and angles enter as their
  exact dyadic values. Gates, controlled gates and branch permutations retain
  phase. Support, cleanliness and action reversal equality use exact algebra;
  the binary64 cache serves display and Born probabilities only. No epsilon
  pruning, global-phase alignment or saved-wavefunction reversal.
- Standalone `run-action`, action `reverse`, basis/mut/in/memory/state inputs,
  explicit adjoints, classical arguments, structured controls/loops and local
  images share the device executor. CLI memory uses one little-endian value
  per cell, including sub-byte-width cells. Oracle calls enumerate basis
  branches and preserve the amplitudes and arbitrary target/fault entries.
- Borrow-frame checks precede hashes: translation lemma or an independent
  exhaustive full-domain permutation proof. Frames are checked before
  non-permutation composition; list controls and active predicates participate.
  The library takes an explicit designated idle pool, including dirty and
  externally entangled wires. Standalone CLI provisions its documented clean
  pool and checks it on return. CC/launch 0.8 has no designation operand;
  those calls still require a caller-designated set rather than treating
  arbitrary live handles as idle (§9.4).

## Gates and exact counts

`cargo test --offline`: **34 passed, 0 failed, 0 ignored**: 6 unit tests,
10 coherent integration tests, 14 differential groups, 2 conformance tests,
1 linked C11 test and 1 eight-case validation regression. The C consumer passes
**160 checks**, including all 41 normative exports and six VM extensions.
`cargo fmt --check`, strict all-target Clippy and `git diff --check` pass.

**1,054 differential comparisons: 1,054 agree / 0 diverge**, including
1,035 successful JSON outputs and 19 expected diagnostics. All 12 published
fixtures, all 57 oracle definitions, all 13 actions and both independently
extracted §16.3 objects participate. Existing 577 comparisons remain included.

| Command | JSON | Diagnostic |
|---|---:|---:|
| check | 64 | 0 |
| hash | 12 | 4 |
| run-oracle | 218 | 0 |
| run-cc | 115 | 1 |
| reverse (all tiers) | 404 | 3 |
| run-action | 91 | 0 |
| step-check | 59 | 0 |
| run-program | 72 | 11 |

Coherent comparisons require identical fields and exact support; real and
imaginary display components have **absolute tolerance 2e-12**, phase-sensitively
(with no phase quotient). Other fields are exact. The inherited opaque memory
fingerprint exclusion is unchanged and described in the historical report below.
Sampled CLI checks are evidence on their recorded domain, not a universal proof.
`every_reached_row_inverse_restores_complete_pre_step_tuple` separately checks
**21,888** one-row forward/inverse pairs at zero, one and all-one fixture inputs
and memory: every sparse location, memory byte, allocation entry, counter,
stack cell, pc, pc-prime and retained frame must equal its pre-step value.
The entangled-borrow witness compares the complete exact state with an external
reference qubit, not only a marginal or a clean-ancilla predicate.

Unchanged published conformance, our binary:

| Suite | Pass | Fail | Unavailable | Total |
|---|---:|---:|---:|---:|
| A | 21 | 0 | 0 | 21 |
| B | 35 | 0 | 0 | 35 |
| C | 339 | 0 | 0 | 339 |
| D | 7 | 0 | 5 | 12 |
| E | 0 | 0 | 15 | 15 |
| F | 33 | 0 | 2 | 35 |
| G | 10 | 0 | 13 | 23 |
| H | 0 | 0 | 10 | 10 |
| **all** | **445** | **0** | **45** | **490** |

The standalone harness exits **2**, not 0: 45 staged outcomes remain explicitly
unavailable. Every available case, including **339/339 C**, is now a hard test
gate. No conformance files or expected outcomes were edited.

## Red-green findings and deciding sentences

The eight p9h negatives first produced §9.2 hash errors. Rule-citing tests
observed red, then green before identity checking:

| Case | Deciding ISA sentence |
|---|---|
| F-borrow-neg-vwv | §5.6: “the frame is `V; W; V⁻¹; W`” (or a full-domain T3 proof) |
| F-borrow-neg-control-in-b | §5.6(i): “controls are all in a set X with X ∩ B = ∅” |
| F-borrow-neg-target-outside-b | §5.6(i): “target is in B” |
| F-borrow-neg-swap | §5.6(iv): “no `swap` appears in the frame” (translation lemma); exhaustive fallback independently fails factorisation |
| F-disjoint-pred | §3.3: “every wire of the effective control context is distinct from every target” |
| F-width-mismatch | §3.3: “`binop`/`icmp` operands equal width” applied verbatim to lifted counterparts |
| F-in-wire-target | §5.1: “`in` wires in actions appear only as controls, as targets of diagonal gates, as `qcopy`/`qxor` sources, or as `in` arguments of calls” |
| F-lex-double-space | §16.1: “tokens separated by exactly one space” |

Additional regressions observed red then corrected:

- H² left 1.0000000000000002 numerically; §4.4 defines the exact H matrix.
  Exact algebra now gives I and exact clean support; rotation/adjoint
  cancellation is tested for all three axes. A separate red/green regression
  covers a symbolic sum of finite angles exceeding binary64: §3.1 permits
  finite f64 angles, so display evaluates bounded phase factors rather than
  manufacturing a NaN. This approximation is display evidence, not a rigorous
  interval certificate.
- Fixed frame layout: §3.6 “Frame layout (fixed per oracle)”; rows originally
  retained different slot counts. Every row, HALT included, now has the same
  derived clean-padded slot layout; every retained value is width-checked.
- Borrow identity outside lemma shape: §5.6 permits “a T3 claim over the
  frame's full domain”; two cancelling CXs now pass exhaustive proof.
- Hidden `mcx` list controls: §5.6 “touched (as control or target) only inside
  borrow frames”; a single borrowed-controlled flip now rejects.
- Separate frames: §5.6 “maximal contiguous permutation-only subcircuits”.
  Two valid frames around H pass; an invalid frame followed by its later
  cancellation across H cannot pass by checking the aggregate operation.

The initial lexical implementation rejected `regs {  }`; §16.2 has two
mandatory spaces surrounding the optional empty declaration list. This exact
form remains legal. The action CLI initially packed sub-byte memory cells;
the reference README's per-cell encoding and the conformance derivation decide
the CLI convention. Current reference `step-check` adds `target_fault` explanatory
text beyond the older README schema; the VM matches that observable CLI field.
No remaining reference-semantic disagreement on the compared corpus.

## Header audit

Published real headers landed at ISA commit **3a10f84**. All 41 normative
function signatures, opaque types, enum values and calling conventions match.
`include/bennett_vm.h` now **consumes** `bennett_launch.h`, removing duplicated
types/enums that would conflict. Bead `bennett-vm-28w` records this difference.
The six intentionally VM-only extensions are individually recorded by
`bennett-vm-flh`, `5nl`, `kr0`, `73w`, `j6r`, `ht2`; the extension header alone
owns their declarations. All seven audit beads are closed with C11 evidence.

## Reproduction and provenance

Final executable SHA-256:
`23a77feed9d9ea0eb06f55fa45a5a45e282d24019203e261991f64cd6ecd3872`.
Reference executable SHA-256:
`a49f07dc608501186d39d5a6c254ea2bbe49ca05647ed053d5dd4ffa2e23cf2c`.
ISA checkout HEAD observed during verification: `2f167cc` (report-only; no
claim that the reference was rebuilt from that HEAD). Another agent rebuilt
the reference during one test run, causing ENOENT; that run failed loudly.
The final run pins a byte-for-byte CLI executable copy in VM `.tmp/` to avoid
racing cross-repository builds. No reference source is a dependency.

```sh
env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u BROWSER \
  TMPDIR=/home/tobias/Projects/bennett-workspace/bennett-vm/.tmp \
  BENNETT_ISA_REF=/home/tobias/Projects/bennett-workspace/bennett-vm/.tmp/reference-m3-oracle \
  BENNETT_HARNESS_COUNTS=1 cargo test --offline \
  --manifest-path /home/tobias/Projects/bennett-workspace/bennett-vm/Cargo.toml -- --nocapture
```

Logs: `.tmp/m3-test.log`, `.tmp/conformance.log`. All supported commands fail
loudly; §8 target lowering and §11 schedules remain explicit milestone-4
refusals, and estimate/coverage/certificate production names milestones 4–5.
Next: **milestone 4**, `bennett-vm-v23`; JIT remains milestone 5.

---

# Historical milestone 2 evidence

# Milestone 2 verification — 2026-10-29

Implemented accepted **ISA 0.8 / launch 0.8**, as actually published in §10
(the brief calls it launch 0.3). Sole repository implementer; std-only Rust;
no source from the ISA repo's crates was read, copied or linked. Reference
and conformance executables were used through their CLIs. No other repo was
modified; no push or Dolt remote sync performed.

## Delivered ABI and runtime

`cargo build --offline` produces `target/debug/libbennett_vm.a` and
`libbennett_vm.so`, plus the CLI. All **41 normative §10 functions** are
exported, with the §10 status/kind discriminants and C calling convention:

- program: `load`, `unload`, `budget`;
- registers: `qreg_new`, `qreg_prepare`, `qreg_free`;
- arguments: `args_new`, `args_set_int`, `args_set_f64`, `args_set_qreg`,
  `args_set_token`, `args_free`;
- runs: `run`, `wait`, `reply`, `suspend`, `resume`, `reverse`, `run_close`;
- results: `result_nvalues`, `result_get`, `result_nfaults`, `result_fault`,
  `result_nmeasurements`, `result_measurement`, `result_tape_length`, `result_free`;
- requests: `request_service`, `request_payload`, `request_free`;
- tapes: `tape_open`, `tape_next`, `tape_free`;
- staged estimates: `estimate_run`, `estimate_nresources`, `estimate_get`,
  `estimate_certificate`, `estimate_free`;
- staged coverage: `coverage_get`, `coverage_jcs`, `coverage_free`.

Names above have the `bennett_` prefix. Estimate/coverage production remains
an explicit diagnostic naming milestones **4–5**, as PLAN.md milestone 2
requires; these are **exports, not implemented estimate/coverage semantics**.
Status-returning staged producers set output objects NULL and return E_TARGET;
non-status staged accessors print the unimplemented diagnostic and return
zero/NULL; matching frees accept NULL. No fake resource/certificate/JCS object.

Runs are cooperative, externally synchronized per program. `run` validates
positional kinds/widths and copies arguments/entropy without executing;
`wait` advances RUNNING until exactly one owned request/result is available.
A valid hostcall pauses before mutation; reply copies the host bytes, resumes
RUNNING, then the ordinary instruction frame records request/reply/overwrites.
Invalid host memory produces fault 7 without a request. Terminal faults expose
no values. Suspend/resume and all terminal-state checks follow §10.

Persistent program memory, registers and modeled devices survive run close;
closing releases the run's history. Locks retain touched register IDs, allocation
entries, overlapping memory ranges, allocator state and device ordering until
CLOSED. Disjoint registers can execute concurrently; all lock acquisition is
checked before mutation. Reversal runs actual resolved Q inverses and LIFO
classical deltas; independent runs' handles/state are preserved. Entropy position,
effect tape, host requests and measurement records are local to each run.
Tokens preserve identity even when two tokens carry identical budgets; foreign
program tokens are rejected. B/heap/stack/depth use the existing independently
tested oracle evaluator and §5.4 validation. Counter widths are derived from
the supplied depth limit (§3.7), rather than imposing an undocumented fixed Dc.

VM extensions (not additions to the published ABI): `run_step`,
`run_snapshot`, `snapshot_jcs`, `snapshot_free`, `run_reverse_to`, plus
`last_error` (six extensions; 47 total C functions). Snapshots are immutable
owned diagnostic JCS: state, dynamic instances, frame flags, trace, memory
digest, devices, tape and host requests. `reverse_to` reverses a retained
prefix to a history length while SUSPENDED/COMPLETED/FAILED, then suspends;
it rejects measurement reversal, out-of-range indices and cuts inside an
already cancelled segment. Normal §10 `reverse` still requires COMPLETED.

**Published-header discrepancy:** both ISA include headers currently contain
only guards/comments (confirmed by conformance/PROGRESS.md), despite the brief's
ABI assumption. VM-owned `include/bennett_vm.h` includes the published
`bennett_launch.h`, and transcribes declarations from the normative §10 C block.
The C test also includes the published `bennett_sched.h`. No declarations were
invented in or written to the ISA repository. Consumers need both include paths.

Two §10 details lack specified encodings/statuses: host prepare/measurement
bits use one byte 0/1 per wire, in wire order; tape exhaustion returns
E_INVALID_STATE with an explicit end-of-tape diagnostic and leaves outputs
untouched (there is no published EOF status). Calls on a program require host
synchronization; pointer-returning accessors without status require valid
objects/out-length pointers and fail loudly on misuse. Recoverable argument
vector allocation failure maps to E_NOMEM; Rust allocator exhaustion elsewhere
can still abort, as with the existing interpreter. Exact arbitrary-interference
cleanliness and the binary64 Born cache remain milestone 3, unchanged in scope.

## Tests and red-green evidence

Final headless offline gate: **20 Rust tests passed**, 0 failed/ignored,
including the C11 integration group. `tests/launch_abi.c` links the actual
static library with `cc -std=c11 -Wall -Wextra -Werror` and passes **160 status /
count checks**, plus hand-derived byte, pointer, measurement, tape, snapshot and
ownership assertions. Every normative entry point and every VM extension is
called by that C program. Comments cite the deciding §10 sentences; budgets
also cite §5.4, effects §7.3 and history §7.4. Status enums are statically checked.

Red: initial C link failed because no library existed. Green: exported library
and host/wait/reply/suspend/result/reversal assertions. Expanded C cases cover
all four kinds and non-byte integer width, duplicate-budget token identities,
copied input buffers, five devices/effects in tape order, entropy atomicity,
failed-prefix reversal/replay, nested CC→R→Q reversal plus inner uncall,
measurement consumption/refusal, surviving register IDs and interleaved runs.
A later C run exposed the test's own allocation-lifetime mistake: a FAILED
allocation persists after close; moving the independent tree scenario before
that exhausted heap fixed the test without resetting program state.

The conformance run exposed silent acceptance of unknown schedule/certificate
metadata. A dedicated test reproduced it RED; top-level §9.1 envelope validation
made it GREEN. Also corrected the non-CC launch diagnostic (now §10 E_ARG before
any tier-specific parser) and added explicit milestone diagnostics for action
reverse options. The target/schedule probes now retain both the VM milestone
and published harness's staged-deliverable wording.

`cargo fmt --check`, strict `cargo clippy --offline --all-targets -- -D warnings`
and `git diff --check` pass. `tests/launch_c.rs` builds the library in a separate
repository-local target directory to avoid Cargo's recursive build lock.

**Original differential harness: 577 / 577 agree, 0 diverge**, unchanged counts
and comparison limits listed under milestone 1 below (558 JSON, 19 diagnostics).
No reference-interpreter disagreement is asserted. No expected output was
changed to match this implementation.

## Reproduction and fingerprints

Final commands (with DISPLAY, WAYLAND_DISPLAY, DBUS_SESSION_BUS_ADDRESS and
BROWSER unset; TMPDIR `/home/tobias/Projects/bennett-workspace/bennett-vm/.tmp`):

```sh
BENNETT_HARNESS_COUNTS=1 cargo test --offline --manifest-path /home/tobias/Projects/bennett-workspace/bennett-vm/Cargo.toml -- --nocapture
cargo fmt --check --manifest-path /home/tobias/Projects/bennett-workspace/bennett-vm/Cargo.toml
cargo clippy --offline --manifest-path /home/tobias/Projects/bennett-workspace/bennett-vm/Cargo.toml --all-targets -- -D warnings
/home/tobias/Projects/bennett-workspace/bennett-isa-spec/conformance/target/debug/bennett-isa-conformance --bin /home/tobias/Projects/bennett-workspace/bennett-vm/target/debug/bennett-vm
```

The last command is also invoked unchanged by the report integration test.
Gate output: `.tmp/final-tests.log`; all 577 HARNESS records were counted.
Observed ISA checkout HEAD: `8e11fe7529d1b52db736a170ac0d99ee91824bc4`.
Observed executable SHA-256 (not a claim about which source built an existing
external executable):

- VM: `624d4ed8306c0ed166b6b445a387aec831d76c3e474ba64d15d3da4834ac90c8`;
- conformance: `1bd5aab7f20690c7959edfd217d5380cfd8ea4a4f7f7bc17bea4b86561e2bb92`;
- reference: `4ade7d6fb8dda864b829fe33f91b0bc53c9f8fd6089ac8cfe9ecc481a9eebaff`.

Milestone bead `bennett-vm-7bj` is CLOSED. Validation follow-up `bennett-vm-p9h`
is OPEN. No remote synchronization or push.

## Published conformance: full 490-case verdict stays RED

`tests/conformance.rs` invokes the existing read-only conformance harness with
`--bin <VM binary>` and no suite/case filtering. It saves the complete raw
verdict at `.tmp/conformance.log`. Standalone harness exits **1**, not success.
The Rust report test verifies the full corpus and milestone-2 launch/probe
obligations; its success does **not** turn the full conformance verdict green.

| Suite | Pass | Fail | Unavailable | Total |
|---|---:|---:|---:|---:|
| A — classical/history | 21 | 0 | 0 | 21 |
| B — oracle/reversal | 31 | 4 | 0 | 35 |
| C — permutations/agreement | 38 | 301 | 0 | 339 |
| D — unitary/channel | 0 | 7 | 5 | 12 |
| E — targets | 0 | 0 | 15 | 15 |
| F — validation/identity | 24 | 9 | 2 | 35 |
| G — launch | 10 | 0 | 13 | 23 |
| H — scheduling | 0 | 0 | 10 | 10 |
| **All** | **124** | **321** | **45** | **490** |

**313 failures are explicit milestone-3 refusals:** 304 run-action, four
step-check, three oracle reverse, one action reverse and one perm_action reverse.
These correspond to deferred literal §3.6/§3.7 stepping and exact-amplitude
output, not a semantic claim about an executed Q calculation. All available
G cases pass; D's seven runnable cases require those unimplemented standalone
exact-action commands; E's launch target refusal probe passes and is reported
UNAVAILABLE by the harness's own rules. Nothing is silently executed without a
schedule or lowered target. C tests separately exercise the stateful launch
scenarios that the unchanged harness labels UNAVAILABLE; the harness has no
mechanism to invoke/promote this C library. Estimate/coverage remain staged.

**Eight F diagnostic disagreements remain red**, tracked by **bennett-vm-p9h**.
Each malformed object is rejected, but as §9.2 stored-hash mismatch before the
required validation diagnostic. These were not included in the 577-comparison
milestone-1 corpus and are not alleged reference bugs:

- `F-borrow-neg-vwv`, `F-borrow-neg-control-in-b`, `F-borrow-neg-target-outside-b`,
  `F-borrow-neg-swap`: §5.6 requires the frame to factor as `P_F = P′ ⊗ I_B`,
  proved by T3 or structurally as `V; W; V⁻¹; W`; V targets B with controls
  outside B, and “no `swap` appears in the frame.”
- `F-disjoint-pred`: §3.3, “every wire of the effective control context is
  distinct from every target”.
- `F-width-mismatch`: §3.3, “`binop`/`icmp` operands equal width”, and these rules
  “apply verbatim to their `lift.*` counterparts”.
- `F-in-wire-target`: §5.1, “`in` wires in actions appear only as controls, as
  targets of diagonal gates, as `qcopy`/`qxor` sources, or as `in` arguments of calls.”
- `F-lex-double-space`: §16.1, “tokens separated by exactly one space”.

Next: milestone **3**, `bennett-vm-gkd`, plus validation bug `bennett-vm-p9h`.
Targets/lowering/schedules stay milestone 4; estimate/certificate/coverage work
stays milestones 4–5 per PLAN. North-Star scope remains intact.

# Milestone 1 verification

Completed 2026-10-29 against accepted ISA **0.8**, std-only Rust, offline and
headless. No reference-interpreter source was read, copied or linked. The
reference was invoked solely through its CLI.

Reference checkout HEAD at verification: `f83fc71604fe8c43eabbc2cd3f905e9aa942dcb6`.
Reference binary SHA-256:
`11244ac5a6977ffc73dc379d6fa20f530e871aec781609ec6b2cda9423e54033`.
These identify the observed checkout and binary separately; no claim is made
that an existing binary was rebuilt from that HEAD.

## Gate and counts

`cargo test --offline`: **17 passed, 0 failed, 0 ignored** (4 unit tests,
13 integration groups). `cargo fmt --check`, `cargo clippy --offline
--all-targets -- -D warnings` and `git diff --check` pass.

**577 differential comparisons: 577 agree / 0 diverge on compared fields**,
including 558 successful JSON outputs and 19 expected diagnostics. All **12
published fixtures** and both **independently extracted §16.3 example objects**
are included. Generated objects supplement, never replace, the published corpus.

| Command | Successful JSON | Diagnostic |
|---|---:|---:|
| check | 64 | 0 |
| hash | 12 | 4 |
| run-oracle | 218 | 0 |
| run-cc | 115 | 1 |
| reverse (CC/R) | 77 | 3 |
| run-program | 72 | 11 |
| Total | 558 | 19 |

Identity commands compare stdout byte for byte and exit status. Execution
commands compare all JSON fields, including full instruction/register/memory
traces, faults, budgets, results, tapes, requests, measurements, reversal and
memory-equality predicates, except **opaque memory-digest bytes**. Both digest
fields are required and checked for 64 lowercase hex characters. ISA 0.8 does
not prescribe a digest serialization, and the reference README describes the
state being hashed without specifying its encoding. Our digest is SHA-256 of
JCS `{bump,bytes,table}`, with nonzero byte pairs sorted by address and the full
allocation table in allocation order. It intentionally differs from the
reference fingerprint; this is not a byte-identical execution-output claim.
Diagnostics require equal exit status and no stdout; diagnostic prose is not
required to be identical. Every mismatch fails and prints both outputs.

Enable `BENNETT_HARNESS_COUNTS=1` with `cargo test --offline -- --nocapture` to
emit one `HARNESS <command> <json|diagnostic>` record per successful comparison.
All subprocesses remove graphical environment variables and inherit the
project-local TMPDIR. Missing reference binaries/fixtures fail, never skip.

## Evidence by integration group

- `check_and_hash_byte_identical_on_every_fixture`: check/hash on all 12 fixtures.
- `spec_examples_are_extracted_and_diffed`: extracts both complete objects from
  the published spec itself; identities and execution agree.
- `run_oracle_differential`: 45 explicit cases covering arithmetic, first faults,
  B=0/2/3 boundaries, recursive activation/monotonic stack/depth limits, mapped
  memory, nested oracle cancellation and 257-bit arithmetic.
- `every_fixture_symbol_executes_through_cli`: all **57 oracle definitions** at
  B=0/1/256 (171 cases), 14 zero-argument CC/R entries, all **13 action
  definitions** reached through generated CC action/adjoint pairs. Lprocs are
  reached through their oracle callers. Borrowing without a designated idle set
  is an explicit §5.6 diagnostic in both binaries.
- `classical_control_and_launch_differential`: 45 cases over classical.bisa and
  registers.bisa, including all four devices, fault atomicity, nested uncall,
  qreg allocation/slicing/concatenation/clean release, measurement and launch
  eligibility; introduced red at the inherited CC stub.
- `host_reply_and_prefix_reversal_differential`: successful/truncated host replies
  and 46 loop/nested history prefixes. Every host request must match a reply and
  every reply must be consumed.
- `retained_instances_survive_segment_cancellation`: pure values survive uncall
  and whole-history reversal restores entry.
- `action_basis_measurements_differential`: 39 observations (three basis inputs
  for every action), including dirty idle handles, coherent while loops,
  predication, gates, local/interface memory and classical angle arguments.
- `malformed_classical_types_are_load_errors`: four malformed objects; seen red
  on accepted width mismatch, green after CC/R type/SSA checking.
- `classical_rows_and_boundary_faults`: all binops/divisions/comparisons, shifts
  at the width, signed MIN/-1, i128 casts, select, pointer casts, alloc/free and
  whole-range memory bounds; includes failure before the first definition.
- `cc_oracle_xor_budgets_controls_and_arguments`: arbitrary initial target/fault
  values, input preservation, token budgets and inherited control, plus real
  involution reversal for B=0/2/3.
- `oracle_interface_memory_is_restored`: initially red on leaked temporary
  stores, green after the F_B evaluator stopped copying scratch cells back.
- `history_reverses_actual_region_invocations`: real action adjoints for every
  two-bit input, whole runs, prefixes and segment cancellation followed by
  measurement. Region frames contain resolved bindings, not wavefunctions;
  classical history snapshots do not contain quantum state. Released simulator
  wire ids are reclaimed only after the actual inverse leaves them clean.

## Divergences found and their deciding contract text

All semantic mismatches found in this session were corrected in the VM.

1. **Load-time CC types/SSA.** A width mismatch was accepted. §3.3:
   “`binop`/`icmp` operands equal width; `select` arms equal width”. CC/R now
   checks definitions, operand types, casts, branch/call/return bindings and
   region-slot widths before execution. Static duplicate-definition rejection
   also matches the reference's SSA convention.
2. **Borrowing.** Inherited actions allocated fresh zero wires for `qborrow`.
   §9.4: “callees `qborrow` only from the caller's designated idle set.” The
   action library now accepts an explicit idle pool. The one-shot 0.8 CLI has
   no designation operand, so it reports a §5.6 diagnostic when borrowing is
   requested; arbitrary other live handles are not silently designated.
3. **Coherent loop cleanup.** Inherited code recomputed a dependency chain in
   forward order to clear flags. §4.6: “flags uncomputed in reverse and freed”.
   Cleanup now walks the chain backwards, including all-one selector inputs.
4. **Local-region lifetimes.** Inherited forward execution created a region at
   its textual instruction and discarded errors during adjoint creation.
   §4.6: “created at function entry (unconditional allocation, X gates from the
   data image, default zero)”. All lexical declarations are created at entry;
   creation errors propagate, including declarations inside zero-trip loops.
5. **Deterministic measurement draws.** Numeric rather than wire-order outcome
   enumeration selected a different Bell outcome for the same `--draws` value.
   §4.1 requires “Born rule (§14.2)”; that sentence does **not** fix the mapping
   from a supplied draw to equally probable outcomes, so neither ordering is
   ruled out by the ISA. The VM adopts the reference's lexical wire-order CLI
   convention for substitutability (README: “wire 0 first”).
6. **Nested oracle memory.** Temporary stores escaped the F_B evaluator into
   caller-owned cells. §5.2: “`in` regions unchanged”; and construction step 4:
   “run the inverse step for t = B..1, then release (1). The commit writes
   persist.” Only payload/code escape the scratch evaluator now.
7. **Complete entry equality.** A first-instruction allocation failure incorrectly
   reported `equals_entry=true`. §3.2 includes “block parameters, activation
   stack” in the classical environment; the reference CLI defines the predicate
   as the “complete entry state ... restored byte for byte”. Control position
   and activation allocator state are now included and restored by reversal.
8. **Region reversal audit.** Initial CC history restored saved amplitudes rather
   than applying Q inverses. §7.4: region calls are “reversed by re-application /
   adjoint, exactly once, by this frame”. History now records resolved invocation
   bindings and executes their inverse exactly once; already reversed frames
   and uncall markers are skipped. Symbolic adjacent inverse cancellation
   witnesses entry identity independently of binary64 Born-cache roundoff.

Inherited code also initially failed compilation because two obsolete fields
remained in the oracle constructor; removing them made the inherited oracle
regressions green. This was not a reference-semantic disagreement.

## Deliberate staging and next milestone

Every §4.1 CC/R row is implemented. The one-shot CLI implements `check`, `hash`,
`run-oracle`, `run-cc`, CC/R `reverse`, and `run-program`, with the reference
command names and required JSON fields. `step-check`, oracle/action-form
`reverse`, standalone exact-amplitude `run-action` explicitly name **milestone
3**; target lowering and scheduled execution name **milestone 4**. No silent
fallbacks or omitted CC rows.

The current sparse action simulator uses binary64 amplitudes for Born
probabilities; exact general interference/borrow-frame proofs remain milestone
3, not a claim of numerical proof. The inverse-boundary witness used by CC
reversal stores operations, not copied quantum states. History snapshots of
classical resources favor correctness over performance; optimize after the
same differential gate protects the changes.

Next: **milestone 2**, full §10 opaque C ABI, stateful launch/wait/reply,
suspend/resume/close, result/tape accessors, single-step/snapshot APIs and device
hardening (`bennett-vm-7bj`). Milestone 3 is tracked as `bennett-vm-gkd`.
ISA 0.9 remains a draft: typed R faults and lproc token/classical parameters
are tracked as architecture touchpoints in PLAN.md; this build stays on 0.8.
