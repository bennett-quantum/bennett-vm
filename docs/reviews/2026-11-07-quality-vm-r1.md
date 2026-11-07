# quality-vm review, round 1

Reviewed the entire 19-commit range `main..job/quality-vm`, base
`38bd0d9d951ddaeb5be3880c90305f1bfdf9a305`, head
`5ea5dcb0a24fd2a132640a44caad39cf34507476` (quality batch, ISA pin update,
K91/K84 adoption, including the requested main-side history comparisons).

**Verdict: CLEAR. P0/P1: none.** No confirmed rung-3 wrong result, silent
fallback, unsound verdict or broken build/gate was found in this review.

P2+ nits: **bennett-vm-k6g** (priority 3) records the dead token-budget guard at
`src/ref_library/oracle_source.rs:90–94` and the existing formatting follow-up
**bennett-vm-cak**. Binding converts every token to `TokenId` before a loop
that matches only `Token`: a memoryless oracle with an unused token argument
accepts tuple `["1","1","0","0"]`, primary `["1","0","0","0"]`, aliases `[0,1]`.
A stdin Rust probe asserted that `validate_budget` rejects that tuple, then
observed successful lowering with 488 leaves; the bead contains its source
and exact commands. This malformed-input case demonstrates no ladder-path
wrong result or unsound proof, so it is P2 under amended rule 4.

The baseline delta is **not restricted to new files**: existing CC, launch,
source-lowering, scheduler and request-binding metrics also increase.
Inspection ties those changes to control restoration, identity propagation,
partition validation and schedule/certificate matching. The pre-existing
long-function limits remain unchanged; new legacy-entry wrappers preserve
callable APIs. The metric implementation is unchanged, and no litter or
process-identifier exception was added. Items 9–11 were checked: the single
quality gate invokes the tested ratchet, HANDOFF.md is deleted, and README
points to the workspace handoff. The anti-slop sweep found the P2 guard above;
remaining protected splits, opcode adapters and scoped litter already have
owners in bennett-vm-bsj/68j/bai/dhx.

Published case commands/expectations were not rewritten. All 26 runnable D/G
cases participate. Generated requests add the required explicit partitions;
generated CC wrappers retain the source ABI and rebind changed object hashes.
A comparison of original test-function names found none removed, and every
top-level integration module is included in `tests/suite.rs`; ignored tests
remain mandatory in gate.toml. Against main-side code, binary-file requests
would fail as unknown options, the opcode test would not compile, and the
new Q CLI/metadata regressions reach missing handling. Those comparisons
were read from `git show main:<path>` without switching branches.

Verification used the brief's headless environment and job-local target/tmp,
`-j3`, four test threads and a timeout on every run: **53 Rust tests passed**.

- `timeout 300s cargo test --offline -j3 --test suite token_partitions09:: -- --test-threads=4`: 10 passed.
- Separate `timeout 120s cargo test --offline -j3 --test suite <filter> -- --test-threads=4` runs for `memory_files::`, `opcodes::`, `jit::`, `frame_domain::`, `coherent_typing09::`, and `ref09::typed_templates_capture_records_and_region_shapes`: 37 passed.
- Separate `timeout 180s cargo test --offline -j3 --test suite <filter> -- --ignored --test-threads=4` runs for `differential::published_d_and_g_cli_cases_differential`, `oracle_cleanup09::source_constructor_lifted_pair_has_one_nonemitting_carrier_per_step_phase`, `retained_instances_survive_segment_cancellation`, and `jit_transformation_boundaries_differential`: 4 passed.
- Separate `timeout 120s cargo test --offline -j3 --lib <filter> -- --test-threads=4` runs for `scheduler_keys_reject_unreadable_operands` and `created_register_tracking_rejects_a_partial_snapshot`: 2 passed.
- `timeout 60s python3 scripts/quality.py`: 5 tests passed, no regression. `git diff --check main...HEAD`: passed.
- `timeout 30s cargo fmt --all -- --check`: exits 1, formatting drift; P2 as instructed. No full repo/sibling gate was rerun.

Coverage: read closely the opcode/execution and catalog splits, engine/cache,
binary input, K91 binding/capture/cleanup, K84 metadata, ratchet and changed
assertions; skimmed repetitive fixture-path migrations, diagnostic edits and
historical evidence prose.

Provenance: implementer gpt-6.1-sol; reviewer gpt-6.1-sol, fresh session (D9).
