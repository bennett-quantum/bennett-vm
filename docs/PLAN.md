# bennett-vm plan

Contract: accepted ISA **0.9** (`../bennett-isa-spec/spec/Bennett-ISA-Spec-0.9.md`),
with byte-for-byte **0.8** compatibility (`../bennett-isa-spec/spec/Bennett-ISA-Spec.md`).
The milestones below retain their historical evidence; Beads tracks the current
0.9 implementation and admission work.
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

## ISA 0.9 watch list (historical draft-era planning)

ISA 0.9 is now accepted. These bullets record the earlier migration analysis;
the active 47t/hsd/clw beads and dual-version compatibility requirements govern
the current work.

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

## Host memory binding design, proposal 0.1 (bennett-vm-8v0)

This is a VM design and integration-vector specification for audit F23, not a
published launch ABI or an implemented backend. Publication and implementation
must follow ISA F04 (`b-isa-spec-x1y`), ISA F08 (`b-isa-spec-6ma`), dialect F15
(`b-dialect-spec-5dl`) and dialect F16 (`b-dialect-spec-dce`). Their passive
issue exports still report all four open. The accepted ISA 0.8 and 0.9
by-value interfaces retain their existing signatures, tags and behavior.

The boundary follows the [mixed-object scope](../../bennett-isa-spec/docs/notes/2026-11-02-rev2/fat-object-scope.md)
and [F23 audit](../../bennett-isa-spec/docs/notes/2026-11-02-4ht-audit/AUDIT.md).
A native host binary keeps its own entry, argv/envp, libc and unrelated globals.
A launch chooses one embedded device program, root and certificate; neither its
object hash nor its certificate inventory denotes the whole native binary.
Native and device functions can share registered storage without promoting
native main or every pointer-sharing caller into the device program.

### Current boundary and proposed extension

The argument variants in [launch.rs](../src/launch.rs) are Int/F64/Qreg/Token;
`bennett_run` has no ordinary Ptr binding. `Resources::new` and `Data::new` in
[cc.rs](../src/cc.rs) create device state from the ISA layout, images and
allocation table. A CLI numeric pointer is an address in that device model.
Copied argument buffers, hostcall payloads and hex memory images do not register
a live native allocation. These are interface observations, not a reproduced
failure of an existing native-sharing implementation.

The proposed extension has its own negotiated profile/version and explicit
registration and invocation objects. Symbol names, numeric tags, C structure
layouts and status values remain for F04/F08 publication; this proposal adds
none to current headers or exports. An unsupported version fails negotiation
before a run or lease is created. A new pointer argument is a registered-memory
capability plus an offset, with validated native/device address and layout
information; an integer argument alone never grants native access. Scalar and
aggregate argument/result marshalling uses the published target ABI, including
field offsets, pointer kinds, padding policy, widths, byte order and floating
point bit patterns. It does not guess those properties from host Rust types.

| Record | Required content and invariant |
|---|---|
| Registration | Allocation owner, canonical allocation identity, generation, base/extent, permissions, liveness authority, target layout, memory/backend profile and allocator policy. Registration is a lifetime protocol, not a claim that a numeric address is readable. |
| Program binding | Registration identity/generation mapped into one loaded device program and its layout. Distinct embedded programs keep distinct ledgers; a bare id from program A is invalid in B. A common registration coordinator arbitrates leases on their shared native storage. |
| Pointer slot | Registration capability and checked offset, including null/one-past rules, plus the declared data/function/opaque pointer kind. The marshalled Ptr bits must satisfy the selected mapping profile. |
| Invocation binding | Selected object/root/certificate identities, resolved arguments, layout/mapping version, registration generations and entry epochs, run ownership and replay profile. Runtime addresses belong here, not in immutable ISA-object bytes. |
| Lease/checkpoint | Read-only or exclusive read/write range ownership, entry allocation metadata, entry memory version, and the run's ordered write/allocation history. Unload, release and reverse respect this owner. |

A registration accepts runtime-owned storage or an allocation whose owner can
hold its lifetime and supply the published liveness/lease contract. Extent checks
alone cannot establish that arbitrary C storage remains alive. C callers must
honor that contract; an owner hook, safe-language borrow or an accepted platform
mechanism supplies it without requiring the host code to become ISA code.
A generation prevents stale capabilities from becoming valid after address reuse.

