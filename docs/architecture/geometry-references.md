# Geometry references

`multicad_core::references` separates saved geometry intent from the face, edge
and vertex ordinals of one evaluation. A document `Link` can carry a stable
reference UUID, topology kind and semantic source provenance. Old links without
geometry intent remain readable.

Resolution requires a current topology snapshot from the linked source. It
matches provenance, with an optional exact boundary discriminator chosen during
repair. It reports missing, ambiguous, stale and invalid results explicitly.
It never chooses a replacement from geometric proximity or a saved face index.
Producer code must supply truthful semantic provenance; this protocol cannot
infer it from an unlabelled mesh or imported solid.

Planar attachments store an offset and rotation relative to a semantic source
frame. Each evaluation applies these to the current source frame. Repeated
rebuilds do not accumulate displacement. A missing or conflicting source frame
prevents attachment resolution.

`inspect` returns reference diagnostics with source, candidates and affected
dependents. Reports and topology snapshots are serializable inspection data;
they are derived results rather than canonical document source. Saving a
document preserves the intent needed to regenerate them. `ReferenceGate` retains
its last valid presentation when a new report contains unresolved references and
rejects stale reports. The native `SceneFeed` applies that gate before publishing
a source scene. An unresolved candidate stays available for explicit repair
inspection while the previous valid scene remains visible and normal model
picking is disabled. Source revision and display generation checks still apply.

`Scene::reference_snapshots` includes hidden upstream geometry producers as well
as placed instances with world-space frames. `ReferenceEvaluation` exposes the
current report, snapshots and candidate scene to the Inspector. The repair view
can isolate a current source and highlight an explicitly chosen face, edge or
vertex without replacing the retained valid scene or changing the document.

`prepare_repair` accepts an explicit current candidate and produces a shared
transaction preview. Cancel drops the preview. Commit replaces the source link
and intent as one attributable, undoable operation, preserving its reference UUID
and authored attachment offsets. Stale, unnameable and wrong-kind candidates
fail without changing source.

`Solid::extrude_profile_named` supplies native prism caps and sides with semantic
feature and curve origins. Authored curve order remains identifiable when a wire
is rewound for OCCT. Planar side frames come from the authored line and extrusion
direction; cap frames come from the authored profile plane and signed height.
Curved sides have no planar attachment frame. `box_named` uses this same path.

The native adapter records exact unchanged, Modified and Generated subshapes
from Boolean and rigid transform operations. Rust propagates their provenance
and semantic frames. Split fragments retain their common source origin and
therefore resolve ambiguously. Conflicting frames are omitted rather than
chosen by iteration order. Face adjacency supplies boundary discriminators and
edge/vertex provenance. Imported or otherwise unnamed geometry has no inferred
source identity and cannot be captured as a stable reference.

`Sketch::read_evaluated` and constrained `Source::read_evaluated` require a current,
matching diagnostic to obtain an attached sketch's world frame. The ordinary
read methods reject geometry attachments so they cannot accidentally display or
manufacture them on the saved datum fallback. The canonical document can still
save and load an unresolved attachment for repair.

Protocol tests exercise reorder, repair, undo/redo, stale results and JSON round
trips. Native OCCT tests additionally cover dimension changes followed by a
face-mounted drill circle and actual Boolean hole, a split and explicit fragment
repair, deleted-face rejection, transformed frames and wire winding with holes.
Worker tests also verify named result publication and revision guards across
rebuilds. The `reference_review` native fixture adds parameter-driven panel
thickness, splitting and top removal, with an attached circle and an actual
Boolean drilling. Its tests exercise the application scene gate, explicit
repair, undo/redo and source save/reopen. This is an acceptance fixture; the
general canonical feature compiler remains separate production modeling work.
The repair Inspector was checked in the native Mac application, including
preparing/cancelling, accepting, undo/redo and reopening the repaired source.
Attached-sketch opening and constraint drafts require current resolved evaluation;
missing, stale and unresolved reports cannot authorize an edit. The native check
also changed the attached circle radius without changing its saved anchor.
Evidence and boundaries are recorded in the
[reference verification report](../verification/reference-repair-2026-10-27.md).
