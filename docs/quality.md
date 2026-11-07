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
