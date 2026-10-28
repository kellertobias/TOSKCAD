# Source-linked sketch geometry

[sketch_external](../../crates/multicad-core/src/sketch_external/mod.rs) supplies
current native geometry to constrained sketches before numerical solving. Version
1 binds a canonical Feature through compact analytic descriptors. Version 2
stores complete source and optional receiving occurrence paths, an exact finite
query and a semantic result selector. Both use an external-reference Extension
owned by one sketch.

Constrained sketch schema 4 requires semantic evaluation and remains sticky after
removing the final reference. Authored point, radius and spline-basis guesses stay
in canonical source. Native evaluation supplies current geometry and transient
solver locks. Successful returned candidates omit those transient constraints.
The native container declares core.constrained_sketch.4, sketch.external.1
and, when needed, sketch.external.2. Unsupported readers reject the capability.

## Capture and native authority

Legacy [CaptureRequest::new](../../crates/multicad-core/src/sketch_external/capture.rs)
checks snapshot ownership, revision, provenance and supported analytic geometry.
Its caller must supply a current source-bound native topology map; the function
cannot authenticate arbitrary caller-created descriptors.

Version 2 uses
[Compiler::query_external_catalog](../../crates/multicad-model/src/external.rs)
for detached exact results. The query names a source Part or one explicitly
retained Feature in that Part's history. Its complete occurrence path identifies
its placement. A receiving context must own or consume the sketch, or own its
mounting plane. A world/definition plane remains an explicit alternative.

Native queries use source-local B-reps, then apply the source-to-consumer relative
pose once. Shared occurrence prefixes cancel before matrix evaluation. Geometry
ordering includes current diverging pose drivers and their suppliers, while
structural links retain invalidation and repair diagnostics. The absolute world
pose used for display is resolved separately. The reasoning and costs are in
[the occurrence reference decision](0023-occurrence-sketch-references.md).

## Supported geometry and selection

The legacy projection math remains in
[geometry.rs](../../crates/multicad-core/src/sketch_external/geometry.rs).
Version 2 uses the
[native finite query adapter](../../crates/multicad-kernel/src/sketch_references.rs)
and [semantic catalogue builder](../../crates/multicad-model/src/external_geometry.rs).

| Version | Method | Native input | Output and boundary |
|---|---|---|---|
| 1 | Coplanar | Point, finite line, circle or circular arc | Same supported family; rejects off-plane geometry |
| 1 | Intersect | Finite straight edge | One crossing point; rejects coincident or absent intersections |
| 2 | Coplanar | Supported finite point, edge or planar face boundary | Exact point or curve choices on the sketch plane |
| 2 | Project | Supported finite point, edge or face boundary | Orthogonal projection; tilted conics use exact rational splines |
| 2 | Intersect | Supported finite edge or face | Actual finite point/curve contacts, including face-hole clipping |

Native edge families include lines, circular arcs/circles, ellipses and supported
Bezier/B-spline curves. Projection trims the native input and maps rational poles
without fitting. Unsupported curve families, singular conics, collapsed splines
and closed retraced line projections produce diagnostics. A finite face query
uses its B-rep extent; a plane descriptor alone never invents an unbounded wall
intersection.

Each binding owns one chosen point or curve. Several captures can form a derived
closed profile. Multiple results require an explicit choice. Persistent selectors
use source ancestry, family and result count. Finite edge contacts also use native
parameter bands and crossing direction where those prove a branch identity.
Native topology ordinals and display coordinates never become persistent result
selectors. Lost or ambiguous support requires repair.

Empty and coincident results remain visible. Proven analytic tangencies are
reported and cannot authorize stable capture. General spline-surface
transversality classification is outside the verified capability.

## Evaluation and source preservation

[references::dependency_hash](../../crates/multicad-core/src/references/dependency.rs)
binds source diagnostics to their canonical dependencies. Version 2 adds native
result receipts with query, selector, plane and pose hashes. Current semantic
preflight checks ownership and paths during receipt consumption. Consumer-only
constraint edits can reuse unchanged scoped geometry; upstream source or relevant
placement changes invalidate it. Public discovery and proposal adoption also
require the complete current source hash.

ReferenceReport carries exact external results and explicit errors. An
inspect-only report marks version 2 geometry unavailable until native evaluation
supplies its receipt. Full model publication includes unconsumed sketch bindings.
The [Scene adapter](../../crates/multicad-ui/src/viewport/scene.rs) retains the
source-bound native report through visibility filtering, so the UI cannot replace
it with a clean report reconstructed from display descriptors.

ReferenceOnly makes a curve construction geometry. Derived permits profile
participation. Native evaluation fixes each output's current geometry during
solving while preserving authored guesses and compatible output identities.
Generic edits cannot move, trim or remove protected outputs. Explicit removal
rejects remaining dependent geometry or constraints rather than deleting them
implicitly. A changed output family or pole count can require dependent repair.

## Native UI and repair

The [reference controller](../../crates/multicad-ui/src/external_references/mod.rs)
provides Project and Intersect tools in an opened sketch. Its Inspector separates
the source occurrence, current Part or retained Feature, receiving context, query
method and exact result. A scoped leaf view uses the complete native placement
receipt. Source-definition inspection is labelled separately when recovering a
broken model.

Discovery, preparation and removal run through a bounded latest-only worker.
The [backend contract](../../crates/multicad-ui/src/external_references/backend.rs)
uses immutable preparation snapshots. Replies retain their complete inputs, and
the controller checks Session ownership, source, resources, actor and canonical
candidate contents. Cancel supersedes queued work without changing source.

The narrow
[ConstraintEditor adoption lane](../../crates/multicad-ui/src/sketch_external_preview.rs)
permits changes only to the owning sketch and requested reference record. Apply
requires a complete candidate model with resolved driving references. Source-only
inspection can supply a replacement while a downstream solid is broken, but
cannot authorize Apply, drawing generation or manufacturing output.

Compatible single-reference repair preserves output identities. Several
independently broken references can require a staged batch before a complete
candidate exists. That separate prerequisite is tracked in
[the batch repair plan](https://plans.tokenet.de/CAD/134) before the generic
integration checkpoint and Room work.

Native geometry coverage is in
[model reference tests](../../crates/multicad-model/tests/occurrence_external.rs),
[kernel query tests](../../crates/multicad-kernel/tests/sketch_references.rs) and
[production backend tests](../../crates/multicad-app/tests/occurrence_reference_backend.rs).
Controller tests cover actual result controls, ownership/resource guards and
native checked preview adoption. Automated evidence, macOS interaction and human
acceptance remain distinct.
