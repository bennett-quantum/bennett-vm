# bennett-vm

Workspace status: [bennett/HANDOFF.md](../bennett/HANDOFF.md).

Independent, std-only Rust device runtime for Bennett ISA 0.8, with partial
ISA 0.9 consumption recorded in [SPEC_VERSIONS.toml](SPEC_VERSIONS.toml).
The published specs define the contracts; the reference interpreter is an
external CLI oracle in tests.

Commands: check, hash, run-oracle, step-check, run-action, run-cc, reverse-action-batch, reverse
(all tiers), and run-program. Run bennett-vm help for options. Diagnostics go
to stderr and exit 1; program faults produce a FAILED JSON run.

Oracle input accepts --mem name=hex and --mem-file name=path. The latter
reads binary bytes directly, including images too large for a hex argument.
It is supported by run-oracle, step-check and oracle reversal; duplicate
region bindings and invalid region sizes fail before execution.

CC/R run-cc, reverse and run-program accept --tape-out file and
--replay-tape file, each once. Replay checks device, operation, integer
arguments and transferred bytes before effects commit. Use identical inputs,
budgets, entropy, replies and prefix length. A mismatch is a diagnostic;
whole-run reversal preserves the saved journal while restoring active state.

Build and test from this checkout, headlessly and offline:

~~~sh
mkdir -p .tmp/local
env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u BROWSER \
  TMPDIR="$PWD/.tmp/local" CARGO_TARGET_DIR="$PWD/.tmp/local/target" \
  timeout 180s cargo test --offline -j3 -- --test-threads=4
~~~

Tests resolve published fixtures, specs, headers and oracle binaries only
through the exact ISA SHA in [gate.toml](gate.toml). Materialise its checkout
with the pinned gate tool's verify command before testing. The resolver uses
the local gate cache or BENNETT_ISA_SPEC, and rejects a mismatched HEAD.
It never defaults to a live sibling. The integration modules link once into
the suite test binary.

The default suite excludes exhaustive source expansion, published-oracle
differentials/conformance and C ABI rebuilds. The gate runs both the default
suite and every ignored test. A focused slow case can be run with
cargo test --offline -j3 --test suite -- FILTER --ignored --test-threads=4.
[Quality evidence](docs/quality.md) records measured timing, coverage, known
exceptions and the per-file structural baseline. Internal issue tracking
uses bd; dated reviews remain under docs/reviews/.

The stateful C library uses [include/bennett_vm.h](include/bennett_vm.h)
with the published ISA headers. Calls on one program require caller
synchronization; execution advances at wait or run_step. VM extensions
provide immutable JCS snapshots and history-prefix reversal. The C consumers
in tests/launch_abi.c, tests/launch_abi09.c and tests/sched_abi.c exercise the ABI.

C providers select their launch family at build time. The default build keeps
launch 0.8's four-argument `bennett_estimate_run`, including for existing
binaries. `scripts/build-abi-providers.sh` builds independent providers under
`.tmp/abi-providers/0.8/debug/` and `.tmp/abi-providers/0.9/debug/`.
Select exactly one directory when linking or loading `libbennett_vm`.
The 0.9 provider is built with `--no-default-features --features abi-0-9`;
its canonical estimate symbol takes eight arguments. Define
`BENNETT_ABI_FAMILY=9` before including `bennett_vm.h` and use the pinned
published 0.9 scheduler header for that provider. Default headers select 0.8.
Keep both families in separate translation units and loader namespaces when
needed; object ISA versions do not select a C ABI family.

Only the 0.9 provider exports the K91 role queries. Both manifests have
empty feature lists: exact alias-aware estimation and scheduler/certificate
production remain incomplete. Neither an exported additive entry nor an
empty feature list authorizes alias-aware dispatch. Callers must negotiate
each selected role with the published K91 rules before issuing such work.

