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

## One folded and flat identity

The folded solid and the developed flat pattern are two evaluations of the
same feature. Both carry the same Part and feature identities.

`Model::sheet_metal` lists every evaluated sheet Part. Each record
contains:

- the Part, feature, Material and stock;
- the thickness, inside radius and K-factor, with their sources;
- the base kind and, for a converted part, its source Part;
- the measured thickness: the spread of the faces parallel to the base face
  in the Part's final solid;
- the volume and solid count;
- the flat pattern: exact loops in base-frame millimeters, outer loop
  counterclockwise and openings clockwise, with its size and area;
- review issues: a measured thickness that differs from the stock, or a
  Part stock assignment that differs from the sheet stock.

The flat pattern is extracted from the base face of a native developed
blank. It is never a projection of the folded solid. Without bends, the
developed blank is the base sheet itself. `Model::sheet_flat_pattern`
returns the contours for one feature.

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

- **Window.** Model → Sheet metal → **Base sheet**. The form places a
  rectangular base sheet from sheet stock, with an optional reversed side
  and radius and K-factor overrides. Preview builds the native solid and
  Apply commits it. Opening an applied sheet part edits it in place; a
  sketch-region or converted base is kept.
- **MCP.** `create_sheet_stock`, `set_sheet_part`, `convert_to_sheet` and
  `sheet_metal`.

## Limits

- **No bends yet.** Flanges, bends, hems, jogs, reliefs, corners and cutouts
  are CAD-77. Until then the flat pattern equals the base sheet.
- **Conversion.** Only flat plates convert. Bent solids are recognized as
  having curved bend regions but are not developed; rebuild them as sheet
  parts. The captured contour does not follow later edits of the source
  Part.
- **Window.** The form places rectangle bases only. Sketch-region bases and
  conversion are available through MCP and the core API.
- **Sheet size.** The stock sheet size is recorded but not yet checked
  against the flat pattern.
