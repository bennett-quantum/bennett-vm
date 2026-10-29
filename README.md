# bennett-vm

Independent, std-only Rust implementation of the accepted Bennett ISA 0.8.
Milestones 1–2 provide object loading/validation and identities, the bounded
oracle interpreter, CC/R execution with retained history and tapes, region
calls, one-shot launch, and a stateful C library. It consumes published specs, never the reference
interpreter's code.

Implemented commands use the reference interpreter's names and JSON shapes:
`check`, `hash`, `run-oracle`, `run-cc`, CC/R `reverse`, and `run-program`.
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

The current gate has 20 passing Rust tests, including 160 C ABI checks and the
577-comparison differential harness. It also runs the unchanged published
490-case conformance harness: **124 pass / 321 fail / 45 unavailable**; full
conformance stays red. The reporting test checks served launch cases and loud
staged refusals without filtering the corpus.

Next: milestone 3, coherent execution, and the outstanding validation negatives.
Exact-amplitude coherent execution, targets/lowering/schedules and JIT follow
in milestones 3–5. No future North-Star capability is removed from scope.
