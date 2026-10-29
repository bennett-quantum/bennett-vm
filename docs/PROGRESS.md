# Progress

2026-10-29: resumed the interrupted implementation at eb20fff. Read the full
brief and required 0.8 contract; the reference is accessed only as a binary.
No new dependencies (std-only, offline).

Verified with `cargo test --offline` (headless, project-local `.tmp`):

- `check_and_hash_byte_identical_on_every_fixture`: all 12 published fixtures,
  24 byte-identical CLI comparisons, including the two §16.3 example fixtures.
- `run_oracle_differential`: 45 comparisons, including 257-bit values, arithmetic
  faults, recursive stack/depth budgets, nested oracles and mapped memory.
  Inherited work did not compile: stale constructor fields were removed; all
  inherited oracle cases then passed.
- `classical_control_and_launch_differential`: seen red at the CC stub, now
  green on 45 command cases over classical.bisa and registers.bisa. Checks
  dynamic SSA traces, nested cancellation, all four devices, fault atomicity,
  handle allocation/slicing/concatenation, measurement, and launch eligibility.
  Successful JSON comparisons include every field except opaque memory digests;
  the ISA does not prescribe their serialization. Diagnostics compare exit status.

In progress: broaden execution coverage to every fixture and independent edge
cases, harden inherited action execution, final CLI documentation and local gates.
No semantic divergence established yet. Memory digests use a VM-specific canonical
encoding of the complete allocation table, bump pointer and nonzero bytes.

Expanded gate: 12 integration groups and 4 unit tests pass; 553 differential
comparisons agree (535 JSON outputs, 18 diagnostics), over all 12 fixtures,
both independently extracted §16.3 objects, all 57 oracle definitions and all
13 action definitions. Added static CC/R type/SSA validation after malformed
widths were accepted, fixed nested oracle `in`-memory restoration (§5.2), and
included control state in launch equality. Clippy now passes with `-D warnings`.
Final history audit in progress: region reversal must apply the recorded
adjoint/involution rather than restore a simulator wavefunction snapshot.
