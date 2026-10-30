# Brief: re-baseline bennett-vm and bennett-targets against the current published ISA conformance suite

Two consumer repos pin the published ISA 0.8 conformance suite of `/home/tobias/Projects/bennett-workspace/bennett-isa-spec`
(HEAD 0b913da). The ISA repo added pending-0.9 witness cases today (each case.json carries an `unavailable` block
whose `reason` names the bead and says "pending 0.9"; see `conformance/README.md`, "Unavailable cases"). The suite is now
445 pass / 0 fail / 54 unavailable / 499 total when run with the reference binary. Both consumers went red on the moving
count, not on a defect. You are the sole implementer for this job in both repos; owned paths are listed below.

## Principle (both repos)

A consumer gate must stay fail-fast on real divergence and stop churning on upstream additions of
`unavailable` cases:

- pin the **pass** count exactly and **fail = 0** (a new case the consumer fails shows as FAIL and must be fixed,
  never relabelled; a new case the consumer passes raises the pass count and is a deliberate re-baseline);
- for **unavailable** cases, do not pin the total: assert that every UNAVAILABLE outcome belongs to a case whose
  `case.json` has an `unavailable` block with a non-empty `reason` naming a bead or staged deliverable, and that no
  case the consumer could run on ISA 0.8 is hidden there (the harness already turns a silently accepted probe into
  a FAIL); print the list of unavailable ids in the test output;
- record the observed rows and the new case ids in each repo's `docs/PROGRESS.md`.

Read each repo's `CLAUDE.md` (rules 1, 3, 5, 8, 10, 11). Rule 5: use the ISA suite only as published data and the
reference binary only as a black box; never read sibling source. Never edit the ISA repo.

## bennett-vm (`/home/tobias/Projects/bennett-workspace/bennett-vm`, HEAD 16caaca)

- `tests/conformance.rs`: replace the exact per-suite unavailable pins with the rule above (keep exact pass pins per
  suite and `all`; keep the "no FAIL line" assertion; keep exit code 2 while any case is unavailable). Red first:
  the current test fails on the count; the new test must fail if you corrupt one case's `unavailable.reason` in a
  scratch copy of the suite (`--cases PATH` exists on the harness) and pass on the published suite.
- Run in both engine modes as `docs/PROGRESS.md` documents. Gate: fmt --check, clippy -D warnings,
  `cargo test --offline`, `scripts/pre-push`, headless with TMPDIR under the repo's `.tmp`.
- Owned paths: `tests/conformance.rs`, `docs/PROGRESS.md`. Commit: `BEADS_HOOK_TIMEOUT=20 git -C /home/tobias/Projects/bennett-workspace/bennett-vm commit -- tests docs/PROGRESS.md`.

## bennett-targets (`/home/tobias/Projects/bennett-workspace/bennett-targets`, HEAD 610f481)

- `scripts/verify.py` currently raises `KeyError: 'E-topology-grid-09'` because its per-case rule table does not know
  the new suite-E cases. Make unknown cases a loud, classified outcome instead of a crash: a case with a published
  `unavailable` block is reported as such (with its reason) and is neither a pass nor a failure of the validator; a
  case without one that the rule table does not know is a hard failure ("unknown published case: <id>"). Add rule
  entries where the validator genuinely can evaluate the new fixtures on ISA 0.8 (the topology case's
  `target.json` must at least canonicalise and hash to its stated `target_hash` if the validator's schema admits the
  optional field; if 0.8 forbids unknown fields, the validator must reject it and the case stays unavailable, say which
  and cite the sentence).
- Re-run the full gate (`scripts/pre-push`), including the VM consumer comparisons now that the VM accepts
  small-exponent JCS numbers (bead b-targets-4kf should close if the divergence is gone; update
  `docs/verification.json` from the actual run, never by hand). Report exactly which acceptance blockers remain
  (topology semantics, eps_decade, ref library, reference CLI surface) and the gate's final exit code; the README
  says exit 2 for known acceptance blockers is expected, so the orchestrator needs the precise reason list.
- Owned paths: `scripts/verify.py`, `docs/verification.json`, `docs/PROGRESS.md`, `.beads/` via `bd` (export before
  committing). Commit: `BEADS_HOOK_TIMEOUT=20 git -C /home/tobias/Projects/bennett-workspace/bennett-targets commit -- scripts docs .beads`.

## Process

Headless, offline, project-local TMPDIR in each repo; no `cd` in compound commands; absolute paths; retry on
`index.lock`; do not push; never edit any other repository. Final message: commit hashes per repo, the observed
conformance rows before/after, the list of unavailable case ids, targets' gate exit code and remaining blockers,
and anything for the maintainer.
