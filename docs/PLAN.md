# bennett-vm plan

Contract: ISA/object/launch/sched/cert/coverage/target **0.8**
(`../bennett-isa-spec/spec/Bennett-ISA-Spec.md`, council-accepted).
This crate is coupled to every other repo only through that spec and
the C headers it publishes (CLAUDE.md rule 5). The reference
interpreter is used solely as a CLI oracle; no code is copied or
linked from it. Std-only Rust; a new dependency needs a stated reason
in `docs/PROGRESS.md` first.

## Crate architecture

| Module | Owes | Spec |
|---|---|---|
| `sha256` | SHA-256 (std has none), test vectors | §9.2 |
| `bigint` | arbitrary-width bit-vectors (oracle widths exceed `u128`, e.g. 257) | §3.3, §14.1 |
| `json` | JSON parse + RFC 8785 JCS emission for metadata lines | §9.1, §16.1 |
| `object` | §16 text parser: header, sections, functions, blocks, instructions, raw line numbers, canonical function bytes | §16 |
| `validate` | lexical/grammar/schema/layout/width/name/terminator/call-graph checks; every rejection cites its rule | §3.3, §5, §6 (surface), §7, §9, §16 |
| `identity` | semantic envelopes, SCCs, interface/closure/object hashes, stored-hash verification | §9.2 |
| `oracle` | §14.1 reference interpreter (lifted rows, §3.7 activations, §7.2 memory, budgets/faults); defines `F_B` for `run-oracle` and for CC/R `oracle_call` | §4.2, §5.2, §14.1 |
| `action` | small state-vector simulator for Q actions reached from CC/R region calls; basis/branch execution of permutation actions for `lift.perm_call` | §4.3–§4.7, §5.3, §14.2 |
| `cc` | classical-control + reversible executor: dynamic SSA, §7.1 memory, §7.3 devices/tapes, §7.4 history/`uncall`/reversal, `qreg` handles, `hostcall`, `measure` | §3.4, §4.1, §7 |
| `launch` | one-shot §10 launch behind `run-program`; later the full opaque-object C ABI | §10 |
| `main` | CLI with the reference interpreter's command names and JSON shapes for implemented commands | §10, ref README |

Milestone 3 adds `step`/`stepcli` (literal §3.6/§3.7 machine), `exact`
(symbolic amplitudes), `qcli` and `qvalidate`. Later modules:
`target` (§8 target
descriptions + launch-time lowering), `sched` (§11 verbatim schedule
consumer), `jit` (§5.7 extensional optimisations, then codegen).

Invariants held from milestone 1: fail fast with the cited rule; no
silent fallbacks; an unimplemented row fails as
`unimplemented: <row> (milestone N)`; classical control never
inspects a wire value except through predication/`measure`
(NorthStar §4.2); history is retained so reversal is LIFO replay of
frames, never re-execution.

## Milestones

1. **Loader + CC tier + differential harness (completed at 188ab05).**
   `docs/PLAN.md`; object loader/validator and identities with
   `check` byte-identical to `bennett-isa-ref check` on the §16.3
   examples and every reference fixture; the CC/R executor above
   (§4.1 rows, §4.7 region calls, §7, §10 one-shot launch) plus the
   §14.1 oracle interpreter those region calls and `run-oracle`
   require; CLI `check hash run-oracle run-cc run-program reverse`
   (CC/R form); `tests/` differential harness diffing every fixture
   and spec example against the reference binary on the
   spec-defined outcome fields, printing both outputs on divergence.
   Explicitly not yet: literal §3.6 machine (`step-check`,
   oracle-form `reverse`), standalone `run-action` with exact
   amplitudes, §8 targets, §11 schedules, §12 certificates, JIT.
2. **Reversible tier + full launch ABI (implemented; evidence in PROGRESS.md).** The complete §10 C API as a
   library (runs, `SUSPENDED`/`HOST_REQUEST` states, wait/reply,
   reverse eligibility, tape iterators, estimates/coverage stubs
   that fail loud until milestones 4–5), single-stepping and
   snapshot APIs over the §7.4 history, and device-model hardening.
3. **Coherent tier (implemented; evidence in PROGRESS.md).** Literal §3.6/§3.7 step machine with derived
   widths, prefix reversal, `step-check`, exact-amplitude
   `run-action`, §5.6 borrow frames, and branch-enumeration
   simulation of permutation segments (NorthStar §4.2).
4. **Devices, targets, lowering, schedules.** §8 target
   descriptions, launch-time lowering to the target gate alphabet,
   §11 schedule consumption verbatim-or-reject, resource accounting.
5. **JIT tier.** §5.7 transformations (inlining, structure
   recovery, dispatch re-indexing) proven extensional by the
   milestone-1 harness, then native codegen for CC and oracle
   segments; scheduling/pebbling policy stays in `bennett-sched`.

## ISA 0.9 watch list (draft, not accepted; build stays on 0.8)

- Typed `fault C` in **R** functions: touches `validate` (a new R
  terminator/row), `cc` fault records, and CLI `faults` output.
  Today `fault` is a lifted terminator only.
- Lproc token/classical params (draft Q14): touches `oracle`
  pseudo-location name resolution, §3.7 activation records (frame
  sizing and stack cells), signature schemas in `validate`/
  `identity`, and label/index derivation. Highest-risk 0.9 item for
  this crate because it changes `R_f` and every recorded hash.
- Any 0.9 envelope change rehashes every fixture: expect a
  flag-day `SPEC_VERSIONS.toml` bump, never a mixed-version mode.

## Verification

Red-green per feature. `cargo test --offline` headless
(`DISPLAY`/`WAYLAND_DISPLAY`/`DBUS_SESSION_BUS_ADDRESS`/`BROWSER`
unset, `TMPDIR` = this repo's `.tmp/`). The differential harness is
the gate: a fixture counts as agreeing only when the compared fields
match the reference binary exactly, and divergences are reported in
`docs/PROGRESS.md` with the spec sentence that decides who is right.
