# VM slice 3 review — round 1

Reviewed **bfa7b5b8860e485155f87b38224dc2d6eda31e52** against **dad1cdc** on job/vm-slice3.
Reviewer: gpt-6.1-sol. Completed 2026-11-08T13:35:45Z. Scope: sparse classical pages and displaced-byte journals, bennett-vm-0jm; second-merger review of shared cc.rs after vm-1wf.

Read the review/implementation briefs, COMMON.md, T3CHILD.md, CLAUDE.md, AGENTS.md, implementation PROGRESS.md/MEASUREMENTS.md, the s1-alloc report in full, and design section 5 (Slice 3) and section 7. No production edits or bd writes. The finding below requests an orchestrator-filed bead.

## Re-runs

Evidence: `.tmp/vm-slice3-review/` (`D` below). Headless, project-local TMPDIR/CARGO_TARGET_DIR. Fresh machine profile: 64 CPUs, 55,401,324,544 available bytes; reserve 16 CPUs/16 GiB and allow 4 GiB per worker, hence **8 workers**. Guards use the existing measured `.tmp/vm-slice3/timeouts.toml` copied beside the review guard, factor 1.0, margin 10; UNKNOWN = 124. Each receipt records full argv, wall/child CPU, stdout/stderr. No guard tripped. The orchestrator owns the full gate on bfa7b5b; no full VM/sibling gate was run here.

`python3 .tmp/vm-slice3-review/run_checks.py` completed successfully; the required baseline failures are checked explicitly. Exact argv reside in `receipts/<label>.json`.

| Receipt label | Result | Wall / CPU seconds |
|---|---|---:|
| `head-memory` | 9 passed; 0 AVL nodes/operations, 8,000 journal bytes, 17,000 probes at each history size; sparse allocation/global: 4 pages. | 23.895 / 32.564 |
| `head-cc` | 24 passed. | 0.617 / 0.746 |
| `head-suite` | 324 passed, 73 ignored. | 57.004 / 161.045 |
| `head-differential` | 39 passed, none ignored. | 112.711 / 280.167 |
| `red-memory_journal_retention_tracks_changed_bytes` | Expected red (101): 8,000 AVL nodes/operations per 500 stores at both history sizes; zero original journal bytes. | 16.415 / 24.507 |
| `red-large_sparse_image_uses_only_touched_pages` | Expected red (101): 64 resident byte units versus 4 required pages; readbacks passed. | 0.032 / 0.028 |
| `oracle-head` | PASS: 64 seeds × 96 actions, 3,699 span writes, 1,263 branch undo deltas; saved/current versions match byte map. Rewound journal: 16 bytes. | 2.020 / 2.024 |
| `retention-old` | Expected red (101) at 375e8bf: 48 retained bytes after rewind versus 16 required. | 0.002 / 0.001 |
| `closed-run-retention` | New P2 reproduced (101): dropped run histories leave 32, 64, 96 original bytes in shared resources. | 16.772 / 25.129 |
| `measurement-receipts` | PASS: all 16 attribution + 16 plain receipts and output identities. | 7.787 / 7.842 |
| `quality` | PASS: self-tests and baseline. | 0.466 / 0.425 |
| `format-owned` | PASS: all four touched Rust files. | 0.115 / 0.089 |

Head commands:

```sh
cargo test --offline -j8 --lib -- --nocapture --test-threads=1 memory_journal_retention_tracks_changed_bytes large_sparse_image_uses_only_touched_pages sparse_global_image_allocates_only_nonzero_pages sparse_pages_preserve_wide_cross_page_versions_and_unrelated_bytes overlapping_span_journal_restores_only_changed_row_bytes faulted_store_retains_no_displaced_bytes unchanged_writes_retain_original_bytes_and_release_long_journal rewound_memory_releases_discarded_journal_suffix journal_forks_preserve_interleaved_bytes_when_uncall_is_reversed
cargo test --offline -j8 --lib cc:: -- --test-threads=8
cargo test --offline -j8 --test suite -- --test-threads=8
cargo test --offline -j8 --test suite differential:: -- --include-ignored --test-threads=8
python3 scripts/quality.py
rustfmt --edition 2024 --config skip_children=true --check src/cc.rs src/classical_memory.rs src/lib.rs tests/step_scaling.rs
```

**Red-first reproduction:** `base/` is a dad1cdc archive with only cc.rs test counters and step_scaling.rs tests from 19e386b. The counter wraps the actual persistent-map operation and reads path-copy allocations; production behavior is unchanged.

```sh
cargo test --offline -j8 --manifest-path .tmp/vm-slice3-review/base/Cargo.toml --target-dir .tmp/vm-slice3-review/target/base --lib step_scaling::memory_journal_retention_tracks_changed_bytes -- --exact --nocapture --test-threads=1
cargo test --offline -j8 --manifest-path .tmp/vm-slice3-review/base/Cargo.toml --target-dir .tmp/vm-slice3-review/target/base --lib step_scaling::large_sparse_image_uses_only_touched_pages -- --exact --nocapture --test-threads=1
```

Head reproduces 500 page allocations and 2,056,000 copied memory/journal payload bytes per 8,000 committed store bytes (257 copied bytes per write byte, excluding directory/address metadata). The 2^25-byte global initializes four nonzero pages without journals. Sparse readback/rewind, wide unaligned i80/i128 spans, immutable versions, zero-page removal, overlapping writes, faulted stores and interleaved uncall forks pass.

