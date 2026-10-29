Milestone 4 PARTIAL; parent bennett-vm-v23 BLOCKED, not closed (2026-10-29).
Commits: 70ac9f0 exact targets; 5111c14 target execution/scheduler exports;
subsequent final commit contains sector/path hardening and this handoff.

Read docs/PROGRESS.md first: exact counts, deciding sentences, limits and next work.
56 Rust tests pass; 160 launch C checks; scheduler C consumer calls all 15
published bennett_sched.h functions. 1,076/1,076 CLI differential comparisons
agree (1,050 JSON, 26 diagnostics); zero remaining CLI divergences.
All 7 runnable D and 10 runnable G commands now participate in the differential
harness; five additional block-image boundary comparisons participate too.

Raw unchanged conformance: 445 pass / 2 fail / 43 unavailable, exit 1.
A21/0/0 B35/0/0 C339/0/0 D7/0/5 E0/1/14 F33/0/2 G10/0/13 H0/1/9.
The two FAILs are obsolete staging-prose probes: E receives strict noncanonical
JCS diagnostic; H receives explicit E_SCHEDULE_INFEASIBLE missing-ref reason.
Integration test preserves the raw verdict and gates exactly those reasons;
it does not claim full conformance green or promote 43 hard-coded unavailable cases.

Implemented supported exact-rational target AST, strict §8 target input format,
UTF16 JCS class order, first matching native gates/non-nested schemes, actual
native-leaf simulation for gate-only actions, capacity/class history, explicit
leaf-event/live-interval accounting. No automatic scheme-correctness proof is
claimed; author owns it (§8.3). Scheduler request setters own copies and valid
requests return UNSUPPORTED, reason ref@0.8, no schedule/certificate. Selector,
binding, path, timing and schema validation utilities have hand-derived tests.
Devices retain all prior behavior; partial block images now reject at device
initialization (not check), matching reference; complete sectors read/reverse.

ISA gaps (all labeled isa-0.9, filed locally):
47t: no published ref expansion document/hash/ordinals/scratch/removable subset;
cxg: eps_decade table index is named but never defined;
clh: E epsilon-zero fixture contains no forbidden eps; JCS hash fixture has newline;
47i: clarify complete-sector block_image encoding/partial-final-sector policy.
Completed child beads v23.1/v23.2/v23.3; parent depends on 47t/cxg and stays blocked.

Remaining M4: full oracle/structured row lowering, dirty-scheme caller idle pool,
requested approximation shares, verbatim schedules, schedule reproducibility and
integrated resource accounting. Do not substitute a private guessed ref library.
After M4: milestone 5 JIT; estimates/rigorous certs/coverage retain loud milestone5
errors. Branch enumeration and exact coherent tier remain implemented and gated.

Headless/offline with TMPDIR under .tmp, no dependencies, other repository edits,
pushes or Dolt remote sync. Gates: fmt --check, strict all-target Clippy,
cargo test --offline, git diff --check. Logs .tmp/m4-final-tests.log and
.tmp/conformance.log. Full conformance still exits 1; report it honestly.
