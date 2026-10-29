# DXF and SVG sketch import

The [import module](../../crates/multicad-core/src/sketch_import/mod.rs) turns a
local DXF or SVG file into ordinary constrained-sketch curves. Imported lines,
arcs, circles and B-splines get fresh entity IDs and are edited, constrained,
extruded and cut like drawn geometry. Nothing is rasterized or tessellated.

## Pipeline

1. **Read.** The UI reads the chosen file on a worker and stops at 16 MiB plus
   one byte. Larger files fail before parsing.
2. **Parse.** The [DXF reader](../../crates/multicad-core/src/sketch_import/dxf.rs)
   or the [SVG reader](../../crates/multicad-core/src/sketch_import/svg.rs)
   produces exact segments in millimeters with Y up. Unsupported content is
   counted by kind. At most 20,000 curves are parsed.
3. **Plan.** The [plan](../../crates/multicad-core/src/sketch_import/plan.rs)
   filters layers or groups, then applies the scale factor, rotation and
   insertion point. These form a similarity, so circular arcs stay analytic.
4. **Append.** Curve endpoints within the weld tolerance share one point
   identity. The default tolerance is 1e-4 mm. Interior spline poles and arc
   or circle centers are never welded. Existing sketch points are never
   merged or moved.
5. **Apply.** The solver's [`prepare_import`](../../crates/multicad-solver/src/import.rs)
   appends the curves to the open sketch, or creates a sketch on an origin
   datum. The same Session request inserts the provenance record. The
   candidate is evaluated before Apply is offered, and Undo removes both.

The preview, the plan and Apply all use the same bytes. A failed or stale
preparation, or Cancel, leaves the document unchanged.

## Units and coordinates

| Source | From file | Explicit unit |
| --- | --- | --- |
| DXF | `$INSUNITS` 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 13, 14 | One drawing unit equals the chosen unit |
| DXF without units | Fails; choose a unit | As above |
| SVG with `viewBox` and absolute `width`/`height` | Physical size divided by `viewBox` size | One user unit equals the chosen unit |
| SVG with one absolute dimension | Uniform scale from that dimension | As above |
| SVG without absolute size | 1 user unit = 1 CSS px = 1/96 in = 0.2645833 mm | As above |

SVG lengths in `px`, `mm`, `cm`, `in`, `pt`, `pc` and `Q` are absolute. A
percentage or font-relative root size counts as absent. If the `viewBox`
aspect ratio differs from the viewport, `preserveAspectRatio` applies,
including its alignment. `meet` and `slice` keep a uniform scale. `none`
scales non-uniformly, so circles become exact ellipses.

SVG Y points down. Import places the origin at the bottom-left corner of the
root viewport, with Y up. Without a viewport height, the origin is the SVG
user-space origin. DXF coordinates keep the DXF origin. The insertion point
gives the sketch-local position of that origin.

## Supported content

| DXF entity | Result |
| --- | --- |
| `LINE` | Line; Z is projected and noted |
| `LWPOLYLINE`, 2D `POLYLINE` | Lines and analytic arcs from bulges; the closed flag closes the path |
| 3D `POLYLINE` | Lines projected onto XY, noted |
| `ARC`, `CIRCLE` | Analytic arc or circle; a −Z extrusion mirrors X and reverses direction |
| `ELLIPSE` | Exact rational quadratic B-spline; a circular ellipse becomes an arc |
| `SPLINE` with control points | Clamped (rational) B-spline with normalized knots |
| `SPLINE` with fit points only, periodic or unclamped knots | Reported, not imported |
| `INSERT`, `TEXT`, `MTEXT`, `DIMENSION`, `HATCH`, meshes, others | Reported by kind, not imported |
| Paper-space or invisible entities, tilted OCS | Reported, not imported |
| Binary DXF | Fails; save as ASCII DXF |

| SVG element | Result |
| --- | --- |
| `path` (`M L H V C S Q T A Z`, relative and absolute) | Lines, cubic and quadratic Bézier splines, arcs or elliptical splines |
| `rect` (with `rx`/`ry`), `circle`, `ellipse`, `line`, `polyline`, `polygon` | Exact equivalents |
| `g`, `a`, `switch` | Containers; `transform` composes |
| `matrix`, `translate`, `scale`, `rotate`, `skewX`, `skewY` | Applied exactly |
| `display:none` | Skipped and reported |
| `text`, `image`, `use`, `script`, `style`, `foreignObject`, nested `svg` | Reported, not imported, executed or fetched |
| `clipPath`, `mask` | Reported; geometry is imported unclipped |
| `defs`, gradients, markers, metadata | Ignored |
| DTD entity declarations | Fail |
| Shape attributes in units other than user units or `px` | Shape skipped and reported |

Groups directly under the root `svg` element are the selectable groups. Each
is named by its `inkscape:label`, its `id`, or `group N`. Elements outside a
group belong to `(root)`. DXF layers come from group code 8. Styling, fill
rules and stroke widths are ignored. Holes come from the sketch's own region
extraction, regardless of winding or fill rule.

## Limits and accuracy

Imported curves share the sketch limits of 256 points and 512 curves,
including existing geometry. Spline poles count as points. The preview shows
how many points the import needs and refuses Apply beyond the limit. Splines
are limited to 256 poles and degree 25.

Conversion is exact up to floating-point rounding. Bulges, circles and arcs
stay analytic. Ellipses and elliptical arcs use rational quadratic spans of
at most 90°. Core tests check these spans against the ellipse equation within
1e-12. Native tests compare region areas and solid volumes with analytic or
independently integrated values within a relative tolerance of 1e-7.

## Provenance

Each import adds a `core.sketch_import` extension record, which needs the
`core.sketch_import.1` capability. It links to the sketch with role `sketch`.
It stores:

- the file name without its path, and the SHA-256 and length of the bytes;
- the unit choice and millimeters per source unit;
- the scale, insertion, rotation and weld tolerance;
- the imported layers, the created curve IDs and the unsupported-content
  summary.

The curve list records what the import created. Later edits may change or
delete those curves; the record does not drive evaluation. Symbol templates
do not accept this record yet. Reimport and linked source updates are not
part of this contract.
