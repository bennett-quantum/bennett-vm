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

0. WORKLOG, KEPT BY WHOEVER COMMITS THE SESSION. Each working session that changes the repo
   leaves one "## Session log - YYYY-MM-DD - ..." block at the top of the highest-numbered
   chunk under worklog/ (ls worklog | sort -r | head -1; past ~280 lines start the next NNN_
   chunk): what changed, gotchas, surprises, failures and root causes not derivable from the
   diff. When an orchestrator runs subagents, the orchestrator writes it; subagents keep their
   job's progress file and put surprises in bd (bd note / bd create) and never need to write
   outside their owned paths. A session that changes nothing writes nothing.
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
7. TRACK TASKS WITH BEADS (bd): bd ready / bd show / bd create; bd init when work starts in a
   repo that has no .beads/ yet. No TODO.md files. Never gitignore .beads/. No Dolt remote sync.
8. SKEPTICISM. Doubt subagent output, earlier work, your own assumptions, and the spec text
   itself; verify and reproduce.
9. GIT POLICY (maintainer, 2026-10-29). Any agent commits its own paths regularly
   (git commit -- <paths>; retry on index.lock); the orchestrator pushes after verified
   milestones and at least every couple of hours. This opts the repo into the Beads
   "team-maintainer" profile for commits only, not for pushes or Dolt sync.
10. NO NARROWING OF NORTH-STAR SCOPE (maintainer, 2026-10-29). Nothing may reject, refuse,
   fault, declare inert or omit a capability that bennett/docs/Bennett-NorthStar.md promises
   (thesis, sections 3-5, every rung of the section 8 demo ladder; Grover over unmodified
   SQLite is the core). A deferral may postpone mechanics only. If a contract truly lacks
   something, raise it as a blocker naming the exact change; never shrink scope to fit.
   See bennett/docs/plan/2026-10-29-path-to-north-star.md.
11. HEADLESS, NO /tmp ARTEFACTS. Never run LLVM -view-* / view-cfg / view-dom / view-callgraph
   passes or viewCFG-style helpers outside an interactive human session (use the dot-* variants).
   Batch and agent commands run with DISPLAY, WAYLAND_DISPLAY, DBUS_SESSION_BUS_ADDRESS and BROWSER
   unset and TMPDIR set to a project-local or job directory (2026-10-28: ~80,000 .dot files reached
   the maintainer's editor via xdg-open).

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:1105d646 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->
