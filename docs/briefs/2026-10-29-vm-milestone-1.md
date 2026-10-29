# Brief: bennett-vm milestone 1 (plan + object loader + CC tier + differential harness)

Repository `/home/tobias/Projects/bennett-workspace/bennett-vm` (Rust skeleton). You are the sole implementer of this
repo. It consumes ISA 0.8 only through the published spec (`../bennett-isa-spec/spec/Bennett-ISA-Spec.md`, council-
accepted) and the ISA repo's C headers (`../bennett-isa-spec/include/`); never copy or link code from the reference
interpreter crate (CLAUDE.md rule 5). The reference interpreter *binary*
`../bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref` (README "Commands": check, hash, run-oracle,
step-check, run-cc, reverse, run-action, run-program) is your differential oracle, used only through its CLI.

## Read first, in full
`CLAUDE.md`, `README.md`, `HANDOFF.md`, `SPEC_VERSIONS.toml`; `../bennett/docs/Bennett-NorthStar.md` §4 (why the VM
is the revolution; execution semantics by tier) and §5; ISA spec §3 (machine model), §4 (instructions), §5 (regions),
§7 (memory), §9 (number formats), §10 (launch 0.3), §14.1 (interpreter), §16 (object format, examples, hashes);
`../bennett-isa-spec/reference-interpreter/README.md` for the CLI you diff against; `../bennett-isa-spec/HANDOFF.md`
to know that ISA 0.9 is drafted (`spec/Bennett-ISA-Spec-0.9-draft.md`, not accepted): build on 0.8, and note in your
plan which 0.9 items (typed `fault C` in R, lproc token/classical params) will touch you.

## Milestone 1
1. `docs/PLAN.md`: the crate architecture for the whole CLAUDE.md scope (classical-control tier, reversible tier with
   history layers and tapes, coherent tier via the predicated step machine, device models, branch-enumeration
   simulation, launch-time lowering, JIT tier), milestones in order, and what milestone 1 delivers. Short, decisive.
2. Object loader + validator for ISA 0.8 objects (§16): parse, validate, compute identities/hashes exactly per the
   spec's envelope rule; `check` must agree byte-for-byte with `bennett-isa-ref check` on every example object in
   the spec and every fixture under `../bennett-isa-spec/reference-interpreter/tests/fixtures/`.
3. Classical-control tier executor (§3, §4 CC rows, §7, §10 launch) with budgets and faults as the spec reports them.
4. Differential harness (`tests/`): for every fixture and spec example, run the same inputs through your binary and
   through `bennett-isa-ref run-oracle` / `run-cc` / `run-program` and compare the JSON outcome fields the spec
   defines (code, payload, steps, faults, results); any divergence is a failing test that prints both outputs.
5. A `bennett-vm` CLI with the same command names and JSON output shapes as the reference interpreter for the
   commands you implement, so the harness and the dialect checker can swap binaries.

Red-green per feature; fail fast and loud; no silent fallbacks; nothing narrowed (rule 10): every CC-tier row the
spec defines is implemented or fails with an explicit "unimplemented: <row> (milestone N)" error naming the plan item.

## Process
Headless (rule 11): `DISPLAY`, `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`, `BROWSER` unset, `TMPDIR` under this
repo (create `.tmp/`). `cargo test --offline` (vendor nothing; std only unless a crate is truly needed, then say so).
`bd init` in this repo if `.beads/` is absent (rule 7), then `bd create` the milestone epic and tasks; `bd export -o
.beads/issues.jsonl` before committing `.beads/`. Keep `docs/PROGRESS.md` current (what is verified, by which test,
against which fixture). Commit your paths regularly: `git -C /home/tobias/Projects/bennett-workspace/bennett-vm commit
-- <paths>` (retry on index.lock); do not push; absolute paths only; never `cd` inside compound commands. Never edit
any other repository. Final message: commit hash, PLAN summary (10 lines), harness counts (fixtures diffed, agree /
diverge), every divergence found with the spec sentence that decides who is right, and the next milestone.
