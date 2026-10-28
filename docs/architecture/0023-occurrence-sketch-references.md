# ADR-0023: Resolve finite sketch references through explicit occurrence paths

| | |
|---|---|
| Status | Accepted engineering design; end-to-end verification and workflow acceptance remain separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

A sketch can reference a reused Part in several translated or rotated assemblies.
Its receiving Part may belong to the same assembly as its source. Depending on
that complete assembly's geometry can introduce a false feedback cycle.

Finite curved edges and faces also require geometry beyond the compact analytic
descriptors used for display topology. Several genuine intersections can share
one source, and their order alone cannot identify a persistent reference.

## Decision

Store complete source and optional receiving occurrence paths in a versioned
external-reference record. Default source selection to a Part's current head;
permit an explicitly retained Feature from that Part's history. Validate source
ownership and receiving sketch ownership when creating or repairing a binding.
Keep broken saved paths readable so users can repair them.

Evaluate source B-reps in their canonical local coordinates. Cancel a shared
occurrence prefix before resolving relative poses, then place the source once in
the receiving definition's coordinates. Resolve the sketch plane separately.
Retain pose links for structural diagnostics and invalidation, while geometry
ordering depends on the current diverging drivers and their actual suppliers.

Keep absolute world placement outside the per-feature relative geometry query.
Public discovery and display receipts carry a separately resolved world pose.
This prevents a shared parent's placement from becoming a prerequisite for
geometry that can be evaluated after cancelling that parent.

Query finite native edges and faces. Coplanar references require geometry on the
plane; explicit projection uses an orthogonal affine map. Project rational poles
without fitting and preserve their basis and weights. Intersect actual finite
geometry with the plane, including face holes. Report empty, coincident, singular
and proven tangent cases explicitly.

Persist semantic source support and a provable branch selector. Native topology
keys are evaluation-local lookup handles. Use source parameter bands and crossing
direction where they distinguish finite edge contacts. Require explicit repair
when count, family, support or branch no longer identifies one result.

Give each captured point or curve fixed output identities. Preserve compatible
identities through repair. Derived curves may form profiles; reference-only
curves remain construction geometry. Fix their current native geometry during
solving without writing evaluated coordinates back into authored guesses.

Run discovery and preparation on a bounded latest-only native worker. Pass a
read-only preparation snapshot retaining the live Session identity. The UI checks
the complete request, source, resources and Session before adopting a reply.
Apply additionally requires a complete candidate model and the normal guarded
transaction. Source-only inspection can help repair a broken consumer, but cannot
authorize manufacturing or replace a complete canonical scene.

Bind internal geometry receipts to document identity, revision, query, semantic
selection and source, plane and pose dependencies. Consumer-only constraint edits
may reuse unchanged scoped geometry. Public discovery and proposal adoption also
require the exact complete source hash.

## Consequences

Reused Parts can supply independent associative references in their actual
placements. Geometry and its persistent links commit together through the same
typed preparation API used by native UI and external control.

The dynamic pose dependency projection and receipt checks add implementation and
testing cost. Unprovable branch identity produces a repair requirement. A changed
output family or pole count may prevent compatible repair. Each reference owns
one point or curve, so a projected boundary can require several captures.

Proven analytic tangencies are conservatively unavailable for stable capture.
General spline-surface transversality classification remains outside the verified
capability. Explicit finite-domain and singular-projection diagnostics reduce the
available selections rather than substituting sampled geometry.

## Alternatives considered

World-space aggregate topology simplifies initial picking and already reflects
placement. Its dependency on complete receiving assemblies can create false
cycles, and it obscures the source definition behind nested reuse.

Nearest-result rebinding is convenient after edits. It can silently attach a
dimension or profile to another intersection, so ambiguous changes require repair.

Sampled projection supports broad rendering quickly. Its approximation depends
on display quality and cannot preserve the analytic manufacturing source.

## References

- [Canonical occurrence contracts](../../crates/multicad-core/src/sketch_external/occurrence.rs)
- [Ownership and dependency contexts](../../crates/multicad-core/src/sketch_external/context.rs)
- [Native finite geometry queries](../../crates/multicad-kernel/src/sketch_references.rs)
- [Occurrence geometry and semantic result catalogues](../../crates/multicad-model/src/external_geometry.rs)
- [Native UI query boundary](../../crates/multicad-ui/src/external_references/backend.rs)
- [Related feature transfer decision](0021-associative-feature-transfer.md)
