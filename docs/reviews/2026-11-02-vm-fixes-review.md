# Review of branch vm-fixes (7f8e35b 406d3dd 07c2964 7a76021), 2026-11-02

Reviewer: opus-vmfixes-review (independent; did not write the reviewed code). Brief:
`bennett/.tmp/orchestrator/briefs/opus-vmfixes-review.md`. Binding advisor rulings of
2026-11-02T07:55Z: R1 (where the 0.8 text is silent, the reference interpreter is the
oracle; gate only where the 0.8 text specifies different behaviour), R2 (§6.2 arith
scratch objects), R3 (one heavy run on the host at a time).

Tools (all under `.tmp/review/`, not committed): `target-base` = bennett-vm d9cb684 (the
merge base), `target-ref` = `bennett-isa-spec/reference-interpreter` (read-only, own
target dir), `target` = this branch, `target-mk/debug/mk` (signs objects:
`mk <ver> <functions-file> <out>`). Scripts: `diff08.py` (all 0.8 fixtures, base vs
branch, both engines), `probes08.py` (one probe per changed rule, ref/base/branch),
`witnesses.py` (bennett-conformance harness witnesses, ref vs VM, both engines),
`diff09.py` (0.9 corpus `check`, base vs branch).

## (a) "0.8 now gets 0.9's lifted checks"

Method: every 0.8 fixture through base and branch (diff08.py: 444 cases + 96 distinct
0.8 objects, both engines): 1234 identical, 38 different, all on ill-formed objects
(f_* fixtures, mutant witnesses). Well-formed fixtures do not exercise the changed
rules, so probes08.py runs one minimal object per changed rule on ref, base and branch.

Changed 0.8 behaviour, each now judged under R1:

