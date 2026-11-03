# bjm-dense pre-merge review (Sonnet 5.5) — 2026-11-03

Range `d9cb684..6cadcb6` (branch `bjm-dense`). Scope: Codex commits 7b39d25 a83df82
48e32db 15deecd c311116 edbe625 4a56cda 6cadcb6; Claude commits and merge 644a212 as
context only. The 2026-11-02 design review is not re-reported.

## Verdict: BLOCK (one P1, bennett-vm-d2j)

### P1 bennett-vm-d2j — `Runtime::equals` returns true for different states
`src/dense/runtime.rs:1238` short-circuits on equal `live_seqs()`. But `Log::record`
(`runtime.rs:237-273`) composes a new `Remap` into the previous live `Remap` **in place**,
keeping the old `seq`. A clone taken before the second `map_basis` therefore has the same
live seqs as the later, different state.

Failure scenario: `h w0; map_basis(xor w0->w1); entry = clone; map_basis(xor w0->w2)`.
Dense `numerically_equals(&entry)` = `Ok(true)`; exact interpreter = `Ok(false)`
(amplitudes differ). Same API that `run-action --reverse` uses for `equals_entry`
(`qcli.rs:278`), so a non-reversible action can be reported as restored (§7.4).

Command (probe crate `.tmp/sonnet-rev-bjm/probe`, path dep on this worktree):
`timeout 900 cargo test --offline -j4 --test probe r1 -- --nocapture` → FAILED, `dense=true: Ok(true)`.
Fuzz `f1` (3000 seeds, h/cx/map_basis/swap/phase, entry clone at a random step): 14
false-positive equalities, 0 `constant_value` mismatches over 15000 wire decisions.
Fix: give a composed op a fresh seq (or compare op contents, not seqs).

## 5vj check (6cadcb6)
Real. `d1b_non_identity_reversal_equals_entry_in_dense` is not ignored (no `#[ignore]`
anywhere in `tests/dense_*.rs`, `src/dense`), runs both engines, and asserts
`!numerically_equals(..).unwrap()` for a 2e-12 residue below the global bound (refuted via
the per-wire branch test). `numerically_equals` is now `Result` and delegates to
`Runtime::equals`; panics became diagnostics and propagate through `?` at every call site
(action, cc, qcli, launch, step, certificate). The new test `cli_reports_undecidable_*`
checks exit 1, no panic, empty stdout. The d1b test does not cover the P1 above.

## Other checks (no defect found)
`timeout 1500 cargo test --offline -j4 --test dense_decisions --test dense_differential -- --test-threads=4`:
24 + 12 passed, 0 ignored. Fuzz `f2` (4000 seeds, h/T/cx/ch/phase/rx/ry/rz via `apply_exact`,
measurements with matching draws, entry clones): 15587 constant decisions agree with the
interpreter, 413 fault "undecidable" (never a different answer), 3938 equalities agree,
62 fault, 0 mismatches. Compaction only removes certified constants (assert-checked against
the numerical state). Tolerances in the new tests (`d3`: bound + 1e-15; differential: final
B + 8ε√keys) are bound-derived, not hiding errors.

## P2+ (bennett-vm-c25, P3)
- `launch.rs:385,400`: undecidable dense decision returned as status 7 (`E_QREG_NOT_CLEAN`); the test pins 7. Host cannot tell dirty from undecidable.
- Undecidable faults where the interpreter answers (rotation uncompute, large remaps, mixed dense/exact equality) are a deferral to bennett-vm-k8w; keep it scheduled before any rung needs rotations (no-narrowing rule).

## Read closely vs skimmed
Closely: 6cadcb6 diff, `runtime.rs` Log/decide/compact/release/measure/equals,
`tests/dense_decisions.rs`. Skimmed: `dense/mod.rs` kernels, `dense_kernels.rs`,
`examples/dense_bench.rs`, design doc, `cc.rs`/`jit.rs` edits. Not checked: bennett-4ht
mixed fat-object reachability beyond the error paths above; benchmark claims; the Claude
commits' kernels.
