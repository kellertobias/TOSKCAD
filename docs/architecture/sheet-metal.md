# Sheet metal

Sheet-metal parts are ordinary model features in the canonical document.
The window and the MCP server use the same core API.

| Code | Contents |
| --- | --- |
| `crates/multicad-core/src/model/sheet_metal/` | Source types, resolution of stock and bend inputs, and transactions |
| `crates/multicad-model/src/sheet_metal.rs` | Native sheet solids, flat patterns, sheet records and solid conversion |
| `crates/multicad-ui/src/modeling/sheet.rs` | The window's Base sheet form |
| `crates/multicad-mcp/src/tools/sheet_metal.rs` | MCP tools |

## Sheet stock

Sheet parts use stock library entries with a sheet specification. The
stock is defined in Model → Materials or with `create_sheet_stock`, and
holds:

- the thickness;
- the sheet length and width;
- a default inside bend radius;
- a default K-factor (neutral-axis position as a fraction of thickness);
- its manufacturing Material.

The sheet solid takes its thickness from the stock, so a stock thickness
edit rebuilds every sheet part that uses it.

## Sheet parts

A **sheet part** is a `sheet_metal.part` feature (schema 1, capability
`sheet_metal.part.1`) with one output Part. A new sheet Part gets the
stock's Material and a manufacturing-properties record of its stock. A
stock change on an existing part moves its Material and stock assignment
with it.

The part has one of three bases:

- **Rectangle.** A length (u) and width (v) on the XY, XZ or YZ plane, with
  its minimum corner at a world origin. Every size can be a parameter
  expression.
- **Sketch region.** A solved sketch region, openings included. The base
  face lies on the sketch plane.
- **Contour.** Exact line and arc loops captured from a converted solid,
  with the frame of its stationary face. The source Part is recorded for
  traceability but does not drive the sheet.

The base face is the **stationary face**. The material runs the stock
thickness along u × v from it, or the other way when the part is
reversed. The four edge faces of a rectangle base carry the provenance
roles `sheet.base.edge.0` to `sheet.base.edge.3`.

### Bend inputs

The inside bend radius and the K-factor are separate editable values:

- without an override, each comes from the stock;
- a part may override either one; the radius is a length and the K-factor
  a ratio, and both may be parameter expressions;
- resolution reports where each value came from (`stock` or `part`).

The values are refused when the radius is negative or the K-factor lies
outside 0–1. They are inputs to bending, not universal constants and not
machine or tooling data.

## Flanges, hems and jogs

A sheet part holds a list of flanges. Each flange sits on one straight edge
of the base outer loop, at most one per edge. Rectangle edges are numbered
0 (v = 0), 1 (u = length), 2 (v = width) and 3 (u = 0). A flange keeps its
forming intent as one of three kinds:

- **Bend.** One bend of an angle above 0° and at most 180°, then a straight
  leg.
- **Hem.** A 180° fold whose leg lies a gap from the base face. The inside
  radius is half the gap, so a hem needs a positive gap.
- **Jog.** Two opposite bends of an angle up to 90° that offset the leg.
  The offset is measured between the base face and the leg's matching face.
  An offset too small for the two bends is refused with the minimum.

Every flange also records:

- the direction: **up** toward the material side, or **down** away from it;
- the straight leg length beyond the last bend;
- an optional inside radius, otherwise the part's;
- optional start and end insets: gaps between the edge ends and the flange,
  for corners and seams;
- an optional rectangular relief at each inset end, cut into the base:
  width along the edge beyond the flange end (at most the inset) and depth
  into the base.

Every value can be a parameter expression.

### Folded geometry

The base keeps its outline; each bend starts at its base edge. A flange is
its cross-section swept along the edge:

- the cross-section is a strip of the sheet thickness made of bends and
  straight segments;
- a bend turns about an axis parallel to the edge, with the inside radius on
  the inside of the bend;
- the native solid is fused with the base.

A bend of angle θ, inside radius r, thickness t and width w therefore holds
θ/2 · ((r + t)² − r²) · w of material.

Bend faces carry the provenance roles `sheet.flange.<i>.bend.<j>.a` and
`.b`. Leg faces carry `sheet.flange.<i>.leg.a` and `.b`, and the end face
carries `sheet.flange.<i>.end`. These roles keep bends distinct from
ordinary edge fillets.

### Cutouts

A cutout is a circle or rectangle cut through one face:

- **Base cutouts** use base-frame (u, v) coordinates.
- **Flange cutouts** use the leg's coordinates: along the edge from the
  flange start, and along the leg from the end of its last bend.

A flange cutout moves with its leg when the angle, radius or thickness
changes. A cutout must lie wholly inside its face. The evaluator checks
that it removes exactly its area times the thickness.

