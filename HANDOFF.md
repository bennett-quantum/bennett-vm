State 2026-11-01 14:29:58 UTC: c2 IN PROGRESS, partial source-proof prerequisite complete; no bead closed this pass. Never push. Sole implementation checkout; detached gate worktree untouched.
Resume: finish c2 source-checked K64 construction/support and K65 non-emitting lifted oracle-uncall carriers, with both reverse/recompute choices, controls and nested invocations; then c3, c5, d, b, hsd, clw. Skip c4 as instructed while the ISA9hv pass runs. Parent c remains open.

Current implementation
Bindings::oracle_cleanup_pairs returns computing/cleanup Point metadata in K64 source-row order from the immutable registry captured by address_oracle. Distinct lproc instances retain their own fixed-binding keys; relinking another object cannot retarget the captured proof. No oracle_invocation token binding or descriptor admission is invented.
The private oracle_cleanup source analysis uses per-operand-identity balanced CFG paths, including cycles, to enforce LIFO matching and one source-call owner on every direct-CFG path. It checks successor target locations and rejects intervening writes to ins; intervening memory stores remain permitted by section5.5. Indirect qjumpr pair proofs remain explicit unavailable.reason obligations. Raw catalog annotation, source admission and existing execution paths are unchanged.
Seven added regressions (focused oracle_address09 now9/0, before2/0) cover nested LIFO, missing/changed invocation inputs, joins with missing/different owners, balanced/underflow cycles, separate lproc instances, captured-registry identity, memory stores and explicit indirect-proof staging. RED is .tmp/c2-source-red.log (missing API); final focused .tmp/c2-source-tests3.log.

Validation
FINAL GREEN unchanged-source sequential .tmp/c2-source-interpreter.log and .tmp/c2-source-jit.log: each268/0/0 (entry261/0/0), differential each18/0/0. fmt, strict all-target Clippy (.tmp/c2-source-clippy.log) and whitespace pass. Totals .tmp/c2-source-totals.json. No fixture/test weakening or removal, dependency changes, foreign implementation source reads/copies, ISA beads/spec edits, push or Dolt sync.

Remaining scope
c2 is NOT closed: metadata is not wired into source-built oracle row/support expansion, no selected K65 carrier execution/recompute was implemented, and the required positive/negative both-choice controlled/nested oracle tests remain. The existing linked catalog call path still uses source::action_body and rejects an oracle until its source constructor builder is supplied. Keep c2 in progress; do not treat this source-proof helper as full oracle integration or an admitted schedule. Direct CFG proof is local to each source function/instance; general indirect path proofs remain to implement.
c3 bound-library occurrence validation, c5 bounded-while/cross-domain histories, d schedule realisation/execution, b K77 descriptors, hsd estimate ABI and clw certificate dispatch are untouched. c4 pair partition was skipped as directed. No new ISA question was found.

Previously closed foundations
u8m saved-tape interface is closed (35bc619); c1 fixed nested-loop/qif histories is closed (entry HEAD4093968). Exact input histories/fresh K80 spaces and the default-trace safety regression for an earlier copy of a temporarily inverted else predicate remain. General history reconstruction stays with c5; never substitute a current external value for an earlier version.

Execution rules
Use BENNETT_ISA_REF=.tmp/c-pinned-reference/bennett-isa-ref. Run full engine gates sequentially (C ABI tests share executable paths). Unset DISPLAY WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS BROWSER; TMPDIR is this repo's .tmp; cargo --offline. No new dependencies, spec text edits, foreign implementation reads/copies or push. Commit only explicit owned paths after green gates; bd export -o .beads/issues.jsonl before committing that file. Worklog belongs to the orchestrator for this implementation-subagent session.
