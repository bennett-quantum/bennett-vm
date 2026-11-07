# VM stepping review — round 1

Reviewed `d68eb760ecb2d76a6c44e9b30b546775ff17e737..01e81952e89810c3daf99f04a7cccf7e861a3c4d`
(`main...job/r3-vm-stepping`, nine commits, 887f49f through 01e8195).

**CLEAR. P0/P1: none.** No confirmed rung-path semantic regression, retained-root mutation leak, unsound oracle, or gate breakage.

The AVL value mutation path copies both the node and its shared payload. Lock ranges merge only on strict overlap, preserving adjacency and empty-range behavior; owner exclusion, discrete locks and close-time release retain the linear rule. The accounting index removes only predecessors with empty wire sets, which cannot satisfy the original dependency predicate. Test-only Clone implementations match the production derives field for field.

Verification at the reviewed head, headless with reviewer-local TMPDIR/target, serialized through `.tmp/r3-vm-stepping/in-slot.sh <seconds> env TMPDIR="$PWD/.tmp/rev-r3-vm-stepping-r1/tmp" CARGO_TARGET_DIR="$PWD/.tmp/rev-r3-vm-stepping-r1/target"`:

- 900 s: `cargo test --offline -j3 --lib step_scaling:: -- --include-ignored --nocapture --test-threads=4`: **3 passed**. Full printf fixture: status/i32 zero, 225956 rows, exact 54 console bytes, target time 225956; fresh pinned-reference comparison passed, no UNKNOWN.
- 600 s: `cargo test --offline -j3 --lib -- --skip step_scaling:: --test-threads=4`: **50 passed**, including retained-root mutation, AVL sharing, interval/point/discrete lock equivalence, exact rollback and replay-mismatch controls.
- 600 s: `cargo test --offline -j3 --test suite -- target_execution:: targets:: replay09:: saved_tape:: classical09_launch:: classical09::terminal_fault_records --include-ignored --test-threads=4`: **41 passed**, including target accounting, immutable snapshots, exact reversal/reset, saved-tape replay and interpreter/JIT terminal faults.
- `timeout 60s python3 -B scripts/quality.py`: **5 selfchecks passed, no regression**; `git diff --check main...01e8195`: passed.

Red-first integrity: compared tests against `git show main:<path>`: inline payload cloning violates the tightened zero-copy assertion, Vec event histories copy retained events, and classical accounting performs N(N−1)/2 predecessor visits. Inspected committed 64f0b30 lock instrumentation/red output and earlier red/mutation logs; the current counters observe real clone/comparison operations. No baseline mutation or branch switch performed. Existing full gate log `bennett-vm/.tmp/orch-gate/gate-01e81952e89810c3daf99f04a7cccf7e861a3c4d.log` records PASS; no full gate rerun.

P2+ nits: **P3 bennett-vm-hc8** — `src/action.rs:2349` copies both complete histories before accounting (`src/persistent.rs:384`); optional snapshot-work improvement, no observed result defect.

Coverage: read closely all production changes, new probes/tests, affected AVL/accounting/launch paths and gate configuration; skimmed the generated object, prior fw8 changes and historical evidence. Checked every COMMON anti-slop rule; no blocking violation. Plan 9–11: existing quality ratchet remains active, only new-file baseline rows added, HANDOFF.md absent and README points to the workspace handoff.

Provenance: implementer **gpt-6.1-sol**; reviewer **gpt-6.1-sol, fresh session**, D9. No implementation files changed.
