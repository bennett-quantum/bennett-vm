Milestone 2 implemented on accepted ISA/launch 0.8 (2026-10-29).
Read docs/PROGRESS.md for exact evidence, limits and all outstanding red cases.
All 41 §10 C symbols exported; estimate/coverage semantics explicitly staged
for milestones 4–5. VM header include/bennett_vm.h consumes empty published
headers and transcribes the normative §10 declarations. Six VM extensions add
stepping, immutable JCS snapshots, history-prefix reversal and last-error text.
Cooperative wait/reply/suspend/resume; persistent program resources; run-local
entropy/tapes; exclusive touched resources; actual Q inverses; results/requests/
tape iterators independently owned; close releases run history.
20 Rust tests pass; the linked C11 test passes 160 checks and calls every export.
577/577 original differential comparisons agree; no disputed reference behavior.
Full read-only published conformance stays RED: 124 pass / 321 fail / 45 unavailable
of 490. 313 failures name milestone 3; eight wrong-validation-diagnostic negatives
are tracked as bennett-vm-p9h. G available 10/10 pass; E/H refusal probes pass.
Stateful G cases stay unavailable in the unchanged CLI-only published harness;
C tests provide the separate stateful evidence. Never call full conformance green.
Next: bennett-vm-gkd (milestone 3) and bennett-vm-p9h; then milestones 4–5.
All operations headless/offline with TMPDIR under this repo. No pushes or
cross-repository edits. Program ABI calls require caller synchronization.