**Independent journal check:** `rustc --edition=2024 -O "$D/memory-oracle.rs" -o "$D/memory-oracle"` then `"$D/memory-oracle"` compares the actual memory implementation against a BTreeMap byte model through branches, cross-page writes, zeros, snapshots and undo. Scratch copies add test-only journal introspection. The same retention assertion on 375e8bf (`memory-retention-old.rs`) fails 48 != 16; head returns 16.

**Fresh identity after rebase:** base/head release builds used `cargo build --release --offline -j8 --manifest-path <base-or-head>/Cargo.toml --target-dir "$D/target/<stage>-release"` and separately linked the unchanged phase_probe.rs; exact build/link argv are in receipts. `python3 .tmp/vm-slice3-review/identity.py` reran the r3-stepping suite fixture in both stages × interpreter/JIT × CLI/ABI, checking fixture results, console, fault and target-row count.

All four complete CLI stdout streams are byte-identical, SHA-256 `e71d8c7ec46f148e7707008f59a6898819bcd889a5ce6917546d69beaad039f1`. All four complete ABI snapshots have **77,536,709 bytes**, SHA-256 `213786b56204164c3bb5a1374a08a1cd39bf1192a9be6767504405fcd3681501`; hash/size also equal the author's pre-rebase receipts (`identity.json`). This exercises 225,956 rows with checked bennett.argv reply and target accounting. The 39 differential tests separately cover transformation modes, fault boundaries, saved replies, history cancellation, prefix/full reversal and oracle memory restoration.

## Shared cc.rs and quality scope

Receipt: `git diff -U0 dad1cdc..bfa7b5b -- src/cc.rs` (`cc.diff`, `scope.json`). Every zero-context hunk:

| Base lines | Head lines | Enclosing region | Granted? |
|---|---|---|---|
| 93 | 93 | `Data.memory` field | Yes |
| 335 | 335 | `Data::new`: initialize memory | Yes |
| 362 | 362 | `Data::new`: initialize globals | Yes |
| 415 | 415–416 | `Data::read` | Yes |
| 419–427 | 420–424 | `Data::write` | Yes |
| 445–446 | 442–444 | `Data::digest` | Yes |
| 2041–2045 | removed after 2038 | `Data::undo_delta`: remove byte-map restoration | Yes |
| after 2047 | 2041 | `Data::undo_delta`: restore displaced spans | Yes |

No hunk touches `Handle`, `Data::new_handle`, qreg.concat, cmd_program plumbing or the measure call site. Data::valid/memory_eq and allocation helpers are unchanged. The only lib.rs change registers the private module. Changed files are exactly cc.rs, classical_memory.rs, lib.rs, step_scaling.rs and docs/quality.md.

**Rebase conflicts: 0**, per review brief. Independently, `git range-diff 773d7be..f80ffc4 dad1cdc..bfa7b5b` marks all three patches `=` (7081857→19e386b, b85bf58→375e8bf, f80ffc4→bfa7b5b). This review performed no rebase.

Rule-3 receipt `git diff dad1cdc..bfa7b5b -- docs/quality.md` is saved as `quality.diff`. Parsed comparison confirms only cc.rs branch_mass/over_100 and the added classical_memory.rs row changed. Unowned rows and surrounding prose are unchanged.

## Measurement honesty

Reran the author's verifier from a scratch copy, changing only its input root/output destinations to preserve author evidence. It validates all 16 accepted profiles and 16 plain commands, request/sample conservation, repeat totals, zero unmatched frees, complete CLI outputs and snapshot hash/length. All 16 tool/binary/fixture provenance hashes independently match (`provenance-check.json`). The profiler-startup SIGPROF and superseded candidate remain excluded.

The stated conclusion is supported: fewer allocation requests, more requested bytes, increased allocator sample share, and no established net speedup. Page copying, directory versions, journals and the diagnostic address set are disclosed. No whole-rung/Amdahl gain is claimed. The finding below adds a lifecycle retention cost. Long instrumented commands were receipt-verified, not all reprofiled; fresh release identity and source work checks were rerun on rebased code.

## Numbered findings

1. **P2 — Closed run histories remain reachable through shared memory's journal.** `src/classical_memory.rs:191–196` links each displaced span to the entire previous journal. RuntimeRun::export copies that tail into Resources (`src/cc.rs:2251`); closing/dropping the last run destroys its frames but leaves the shared tail reachable. A long-lived loaded program therefore accumulates original bytes and one node per historical store across completed, closed runs with a constant resident footprint and no remaining reversible history.

   Reproducer: bfa7b5b scratch archive plus one cc.rs test, `reviewer_closed_runs_release_displaced_spans`. Each of three runs performs 32 alternating stores to the same pinned byte, exports through normal sync/step/export, and is dropped with no remaining runtime/checkpoint. The sole current byte stays `(16, 2)`; retained original bytes are **[32, 64, 96]**, versus **[0, 0, 0]** once all histories are discarded. Dropping the runtime matches bennett_run_close's removal; that function does not prune memory journals.

   Exact command: `cargo test --offline -j8 --manifest-path .tmp/vm-slice3-review/head-lifecycle/Cargo.toml --target-dir .tmp/vm-slice3-review/target/lifecycle --lib cc::reviewer_closed_runs_release_displaced_spans -- --exact --nocapture --test-threads=1`. Exit 101; evidence `receipts/closed-run-retention.{json,stdout,stderr}`. Production worktree unchanged.

   **Orchestrator bead request:** reclaim spans once no live run/history needs them, with a repeated completed-run-close regression at fixed memory footprint; preserve active/interleaved reversal. No wrong result, broken rewind, build failure or scope violation was found, so this is P2 under the supplied severity rule. No repair round requested. No nit findings.

Disposition: ACCEPT with P2 beads
