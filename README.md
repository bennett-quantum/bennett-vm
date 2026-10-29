# bennett-vm

Independent, std-only Rust implementation of the accepted Bennett ISA 0.8.
Milestones 1–3 provide object loading/validation and identities, the bounded
oracle interpreter, CC/R execution with retained history and tapes, region
calls, one-shot launch, and a stateful C library. It consumes published specs, never the reference
interpreter's code.

Implemented commands use the reference interpreter's names and JSON shapes:
`check`, `hash`, `run-oracle`, `step-check`, `run-action`, `run-cc`,
`reverse` (all tiers), and `run-program`.
Run `bennett-vm help` for options. Errors go to stderr and exit 1; a program
fault is a JSON `FAILED` run with no results. Unsupported later milestones
fail explicitly with their plan item. Memory-digest serialization is local;
see [verification and comparison limits](docs/PROGRESS.md).

Build/test headlessly, without network or dependencies:

```sh
mkdir -p /home/tobias/Projects/bennett-workspace/bennett-vm/.tmp
env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u BROWSER \
  TMPDIR=/home/tobias/Projects/bennett-workspace/bennett-vm/.tmp \
  cargo test --manifest-path /home/tobias/Projects/bennett-workspace/bennett-vm/Cargo.toml --offline
```

The differential harness requires the sibling ISA repo's built reference
binary and published fixtures. Override their locations with
`BENNETT_ISA_REF` and `BENNETT_FIXTURES`; absence is a test failure. The spec
examples are independently extracted from the sibling published spec.
Set `BENNETT_HARNESS_COUNTS=1` and add `-- --nocapture` to print comparison counts.
Milestone 1's gate is 17 passing tests and 577 agreeing comparisons over all
12 fixtures, both spec examples and generated boundary cases.

[Architecture and milestones](docs/PLAN.md) · [Verified evidence and divergences](docs/PROGRESS.md)

The stateful library exports all 41 §10 functions; estimates and coverage
remain explicit milestone-4–5 stubs. Use [include/bennett_vm.h](include/bennett_vm.h)
with the published ISA include directory. `cargo build --offline` emits static
and shared libraries under `target/debug/`. Calls on one program require caller
synchronization; execution advances at `wait` or `run_step`. VM extensions add
immutable JCS snapshots and history-prefix reversal. See the C11 consumer in
[tests/launch_abi.c](tests/launch_abi.c) and contract details in PROGRESS.md.

The current gate has **34 passing Rust tests**, including 160 C ABI checks,
**1,054 agreeing CLI comparisons** and 21,888 complete one-row inverse checks.
The unchanged published conformance harness reports **445 pass / 0 fail /
45 unavailable** (exit 2); suite C is **339/339**. Exact support and cleanliness
use symbolic amplitudes; display amplitudes are compared phase-sensitively at
absolute tolerance 2e-12. No global phase is discarded.

Next: milestone 4, devices/targets/lowering/schedules, then milestone 5 JIT.
No future North-Star capability is removed from scope.
