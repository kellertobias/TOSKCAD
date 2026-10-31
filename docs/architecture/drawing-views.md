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

### Section ranges and windows

A section view also stores how much lies behind the cut, and an optional
lateral window. Both are omitted from the wire form when unused, so older
sections are unchanged. Projects that use either require
`drawings.sections.1`.

| Range | Output |
| --- | --- |
| Unlimited (default) | Every retained solid behind the plane: the original half-space path |
| Finite depth | Uncut geometry only between the cut plane and a far plane `depth_mm` behind it. Solids crossing the far plane are clipped there, and farther solids are excluded. |
| Section only | Only the exact plane intersection of each solid, outlined and hatched. Nothing in front of or behind the plane is drawn. |

The window limits the section in view-frame u (right) and v (up)
coordinates, measured from the view origin, so it describes a finite section
line or region. Geometry outside the window is not shown.

Finite and windowed sections clip each solid with one box in the view frame.
An unlimited side extends past every source by a margin, so its faces never
touch geometry. Section only uses the exact common of each solid with a
bounded face on the cut plane.

Hatching follows the existing rule: only planar faces at the cut plane, on
solids that lost volume, are hatched. A solid inside the range behind the cut,
or one clipped only at the far plane or the window, has visible and hidden
edges but no hatch. Window clipping leaves straight boundary lines where it
cuts a solid. An empty range or window is an unresolved view with an explicit
message.

**Pick section line** on a projection takes two clicks on the page.
`section_from_line` turns the picked segment into a section frame:

- the cut plane contains the line and the parent's view direction;
- the section looks toward the left of the picked direction and keeps the
  parent's normal up;
- the window spans exactly the picked segment.

The new view shares the parent's sources and scale. Picking the points in the
reverse order flips the viewing side.

`section_guide` places a section's cut line in another view of the same page
when that view sees the plane edge-on. The line is trimmed to the window along
the section's u axis, and a finite depth adds the far limit. The page canvas
draws the cut solid with the section's name and the far limit dashed, so the
chosen cut and range are visible before Apply. A changed section still needs a
current native preview before Apply, and Undo, Redo and save/reopen keep its
settings.

### Exterior views

The inspector offers the six principal directions and isometric. It also
offers a custom look direction: the camera looks along that direction with
world Z up, or world Y when looking vertically. Every exterior view projects
the selected Parts, occurrences or components through the same hidden-line
path.

### Details

Circular details intersect projected curves with the authored region. The
adapter builds the projected curves' spatial representation before Boolean
clipping. Curve, point, source and hatch budgets produce explicit errors.
These bounds constrain output size; they do not establish a low-power timing
budget or cancellation inside a running OCCT operation.

## Title block

[`drawings/title_block.rs`](../../crates/multicad-core/src/drawings/title_block.rs)
lays out the default title block at the lower right of the printable area,
at most 180 mm wide, on a four-column grid:

| Row | Cells |
| --- | --- |
| Identity | Project (3 columns), app logo (1 column) |
| Title | Drawing title across the full width, 3.5 mm text |
| People | Drawn by, Checked by, Revision, Drawing no. |
| Format | Scale, Units (mm), Projection, Sheet (`n / total`) |

Labels are 1.8 mm and values 2.5 mm monospaced text. The block has a 0.5 mm
outer frame and 0.25 mm cell rules. Values wrap inside their cell and rows
grow; missing values print an em dash, and a block that no longer fits the
paper is rejected rather than clipped. The layout is independent of view and
room-plan rendering.

The ToskCAD logo (`assets/branding/toskcad-logo-titleblock.png`, a 640 px
copy of the approved blueprint logo) is shown by default with its
proportions preserved. **Show app logo in title block** in Page settings
turns it off for that page; the project cell then spans the full width, so
no empty cell remains. The setting is saved with the page. It is omitted
from the wire form while on; a page that hides the logo requires
`drawings.title_logo.1`. An application-wide default for new pages belongs to
the Settings work (CAD-187).

## Scales and fitting

Every view prints a caption with its name and stored paper:model scale, such
as `Plan — Scale 1:23`. The caption is centered below the view's evaluated
paper extent, or above it when it would leave the printable area or cross the
title block. It is derived from the page at render time, so the native sheet,
PDF, SVG and drawing DXF (layer `VIEW_LABELS`) always show the scale of the
revision being drawn or exported. The contract lives in
[`drawings/scale.rs`](../../crates/multicad-core/src/drawings/scale.rs).

The title block's Scale field shows the page default only when the page has
no views. With views it shows their common scale when every view uses the
same ratio, and `Mixed, see views` otherwise, so a 1:1 page default can never
contradict a 1:23 view.

**Fit view to page** centers the view in the larger free printable region,
above or to the left of the title block, with clearance and room for the
caption. The Inspector's fit mode chooses the scale:

| Mode | Scale |
| --- | --- |
| Standard scale (default) | Largest standard scale that fits: 100:1, 50:1, 20:1, 10:1, 5:1, 2:1, 1:1, then 1:2, 1:5, 1:10, 1:20, 1:25, 1:50 … 1:100000 (ISO 5455 1-2-5 series plus 1:25 multiples). Small parts are enlarged. |
| Exact ratio | Tightest whole-number ratio, n:1 or 1:n (the earlier behavior, e.g. 1:23 for the 4 m × 3 m room on A4) |
| Keep current scale | Only centers the view, e.g. to retain 1:1 |

A standard-scale picker and the numerator/denominator fields set the scale
directly. Fitting edits the page draft only; Apply commits it as one undoable
edit and Discard restores the saved page. Geometry is never resampled for
scale: the exact vectors are placed with the stored ratio.

`multicad-core/tests/drawing_scales.rs` covers labels, the series and region
choice. `multicad-model/tests/drawing_scales.rs` fits native views of a
16 mm knob (5:1), a 200 mm sheet (1:2 on A4), a 720 mm cabinet (1:5), the room
(exact 1:23) and the knob kept at 1:1, exports PDF and parses its content
stream: each caption's printed ratio equals the paper extent of the view's
strokes divided by the model size, within chord and point rounding.

## Annotations

Dimensions, notes, leaders, weld symbols, tables and sheet drafting are
described in [drawing annotations](drawing-annotations.md).

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
It checks:

- model dimensions, hidden edges and exact circles/ellipses;
- finite detail crops, section holes and material paper spacing;
- placement and parameter updates, save/reopen, unavailable producers and
  stale-result rejection;
- section-only, finite-depth and windowed sections against fixtures at the
  cut, within range, crossing the far limit (a ramp whose clipped height
  proves the cut position) and beyond range. Page editing and
identity tests remain in
[`core/tests/drawing_views.rs`](../../crates/multicad-core/tests/drawing_views.rs).
Native page rendering and PDF/DXF writers require their own UI and output checks.
