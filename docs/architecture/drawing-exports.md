# Drawing PDF and DXF export

Drawing pages export through the same saved export sets and native provider as
3D formats. A [page render](../../crates/multicad-core/src/drawings/render.rs)
turns one checked page into a device-independent description, in paper
millimeters with y down:

- polylines with line width and dash;
- arcs and filled arrowheads;
- text lines with height and rotation;
- logo rectangles for the bundled application logo.

Every item carries a role layer: border, title, visible, hidden, section,
hatch, dimension, note, leader, weld, table, draft or view label. The SVG, PDF and DXF
writers all consume this render, so every format shows the same geometry,
text and weights.

## Current-result policy

A page renders only from a `DrawingEvaluation` that `checked_page` accepts: the
same document, revision and source hash, every view ready and every model
dimension attached. Otherwise the export provider marks the PDF or
drawing-DXF recipe Unsupported, with a reason naming the page, for example
"Page A-101 cannot export: Detached dimension: …". The whole Run is then
blocked. A table placed from a schedule must also still match a fresh
schedule of the source; a stale one blocks the page the same way until it is
refreshed ([schedules](schedules.md#drawing-tables)). A preview made before a
source edit cannot run; it reports Stale. Nothing old is ever presented as
current.

## PDF

[`pdf::render_pdf`](../../crates/multicad-core/src/drawings/pdf.rs) writes one
PDF page per drawing page, in plan order:

- **Page size.** The MediaBox is the page's paper size and orientation in
  points, so A3 landscape is 1190.55 × 841.89 pt.
- **Geometry.** Geometry stays vector:
  - line widths are the native paper widths;
  - hidden lines are dashed 3/1.5 mm;
  - arrowheads are filled;
  - arcs are polylines of 96 segments per turn.
- **Text.** Text uses standard Courier with WinAnsi encoding, matching the
  monospaced layout. ° Ø ± · × — map to their WinAnsi codes; other characters
  outside it become "?".
- **Logo.** When a page shows the app logo, one shared RGB image XObject
  holds the bundled 640 × 215 px PNG. Its zlib data is reused unchanged with
  PNG predictors, so nothing is resampled; it prints above 300 dpi in the
  title block. Pages without the logo reference no image.
- **Determinism.** There are no timestamps or random identifiers, so
  identical source gives identical bytes. The document title is the project
  name.

## Drawing DXF

[`dxf::render_dxf`](../../crates/multicad-core/src/drawings/dxf.rs) writes one
ASCII DXF R12 file per page:

- **Content.** It holds the whole sheet at paper scale with y up, scaled to
  the recipe unit, with `$INSUNITS` set. Views keep their own scales because
  their vectors are already placed on paper. This route is separate from 1:1
  [planar part DXF](planar-dxf.md).
- **Layers.** These are SHEET_BORDER, SHEET_TITLE, VIEW_VISIBLE, VIEW_HIDDEN
  (DASHED linetype), VIEW_SECTION, VIEW_HATCH, DIMENSIONS, NOTES, LEADERS,
  WELD_SYMBOLS, TABLES, SHEET_DRAFTING and VIEW_LABELS. R12 has no embedded
  raster images, so the logo is omitted from drawing DXF; SVG embeds it as a
  PNG data URI.
- **Entities.** Lines, circles, arcs, solids (arrowheads) and text.
- **Symbols.** Degree, diameter and plus-minus use `%%d`, `%%c` and `%%p`.
- **Limits.** R12 has no per-entity lineweights; weights are carried by the
  role layers. The recipe's custom layer names are not used for drawings yet.

## Section semantics

Section-only and finite-depth views export their exact evaluated vectors:
cut outlines on VIEW_SECTION, hatch on VIEW_HATCH, and uncut geometry within
range on VIEW_VISIBLE and VIEW_HIDDEN. Nothing is re-derived, so a solid
excluded by the view range is absent in every format.

## Publication

Publication writes into the existing staged bundle. It then checks that the
bytes equal the generated output, that the PDF header, trailer and page count
are intact, and that each DXF holds exactly one page. Repeated identical runs
reuse a verified bundle. Unlinked tables are authored content and need no
producer; missing BOM, bend or weld producers never block ordinary pages.
