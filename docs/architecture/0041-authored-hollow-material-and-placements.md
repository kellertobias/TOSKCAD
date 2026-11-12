# ADR-0041: Authored hollow material and common placements

| | |
| --- | --- |
| Status | Accepted implementation design; application cutover and human acceptance remain separate |
| Date | 2026-11-12 |
| Deciders | Codex, within the implementation authority for CAD-290 |

## Context

A short hollow frame member can lose part of its section during the first miter edit.
The generator-preserving trimmed sweep requires positive thickness over the entire original section and cannot represent that intermediate shape.
The general planar Boolean provider therefore needs material cells for straight profiles containing holes.

Evaluating rotated stock in world coordinates also subdivided an untouched wall into three reference candidates.
The same source cut in its original chart retained one wall.
The [posed stock test](../../crates/multicad-kernel/tests/rust_hollow_planar_boolean.rs) retains this regression.

## Decision

Partition authored straight wires using a constrained triangulation as a candidate.
Certify unchanged original coordinates, positive triangle orientation, complete oriented boundary incidence and paired opposite interior incidence before constructing convex material cells.
Each exterior cell face retains its authored cap or side label; partition walls remain unnamed.
The partition uses the source wires directly and creates no display mesh.

Retain the most recent authored placement and its immediate source for planar Rust bodies.
When all Boolean operands share that placement, evaluate their original sources and apply the placement once.
Check metric bounds before reuse.
A live weak body identity, exact frame and current metadata must still match the retained receipt.
Geometry, chart and metadata changes invalidate reuse; source snapshots contain no recursive placement history.

For a straight member that the trimmed-sweep provider rejects as invalid input, try the extended authored stock with planar end cuts.
Both providers retain their existing geometry validation and resource limits.

## Consequences

**Positive:** Hollow stock enters the existing planar material proof.
The short-miter workflow succeeds, and a shared rotation preserves the tested untouched wall reference without reconstructing local coordinates from rounded world points.

**Negative:** Planar placements retain an additional source snapshot and copy its metadata.
Differing placements or changed receipts use the existing evaluation path, whose broader reference behavior still requires qualification.

**Neutral:** The canonical saved document and public Boolean command remain unchanged.
The retained placement is an internal runtime receipt.

## Alternatives considered

**Evaluate every operand in world coordinates.** This supports arbitrary placement combinations without retaining source snapshots.
The rotated untouched-wall regression makes it insufficient for this case.

**Recover a source chart by inverting world coordinates.** This needs less retained state.
It cannot recover the original authored coordinates after floating arithmetic has rounded them, so it provides weaker evidence for reference identity and small residual material.

## References

- [Source partition and failure controls](../../crates/multicad-rust-kernel/src/planar_boolean/straight_profile/tests.rs)
- [Common placement and invalidation controls](../../crates/multicad-kernel/src/geometry/planar_boolean_body/certified.rs)
- [Short frame-member workflow](../../crates/multicad-model/tests/weldment.rs)
