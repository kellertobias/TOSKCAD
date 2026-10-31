# Drawing annotations

Drawing pages carry annotations alongside their views. The source lives in
[`drawings/annotations.rs`](../../crates/multicad-core/src/drawings/annotations.rs)
and the native evaluation in
[`multicad-model`](../../crates/multicad-model/src/drawings.rs). Annotations are
page content only. None of them edits a parameter, sketch or feature, and every
change is part of the page draft that one Apply commits.

Pages with annotations require `drawings.annotations.1`. Pages without them
keep their previous wire form.

## Kinds

| Kind | Source | Behavior |
| --- | --- | --- |
| Model dimension | View, measure, paper offset, decimals, reference flag | Associative; evaluated from current geometry |
| Note | Paper position, text | Sheet content |
| Leader | 2–16 paper points, text | Arrow at the first point, text at the last |
| Weld symbol | Leader points, symbol, arrow/other/both side, size, length, all-around, field, tail | Drawn on a 24 mm reference line |
| Table | Top-left position, title, columns with widths, rows, row height | Placeable schedule; content authored or supplied by a producer |
| Sheet geometry | Line, circle, arc or hatch polygon, line width | Paper millimeters, independent of model sketches |
| Sheet dimension | Two paper points, offset, decimals | Reads paper length divided by the page scale |

- **Measures.** A model dimension's measure is one of:
  - linear between two references, measured horizontally, vertically or
    aligned in the view;
  - the radius or diameter of a circular edge;
  - the angle between two straight edges.
- **References.** References are persistent topology intents, the same ones
  that pinned measurements use. A reference-only dimension is shown in
  parentheses.
- **Weld symbols.** The supported symbols are fillet, square groove, V groove,
  bevel groove, plug, spot and seam. Weld and table records do not depend on
  any welding or cabinetry producer.
- **Validation.** Every coordinate must lie on the paper, and identities must
  be unique across the page. Text, row, column and point counts are bounded.
  Radii must be positive, arc sweeps are at most 360°, and hatch spacing is at
  least 0.2 mm.

## Associativity

Dimension sources join the drawing book's inputs, so editing a referenced Part
invalidates the page. On each evaluation the native model resolves the
references:

- **Linear.** The native closest-point witnesses of the two references are
  projected into the view's u/v plane. The value is the u, v or in-plane
  distance in model millimeters, as on a standard drawing: a feature oblique
  to the view reads its projected length.
- **Radius and diameter.** The resolved circular edge gives the true model
  radius. The leader starts at 45° on a full circle, or at the middle of an
  arc.
- **Angle.** The unoriented acute model angle between the two resolved
  lines, in degrees. The arc is drawn about their projected intersection.

Values do not depend on view or page scale; only the paper geometry scales.

If a reference no longer resolves, the dimension is detached. It can be
missing, ambiguous or stale, the wrong kind (a radius on a straight edge, or an
angle on non-lines), or its view may have left the page. A detached dimension:

- carries a message instead of a value;
- appears on the canvas as a red marker;
- makes `checked_page` fail, so writers, including the SVG page writer, refuse
  the page.

No old value is ever kept.

## Sheet transforms and layout

`SheetTransform` rotates about a point, then translates. It is rigid, so
drafting lengths, and therefore sheet-dimension values, are preserved. Arc
start angles and hatch angles rotate with it. Notes and tables move their
anchor. Model dimensions are not transformed; their paper offset is edited
instead.

`page_conflicts` reports:

- views or annotations outside the printable area;
- anything overlapping the title block;
- sheet annotations overlapping views or each other.

Model dimensions are expected next to their views and are checked only
against the title block and the printable area.

## Vector output

[`svg::page_svg`](../../crates/multicad-core/src/drawings/svg.rs) writes a
checked page as SVG in paper millimeters:

- border and title block;
- view vectors with their native line widths, with hidden lines dashed;
- sheet annotations, dimensions and filled arrowheads;
- monospace text at the annotation height.

It exists for verification, and as input for the PDF and DXF writers that
CAD-93 owns.

## Editing

The Drawings ribbon has an Annotate tab (model dimension, leader, weld symbol,
table) and a Drafting tab (sheet line, circle, arc, hatch, dimension).

- **Model dimensions.** They come from a pinned measurement, so the existing
  Measure tool supplies the persistent references. The page inspector offers
  Horizontal, Vertical, Aligned, R of A, Ø of A and Angle for each pin, in the
  selected view.
- **Editing in the inspector.** Selecting an annotation on the canvas or in
  the inspector edits its fields. It also offers Move, Rotate (about its first
  point), Copy (a new identity, offset) and Remove.
- **Dimension values.** A model dimension shows its current value, or why it
  is detached.

### Placing notes and drafting on the paper

[`drawings/placement.rs`](../../crates/multicad-ui/src/drawings/placement.rs)
handles placement from the UI. A placement is pending: it is shown on the
sheet but is not part of the page draft until it is completed.

- **Text note.** The pending note editor opens at the top of the Inspector
  with keyboard focus, even when a drawing view was selected. A dashed marker
  shows the note on the sheet. Clicking the paper moves it. **Add note** is
  enabled only once the note has text; it adds the note to the page draft.
  Cancel or Escape discards it. An empty note is never added.
- **Sheet drafting.** The tools collect clicks while the sheet previews the
  pending geometry, dashed under the pointer. The Inspector and the sheet
  show the next step.

  | Tool | Clicks |
  | --- | --- |
  | Sheet line | start, end |
  | Sheet circle | center, point on the circle |
  | Sheet arc | center, start (sets the radius), end |
  | Hatch | three or more corners; click the first corner, press Enter or use Finish hatch |
  | Sheet dimension | first point, second point, dimension-line position (sets the offset) |

- **Snapping.** A point snaps to an existing drafting point within 2 mm of
  paper. Otherwise it snaps to a 0.5 mm grid, and Shift keeps it horizontal
  or vertical from the previous point. Points stay inside the printable area.
- **Handles.** A completed annotation is selected. Selected drafting shows
  square handles at its defining points: line ends, circle center and radius,
  arc center/start/end, hatch corners, and the two dimension points plus
  offset. Dragging a handle edits the same draft data as the Inspector's
  numeric fields.
- **Cancel and undo.** Escape (routed through the application's Cancel
  shortcut), Cancel or Discard drops a pending placement without touching the
  draft. Apply commits completed drafting as one undoable page edit, and Undo
  restores the page without it.

Non-UI channels keep the earlier immediate insertion at default positions.
Headless egui tests in
[`placement_tests.rs`](../../crates/multicad-ui/src/drawings/placement_tests.rs)
drive real pointer clicks, drags, typing and Escape through the editor
frame.
