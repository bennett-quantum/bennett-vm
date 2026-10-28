# bennett-vm

## What this repo is

The device runtime: classical-control tier, reversible tier with history layers and tapes, coherent tier via the predicated step machine, device models, branch-enumeration simulation, launch-time lowering to the target alphabet, and a JIT tier. Implements launch 0.3 (Bennett-ISA-Spec.md section 10).

It is one of several independent repositories in the Bennett reboot. It talks to
the others only through published specs.

## Where the spec is

Launch ABI: Bennett-ISA-Spec.md section 10 (launch 0.3). Machine model: section 3; instructions: section 4; regions: section 5; memory: section 7.
The conformance suites for the versions in SPEC_VERSIONS.toml live in bennett-isa-spec.
Spec sources of truth: Bennett-ISA-Spec.md and Bennett-NorthStar.md in the Bennett.jl repo
until bennett-isa-spec is published.

## Non-negotiables

0. MAINTAIN THE WORKLOG. Every session, prepend a "## Session log - YYYY-MM-DD - ..."
   block to the highest-numbered chunk under worklog/ (check: ls worklog | sort -r | head -1).
   Past ~280 lines, start a new chunk with the next NNN_ prefix. Record gotchas, surprises,
   failures and root causes that are not derivable from the diff. Edit chunks by hand.
1. FAIL FAST, FAIL LOUD. Assertions and explicit errors, not silent returns or defaults.
   Unsupported input crashes with context.
2. ONE IMPLEMENTER PER CHANGE. Review is batched over accumulated diffs, not per change.
3. RED-GREEN TDD. Write the test first, watch it fail, write the minimum to pass, refactor.
   Every change has tests; "runs without errors" is not a pass.
4. EXHAUSTIVE VERIFICATION against bennett-isa-spec's conformance suites for the versions
   in SPEC_VERSIONS.toml. Check actual outputs against known-correct answers.
5. COUPLED ONLY THROUGH PUBLISHED SPECS. Never share code across a contract boundary.
   This repo must be replaceable by a competitor's implementation of the same spec,
   and must itself be able to replace one.
6. NO GITHUB CI, NO REMOTE AUTOMATION. Local gates only: scripts/pre-push
   (install with scripts/install-hooks.sh). No .github/workflows, no scheduled runs.
7. Track tasks with beads (bd) if initialised in this repo; no TODO.md files.
   Never gitignore .beads/.
8. SKEPTICISM. Doubt subagent output, earlier work, your own assumptions, and the spec text
   itself; verify and reproduce.