### What the evaluator refuses

The evaluator refuses these cases instead of building invalid material. The
error names the failing feature, and the document keeps its last valid part.

- **Corners and seams.** Two flanges that overlap or touch, for example at a
  shared corner. Give their insets a gap.
- **Base collisions.** A flange that runs into the base.
- **Edges.** A curved or missing edge, or insets that leave no flange.
- **Reliefs.** A relief wider than its inset.
- **Cutouts.** A cutout outside its face, or one that overlaps another
  cutout.
- **Fragments.** A part that falls apart into pieces.

## One folded and flat identity

The folded solid and the developed flat pattern are two evaluations of the
same feature. Both carry the same Part and feature identities.

`Model::sheet_metal` lists every evaluated sheet Part. Each record
contains:

- the Part, feature, Material and stock;
- the thickness, inside radius and K-factor, with their sources;
- the base kind and, for a converted part, its source Part;
- the measured thickness: from the base face to the nearest opposite face in
  the Part's final solid, so hem legs and jogs do not count;
- the volume and solid count;
- the flat pattern: exact loops in base-frame millimeters, outer loop
  counterclockwise and openings clockwise, with its size and area;
- every bend: flange, edge, direction, angle, inside radius, bend allowance,
  its zone along the developed flange and its center line in the flat
  pattern;
- review issues: a measured thickness that differs from the stock, or a
  Part stock assignment that differs from the sheet stock.

The flat pattern is extracted from the base face of a native developed
blank. It is never a projection of the folded solid. The blank is one
profile:

- the base outline, with each flange's developed rectangle spliced into
  its edge;
- relief notches in the outline;
- openings for base holes and every cutout.

A flange develops to its straight lengths plus a bend allowance of
θ · (r + K · t) per bend. Leg cutouts move out by the allowance. With a
K-factor of 0.5 the developed area times the thickness equals the folded
volume exactly, which the tests use as a cross-check.

Development fails with a reason when developed flanges overlap each other
or the base, when the blank falls apart, or when the extracted area differs
from the developed area. `Model::sheet_flat_pattern` returns the contours
for one feature.

## Unfolding a selected Part

`Model::unfold(document, part)` develops one sheet Part. It reads the model
and changes nothing: the Part keeps its folded geometry. It refuses:

- a Part that is not a sheet part;
- a sheet Part whose final solid comes from a later feature, because that
  feature would not be developed.

The result, `DevelopedPattern`, is the producer contract for flat outputs
such as DXF (CAD-105). It carries:

- the Part, feature, name and `units` (always `mm`);
- the base frame that places the pattern in the world;
- the thickness and the K-factor with its source;
- **cut loops**: closed exact loops, outer counterclockwise and openings
  (cutouts, base holes) clockwise. Relief notches are part of the outer
  loop.
- **bend lines**: each with a stable id `flange.<i>.bend.<j>`, a proposed
  order, its up or down direction, angle, inside radius, allowance,
  developed zone and center line;
- **drill points**: each circular cutout's id `cutout.<i>`, center and
  diameter;
- the size and area;
- diagnostics.

The ids are stable within the feature, so an output can name the bend or
hole it came from. The proposed order follows the feature order: flanges in
order, then the bends within each flange. It is not a press-brake sequence.

### Diagnostics and their scope

Each diagnostic states its scope:

| Kind | Scope | Finding |
| --- | --- | --- |
| `hole_near_bend` | rule of thumb | A cutout is closer than 2·t + r to a bend zone and may distort |
| `short_leg` | rule of thumb | A leg is shorter than 2·t + r; the real minimum depends on the die opening |
| `tight_radius` | rule of thumb | An inside radius is below the thickness; check the material's minimum |
| `missing_relief` | rule of thumb | An inset flange has no relief and may tear the base |
| `shallow_relief` | rule of thumb | A relief is shallower than r + t |
| `exceeds_stock` | model | The blank does not fit the stock sheet in either orientation |
| `tooling` | needs data | Always present: no press-brake, die, punch or springback data is defined |

None of these diagnostics guarantees machine or process feasibility.

## Developed DXF

`Exporter::developed_sheet_dxf` writes the developed blank of one sheet Part
as a 1:1 DXF in millimeters, centimeters, meters or inches. The code is in
`crates/multicad-model/src/export/sheet_dxf.rs`, separate from the planar
panel DXF.

The exporter works in three steps:

1. It evaluates the document afresh at the given revision.
2. It unfolds the Part.
3. It encodes the `DevelopedPattern`.

A failed evaluation or unfolding returns an error and no bytes. The result
carries the source stamp. `SheetDxf::check_current` refuses bytes made from
an older or different source, and the MCP tool runs that check before
writing.

