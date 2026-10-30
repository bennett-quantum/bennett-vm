# Brief: JCS number canonicalisation (bead bennett-vm-ciz) and the conformance baseline

Repository `/home/tobias/Projects/bennett-workspace/bennett-vm` (HEAD a70574d, main). You are the **sole implementer**
for this job. Two defects block the repo's own gate and a sibling's milestone; fix both.

## Read first

1. `CLAUDE.md` (rules 1, 3, 4, 5, 8, 10, 11). Rule 5: you implement from the published ISA 0.8 text
   (`/home/tobias/Projects/bennett-workspace/bennett-isa-spec/spec/Bennett-ISA-Spec.md` §8.1, §9, §13 as relevant)
   and never read or copy code from `../bennett-isa-spec/reference-interpreter`, `../bennett-targets` or
   `../bennett-circuits`; their CLI binaries and published fixtures may be invoked as black boxes.
2. `docs/PROGRESS.md` (top entry), `HANDOFF.md`, `bd show bennett-vm-ciz` (run `bd` from the repo root with
   `BEADS_HOOK_TIMEOUT=20`).
3. `src/json.rs` (`canonical_number`, line ~125, and the JCS emission/comparison around it), `tests/conformance.rs`.

## Task 1 — bead ciz: RFC 8785 number serialisation

§8.1 requires RFC 8785 (JCS) canonical bytes. JCS §3.2.2.3 serialises numbers exactly as ECMAScript
`Number::toString` (ES2019 §7.1.12.1): shortest round-trip digits; plain decimal for 10^-6 ≤ |x| < 10^21; exponent
form otherwise (`1e-8`, `2e-7`, `2e-8`, `1e+21`); `-0` becomes `0`; NaN/Infinity are errors. The VM currently rejects
`1e-8` / `2e-7` / `2e-8` as non-canonical (found by bennett-targets milestone 1; its witness file is the grid projection
target it feeds to `bennett-vm run-program registers.bisa views --target <file>`, see
`/home/tobias/Projects/bennett-workspace/bennett-targets/docs/verification.json` → `vm_divergences`, and the
bennett-targets validator at `validator/target/debug/bennett-targets-validator canon <file>` produces the canonical
text if you need a second opinion on expected bytes; do not read its source).

- RED first: unit tests in `src/json.rs` (or `tests/`) for both directions, serialising and accepting as canonical:
  `1e-8`, `2e-7`, `2e-8`, `1e-7`, `0.000001` (canonical: `0.000001`), `1e-6` (non-canonical spelling of the same
  value), `1e+21`, `1e21` (non-canonical: ES prints `1e+21`), `123456789012345680000` (canonical, < 10^21),
  `0.1`, `1.5e-7`, `-0`, `5e-324`, `1.7976931348623157e+308`, integers; plus negatives for leading zeros, `+`,
  trailing zeros, uppercase `E`, and values that do not round-trip.
- Implement the shortest-round-trip digit generation yourself (std-only crate: no new dependency); Rust's
  `f64::to_string`/`{:e}` give shortest round-trip digits you can re-layout into the ES form, but verify every
  boundary case above against the ES algorithm, not against Rust's formatting choices.
- Re-run the exact targets witness through the VM CLI and record the result in PROGRESS.md.

## Task 2 — tests/conformance.rs baseline

ISA commit 91aa124 ("Draft resolved estimate and sampled replay contracts for ISA 0.9") changed the published
conformance suite; `tests/conformance.rs` pins the per-suite rows (`| all | 445 | 0 | 46 | 491 |`) and now fails.
Run the suite, inspect **which** cases changed and why (diff the suite directory against the previous ISA commit with
`git -C /home/tobias/Projects/bennett-workspace/bennett-isa-spec log/diff -- conformance`), and update the pinned rows
to the published counts only where the new cases are genuinely `unavailable` under the documented reasons (ISA 0.9
contracts not yet published; the beads 47t/hsd/clw/cxg). A new case that the VM could pass on ISA 0.8 must be made to
pass, not relabelled. Any `FAIL` is a defect to fix. Record the before/after rows and the new cases' names in
PROGRESS.md.

## Gate (headless, offline, project-local TMPDIR), all must be green before each commit

```sh
mkdir -p /home/tobias/Projects/bennett-workspace/bennett-vm/.tmp
env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u BROWSER TMPDIR=/home/tobias/Projects/bennett-workspace/bennett-vm/.tmp \
  cargo fmt --manifest-path /home/tobias/Projects/bennett-workspace/bennett-vm/Cargo.toml -- --check
# same env: cargo clippy --offline --all-targets -- -D warnings ; cargo test --offline ; and
/home/tobias/Projects/bennett-workspace/bennett-vm/scripts/pre-push
```

Run the test suite in both engine modes if PROGRESS.md documents that practice (`BENNETT_VM_ENGINE=jit`).

## Owned paths and process

- Write only `src/`, `tests/`, `docs/PROGRESS.md`, `.beads/` (via `bd`), and this brief's progress notes. Never
  `worklog/`, `HANDOFF.md`, `CLAUDE.md`, sibling repos.
- Commit owned paths after each green task (`BEADS_HOOK_TIMEOUT=20 git -C /home/tobias/Projects/bennett-workspace/bennett-vm commit -- src tests docs/PROGRESS.md`;
  retry on `index.lock`). Before committing `.beads/`: `bd export -o .beads/issues.jsonl`. Close ciz with `bd close`
  only after the gate is green. Do not push. No `cd` in compound commands; absolute paths.
- Final message: commit hashes, the before/after conformance rows with the names of the changed cases, the test counts
  in each mode, the targets witness result, and anything that needs the maintainer (file a bead).
