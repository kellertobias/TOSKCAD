# ADR-0020: Drive assemblies through directed joints and explicit physical leaves

| | |
|---|---|
| Status | Accepted engineering decision; workflow acceptance remains separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

Reusable subassemblies need stable member identities while their outer placements
and internal joint coordinates change. The source document, native geometry,
viewport, interference queries and manufacturing exports must agree on which
occurrences physically exist. Repeated copies of one definition can contain the
same member IDs at different world positions.

The [Instance contract](0017-part-and-instance-placement.md) provides rigid
placement transactions. It does not define flexible assembly solving or the
meaning of template geometry in a reusable component.

## Decision

Store one reusable Assembly definition with authored member Instances. Explicit
`model.assembly_component` metadata identifies its definition-only Parts. Preserve
these source entities when compiling and omit their templates from the physical
occurrence list. Preserve compatible unplaced legacy Assembly roots as identity
placements. A placed Assembly requires the versioned component capability.

Drive each moving Instance through at most one incoming relationship. Fixed,
slider, revolute, cylindrical and planar relationships form an acyclic directed
graph. Their coordinates are respectively none, local Z, local Z rotation,
local Z with rotation, and local X/Y with rotation. Bound parameter expressions
and inclusive limits use the existing typed transaction system. The parent chain
has an explicit depth budget; this is a deterministic placement evaluator rather
than a general assembly constraint solver.

Anchors use an explicit component-local numeric frame or a persistent geometry
reference with an authored attachment frame. A parentless target frame is in
world coordinates. Do not invent an origin from an arbitrary plane's UV basis.
Missing or ambiguous references require repair. Named poses store explicit joint
values; applying a pose is atomic, while editing its stored values leaves the live
configuration unchanged.

Compile exact native compounds and name each member with its authored UUID.
Represent every physical Part leaf with a complete `OccurrencePath`: outer root
plus the ordered member chain. Publish source hash, component, producer and exact
world pose in a native evaluation receipt. A repeated member ID alone cannot
identify a physical leaf. STEP and planar DXF resolve the complete path to its
exact native shape; source/display ordinals cannot substitute for it.

Keep exploded display offsets outside the source document. Expand aggregate
meshes into per-leaf presentation instances with resolved Part colors. These
presentation scenes cannot authorize geometry picking, measurements, export or
candidate Apply. Interference always uses assembled native geometry.

Compute overlap from occupied native solid volume and minimum clearance from
finite B-rep extrema. Normalize compound occupancy before measuring intersection
volume to avoid double counting overlapping solids. Preserve consistent native
witnesses and report incomplete witness coverage explicitly. Movement-range
queries evaluate listed temporary poses and show their actual intervals, including
a shortened final interval. A query does not commit joint edits or establish a
continuous collision-free path, structural suitability or physical dynamics.

## Consequences

Positive: edits, reusable placements, saved poses, Undo/Redo and geometry consumers
share canonical IDs and the same native evaluator. A copy keeps its definition
shared, while outer placement edits remain independent. Namespace provenance
survives member reordering without allocating entities during evaluation.

Negative: placed copies share their definition's internal joint configuration.
Flexible per-copy internal overrides and closed kinematic loops are unsupported.
A free Move/Copy of a joint-controlled Instance is rejected explicitly; the user
must edit the relationship or place a separate component. Assembly materials
remain on individual Parts, so the aggregate viewport uses a neutral appearance.

Neutral: expansion, native pair counts and sampled checks have separate bounded
budgets. A failed candidate or sample cannot silently substitute a display mesh
or a partial successful model. Windows/Linux interaction and low-power performance
require separate acceptance evidence.

## Alternatives considered

**General simultaneous assembly solver.** Closed loops and coupled mechanical
constraints would become possible. Choosing a robust solver, defining branch
selection and exposing under/overconstraint diagnostics require a separate design;
the directed model gives the supported joints predictable behavior now.

**Flatten every copy into independent Parts.** Independent editing and material
assignment would be straightforward. It duplicates definitions and breaks shared
parametric updates, so copying retains reusable component identity.

**Use bounds or display meshes for collision and export.** This reduces native
work and is useful for broad-phase pruning. It cannot establish native overlap,
minimum clearance or manufacturing contours, so exact queries retain native solids.

## References

- [Assembly schema and commands](../../crates/multicad-core/src/model/assembly.rs)
  define ownership, relationships, poses and capability boundaries.
- [Native component compilation](../../crates/multicad-model/src/assemblies.rs)
  preserves full member paths and exact geometry.
- [Inspection query contracts](../../crates/multicad-core/src/assembly_inspection.rs)
  own budgets, source stamps and sampled interval semantics.
- [Native pair adapter](../../crates/multicad-kernel/src/bridge.cpp)
  computes occupied overlap and finite-distance witnesses.
- [Native UI controller tests](../../crates/multicad-ui/src/assembly/native_tests.rs)
  cover limits, references, saved pose edits, candidate receipts and source history.
- [Nested output regression](../../crates/multicad-model/tests/assembly_exports.rs)
  writes and rereads individually placed STEP solids and planar DXFs.
