State 2026-11-01 10:13:54 UTC: 47t-a source checkpoint green; overall requested implementation remains incomplete.
Resume bennett-vm-47t-a after ISA b-isa-spec-7js pins coherent-memory source address widths/A=0 binding. Do not start c/d/hsd/clw before a is complete. Never push.

Current implementation
link::resolve automatically resolves supplied canonical source objects, assembles a deterministic graph, merges devices/layout/data, deduplicates source definitions, recomputes transitive interface/closure identities and repeats link::admit plus full validation/fixed-instance/index-space checks. Exact import signatures and pins are enforced. Resolution.inputs preserves original schedule/certificate/coverage documents for d/clw composition; the assembled source object does not assert that input certificates certify changed identities. Imported rows execute through the assembled object.

The source audit and its outstanding contract question are in docs/admission09-audit.md. The current pass adds CC fixed-point name typing (§3.4), exact type/measurement/transfer rules, qreg view CFG/name checks, coherent memory roles/shapes, allocation/consumption/reacquisition and loop ownership, function-wide predicates/borrow analysis, qinplace control disjointness, canonical INTs/empty data lines, and modelled-device arity. Existing fixtures/tests were not weakened, removed or excluded. No foreign implementation source read/copied.

Blocking ISA question
b-isa-spec-7js (consumer,isa,vm), issues-only commit 7dd1ffd in ../bennett-isa-spec: §3.3 permits region[0,W] but not wire[0]; §4.6 leaves source address width/spelling implicit while the published catalog uses A-bit addresses. The issue quotes both readings: an explicit empty source actual at A=0, versus a nonempty address with defined comparison/adaptation. The existing exact-A load/unload rule and catalog shape remain; no private zero-width source channel is invented. a stays IN PROGRESS with an external dependency; no bead closed. No spec text changed; concurrent ISA editor files are untouched.

Validation
Before this pass, interpreter/JIT each 170 PASS / 0 FAIL / 0 ignored, differential each 18/0. After: interpreter/JIT each 188 PASS / 0 FAIL / 0 ignored, differential each 18/0. All ten published 0.9 object hashes and the existing 8,192 complete-state fixed-instance witnesses pass. fmt, strict Clippy and whitespace green. Final logs .tmp/47t-a-final3-{interpreter,jit,fmt,clippy}.log and .tmp/47t-a-final3-totals.json. Focused red-green logs cover resolve, types, memory, views, transfers, lexical rules, effects, qinplace controls, loop ownership and literal/name separation. Final3 is the complete gate for the final source state; earlier runs preceded the last regressions or were superseded.

Ordered remaining beads
47t-a: resolve 7js, implement/adapt the published rule, finish and close the audit. Certificate composition execution remains d/clw.
47t-c OPEN: K49–K65 invocation/iteration/instance/call-index and support/cleanup addressing; unworked this pass.
47t-d OPEN: full schedule admission, realisation/execution and remaining admitted CC/R runtime dispatch; unworked this pass. Full integration also depends on b.
47t-b remains BLOCKED on ISA 5xm. The editor has specified oracle_invocation/K77 at 1d86f00..e0bd3a8, pending a council fix round. Consider unblocking/implementing only AFTER a, c and d as instructed; it was not worked this pass.
hsd OPEN: launch 0.9 estimate ABI, resolved request/token ownership, padded/default realisation and bound estimate certificate/reconstruction.
clw OPEN: object/launch certificate dispatch, controlled/scheduled/structural/inverse replay, K74 reports and K75 host-limit outcomes.

Contract/process
SPEC_VERSIONS.toml remains 0.8 with partial 0.9 pin 9df1541 and the accepted-0.8 frame erratum 6182d7c. The authenticated 219-recipe envelope remains 5de4e4995350313e50217880ba1b95de39cd043283b726337d4894efb005d197. fat remains closed with the independent public frame-domain certificate guard. All cargo commands ran offline/headless with project-local TMPDIR. No new dependencies, pushes or Dolt sync. Session worklog belongs to the orchestrator because this is the implementation sub-agent.
