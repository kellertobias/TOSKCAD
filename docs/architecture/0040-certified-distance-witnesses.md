# ADR-0040: Certified distance witnesses without an enumerated total

| | |
| --- | --- |
| Status | Accepted implementation design; application cutover remains separate |
| Date | 2026-11-12 |
| Deciders | Codex, within the implementation authority for CAD-290 |

## Context

A distance proof can establish the minimum and provide a valid closest pair without enumerating every finite solver solution.
The machined-stock fixture with twenty rectangular openings exceeds the 64 stored-witness limit.
The previous result contract required a known solution count and therefore refused this otherwise certified distance.

Stock bounds also permit a useful partial proof after machining.
The uncut stock supplies a distance lower bound, and a surviving closest pair establishes the same minimum for the retained material within the query precision policy.
A conservative cut bounding box can leave other candidates unclassified.
Those candidates cannot be counted as either retained or removed.

## Decision

Represent `total_solutions` as an optional unsigned count in the kernel, model and desktop result types.
A known count retains the existing finite solver enumeration semantics.
An unknown count is `None`, serialized as JSON `null`, with `witnesses_complete` set to `false`.
Serialization emits the field; it does not omit unknown totals.
These are result objects, and this change does not add an MCP tool or response field.

Accept a machined-stock scalar proof when at least one returned stock pair is proved to remain material on both sides.
Keep the original count only when the stock enumeration is complete and every original candidate survives.
Otherwise retain the certified witnesses and report an unknown total.
If no candidate proves the minimum, return the existing unsupported result.

The desktop accepts valid incomplete results and labels their total as unenumerated.
It does not infer unique axis components from incomplete witnesses.
The existing occupied-volume proof still determines contact and interference classification.
Canonical measurement pins continue to store references and intent rather than numeric results.

## Consequences

**Positive:** Bounded witness publication no longer prevents a certified clearance result.
Existing known counts retain their values and JSON number representation.
One metadata validator serves assembly reports and desktop measurement publication.

**Negative:** Typed consumers must handle an unknown total.
Partial witnesses cannot establish unique axis components or an exhaustive solution count.
The stock proof still cannot solve a closest pair that lies on a removed cut wall.

**Neutral:** Pure planar and native solvers continue to compute their existing known counts.
This change does not claim faster exhaustive enumeration or complete curved-surface distance support.

## Alternatives considered

**Require a complete count.** This preserves the old result shape but rejects the reproduced dense-stock clearance.

**Assign a sentinel count or count only the displayed witnesses.** This would misrepresent unclassified candidates and could support false completeness or uniqueness claims.

## References

- [Kernel result](../../crates/multicad-kernel/src/distance.rs)
- [Stock proof and witness publication](../../crates/multicad-kernel/src/distance/machined.rs)
- [Shared result semantics](../../crates/multicad-core/src/inspection/values.rs)
- [Report validation and JSON round trip](../../crates/multicad-core/src/assembly_inspection.rs)
- [Dense-stock and material controls](../../crates/multicad-kernel/tests/rust_machined_distance.rs)
- [Desktop publication controls](../../crates/multicad-ui/src/inspection/tests.rs)
- [CAD-290](https://plans.tokenet.de/CAD/290)
