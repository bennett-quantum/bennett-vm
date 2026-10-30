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

The current gate has **80 passing Rust tests**, **160 launch C checks**, a
scheduler C consumer calling all **15** published exports, **1,235 agreeing CLI
comparisons per pass and JIT mode** and **21,888 one-row inverse checks**. Published conformance is
**445 pass / 0 fail / 46 unavailable**, 491 cases (exit 2). See PROGRESS for the
per-suite report, comparison limits and unchanged unavailable classifications.

Milestone 4 remains **partial and blocked** on the unpublished §11 `ref` expansion
library and undefined `eps_decade` indexing. Target-backed structured actions and
native QRAM execute actual native operations. Designated dirty pools, explicit
accuracy-request batches, class capacities and actual sequential event accounting
are implemented. Target-backed CLI output and immutable launch snapshots expose
`target_resources` / `target_error` (accumulated program ledger at that boundary).
These are not estimates or certificates. Target schema remains exactly ISA §8.

No schedule/library hash/ordinals are fabricated. Lifted/oracle row decomposition,
non-native memory comparators, verbatim scheduled execution/accounting and
reproducibility wait on the published expansion contract. Scheduler requests
return explicit UNSUPPORTED. Finish milestone 4 after publication. Milestone 5
JIT is implemented independently of that missing target library. No future
North-Star capability is removed from scope.

Select native execution of CC/R and oracle segments with `BENNETT_VM_ENGINE=jit`
(current host backend: x86-64 Linux). Interpreter remains default. Individual
optimisation modes are `inline`, `structure`, `reindex`, `simplify`; `optimised`
combines them without codegen. The four ISA §5.7 passes preserve reference-row
budgets, canonical labels, faults, history and identities. Native code threads
runtime row helpers and emits nine host integer arithmetic kernels; arbitrary
widths and complex rows use runtime helpers. Unsupported hosts fail explicitly.
`BENNETT_JIT_AUDIT=1` prints native execution counters to stderr.

The differential gate compares all 75 object images in the recorded corpus
through each pass, their composition, and JIT mode against the interpreter and
the CLI reference: **1,235 agree / 0 diverge per mode**. Of these, **718
invocations on 68 images enter native segments**; identity, validation, pure Q
and literal debug commands are also compared. Published JIT conformance is
**445/0/46**. See PROGRESS for precise comparison and counting limits.