### Address identity, aliases and allocators

The initial proposed profile preserves native pointer bits for registered
storage when native and target pointer width, representation and layout agree.
Interior pointers are one allocation identity plus offsets. Overlapping argument
ranges retain one underlying storage identity and one coordinated lease; writes
through one permitted alias are visible through the others. A second copied
buffer is not an alias implementation. Source `noalias` obligations, when present
in the accepted binding description, remain obligations rather than a blanket
ban on legitimate aliased Ptr arguments.

Bounds and nonwrapping address arithmetic are checked before converting an ISA
address to a host-sized offset. A registered range cannot silently overlap the
null area, a different live allocation or device-private storage. Shared globals
have an explicit common binding. Unrelated native globals stay outside device
layout and proof inventory. F16 must publish the association and relocation rules
for other layouts; silent pointer swizzling, changed pointer comparisons/hashes,
or copying unidentified integer words as pointers are not substitutes.
Function pointers require an explicit native-address-to-device-dispatch mapping;
registering data bytes grants no permission to call native code from a device
closure. Existing CC hostcall and device callback/re-entry rules stay separate.

Device-private bump allocations retain their current semantics. A native-visible
escaping allocation needs an accepted host-compatible allocation/transfer profile
covering native free/realloc, aliases, allocation-table metadata, budgets and
reverse history. Mapping a private device integer into a returned native pointer
is not that profile. F08/F15 must specify how allocator and shadow-stack entry
state is imported and how escaping storage becomes host-owned. This explicitly
retains pointer-rich allocation and native ownership in scope; caller-provided
borrowed buffers are not the final capability boundary.

### Backend access and coherent execution

The implementation boundary is a checked memory backend shared by the interpreter
and JIT. Device-owned sparse memory remains one backend. A future registered-host
backend resolves an address to a validated registration, generation and offset,
checks its lease/liveness/permissions, and only then performs a bounded access.
There is no native pointer dereference in today's integer-address backend.
JIT memory operations must use the same checks or an accepted guarded lease/epoch
path, with invalidation on release or rebinding; cached integer bits do not carry
a lifetime proof. No run-wide atomicity is inferred from one checked load/store.

CC/R classical memory sharing and coherent device memory are distinct storage
kinds. A lifted/oracle construction imports a consistent, registered entry image
and equivalent allocation/shadow-stack metadata before private allocations, then
uses its prescribed private forward/reverse machine. It does not perform an
observable native RAM load at a quantum address or expose unmeasured coherent
state as C bytes. Existing snapshot/replay remains an admitted construction;
F08 must also provide the live native-sharing route, with explicit conversions
where needed, rather than require universal host promotion. A numeric CLI image
or a successful snapshot replay is not evidence of a live host binding.

### Visibility, reversal and effects

The proposed mutable profile takes an exclusive lease before execution and holds
it through run close; compatible read-only leases can coexist. Native mutation,
free/realloc, a conflicting launch and unregister cannot proceed while that lease
is held. Writes may already reside in the registered pages during execution;
visibility is synchronized at the lease boundary for its participants, not an
assertion of hardware-atomic publication of an arbitrary memory graph. Hashing
bytes once cannot detect every unsynchronized raw write. The accepted owner and
interference protocol must state which accesses it excludes or detects.

A synchronous native adapter waits for completion, obtains results, then commits
and closes before returning control that may access the shared storage. While a
completed eligible run still owns its lease, reverse restores that run's entry
bytes, aliases, allocation metadata and ownership history before close. It never
rolls back native main, host-only libc, a sibling run or host edits made before
this invocation. Each new launch observes the current native state and records a
new entry epoch; replay cannot reuse an earlier launch's snapshot silently.

A failing access changes no state for that operation. Earlier valid writes remain
part of the failed run's prefix and its journal. Eligible reversal restores that
prefix; an irreversible failure must report its actual committed prefix when the
published close policy releases ownership. Measurement and other existing reverse
restrictions still apply. Stale/revoked storage or detected interference fails
before a native access or reverse write, through negotiated device/launch status,
not an unchecked pointer access or a process-aborting assertion. F04/F08 must
publish these state/visibility rules and status encodings before implementation.