| Rule | d9cb684 at 0.8 | reference at 0.8 and 0.9 | branch | basis |
|---|---|---|---|---|
| K17 exact draw tie (`h`, `--draws 0.5`) | 0 | 1 | 1 | text silent at 0.8 (defect b-isa-spec-0h4) → reference |
| row after `trap` | runs, fault 4 | §3.4 reject | §3.4 reject | text silent → reference |
| `lift.const` literal range | accepted | §3.3 reject | §3.3 reject | text silent → reference |
| lifted call classical arg not an entry binding | accepted | §3.3 reject | §3.3 reject | text silent → reference |
| region-call classical operand without instance | diagnostic | FAILED fault 4 | fault 4 | text silent → reference |
| crossing / skipping uncall pairs (different tgt, fr') | accepted | accepted (0.8 and 0.9) | accepted | reference (see below) |
| §3.3 lifted widths, §3.7 name resolution, §3.3 disjointness trap, handle liveness, §5.5 unmatched uncall, §6.2 discipline, §16.2 empty body / regs | accepted | rejected | rejected | 0.8 text states the rule |
| qif selector in its own control (r4e) | ran (state differs from ref) | accepted | §3.3 reject | 0.8 §3.3 + §4.6: the expansion is the §3.3 sentence's own ill-formed example; bead r4e |

The reviewer's first pass (17bbffd, WIP, never compiled) gated the first six rows to
d9cb684 behaviour. R1 overrules that: those gates are removed again, and
`tests/isa08_reference.rs` pins the reference's behaviour at 0.8 and 0.9 for each row
(both engines).

Uncall matching: the reference keys `lift.oracle_call`/`lift.oracle_uncall` pairing on
all seven operands, `tgt` and `fr'` included, at 0.8 and at 0.9 ("no outstanding
matching lift.oracle_call on every path with unchanged ins, tgt, fr and supplied
cells"): `call r; call s; uncall r; uncall s` and `call r; call s; uncall r` are
accepted at both versions; base accepted them too. 07c2964 rejected both at 0.8 and
0.9 ("takes the tgt and fr of its call's successor versions"), a false rejection. Both
the load check (`src/call_matching.rs`) and the binding stage
(`src/ref_library/oracle_cleanup.rs`, `oracle_cleanup_pairs`) now pair by the full
operand identity, so a loaded object never fails pairing later. Note for the spec owner:
0.9 §5.5 (E17.5) reads as LIFO on callee, token, `ins`, `args`, `mem` only, which would
make the crossing program ill-formed at 0.9; the branch follows the reference per R1
(and the no-narrowing rule), and this text/reference difference belongs with
b-isa-spec-0h4's owner.

h6j (panic → diagnostic) is kept at 0.8: gating it would restore a panic.

The one version gate 7f8e35b keeps is text-specified, and matches the reference: 0.8
§10 lists only `COMPLETED → REVERSED`, 0.9 K6 adds `FAILED → REVERSED`. A run ending in
`trap`, `run-program … --reverse`: reference 0.8 "§10: E_INVALID_STATE: bennett_reverse
applies to a COMPLETED run; this run FAILED", branch 0.8 "§10: E_INVALID_STATE: cannot
reverse a FAILED run"; reference and branch 0.9 REVERSED (d9cb684 refused at 0.9 too).
The same gate is in `bennett_reverse` (src/launch.rs).

Probe rerun on c139974 (`probes08-c139974.out`): every remaining d9cb684 → branch change
at 0.8 is either stated by the 0.8 text or the reference's behaviour. Where the branch
still differs from the reference at 0.8, it is either text-mandated (r4e) or a
pre-existing divergence that base shares or that the branch narrows (4q5: run-time trap
instead of static rejection; bks, f2w: accepted by base and branch alike).

Verdict (a): CLEAR (with R1 applied in c139974; 07c2964 itself was NOT CLEAR: it rejected
crossing/skipping uncall pairs that base and the reference accept, at 0.8 and 0.9).

## (b) cleanup_fresh09 programs replaced

`else_history_does_not_substitute_the_temporarily_inverted_predicate` and
`replayed_independent_branch_histories_do_not_reapply_cleanup_policy` had an empty `qif`
arm. The reference rejects both old programs at 0.8 and 0.9 ("§16.2: empty or
unterminated structured body") and accepts both new ones; base and branch identity
hashes agree on the new ones. The empty arm became `qloop 0 { x x }` (bounds {0}), a
no-op, so the rows that carry the guarded behaviour (the else arm under the temporarily
inverted predicate in the first test; both arms of the first qif and the then arm of
the second in the second test) are unchanged row for row. The
selector moved entry:3 → entry:5 because the then-arm gained two rows; by row count both
name the same `adj action_call @mix` row, and the test still requires the specific
refusal "nonempty call cleanup needs its exact clean-input computing sequence" (not an
earlier, unrelated error) plus replay identity on all inputs; entry:10 in the second
test is unchanged by the same count. Verdict (b): CLEAR, a correction, not a weakening.

## (c) oracle pairing tests: load → parse

At 07c2964 the positive pair programs in `tests/oracle_address09.rs` and
`tests/oracle_cleanup09.rs` were only parsed, so nothing asserted that `load` admits what
the binding stage admits and refuses (under §5.5) what it refuses: coverage was lost.
Restored: `load_verdicts_match_the_binding_stage` (oracle_address09) holds load to the
binding stage's verdict on every pairing program, and oracle_cleanup09's constructor
test asserts load's §5.5 refusal before exercising the constructor. Under R1 the
crossing and skipping programs moved to the admitted lists of oracle_address09 (the
binding-stage pairing test now asserts their pairs); oracle_cleanup09's two refused rows
(an orphan uncall, an uncall on a different `tgt`) stay refused by the reference too.
Verdict (c): CLEAR after the restoration (07c2964 alone: NOT CLEAR, coverage lost).

## Harness witnesses (bennett-conformance)

`witnesses.py` (each witness's argv on the reference and on the VM under both engines;
digests stripped): at c139974 and again on the merged build, against the reference
built from the gate pin bennett-isa-spec ecaaac0: 23 of the 31 witnesses with a
`witness.json` match; the 32nd (`manual-oracle-writes-lost-after-action-call`, README
only) was run by hand and matches (measurement value 1 on reference, interpreter, jit).
24/32 as R1 expects. The 8 DIFFs are exactly as the implementer claimed:

| witness | reference | VM | bead |
|---|---|---|---|
| fixture-objects-…-5bce4be99cf7 | §9.3 unresolved imports | runs | 5ye |
| fixture-objects-…-ce360f814001, …-ef4bec623a8d | runs | §12.1 certificate target_hash | af4 |
| gen-r-…-817970b77cfd | reverse JSON | reverse JSON, equals_entry differs | jub |
| gen-mutant-…-1ab595f5947d, -9854053ea356, -f54dd058180f, -fa6b34632d87 | §3.3 SSA definition reused | runs | SSA reuse (text admits several definitions; `cc_names_may_have_several_defining_instructions`) |

## §6.2 on the 0.9 corpus (R2)

diff09.py: 235 distinct 0.9 objects, 227 identical base vs branch. The branch newly
rejects scratch objects under `bennett-arith/.tmp/opus-arith-circuits/objects/`:
gid2.bisa, arith.add.gidney_t.w3.bisa, arith.bij.addmod.w4.mb.bisa, and_use.bisa (and
arith.test.broken.bisa, meant to fail). Rechecked by exact path: the reference rejects
all of them with "§6.2: released value has not been uncomputed"; and_use2.bisa is
accepted by both. Reference-consistent, not a narrowing: bennett-arith-a5e (related to
1e8 and l2p, the spec-change requests).

## Residuals (pre-existing, base behaves the same or worse; beads filed)

- bennett-vm-gf7: §5.5 load check accepts a row writing `tgt`/`fr'` between a pair; the
  reference rejects. qjumpr functions skip the load-time §5.5 check.
- bennett-vm-bks: `qfree` of a `qcopy` result accepted; reference §5.1 rejects.
- bennett-vm-4q5: same-name aliased region-call operands trap at run time instead of
  being rejected at load like the reference does.
- bennett-vm-f2w (existing): §6.2 analysis does not follow writes under another predicate
  context (probe qfree-qif-arm-commute: reference rejects, branch accepts).
- bennett-vm-r4e (existing): qif selector in its own control, above.

## Fat-object reachability (bennett-4ht)

Nothing on the branch assumes the device object is the whole binary: it adds no entry
point, argv/envp, libc-closure, native-comparison or certificate-inventory logic. Entry
stays a caller-named symbol (`run-* OBJ <entry>`, `bennett_launch` by name), and every
new load check (§3.3/§3.7 lifted typing, §5.5 pairing, §6.2 discipline, §16.2) is per
object and per function, so each marked module of a mixed fat object validates as its
own VM object without seeing the host code. The changes that decide launch, handle or
effect-tape behaviour, and how a mixed fat object uses them:

- FAILED → REVERSED at 0.9 (`bennett_reverse`, `run-program --reverse`): eligibility is
  per run and keyed on the loaded device object's ISA version. A host binary that
  launches one device module and sees it fail can reverse that run alone; other device
  runs and the host's own state are untouched.
- Host-limit transfers (`Data::read`): a console/block write or `hostcall` payload too
  large for the host to allocate is a "host limit" diagnostic, not a panic, and an
  unallocatable `hostcall` request is not surfaced to the host as a request. In a fat
  object the transfer buffer is the native process's memory; a diagnostic keeps the host
  process alive and in control, which is what a native caller of the launch ABI needs.
  The effect tape format is unchanged.
- Region-call handle checks (a consumed or viewed register used through any name, slot
  widths, aliasing → CC trap / R fault 4 before any effect, ledger untouched): these
  read only the run's device-side handle ledger. Handles stay device values; nothing is
  exported to or read from host native code, the boundary bennett-4ht prefers.
- `measure.iN` draws in `Runtime::step`: the launch supplies a draw for both spellings,
  per run, from the launch's entropy, independent of what else the binary contains.
- E17.7 (a region call's classical argument or oracle token without a current instance
  traps): device-internal semantics, no host-visible change beyond the fault record.

## Merge and gates

PENDING: full per-engine gate waiting for the host heavy slot (ruling R3); filled in by the next commit.
