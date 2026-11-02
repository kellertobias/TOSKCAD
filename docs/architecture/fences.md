# Fence constructions and runs

Fences use one canonical construction, a placed path and named independent
member Parts. The native editor, core source API and native evaluator share
these records. MCP uses the same source transaction and exact native preflight.
Cross-project Library integration is still pending. The source lives in
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

Compatible semantic keys within the same infill kind keep their Feature and Part
IDs. Changing a panel, rail or picket recipe into another kind replaces its
physical members, even when the recipe ID remains stable; old references require
repair instead of following the first replacement member. Parts retain unrelated
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

The Architecture Fence command opens a representative-bay construction editor
and run form. Selecting a post or infill band synchronizes the dimension/material
fields. Saved constructions are read-only until an explicit shared edit; the form
lists affected runs and can duplicate the draft as a separate construction.
Subsequent drawn runs reuse the selected saved construction.

Viewport placement shares the bounded wall/slab outline interaction and snapping.
Retained corners keep local IDs through moves; redraw cancellation restores the
previous identities. The form shows actual bay widths, level/base placement,
post-height/spacing overrides and explicit interior-post positions. A distinct
post-plan query keeps automatic positioning choices available when infill cannot
fit; it does not establish a complete layout or native geometry. Apply requires
the current candidate's successful exact native preview.

Component tests exercise painted recipe selection, shared/duplicate controls,
viewport path clicks, native Preview/Cancel/Apply, history and TCD reopen. These
checks do not establish actual macOS window interaction.

Project-authored fence constructions can also expose selected typed settings
through [pinned architectural symbols](architectural-symbols.md). The existing
Library opens Draw Fence for these definitions; each receiving run owns its path
while generated members follow the canonical fence rules.

### External control

`set_fence_type` creates a named construction when `fence_type` is omitted.
Supplying an existing ID explicitly edits every consuming run. An unused type
is source-only; used types require native preflight. `inspect_fence_type` returns
the construction in the unit-bearing request shape. Retain recipe IDs on edits;
unchanged expressions keep their original parameter bindings through renames.

`preview_fence` and `set_fence` accept the same complete run request. Omit `fence`
to create, or supply its canonical run ID to edit. Supply all path vertices,
context and height/spacing overrides. Retain a vertex ID when moving that corner;
an omitted ID creates a new corner. Null level selects the world plane, and null
height/spacing inherits the construction. Omitted or null `post_positions`
retains existing explicit offsets; an empty array restores automatic positions.
Each explicit offset names its directed start/end vertices and interior ordinal.
`edit_shared_construction` opts into an atomic construction change and current-run
repair. Other affected runs must remain valid.

Preview checks exact solids without committing or queuing a review. Newly
allocated preview IDs are temporary. Apply repeats preparation and native
preflight at the requested revision, then uses the owner's commit/review policy.
Cancellation and stale revisions refuse mutation. `inspect_fence` returns
canonical context, source-resolved post positions/bays and named member IDs;
inspection explicitly does not establish native geometry validity. The generated
MCP tools catalog owns exact field schemas.

### Drawing evidence

The ordinary Part drawing/material path publishes the actual independent solids,
without separate manually authored fence geometry. Native drawing tests measure
90 mm post profiles, 1,910 mm full clear panel spans and a 410 mm short final span
in independently parsed PDF and DXF at 1:100, 1:50 and 1:10. Linked plan and
elevation views include a native face-associated overall dimension: moving the
last path vertex updates 4,590 mm to 4,890 mm while retaining member Part IDs.
The same evaluated model's BOM contains four Timber posts and three Metal panels,
each with quantity one and its canonical material. These fixtures cover a flat
panel run; native solid tests separately cover open pickets/rails and round posts.

[MCP tests](../../crates/multicad-mcp/tests/fences.rs) exercise source-neutral
native preview, attributable edits/history, TCD reopen, shared updates, retained
formula bindings and round-post picket-to-rail conversion. Invalid source,
overlap, stale revisions, cancellation and read-only writes refuse mutation.
These focused checks do not substitute for the final workspace gates or actual
macOS interaction.

Project-authored path/context symbol materialization and pinned Library
updates belong to [CAD-206](https://plans.tokenet.de/CAD/206).

Terrain following, raked/stepped slopes, gates and engineered foundations are
outside this flat-run implementation. Actual macOS graphical checks and human
acceptance remain separate from native CLI and source tests.
