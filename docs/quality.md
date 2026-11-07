# VM quality evidence

The earlier review findings (segment cancellation, spectator measurement,
conditional adjoints, dynamic classical rows, allocation lifetimes and phase
signs) already have fixes at this branch's starting commit, 805f0ec.

Verification uses headless execution, a job-local TMPDIR and CARGO_TARGET_DIR,
`cargo -j3`, and `--test-threads=4`. The existing regression tests were retained.

Prior-fix verification:

- `timeout 300s cargo test -j3 --lib exact::tests:: -- --test-threads=4`: 5 passed.
- `timeout 240s cargo test -j3 --lib segment_cancellation -- --test-threads=4`: 2 passed.
- `timeout 300s cargo test -j3 --test measurement_exact09 --test allocation_bindings09 --test classical09 -- --test-threads=4 --skip incoming_tokens_with_equal_tuples_keep_distinct_identities`: allocation, dynamic-row and exact-measurement regressions passed. The skipped token-identity differential belongs to the pinned-oracle gate.

The review's original red-first evidence is preserved in the closed issues;
this round adds no replacement implementation for already-fixed findings.

Binary region input: `cargo test -j3 --test memory_files -- --test-threads=4`
reproduced 2 failures (`unknown option --mem-file`) before implementation,
then passed all 3 tests. Coverage includes both engines, both ISA versions,
step enumeration, reversal, duplicate/unknown/bad-size bindings and the last
byte of a 64 KiB image. Payload assertions now use the published JSON integer
convention (small integers are numbers), retaining exact hand-derived bytes.

Default test split: an initial consolidated run reached the 180 s timeout
inside exhaustive oracle-cleanup expansion. Published-oracle comparisons,
published conformance, C ABI rebuilds and exhaustive source expansion now use
`#[ignore]`; the second gate command runs **all** ignored tests. No assertion
or case was removed. Integration modules link into one `suite` binary.

`/usr/bin/time -p timeout 180s cargo test --offline -j3 -- --test-threads=4`
passed in **51.54 s** (warm test build, line tables only). All sibling fixtures,
specs and binaries resolve through `tests/support/pins.rs` to the full SHA in
`gate.toml`; a wrong checkout is an explicit failure reporting both heads.
The pin was materialised with the pinned gate tool's `verify` command against
the main checkout. It is unchanged and currently not reachable from origin/main;
publication remains orchestrator work.

Coverage (default suite, before the structural refactors):
`timeout 600s cargo llvm-cov --offline -j3 --lcov --output-path .tmp/quality-vm/coverage.lcov -- --test-threads=4`
passed (44 unit + 310 integration; 72 gate-only ignored cases). Measured source
line coverage is **81.71% (23,850 / 29,189)**, summing LCOV LH/LF records for
`src/`. The resolver verified ISA pin fd626d37aba1f719c3b7b2059be46cbfaf462886.
This number excludes the mandatory slow gate, so it is a reproducible partial
measurement, not a full-contract coverage claim.

The five least-covered modules and reasons for the remaining gaps:

| module | covered / lines | reason |
| --- | --- | --- |
| sched_abi.rs | 0 / 226 | C consumers require a separate static-library build and run in the gate |
| ref_library/oracle_direct.rs | 42 / 139 | exhaustive row-carrier execution belongs to oracle_cleanup09 in the slow gate |
| ref_library/oracle_memory.rs | 113 / 264 | exhaustive memory-carrier reversal belongs to the same slow suite |
| qcli.rs | 159 / 275 | malformed CLI combinations and state display branches are incompletely covered; the larger differential suite adds command coverage |
| launch.rs | 568 / 961 | static-library C ABI consumers and oracle differentials run in the slow gate |

Test-quality pass: the three audit weak witnesses now check a negative large
region shape, the exact hand-derived recursive sum 1+3+7+11=22 through all
engines, and the captured external control/single nested scratch identities.
Focused `cargo test --offline -j3 --test suite -- <filter> --test-threads=4`
runs for `huge_regions`, `transformations_change_private_graph` and
`typed_templates_capture` passed (one test each). Binary-input tests retain
exact bytes, explicit success/failure and both engines/versions.