| Layer | Content | Cut |
| --- | --- | --- |
| `CUT` | The closed outer contour, relief notches included, as an `LWPOLYLINE` with exact arc bulges | yes |
| `CUT_INNER` | Closed openings: cutouts and base holes | yes |
| `BEND_UP` | `LINE` center lines of bends toward the material side | no |
| `BEND_DOWN` | `LINE` center lines of bends away from it | no |
| `DRILL` | `POINT` at each circular cutout center | no |
| `LABEL` | `TEXT` naming each bend and drill point, and the part | no |

The labels carry the source ids from the pattern:

- a bend reads `flange.<i>.bend.<j> UP|DOWN <angle> R<radius> #<order>`;
- a drill point reads `cutout.<i> D<diameter>`;
- the part label holds the name, thickness, K-factor and feature id.

Layer names are configurable but must be distinct. The file contains no
nesting, kerf, tool offset or machine-specific data.

The [test](../../crates/multicad-model/tests/sheet_dxf.rs) reopens the files
with the independent `dxf` reader. It compares them with the editable part's
unfolding:

- the contour areas, including openings and reliefs;
- the bend lines by direction;
- the drill points and the labels for every id.

The test also covers:

- inch scaling;
- regeneration after a K-factor and direction edit;
- stale receipts;
- refusals: touching flanges in a candidate document, a non-sheet entity,
  and clashing layer names.

## Converting a solid

`Model::analyze_sheet_conversion` checks whether a Part's current solid is
one constant-thickness plate:

- the largest planar face and one opposite face of equal area at a
  constant distance;
- every other planar face perpendicular to them;
- curved faces only as side walls through the thickness (holes, rounded
  outline corners), whose area centroid lies at mid-thickness;
- a volume equal to the base area times the thickness;
- a thickness matching the sheet stock, when one is given.

The stationary face is the side facing away from +Z (then +Y, then +X).
Its exact loops become the captured contour base.

Anything else is refused with findings that name the measurement:

- inclined faces (chamfers, drafts);
- faces at an intermediate depth (pockets, steps);
- split or stepped plates;
- a thickness that differs from the stock;
- curved faces that are not through-thickness side walls. These are
  reported as bend or rounded regions, with their centers.

`sheet_metal::prepare_conversion` and the MCP `convert_to_sheet` tool create
the sheet Part from a convertible solid. A refused conversion changes
nothing.

## Access

- **Window.** Model → Sheet metal → **Sheet part / flanges**. The form
  places a rectangular base sheet from sheet stock, with an optional
  reversed side, radius and K-factor overrides, and a bend, hem or jog with
  an equal corner inset on each edge. Preview builds the native solid and
  Apply commits it. Opening an applied sheet part edits it in place.
  Sketch-region or converted bases, flanges the form cannot show and
  cutouts are kept unchanged.
- **MCP.** `create_sheet_stock`, `set_sheet_part`, `set_sheet_forming`,
  `convert_to_sheet`, `sheet_metal`, `unfold_sheet` and `export_sheet_dxf`.
  `export_sheet_dxf` writes into the server's export folder.

## Limits

- **Flange placement.**
  - Flanges sit on base edges only. There are no flanges on flange edges,
    no edge chains in one flange and no miter or closed corners. Corners
    are open gaps set by insets.
  - The edge is referenced by its index in the base outer loop. A sketch
    edit that reorders a profile's edges moves the flange. This is not yet
    a persistent geometry reference (CAD-55).
- **Missing forming features.** There are no rip or seam features on
  converted closed shapes, no sketched or contour flanges, no louvres or
  formed features, and no punch tool library.
- **Unfolding in the window.** The Flat pattern and Bend schedule ribbon
  entries are still placeholders. Unfolding is available through
  `Model::unfold` and the MCP `unfold_sheet` tool.
- **Developed DXF routes.** The window's Export mode has no developed sheet
  DXF recipe yet. The DXF is written by the exporter API and the MCP
  `export_sheet_dxf` tool.
- **Bend allowance.** Bend allowance uses the K-factor model only. There are
  no bend tables or bend deduction data.
- **Window flange editor.** The form edits one flange per rectangle edge
  with equal insets. Reliefs, per-flange radii, cutouts and sketch-region
  bases are API and MCP only.
- **Conversion.** Only flat plates convert. Bent solids are recognized as
  having curved bend regions but are not developed; rebuild them as sheet
  parts. The captured contour does not follow later edits of the source
  Part.
- **Window.** The form places rectangle bases only. Sketch-region bases and
  conversion are available through MCP and the core API.
- **Sheet size.** The stock sheet size is recorded but not yet checked
  against the flat pattern.