Target-backed structured actions and native QRAM execute native operations.
Actual sequential resource/error ledgers are distinct from resource estimates
and certificates. Published schedule/library/ordinal support is consumed only
where implemented; unsupported construction, estimate or schedule requests
return explicit reasons. ISA 0.9 integration is incomplete.

Set BENNETT_VM_ENGINE=jit before process startup for native CC/R and oracle
execution (x86-64 Linux). Engine selection is cached once per process.
Interpreter is the default. Modes baseline, inline, structure, reindex,
simplify and optimised expose the private execution passes. They preserve
reference-row budgets, canonical labels, faults, history and identities.
Native code threads runtime row helpers and provides nine integer kernels;
arbitrary widths use runtime helpers. Unsupported hosts fail explicitly.
BENNETT_JIT_AUDIT=1 prints native execution counters to stderr.

The mandatory differential suite compares each optimisation mode and native
execution with the interpreter and pinned published CLI oracle. Set
BENNETT_HARNESS_COUNTS=1 and add --nocapture to print comparison counts.

The additive `bennett_estimate_run_at_accuracy` entry is present in the 0.9
provider. Its declaration comes from the pinned
`spec/bennett_estimate_accuracy_1.h`; add the pin's `spec/` directory to the
include path alongside `include/`. The request copies and validates the
explicit decimal accuracy exactly, binds it independently of the args-only
parameter hash, and selects a matching schedule or an empty default schedule
at that requested value. Valid requests still reach the existing unavailable
estimator boundary. The published feature identifier is `estimate_accuracy/1`;
it remains unadvertised until complete K12/K91 realization, exact resources
and certificate production/checking work. For launch 0.9 legacy entries without accuracy, a schedule matching all other
request bindings supplies its exact epsilon. Otherwise they return
`BENNETT_E_ARG` with
`E_ESTIMATE_ACCURACY_REQUIRED: use bennett_estimate_run_at_accuracy with epsilon_decimal`.
There is no numerical default; this request diagnostic leaves the output null.
The separately selected 0.8 ABI family retains its published behavior.

ISA 0.9 programs carrying authenticated schedules can be loaded for estimate
request resolution. Scheduled execution remains a capability gap and fails
before a run is created; loading a schedule does not authorize ignoring it.

`reverse-action-batch OBJECT ACTION [--range LO..HI] [--format binary|jsonl]`
streams action forward/inverse checks with one admitted object per process.
Fixed `--arg`, `--args`, `--token-aliases` and `--adjoint` options match ordinary
action invocation. Without a range, stdin is JSONL with exactly `mut`, `in`,
`mem` and `borrowed` entries; each row supplies the complete interface tuple.
Ranges use arbitrary-precision half-open decimal ranks, ordered by inputs,
mutable bundles, memory cells and the borrowed pool, last component fastest.
Workers can use disjoint ranges and validate each stream independently.

Every tuple starts from fresh state and invocation identities, executes the
selected direction and its inverse, and checks exact phase, workspace release,
preserved inputs/memory/pool and full restoration. The immediately flushed JCS
header describes physical bit packing, rank order and fixed record size.
Binary case records contain kind 1, seven check flags and little-endian packed
forward/reversed states; JSONL records expose the same flags and hex payloads.
All seven flags set (127) means the local checks passed. A complete footer gives
exact case/failure counts; parse, execution or output errors abort without a
complete footer. Failed local checks still produce records and a complete
footer, with exit 1. `--flush-each` flushes after every case.

The byte-stream contract is the published
[reference batch interface](https://github.com/bennett-quantum/bennett-isa-spec/blob/c3df1d32f1ad3139dc863a90085604061ef483de/reference-interpreter/BATCH-REVERSE.md).
Its independent `bennett-batch-reference` gate pin builds the release oracle;
`BENNETT_BATCH_ISA_SPEC` resolves only that exact checkout. The mandatory ignored
batch differential compares complete stdout bytes, including headers/footers.
The bounded pure-oracle value cache retains only outcomes after literal private
forward/inverse restoration; `BENNETT_VM_BATCH_CACHE=0` disables this reuse.
