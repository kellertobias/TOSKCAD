# ADR-0009: Keep face sketches in producer coordinates and place them through UI occurrences

| | |
|---|---|
| Status | Accepted engineering decision; native workflow acceptance remains separate |
| Date | 2026-10-27 |
| Decider | Codex, using the delegated architecture authority |

## Context

A part can appear at several assembly placements while its feature history owns
one reusable sketch definition. Linking that sketch to the part's final output
can introduce a dependency cycle when the sketch drives another feature in that
part. The supporting face needs an upstream producer identity.

The source frame and the displayed frame can have different origins and axes.
Rigid occurrence placement supplies that difference. Existing explicit Instance
anchors already resolve from world-space topology, so applying an additional
occurrence transform would place them twice.

## Decision

Persist each new face-plane reference against its canonical, unsuppressed Feature
producer in producer coordinates. Capture semantic face intent through the shared
reference API. Display triangle indices and evaluation-local face ordinals remain
temporary selection data. [Geometry references](geometry-references.md) owns the
provenance and repair contract.

Keep the selected occurrence and its rigid placement in UI state. Use the shared
`sketch_context::occurrence` resolver when previewing or editing a component-local
plane or sketch. A current scene supplies the displayed pose. Independent datum
planes stay in world coordinates. Explicit Instance anchors that already resolve
in world coordinates receive no additional occurrence placement.

Keep plane selection and preview source-neutral. The picker returns the producer
snapshot and selected planar frame separately from the occurrence and world
preview. `PlaneOptions::apply` supplies the offset and angle calculation used by
both preview and creation.

Create sketches through `sketch::prepare_new`, followed by `Session::commit`.
Callers supply an actor and the expected source revision. The factory prepares
the required plane entities and an empty constrained sketch as one undoable
transaction. The typed command is the common boundary for UI and external-control
adapters.

Require face snapshots from the host's current trusted geometry evaluator. The
factory checks producer kind, revision, topology kind and reference resolution.
The UI also checks the document and display generation before accepting a hit.
The host remains responsible for supplying truthful evaluator metadata; these
checks do not authenticate arbitrary caller-created snapshots.

After this empty-sketch insertion alone, the host may inspect the unchanged
producer topology at the resulting revision to initialize the new attachment.
Bind that creation-only report to the document identity and exact revision. Drop
it on the next source edit. A subsequent attached-sketch edit requires evaluation
for its own current revision.

## Consequences

**Positive:** A component can reuse its local sketch definition across placements
without making the final part or its occurrence a feature-history parent. Preview
and committed creation use the same plane transform and transaction validation.

**Negative:** UI adapters must carry occurrence context and convert between source
and display frames. A component-local preview requires current placement data;
retained geometry from another revision cannot authorize it. External-control
adapters must obtain trusted snapshots and refresh diagnostics after later edits.

**Neutral:** Reopening a sketch requires an occurrence-selection policy in the UI.
The canonical plane reference does not preserve that viewing choice. Existing
explicit Instance anchors keep their world-frame interpretation.

## Alternatives considered

**Anchor to the selected final Part or Instance:** Picking already returns that
identity and its displayed frame, which would reduce coordinate conversion in the
UI. A final-part link can create a history cycle as its feature list grows, and
an occurrence parent couples the definition to one placement. New face sketches
therefore reference their upstream Feature producer.

**Store the displayed plane as fixed world coordinates:** Rendering and source
inspection would need no occurrence context. The plane would lose its semantic
attachment to upstream geometry, and each placement would need its own definition.
Producer-local intent preserves that parametric relationship.

## References

- [Shared plane inspection and prepared sketch creation](../../crates/multicad-core/src/sketch.rs)
- [Core plane, stale-reference and acyclic-history tests](../../crates/multicad-core/tests/sketch_planes.rs)
- [Source-neutral picker and rotated-occurrence preview tests](../../crates/multicad-ui/src/sketch_plane.rs)
- [UI occurrence resolution and world-anchor bypass](../../crates/multicad-ui/src/sketch_context.rs)
- [Creation-only report lifetime and sketch editing context](../../crates/multicad-ui/src/sketch_editor.rs)
- [Shared immutable display geometry](0007-display-scenes.md)
