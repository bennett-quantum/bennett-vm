# bennett-vm

Independent, std-only Rust implementation of the accepted Bennett ISA 0.8.
Milestone 1 provides object loading/validation and identities, the bounded
oracle interpreter, CC/R execution with retained history and tapes, region
calls, and one-shot launch. It consumes published specs, never the reference
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

Next: the stateful launch C ABI and reversible runtime API (milestone 2).
Exact-amplitude coherent execution, targets/lowering/schedules and JIT follow
in milestones 3–5. No future North-Star capability is removed from scope.
