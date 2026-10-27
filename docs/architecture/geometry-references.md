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
rejects stale reports. A host must wire that gate to its actual scene and use
the current document revision.

`prepare_repair` accepts an explicit current candidate and produces a shared
transaction preview. Cancel drops the preview. Commit replaces the source link
and intent as one attributable, undoable operation, preserving its reference UUID
and authored attachment offsets. Stale, unnameable and wrong-kind candidates
fail without changing source.

The protocol tests exercise reorder, split, deletion, repair, undo/redo, stale
results and JSON round trips with synthetic topology snapshots. They do not
establish native topology provenance or visible application repair behavior.
