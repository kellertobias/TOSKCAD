# ADR-0007: Share immutable display geometry across scene instances

| | |
|---|---|
| Status | Accepted engineering decision; interactive viewport acceptance remains separate |
| Date | 2026-10-27 |
| Decider | Codex, using the delegated architecture authority |

## Context

A room or assembly can contain many instances of the same part. Duplicating its
mesh and picking structure for every placement wastes memory. Display triangles
also change when tessellation quality changes, so they cannot identify features,
faces or repair candidates in saved source.

## Decision

Keep the native B-rep in the geometry worker. Publish owned display triangles with
their evaluation-local B-rep face indices, sampled B-rep edges and B-rep vertices.
Use the original face map when tessellating a detached copy. Wireframe edges must
come from the B-rep rather than triangle diagonals.

Use immutable `viewport::Geometry` assets shared through `Arc`. Each asset owns
its mesh and a triangle bounding-volume hierarchy. A `Scene` stores rigid instance
placements and canonical instance identities separately. Picking transforms a
world ray into each instance's local coordinates, tests its shared hierarchy and
returns the nearest surface, regardless of instance order.

A hit carries the canonical instance and geometry identities. Its face ordinal
belongs only to that evaluation. Persistent face intent is captured from semantic
provenance; an unnamed or ambiguous face supports body selection without producing
a persistent topology reference. Instance topology snapshots transform semantic
attachment frames to world coordinates.

Use one normalized camera ray calculation for both solid picking and sketch-plane
intersection. Keep scene publication behind document, revision and generation
checks. A pending or rejected build retains the last visible scene, and the host
must check `SceneStore::is_current` before using it for source edits.

## Consequences

One thousand placements can share one mesh and hierarchy. Scene assets contain no
native handles and can be prepared outside the UI event loop. Asset construction
is bounded and validates source-face associations, indices and finite coordinates.

The scene module currently provides headless face picking and publication. GPU
buffer caching, edge/point picking, overlays and application integration remain
implementation work. Its CPU timing tests do not establish GPU frame rate, full
pointer latency, process memory or idle CPU. Those require the physical Mac
acceptance scenes in CAD-57.

## Evidence

- `multicad-kernel/tests/display.rs` checks source-face associations at two mesh
  qualities and a box's twelve real edges and eight vertices.
- `multicad-ui/tests/viewport.rs` checks nearest-surface selection, world attachment
  frames, both camera projections, real profile holes and curved faces, shared
  geometry for 1,000 placements, malformed input and stale publication rejection.