The journal association includes the selected device run/root, binding and entry
epoch. Device R effects and CC host replies keep their existing ordering and
exactly-once replay rules. Ordinary native I/O is not silently added to the R tape,
and reverse does not retract observed external events. Budgets and estimates bind
that device invocation and its actual replay work; the native process acquires no
implicit global budget. Host registration/shim costs need separate reporting, not
invented oracle units. Quantum-register ids and token identities stay in their
own program/run namespaces and are never reinterpreted as native pointers.

### Bounded integration vectors for the accepted implementation

These are planned acceptance vectors, not executed tests or claims that the
backend exists. Each vector must run through interpreter and JIT after the four
contracts are published. A native comparison uses the same selected materialised
closure, target layout and entry allocation graph; sequential baseline restoration
must preserve pointer identity when comparing pointer-valued or address-hashed
results. It does not compare a whole native process against one device root.

| Vector | Fixture and determining observations |
|---|---|
| HMB-01 by-value compatibility | Run the existing scalar, float-bit, qreg and token launch corpus unchanged. Compare statuses, results, tapes, resource ownership and reverse outcomes; the new profile is not selected. |
| HMB-02 host edits between calls | Register four bytes `[10,20,30,40]`. First launch adds 3 to byte 1 and closes, exposing `[10,23,30,40]`. Native code writes byte 0 to 99. The next launch XORs byte 0 with 1; reverse before close restores `[99,23,30,40]`, not the first launch's baseline. Assert distinct invocation entry epochs. |
| HMB-03 pointer graph and interior aliases | Two nodes link cyclically and one field points to the other's value. Two arguments denote the same interior value. Write 7 through one and read 7 through the other; compare pointer equality and address-derived outputs with the native closure. Reverse restores original bytes, links, allocation identity and liveness. |
| HMB-04 overlapping views | Two registered views of an 8-byte allocation overlap. Verify one canonical owner, shared updates, correct strongest lease, read-only-write failure before the store, and unchanged sentinels outside the views. Apply `noalias` checks only when the accepted signature requires them. |
| HMB-05 liveness and representation | Exercise null, one-past, zero-length, range overflow, native/target width or layout mismatch, unregister and free/reuse at the same address. Stale generations and unregistered numeric Ptr bits fail before access; no sentinel native memory is touched. |
| HMB-06 interference and program ownership | While a run owns a mutable lease, a cooperating host write/free/unregister and a second program's overlapping launch are denied or invalidate exactly as the published profile specifies. Verify the documented diagnostic before mutation. Wrong-program capabilities fail; compatible disjoint/read-only leases exercise the permitted paths. |
| HMB-07 partial failure and reverse | A valid first store changes byte 0 from 3 to 7; a subsequent access to an unregistered range faults without touching that range. Assert one fault record and the exact retained prefix. Eligible reverse restores 3 before close; exercise the separately published irreversible-close path as well. |
| HMB-08 effects and reverse eligibility | Combine a registered write with an R effect/CC reply, then a measurement case. Verify device journal association, no duplicated effect/reply, the existing irreversible decision and the exact visible bytes at close. Native-only I/O remains a separate observation. |
| HMB-09 escaping allocation | Under the accepted host-compatible allocator profile, create and return a node linked into the registered graph. Native code can use and free/realloc it after close. Reversal before publication restores the graph and allocator metadata. Test interior aliases, stale generations and an attempted private-device-pointer escape. |
| HMB-10 lifted entry import | Import the bytes and allocation/shadow-stack metadata of a small registered graph at one epoch before the oracle's private allocations. Check payload/fault, private-memory restoration and unchanged host `in` storage in both engines. A later host edit changes the next entry image. This is snapshot evidence, distinct from live CC/R sharing. |
| HMB-11 mixed native binary | Native main calls two marked roots in separate embedded objects, retaining a native-only dependency and unrelated globals. Check per-root identities/certificates/budgets, shared registration coordination, strict wrong-ledger rejection and independent reverse/close. No native main/argv/libc certificate or implicit process budget is invented. |

Implementation and executable evidence remain dependent on the published F04,
F08, F15 and F16 contracts. This proposal makes the required backend boundary and
acceptance domain reviewable while preserving the existing by-value ABI.
