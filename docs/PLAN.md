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
| `certificate` / `interval` | Independent claim replay, typed verification indices, structural/library graph checks, inverse pair totality; rigorous interval/Taylor/matrix algebra; executable scheduled claims explicitly blocked on ref@0.8 | §4.5, §5.6, §12 |
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
4. **Devices, targets, lowering, schedules (in progress; blocked on 47t/cxg).** §8 target
   descriptions, launch-time lowering to the target gate alphabet,
   §11 schedule consumption verbatim-or-reject, resource accounting.
   Delivered: exact target AST/schema/hash, ordered matching and nonnested
   native templates; structured action/bundle/loop/local-image lowering;
   designated class-aware dirty pools; explicit accuracy request batches and
   achieved-error charging; native QRAM primitives; integrated actual
   leaf/classical/effect accounting exposed in CLI and immutable snapshots;
   schedule validation utilities and all 15 scheduler ABI exports.
   Full lifted/oracle rows and non-native memory comparator decomposition need
   the published ref library (47t), as do schedule ordinals/scratch, verbatim
   execution, default scheduled accuracy allocation, timing and reproducibility.
   No schedule is accepted/emitted while that library is absent. eps_decade
   indexing also lacks a definition (cxg). No capability is removed from scope.
   Current raw conformance: zero failures, unavailable cases retained; see PROGRESS.
5. **JIT tier (implemented; evidence in PROGRESS.md).** §5.7 transformations (inlining, structure
   recovery, dispatch re-indexing) proven extensional by the
   milestone-1 harness, then native codegen for CC and oracle
   segments; scheduling/pebbling policy stays in `bennett-sched`.
   Implemented: private row-preserving call-site clones, guarded CFG continuation
   recovery, independent canonical/private dispatch maps and block-local integer
   folding; native x86-64 Linux direct-threaded CC/R and F_B code with runtime
   helpers plus nine host integer kernels. Every individual pass, composition
   and JIT is gated by the original harness (1,235 comparisons per mode).
   Logical budgets, activation/history boundaries, object identities and
   observable label values remain reference-defined. Other hosts fail explicitly;
   native execution is opt-in via `BENNETT_VM_ENGINE=jit`.

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

## Certificate and report follow-up (2026-10-30)

`bennett-vm-abx` remains blocked on 47t, hsd and clw. Available work is
tracked separately by abx.1. No prior milestone is silently redefined to claim
full scheduled certificates or estimates.

Independent Rust replay accepts caller-resolved arguments/budget; table tuples
use ABI bundle order, dirty bundles in allocation order, then region cells.
Domains enumerate in their declared order, last component fastest. These API
conventions do not invent missing interchange/default contracts (clw).
`inverse_pair` and `borrow_frames` are verification references, not extra claim
kinds. Structural adjoints prove the table transformation, never an expected
oracle relation. Matrix algebra is separate from executable-claim acceptance.

Executable numerical/channel/expected-oracle claims, root-schedule contexts,
realisation construction and multiplicity composition require b-isa-spec-8r4.
Complete estimates additionally require hsd: resolved parameters, per-class
scalar-accessor encoding and nonterminating-rational decimal policy.
Source coverage belongs to bennett-driver; the VM returns the supplied §13
report as an independently owned JCS object, and rejects absent coverage.
NorthStar §5/§8 @cost, padded SQLite estimates and certificates remain in scope.
