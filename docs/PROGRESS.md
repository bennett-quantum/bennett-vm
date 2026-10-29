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
