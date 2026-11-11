# bennett-vm

## What this repo is

The device runtime: classical-control tier, reversible tier with history layers and tapes, coherent tier via the predicated step machine, device models, branch-enumeration simulation, launch-time lowering to the target alphabet, and a JIT tier. Implements launch 0.3 (Bennett-ISA-Spec.md section 10).

It is one of several independent repositories in the Bennett reboot. It talks to
the others only through published specs.

## Where the spec is

Launch ABI: Bennett-ISA-Spec.md section 10 (launch 0.3). Machine model: section 3; instructions: section 4; regions: section 5; memory: section 7.
The conformance suites for the versions in SPEC_VERSIONS.toml live in bennett-isa-spec.
Spec sources of truth: Bennett-ISA-Spec.md and Bennett-NorthStar.md in the Bennett.jl repo
until bennett-isa-spec is published.

## Rules

The one rulebook is bennett/docs/process/COMMON.md; it binds here. Gate: `scripts/pre-push`. Tasks: `bd` (`bd prime` lists its commands).
