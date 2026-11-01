# Fence constructions and runs

Fences use one canonical construction, a placed path and named independent
member Parts. The core source API and native evaluator implement these records.
Graphical authoring, fence MCP tools and cross-project Library integration are
still pending. The source lives in
[core/model/fence](../../crates/multicad-core/src/model/fence/mod.rs); the exact
provider is [model/fences.rs](../../crates/multicad-model/src/fences.rs).

## Canonical records

| Record | Ownership and use |
| --- | --- |
| `architecture.fence_type` schema 1 | Named project extension containing `FenceConstruction`; no sample path or generated geometry |
| `architecture.fence` schema 1 | Run extension linking the construction, stable path vertices, group, optional level, instance overrides and explicit interior-post positions |
| `architecture.fence_member` schema 1 | One Feature per post or infill member, linking its run and semantic key and naming one output Part |

The corresponding required capabilities are `architecture.fence_type.1`,
`architecture.fence.1` and `architecture.fence_member.1`. Older readers refuse
unknown required capabilities. Materials are canonical Material entities.
Types retain bound dimensions and material links; they do not create another
installation Library.

Construction height is the post height. Rectangular posts have local along-run
width and cross-run depth; round posts have a diameter. Preferred spacing is
between post centers. Stable named infill recipes have a material, bottom,
height, signed normal offset and one supported construction:

- Panel: actual thickness and either a clear-bay fit or a purchased width.
- Rail: one horizontal rectangular member at the recipe's authored height.
- Pickets/slats: repeated vertical members with fixed width/depth and an explicit
  maximum clear gap.

Separate recipes author separate rails or materials. Post-height and spacing
overrides belong to a run. Fixed infill dimensions remain physical dimensions;
changing post height never scales a purchased panel. An infill above the posts
requires an explicit construction repair. Optional level context adds the level
elevation to the retained floor-relative `base_z` value.

## Post and bay layout

Every path vertex owns one post, including shared corners and closed-path
closure. Interior post keys contain directed segment start/end vertex IDs and
an ordinal. Bay keys contain their bounding post keys; infill keys additionally
contain the recipe ID and member ordinal. Names follow traversal order and do
not define identity.

Equal layout divides each path segment into equal center pitches no longer than
the preferred spacing. ShortLast retains the preferred pitch and shows the
remaining shorter bay. Clear widths subtract the actual projected post profiles,
including a rectangular corner post's rotated depth. Rectangular-post paths
currently support straight and right-angle turns. Round-post paths support other
nondegenerate turns, subject to exact native overlap validation.

An interior-post override is a center distance from its directed segment start.
Moving a path vertex edits the path itself. Duplicate, crossed, out-of-segment
or missing post overrides require repair. Rewiring an edge never silently
retargets an old override. Self-crossing, overlapping/touching, doubled-back and
zero-length paths are refused.

Purchased panels either refuse a shorter bay or explicitly permit cutting down;
their source width never stretches. Native layout records identify a cut panel's
original width. Pickets keep their authored width with equal end/interior gaps
bounded by the configured maximum. Impossible spans fail as a complete candidate.

The member limit derives from the canonical transaction limit: one Feature and
Part per member plus reserved type/run/group/hierarchy edits. A single new run
can contain at most 2,046 members. Shared changes must also fit one atomic
transaction; an oversized change is refused before source mutation.

## Atomic editing and references

`prepare_fence_change` stages an optional new or shared construction with an
optional run create/edit. Shared edits reconcile all consuming runs. A current
run's post-position repair can be included in the same candidate; another run's
unresolved override refuses the entire edit and identifies that run.

Retained semantic keys keep their Feature and Part IDs. Parts retain unrelated
description, tags and downstream history. Removed members with downstream or
external references require repair. Their owned manufacturing metadata is removed
with them only when removal is safe. Source and group names remain editable;
long Unicode names stay intact in source while generated labels are bounded.

These preparations are source-only. Native preflight must succeed before an
adapter offers Apply or commits a manufacturing-capable result. Undo/Redo and
TCD persistence retain the complete definition and generated identity map.

## Native solids and cached results

The provider checks the complete member set and material assignments once per
run in each native compile, before accepting any member cache hit. Missing,
stale, suppressed or misassigned members fail explicitly.

On a miss, the provider builds the entire run before returning its first member.
Rectangular members use exact named profile extrusion; round posts use exact
circles. Every member must be one positive-volume solid with measured volume
matching its source dimensions. Dimensions below the active kernel minimum fail
without inflation. Members remain independent material Parts.

A sweep of native bounds prunes possible collision pairs. Boolean COMMON with
zero added fuzzy gap closing decides occupied material overlap. Touching members
remain separate; positive-volume overlap is refused without fusing it away.
Cancellation between native operations returns no partial member result.

Cached geometry depends on typed construction/run/path/level/parameter input
closures. A completed member entry follows a completed whole-run geometry proof,
including for source-scoped compilation. Complete current source and material
validation still precedes reuse. Parameter or level changes invalidate geometry;
unchanged repeats reuse exact solids and display snapshots.

## Evidence and remaining integration

[Core source tests](../../crates/multicad-core/tests/fence_construction.rs),
[layout tests](../../crates/multicad-core/tests/fence_layout.rs) and
[transaction tests](../../crates/multicad-core/tests/fence_records.rs) cover
dimensions, identities, explicit layout/repair policy, atomic shared updates,
history, TCD compatibility and transaction bounds.
[Native tests](../../crates/multicad-model/tests/fences.rs) measure real Parts,
gaps, materials, rotated/closed layouts, STEP readback and cache failure cases.

The graphical construction editor, viewport placement/edit controls and fence
MCP adapters remain to be implemented. Representative linked plan/elevation and
PDF/DXF verification remain pending. The existing ordinary Part drawing/material
path is the intended publication path, without separate manually authored
geometry. Project-authored path/context symbol materialization and pinned Library
updates belong to [CAD-206](https://plans.tokenet.de/CAD/206).

Terrain following, raked/stepped slopes, gates and engineered foundations are
outside this flat-run implementation. Actual macOS graphical checks and human
acceptance remain separate from native CLI and source tests.
