# Drawing views

Drawing pages store view intent and source identities. The native geometry
worker generates immutable vector results from current solids. A page consumer
must check the result against the complete current document before exporting.
The contracts live in
[`drawings/views.rs`](../../crates/multicad-core/src/drawings/views.rs), with native
generation in [`multicad-model`](../../crates/multicad-model/src/drawings.rs).

## Source and result boundaries

Each view has a stable identity, source selection, orthonormal frame, independent
scale and paper position. Its frame points right and up; the camera looks from
the positive normal. Projection, offset section and circular detail definitions
share that contract. Hidden-line and tangent-edge settings belong to each view.
The chord tolerance is measured on paper and converted to model units for
preview sampling.

Pages with views use drawing-book schema 2 and require `drawings.views.1`.
Removing every view retains that schema. Older page-only books retain their
wire representation. Book inputs are the sorted union of view source identities,
so source and material invalidation reach dependent drawings. Duplicate page
allocates new view identities while preserving their source selections.

A `DrawingEvaluation` carries document identity, revision and a complete source
hash. Its view records contain exact lines, trimmed circles/ellipses or rational
B-splines, with separate sampled preview paths. Each vector records its source
occurrence and manufacturing Part, visibility role and paper line width.
Unsupported curves fail the view explicitly rather than disappearing.
`checked_page` rejects stale, missing or unresolved results before a writer can
consume a page. These records are derived data and do not enter canonical source.

## Native projection and sections

The adapter uses OCCT's B-rep hidden-line algorithm across the selected solids.
Visible edges, silhouettes and optional hidden/tangent edges remain vector
curves. The [OCCT modeling algorithms guide](https://github.com/Open-Cascade-SAS/OCCT/blob/master/dox/user_guides/modeling_algos/modeling_algos.md)
describes the underlying extraction filters; the repository pins OCCT 7.9.3.
Display meshes and screenshots are not inputs to this path.

Section views retain material on the negative-normal side of their offset plane.
Native solid/half-space intersection produces the cut geometry. Only planar
faces at that cut, on solids that lost volume, receive section outlines and
material hatching. Exact line/face intersections clip hatches around holes.
An uncut Part behind the section plane receives no hatch. Material spacing and
line width use paper units; view scale changes the corresponding model spacing.

Circular details intersect projected curves with the authored region. The
adapter builds the projected curves' spatial representation before Boolean
clipping. Curve, point, source and hatch budgets produce explicit errors.
These bounds constrain output size; they do not establish a low-power timing
budget or cancellation inside a running OCCT operation.

## Producers and verification

The default source provider resolves selected final Parts, placed Part
occurrences, components and groups. Explicitly selected consumed Parts fail;
component traversal uses final bodies. Occurrences use their current world
B-reps and retain their source Part for material resolution. View selection is
independent of viewport visibility.

`DrawingProviders` accepts registered Model, FlatPattern and Exploded source
providers using borrowed or owned native solids. Flat-pattern and exploded
producers are unavailable by default. Registration tests establish the shared
contract using genuine fixture geometry; they do not implement sheet-metal
flattening or exploded assembly authoring.

Native coverage is in
[`kernel/tests/drawing.rs`](../../crates/multicad-kernel/tests/drawing.rs) and
[`model/tests/drawings.rs`](../../crates/multicad-model/tests/drawings.rs).
It checks model dimensions, hidden edges, exact circles/ellipses, finite detail
crops, section holes, material paper spacing, placement and parameter updates,
save/reopen, unavailable producers and stale-result rejection. Page editing and
identity tests remain in
[`core/tests/drawing_views.rs`](../../crates/multicad-core/tests/drawing_views.rs).
Native page rendering and PDF/DXF writers require their own UI and output checks.
