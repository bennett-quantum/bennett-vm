# bjm dense engine design review — 2026-11-02

Second-round review of the dense state-vector engine on branch `bjm-dense` at
`4a56cda` (sol-bjm's final commit): `docs/design/dense-engine.md`,
`src/dense/mod.rs`, `src/dense/runtime.rs` and the `action.rs` wiring, checked
against the design doc's own claims and ISA 0.9 §5.6/§7.4 cleanliness. No source
was edited. Findings with a bead have a red test in the scratch crate
`.tmp/opus-reviews/probe` (path dependency on a worktree at `4a56cda`), run with
`CARGO_TARGET_DIR=.tmp/opus-reviews/target cargo test --offline -j2 --test probe`:
7 tests, 2 green controls, 5 red. Beads are in bennett-vm.

## Findings

### D1 (bennett-vm-yt9, P1). Cleanliness and reversal equality are decided within B
`Runtime::constant_bit` treats a wire as constant when the other branch has norm
at most `min(B, 0.25)`. `QuantumState::numerically_equals` accepts any distance up
to `B_self + B_other`. As a result, `qfree` (§5.6), `rewind_clean_wires` (§7.4) and
`run-action --reverse` `equals_entry` succeed under `BENNETT_VM_ENGINE=dense` where
the exact interpreter faults or reports false. That is a wrong discrete answer, and
it contradicts the doc's "discrete outputs must agree exactly".

Witness: 2000 `h` on wire 0 gives B = 1.42e-11; then `ry(2e-12)` on wire 1. The
interpreter reports `is_clean = false`, dense reports `true`
(`d1_dirty_wire_is_not_clean_in_dense`). Comparing that state with its clone from
before the `ry`, the interpreter reports not equal and dense reports equal (`d1b_*`).
The control, the same `ry` with only B ≈ 7e-15, sees the residue (green). The
differential corpus passes because no fixture leaves a residue below B.

### D2 (bennett-vm-ykb, P2). Compaction turns a pessimistic bound into real error
`compact` runs before every measurement and `map_basis`, and on release. It
factors a wire whenever the branch norm is at most `min(B, 0.25)`. Conditioning on
an outcome of probability p sets B to about 2B/√p, which is large even when the
state came from a single 2×2 product.

Witness: `ry(π−2e-13)` on wire 0, then measure with draw 0, giving outcome 0 with
P ≈ 1e-26 and B ≈ 0.14. Next, `ry(0.2)` on wire 1 gives P(1) = 0.00997, but that
wire is factored to |0⟩. A measurement with draw 0.995, 0.005 from the boundary,
then reads 0 in dense and 1 in the interpreter (`d2b_*`). The final evidence is
honest (`error_bound` 0.486, `measurement_error_bound` 0.54,
`numerical_projections` 1), but the engine caused this error itself.

In a related case, after conditioning on P ≈ 1e-20 (B = 1.56e-3), a wire with
P(1) = 1e-8 is declared clean (`d2_*`). Not claimed as a defect: `d2a_*` flips a
draw-0 outcome at a 1e-28 boundary, which falls under the doc's stated
boundary-draw caveat.

## Checked, no defect found
- **Error accounting.** The per-gate recurrence
  B' = A·B + γ(j)·A·(R+B) + j·MIN_POSITIVE·√2ⁿ with j=32 conservatively covers a
  complex 2×2 matvec, including ‖|M|‖ > ‖M‖ for `h`. Fused execution charges every
  original gate. Permutations charge zero, which is correct since they do no
  arithmetic. The 32u symbolic-to-binary64 projection is charged on `apply1`.
  Measurement uses the standard 2‖x−y‖/‖Py‖ bound, with p lowered by reduction
  error. Control `d3_control_bound_holds_against_exact`, a 400-gate Clifford+T+CX
  circuit on 5 wires against the exact symbolic interpreter, gives distance 1.46e-14
  against B = 1.71e-12. The bound holds and is about 100× loose.
- **Layout.** SoA uses two `Vec<f64>` of 2ⁿ each; gate and measurement kernels make
  no second state allocation. Promotion can reallocate, as `Vec` growth does, but
  that happens outside the kernels the claim covers.
- **Cache blocking.** The tile path (all wires below 10, 1024 amplitudes = 16 KiB
  over both arrays) and the gather path (up to 64 amplitudes) both apply the
  original gates in order. Gather-path lines are reused across consecutive outer
  indices. Read only, not benchmarked.
- **Citations.** All four papers are real, with correct authors and venues:
  qHiPSTER arXiv:1601.07195; QuEST arXiv:1802.08032 (Sci. Rep. 9, 10736); Qulacs
  arXiv:2011.13524 (Quantum 5, 559); Häner–Steiger arXiv:1704.01127 (SC'17). One
  section claim was spot-checked against the PDF text: qHiPSTER §4.4 is "Cache
  Blocking through Gate Fusion", with Fig. 4 as the fused-block pseudo-code. Note
  that qHiPSTER blocks in the LLC, whereas this engine uses an L1-sized tile. The
  QuEST, Qulacs and Häner–Steiger section claims were not checked.
- **Comparison with the interpreter** (`tests/differential.rs`). Exit status,
  diagnostics and non-state JSON must be equal. States must agree within the final B
  plus 8ε√keys. That is sound, but it misses D1 and D2 because no fixture reaches
  those regimes.

## Not reproduced
The benchmark tables (gates/s, fusion speedups) were not rerun. The host had load
average about 28, and the review rules allow only focused probes, so no
performance claim is confirmed or refuted here. `bennett-vm-1x1` remains the open
performance bead.
