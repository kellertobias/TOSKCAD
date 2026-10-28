# ADR-0018: Scale exact geometry through owned occurrence components

| | |
|---|---|
| Status | Accepted engineering decision; workflow acceptance remains separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

Scaling a Part must update its dependent instances. Scaling one selected Instance
must preserve the other occurrences and the shared Part definition. Both operations
need editable factors, a pivot and a coordinate frame that survive native export
and project reopening.

The [placement contract](0017-part-and-instance-placement.md) keeps occurrence
poses rigid. Its reference consumers depend on orthonormal rotations and do not
interpret a pose as an arbitrary affine geometry operation. Display mesh transforms
also cannot supply the exact solids required by those consumers.

## Decision

Persist Scale as a distinct `model.scale` feature with the `model.scale.1`
capability. Store positive dimensionless factors, a millimeter pivot and a
right-handed orthonormal basis. Factor and pivot expressions use the canonical
parameter bindings. Zero, negative, nonfinite and unsupported transforms return
diagnostics before committing source.

For basis matrix `A`, factors `f` and pivot `p`, resolve the world transform as
`M = A * diag(f) * transpose(A)` and translation `p - M*p`. Exact equality of
the factors denotes uniform scaling, producing a literal scalar identity matrix
without rounding small off-axis values. Native B-rep validity still uses the
configured [geometry policy](0006-geometry-policy.md).

A Part definition Scale freezes its preceding `BodyInput` producer and appends
to that same Part history. A selected Instance instead receives an owned derived
Part containing a Scale feature. That feature references the source component's
current head, so upstream geometry edits continue to propagate.

Capture the Instance's preceding rigid placement base `B` in the owned Scale
feature. Evaluate the geometry in component coordinates using `inverse(B) * S * B`,
then place that geometry with the Instance's current rigid pose. Rebind only the
selected Instance's component; preserve its placement and driver identities.
Later rigid moves therefore operate on its scaled geometry without applying scale
to `RigidPose`.

The owned feature's Instance identity is ownership metadata, with no dependency
back to that Instance. Its component and captured preceding driver are actual
dependencies. Validate older alignment captures against the explicit typed
component ancestry, preserving their authored local geometry source. Missing,
cyclic or differently owned chains fail instead of selecting a historical fallback.

Keep owned components in the reference-source map, but omit their independent
entries from `ModelSnapshot.bodies`. Scene publication and native consumers then
receive each explicit Instance's world solid once. Whole-project export skips
owned standalone Parts, and direct selection of such a private Part requires
export through its owning Instance.

Clone the source Part's material, tags and explicit stock/appearance/hatch
assignment record when creating an owned component. The new record has independent
ownership and retains its live library references with acknowledged content hashes.
This captures the assignment at creation; later reassignment of the source Part
does not implicitly reassign the owned component.

Allocate copied Instance, component, feature, driver, reference and assignment
identities in `ScaleBatchRequest::new`. Repeated candidate preparation retains
those identities. Native preview and Apply use the existing source-bound candidate
lane, with the original Session, revision, form generation and native receipt
checked before one transaction can commit.

The kernel adapter applies Scale to native solids and preserves topology provenance
through the exact native modification history. Supported planar, linear and point
descriptors follow the affine transform mathematically; a descriptor that cannot
be established remains unavailable. Curved geometry never receives a circular
descriptor merely because its source was circular.

## Consequences

Part edits and independent occurrence edits share the canonical transaction and
native evaluation boundaries. An Instance's exported solid includes its exact
scaled component and rigid placement once. Fixed identity allocation preserves
copy ownership through preview retries, Undo/Redo and container reopening.

Each independently scaled Instance adds a component feature branch and native
geometry. The compiler must retain that branch for references while excluding it
from standalone physical outputs. Consumers that enumerate raw Part entities
instead of canonical physical outputs must apply the same ownership rule.

Captured assignments can diverge after later source reassignment. The owned
component retains its explicit stock specification even when its scaled dimensions
differ; that metadata alone makes no manufacturability assertion. Geometry ancestry
and cloned alignment drivers also add validation and dependency work.

General nonuniform transforms can produce exact NURBS representations of planar
faces. The native planar-contour DXF extractor currently rejects those unsupported
face or curve representations. Exact STEP roundtrips and semantic planar picking
have native tests; neither establishes DXF eligibility for every scaled solid.

Negative factors and subassembly scaling remain outside this operation. The host
owns target acquisition, coordinate selection and physical interaction acceptance.
Low-power performance and Windows/Linux runtime need separate evidence.

## Alternatives considered

**Scale only viewport geometry.** This reuses display transforms and avoids new
native geometry. Exact export and reference consumers would still see the source
solid, so the displayed model would have no matching manufacturing geometry.

**Extend `RigidPose` with scale.** This keeps one occurrence record and permits
compact display transforms. It changes every rigid reference and placement
consumer's contract, while native consumers still require a non-rigid solid
operation.

**Scale the shared Part for an Instance selection.** This reuses existing Part
history without a private branch. It also changes every dependent occurrence,
contradicting explicit selection of one Instance.

**Clone the complete Part history.** This supplies independent editable history
using existing producers. It duplicates upstream intent and removes the desired
dynamic relationship to the shared source geometry.

## References

- [Scale parameters and affine math](../../crates/multicad-core/src/model/scale/math.rs),
  [source contract](../../crates/multicad-core/src/model/scale/source.rs) and
  [fixed request identities](../../crates/multicad-core/src/model/scale/request.rs)
- [Atomic source preparation](../../crates/multicad-core/src/model/scale/prepare.rs)
  and [canonical transaction tests](../../crates/multicad-core/tests/model_scale.rs)
- [Native evaluation](../../crates/multicad-model/src/scale.rs),
  [physical output publication](../../crates/multicad-model/src/compiler.rs) and
  [exact native regression tests](../../crates/multicad-model/tests/scale.rs)
- [Export scope resolution](../../crates/multicad-core/src/exchange/resolve.rs)
  and [manufacturing assignment contract](manufacturing-properties.md)
- [Native affine adapter](../../crates/multicad-kernel/src/affine.rs) and
  [adapter fixtures](../../crates/multicad-kernel/tests/affine.rs)
- [Scale Inspector](../../crates/multicad-ui/src/modeling/scaling.rs),
  [gizmo](../../crates/multicad-ui/src/modeling/scale_gizmo.rs) and
  [native pointer tests](../../crates/multicad-ui/src/modeling/controller/scale_tests.rs)
- [Editable native fixture generator](../../crates/multicad-app/examples/scale_review.rs)
