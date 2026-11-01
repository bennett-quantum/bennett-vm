# ISA 0.9 source admission audit

Scope: bennett-vm-47t-a, §§3.3, 3.7, 4, 5 and 9. Admission is independent
of executor dispatch, schedule realisation and certificate composition. The
0.8 implementation/version declaration remains unchanged. The published 0.9
source is read as a contract; no other implementation source is used.

| Source obligation | Admission boundary and regression evidence |
|---|---|
| Versioned envelopes, canonical source, complete signatures | `object`, `validate`; `object09` checks all ten published objects and exact hashes. `admission09` covers canonical integer tokens and the empty-HEX data-line exception. |
| CC/R parameter, block and result types, width suffixes | `validate`, `typing`; `admission09` checks iN bounds, static measurement widths and qreg transfers. |
| CC/R definitions independent of block order | `typing` infers to a fixed point, accepting repeated definitions only at one type (§3.4). Missing **dynamic** instances fault at execution; dominance is not an admission requirement. `admission09` includes both textual forward references and path-dependent instances. |
| CC/R row arity, tiers and operand types | `validate_opcode`, `typing`; classical/conformance/differential suites plus modelled-device arity regressions. Unknown devices retain their specified runtime fault. |
| Qreg name overlap and view use on every path | `qreg_typing` traverses CFG states per slice; `admission09` checks branch exits, multiple consumption, parent uses, prohibited view operations and same-name concatenation/call slots. Device-state aliasing remains a runtime precondition. |
| Oracle/lproc group types, roles and immutable entry bindings | `instances`, `lifted_typing`; `object09` checks all five groups, exact forwarding, pseudo-locations, fixed-position instances, lifted widths, fault widths and complete index spaces. |
| Structured interface, classical/token and region shapes | `instances`, `coherent_typing`; `coherent_typing09` checks exact call/qinplace slots, roles, angles, controls and regions without enumerating region cells. |
| Every coherent primitive's operand shape | `validate`, `coherent_typing`, `qvalidate`; gate arity and single-wire targets, equal-width bundle operations, memory address/value widths, image sizes, region mutability and inherited-control disjointness. |
| Linear locals and function-wide regions/predicates | `coherent_typing` tracks live allocation kinds, consumption and reacquisition, requires loop entry/exit bindings to agree, and walks action blocks in text order. `qvalidate` uses the same function-wide predicate/frame sequence. `coherent_typing09` checks use after release, leaks, duplicate live names, wrong release kinds and predicates spanning blocks. |
| Action/permutation restrictions, bounds and cycles | `validate`, `coherent_typing`; existing conformance, `coherent_typing09` controlled-adjoint cycles/angles, and `ref09` source/control witnesses. |
| Borrow-frame entry-domain admission | `qvalidate`; unchanged `frame_domain`/certificate suites retain the K76 Cartesian domain and public certificate guard. |
| Names, signatures, pins and source provenance after resolution | `link::resolve` automatically assembles canonical sources, merges metadata/data, deduplicates definitions, recomputes transitive identities and calls `link::admit`. `link09` checks order independence, duplicate/missing providers, pins, all five signature groups, conflicting metadata/source, post-link index widths and actual imported-row execution. |

`Resolution.inputs` retains the original objects, including their schedule,
certificate and coverage documents. `Resolution.object` is the resolved source
object; it does not carry unverifiable input certificates under changed hashes.
The resolution ledger contains every imported closure hash. Composition and
rebinding of executable certificate contexts are the separate 47t-d/clw stage;
they are not certified by this source-admission API.

The separate execution/catalog/scheduling obligations remain 47t-b/c/d,
hsd and clw. In particular this audit does not advertise a full 0.9 executor,
turn an admitted schedule into an executable one, or replace §6 cleanup and
inverse-pair evidence with successful source typing.

## Outstanding contract question

The audit remains open on ISA `b-isa-spec-7js` (consumer, isa, vm;
issues-only commit `7dd1ffd`). §3.3 permits `region[0,W]` while source wires
have positive width. §4.6 leaves the source-address width/spelling implicit;
the published catalog specifies an A-bit address. The bead quotes the text
and asks whether a zero-width empty source actual must be specified, or a
nonempty source address is permitted with defined comparison/adaptation rules.
The existing exact-A qload/qunload rule and the catalog shape are retained;
no private zero-width source binding is invented. This is an outstanding
source-admission obligation, not a reason to drop one-cell regions from scope.
