Milestone 5 JIT VERIFIED 2026-10-30; epic bennett-vm-npe completed.
Read docs/PROGRESS.md top section for full evidence, counting limits and JIT modes.

Implemented ISA 0.8 §5.7 inlining, structure/CFG continuation recovery, private
dispatch re-indexing and block-local integer folding. Logical rows/call records,
canonical lift.label values, budget/fault boundaries and source history survive.
Native codegen for x86-64 Linux emits direct-threaded CC/R and oracle segments
and nine integer kernels. Runtime helpers retain arbitrary-width data, complex
rows, Q simulation and history. No dependency, reference code or scheduler policy.

BENNETT_VM_ENGINE=jit opts in (CLI and C library); interpreter is default.
Individual pass modes inline/structure/reindex/simplify and combined optimised
are available. BENNETT_JIT_AUDIT=1 reports native entries/callbacks/kernels.
Code mappings are RW then RX; native ABI errors/panics propagate safely.
Other hosts and unknown engines fail explicitly. No speed claim was measured.

Final gates:80 Rust tests,160 launch C checks,15 sched exports,21,888 row inverses;
fmt/strict Clippy/diff clean. All tests run in interpreter-default and JIT modes.
1,235/1,235 comparisons agree per individual pass, composition and JIT; zero
divergences. 75 distinct object images,1,209 JSON/26 diagnostics. Of those,
718 invocations/68 images actually enter native segments (718 agree/0 diverge);
517 are identity/validation,pure-Q,literal-debug or pre-execution diagnostics.
All12 fixtures and both spec examples remain included; six new object witnesses
exercise canonical labels,indirect faults,parallel moves,loops,folds and prefixes.
Raw logs .tmp/m5-final-tests.log,.tmp/m5-final-all-jit.log,.tmp/m5-final-clippy.log.

Published JIT conformance (unchanged runner,exit2):445 PASS/0 FAIL/46 UNAVAILABLE.
Per-suite pass/fail/unavailable:A21/0/0 B35/0/0 C339/0/0 D7/0/5 E0/0/16
F33/0/2 G10/0/13 H0/0/10. Interpreter mode has the identical verdict.
No published fixture or expected value changed,no new ISA gap found.

M4 parent v23 remains BLOCKED on47t(unpublished §11 ref library) and cxg(undefined
eps_decade). Full lifted/oracle target decomposition,non-native memory comparators,
verbatim schedules,scheduled accounting/reproducibility still fail explicitly as
milestone4. Estimates,rigorous certificates and coverage remain explicit errors;
native codegen does not implement them. No North-Star scope reduction.

Next:finish M4 after the expansion/index contracts publish; preserve the new JIT
obligations when implementing those paths. Stay on accepted ISA0.8 until a
maintainer-authorized version migration. Headless/offline,TMPDIR under .tmp,
absolute paths,path-scoped commits,BEADS_HOOK_TIMEOUT=20; no push,Dolt sync or
foreign edits. Current session begins with c6d826c; final follow-up commit adds
the expanded boundary corpus and final records.
