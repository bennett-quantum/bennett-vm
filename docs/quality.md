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
