State 2026-11-01 08:55:44 UTC: partial ISA 0.9 consumer; task NOT complete; no bead closed.
Resume bennett-vm-47t before hsd, then clw. fat / qvalidate.rs remains frozen.
Top-level SPEC_VERSIONS.toml remains 0.8 with isa_0_9 partial-consumer status;
admission of 0.9 objects does not claim complete 0.9 launch/schedule execution.
Published source pin: spec/Bennett-ISA-Spec-0.9.md at
b77cbed2989d94b164d734563743c1a4191a4820 (pin-only commit e834e9b).
Authenticated ref@0.9: 219 recipes, envelope
5de4e4995350313e50217880ba1b95de39cd043283b726337d4894efb005d197.

47t implemented: typed catalog values/records/regions/classes/allocation arguments,
events/images, templates, closure-bound arguments and authenticated symbol bodies.
Inverse expansion validates allocation lifetimes after traversal; zero-width identities,
clean scratch LIFO, stable first-occurrence controls and primitive action source lowering
are retained. Semantic action calls preserve callee ordinals and occurrence metadata.

0.9 admission now accepts consistent versioned headers/signatures and verifies all ten
published objects' stored hashes. K46/K47 layout selection uses structured key paths,
rendered as injective JCS arrays only after selecting original owners. Lproc signatures
have mandatory tokens/args/mem arrays; fixed bindings close transitively by root entry
positions, retaining distinct equal-valued keys. Both oracle engines and the reversible
step machine use per-instance locations, canonical row ranges, typed pseudo-locations and
mapped regions. Private execution aliases never affect source or closure identities.
Lifted schemas enforce destination/read/address/cast/literal widths and typed immutable
semantic-call arguments/regions. Fault-code widths are checked against each containing
oracle; label/jumpr widths use the complete resolved fixed-instance index space. Exact
unresolved-import row spaces still need rechecking in the complete link integration.
qvalidate.rs has no changes.

47t remainder, in order: finish general 0.9 source/type/link admission; complete source-
checked catalog descriptors for every oracle row, activation, step and construction;
complete source invocation edges/iterations/instances/call indices and K49-K65 support/
cleanup addresses; schedule admission, target realisation/execution, realised matrices
and certificate contexts. sched::consume still refuses unrealised schedules. Admitted
0.9 CC/R syntax is not a complete executor: dynamic-qreg, budget.mint, typed CC/R faults
and full qinplace runtime dispatch still need integration. No full catalog executor claimed.

ISA b-isa-spec-5xm is OPEN (consumer,isa,vm), committed only as .beads/issues.jsonl
in ISA commit bbb758d. Section 11.1 requires the linked symbol's own formal order and
no implicit bindings/defaults, but oracle_call/lift.oracle_call recipes take only S/A
while the invocation token is outside the oracle's declared header interface. Reading A
adds an explicit token formal (position/name undefined); reading B supplies source-call
invocation context outside A (channel/propagation undefined). The same oracle closure
and declared operands under B=2/B=3 need different constructions. This blocks complete
semantic oracle-call construction; no private token channel/default was invented.

hsd remains OPEN/unchanged: launch 0.9 estimate ABI/request ownership/token validity,
full padded realisation/default effective schedule, bound estimate certificate and
independent reconstruction. clw remains OPEN/unchanged: scheduled/controlled/universal
replay contexts, object/launch certificate dispatch, inverse/structural bindings,
K74 report serialization and K75 unchecked-host-limit outcomes. Their complete remainders
remain in the beads. fat / ISA b-isa-spec-73y ruling is pending; no frame admission edit.

Final sequential gate: interpreter and JIT each 142 PASS / 5 FAIL / 0 ignored (147).
Before each 131 PASS / 5 FAIL / 0 ignored (136); original JIT baseline rebuilt from
archived 93368f0 under .tmp/baseline. Differential remains 14 PASS / 4 FAIL in each.
All 11 added tests pass in both engines. The exhaustive forwarding witness covers all
8,192 configurations and checks expected results/faults plus complete step/inverse state;
separate binding positions retain distinct locations and canonical ranges.
Only the five known fat failures: coherent::every_reached_row_inverse_restores_complete_pre_step_tuple
and differential::{check_and_hash_byte_identical_on_every_fixture,
action_basis_measurements_differential, every_fixture_symbol_executes_through_cli,
coherent_commands_on_every_fixture}. No fixture/test removed, weakened or excluded.
fmt, strict Clippy and diff whitespace checks pass. Evidence:
.tmp/47t-final-{interpreter,jit}.log, .tmp/47t-final-totals.json,
.tmp/47t-next-{before-interpreter,baseline-jit}.log, and focused red/green logs.
Interim parallel-gate totals are invalid due shared C executable/binary replacement races;
only the final sequential gates count. Details/gotchas are in the top worklog session.

Local implementation commits: e834e9b, 501b9c1, cb5821c, 2fcb413, 27e79d1.
No dependency changes, sibling spec-text edits, push or Dolt sync. Main remains local
while the gate is red; prior backup is origin/wip/m5-2026-10-31.
