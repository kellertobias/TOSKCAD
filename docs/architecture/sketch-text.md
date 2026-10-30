# Sketch text (CAD-142, CAD-149)

Sketch text gives parts lettering for labels, engraving and embossing. A text
element stays editable after you place it. It evaluates to exact closed glyph
outlines, and these outlines become ordinary sketch regions that Extrude (New
body, Join, Cut) and Revolve can use.

## Canonical source

A constrained sketch holds an optional list `sketch.texts` beside points,
curves and constraints. If the list is empty it is not serialized, so the JSON
of existing sketches does not change.

```json
{ "id": "<uuid>",
  "style": { "text": "MultiCAD\nlabel", "font": "cantarell_regular",
             "height_mm": 6.0, "position_mm": [10.0, 5.0], "anchor": "center",
             "rotation_deg": 0.0, "alignment": "center" } }
```

- `text` is one or more lines separated by `\n`. It may hold up to 64
  characters (line breaks included) and up to 16 lines. Blank lines are
  allowed between lines, but not at the start or end.
- `font` is `cantarell_regular` (the default for new text) or
  `hack_regular`.
- `height_mm` is the **size**: the cap height, meaning the height of capital
  letters such as H. It ranges from 0.1 to 10 000 mm. For both fonts, the H
  glyph spans exactly `0..height_mm` above its baseline.
- `position_mm` is the **anchor position** in sketch coordinates.
- `anchor` picks the point of the text block box that sits on
  `position_mm`. The choices are `top_left`, `top_center`, `top_right`,
  `left_center`, `center`, `right_center`, `bottom_left`, `bottom_center`
  and `bottom_right`.
- `alignment` (`left`, `center`, `right`) aligns each line inside the block.
- `rotation_deg` turns the block counterclockwise about the anchor, from
  sketch U, within ±360°. Multiples of 90° use exact sine and cosine values.
- Text forces constrained-sketch schema ≥ 2. Any project with text requires
  the capability `core.sketch_text.2`. Older readers refuse such a file with
  a clear message instead of silently dropping or misplacing the text.

Only the parameters are stored. Contours are derived during evaluation and are
never solver points or source curves. The 256-point and 512-curve solver
budgets therefore do not apply to glyph poles.

### Text block box and anchors

The anchor box is a typographic layout box. It is not the ink bounding box,
so it stays the same while you type and does not jump when a letter with a
descender is added.

- **Width:** the advance width of the widest line, from the left edge of the
  first character's advance to the right edge of the last one.
- **Top:** the cap height of the first line.
- **Bottom:** the baseline of the last line. Descenders (g, p, y) and the
  comma hang below it.
- **Line pitch:** the font's ascent − descent + line gap, scaled by the
  size.

The nine anchors are the corners, the edge midpoints and the center of this
box. Changing the anchor keeps `position_mm` fixed and moves the block around
it. Core tests check this exactly for all nine anchors at 0°, 90°, −180° and
270°, and MCP tests check it through `inspect_sketch`.

### Migration from CAD-142

CAD-142 stored `origin_mm` (a baseline point) with `alignment`, wrote no
`anchor`, and required `core.sketch_text.1`. Such files still open, because
`core.sketch_text.1` stays a supported capability. On read, `origin_mm`
becomes `position_mm`, and the alignment selects the matching bottom anchor:

| CAD-142 alignment | anchor          |
|-------------------|-----------------|
| `left`            | `bottom_left`   |
| `center`          | `bottom_center` |
| `right`           | `bottom_right`  |

For single-line text this is the same placement. A test compares every pole
with the verbatim CAD-142 algorithm for all three alignments and for
rotations of 0°, 90° and 33°. The font stays Hack. The next save writes the
anchor form. A record that mixes `origin_mm` with `anchor`, or that has
neither form, is refused.

## Fonts and licences

The user asked for a vector font. Hack is already an outline font, so this
request is read as a **proportional** vector outline font bundled with the
app under a permissive licence, with Hack kept as a second choice.

- **Cantarell Regular 0.05** is a proportional sans-serif and the default for
  new text. It is vendored unmodified as
  `crates/multicad-core/fonts/Cantarell-Regular.ttf` (31 236 bytes, SHA-256
  `d15f25bc…e93b55f`). The file is byte-identical to the copy in the
  `sctk-adwaita` 0.10.1 crate from the local cargo registry, so no network
  fetch was needed. Its licence is the SIL Open Font License 1.1. Copyright
  2009–2011 Understanding Limited and 2010–2011 Jakub Steiner. The full
  licence and copyright text is in
  `crates/multicad-core/fonts/Cantarell-Regular.txt`. Release packaging
  copies it to `third-party/bundled-fonts/` in the app resources, and a node
  test checks this.
- **Hack Regular** comes from the pinned `epaint_default_fonts` 0.36.2 crate,
  which already ships through egui. It is MIT-licensed (Source Foundry) on a
  Bitstream Vera licence base, and the DejaVu changes are public domain. Its
  same-stem notice (`Hack-Regular.txt`) and `UFL.txt` are collected with that
  crate's notices.
- The outline parser is `skrifa` 0.44.0 (MIT OR Apache-2.0).
- No system font is read, so evaluation is reproducible. No crate was added
  or changed.
- `TextFont` is an enum. A new font is a new variant, so an existing variant
  never changes its outlines.

## Outline representation

TrueType (`glyf`) contours are piecewise quadratic, and the mapping is exact:

- Each contour becomes **one closed, clamped degree-2 B-spline**. Its knots are
  `0..n` with multiplicities `[3, 2, …, 2, 3]`, so the Bézier spans join with
  C0 continuity.
- A straight span is written as a quadratic whose control point is the
  midpoint. This is the exact degree-2 form of a line.
- Consecutive collinear straight spans are merged in integer font units, which
  makes the merge exact. A seam that splits a straight edge moves to that
  edge's start.
- Cubic outlines are refused. Neither bundled font has any.
- There is no polyline tolerance. Glyphs are only translated in font units,
  scaled by `height_mm / capHeight`, rotated and translated.

Each contour has an identity derived by name: SHA-256 over the text ID, the
character index in the whole string (line breaks count) and the contour
index, laid out as a UUID v8. Edits to the size, anchor position, anchor,
alignment or rotation keep every contour identity. A string edit keeps the
identities of characters that keep their position and contour structure
("ABO" → "ABD" in Hack). In both cases dependent extrusions rebuild without
reselection.

## Regions

`sketch_regions` adds every contour as a closed spline boundary. Under the
existing rule, each closed loop is offered as a region whose immediate
children are holes. "ABO" therefore yields seven regions: the bodies of A and
O with one hole each, B with two holes, and the four counter disks. Select the
letter bodies to extrude lettering with open counters.

Crossing or touching text, for example overlapping strings, is diagnosed. It
is never merged.

Spline regions use the exact native query service, which allows 64 curves per
sketch. This iteration keeps that budget: text is limited to 64 glyph contours
per sketch, at most 16 text elements and 64 characters per element. Region
extraction skips native predicates when convex-hull bounding boxes are
disjoint. This is an exact exclusion, and it keeps text extraction fast.

### Kernel fix

OCCT's single-curve self-intersector reported false "overlap" zones on C0
splines with parallel straight pieces, such as L and H outlines. Non-rational,
non-periodic C0 B-splines are now split at their C0 knots. Each piece is
checked on its own and against the others, and hits at a shared joint or at
the closed seam are ignored. Region queries and profile builds share this
check. Tests show that it still finds a bow-tie crossing and a retraced edge.

### Supported characters

The common set is printable ASCII (U+0020–U+007E) plus `ÄÖÜäöüßéèàç°±µ`, and
line breaks.

- **Hack** supports the whole set.
- **Cantarell** refuses `|` and `ç`. Its `|` is drawn as two touching bars,
  and its cedilla touches the c, so neither forms a clean region. The refusal
  message names Hack Regular as the alternative.

A native test proves that every supported character in each font extracts
closed, non-intersecting and correctly nested regions. A second native test
checks the same for three-line text in all three alignments.

## Click selection

A plain click on a letter selects its text element and opens it in the Text
tool's Inspector for editing. This works with no tool active and with the
Text tool active. With the Text tool, a click on empty space still places new
text. A click counts as on a letter when it lands on a glyph outline (one of
the text's derived contours, within the usual 7-pixel pick distance) or
inside a glyph, counters included. The inside test uses display samples of
the exact spans (`SketchText::contains`), never manufacturing geometry.
Shift-click or Cmd-click keeps the previous behaviour and picks profile
regions, including letter regions. Selection changes nothing in the
document.

The selected text is drawn highlighted, and its anchor position is marked
with a small circle.

## Editing paths

- `TextOperation` (`add`, `edit`, `remove`) is the single typed operation.
  `multicad_solver::prepare_text` validates it against the current canonical
  source and prepares one labelled transaction (`Add/Edit/Remove sketch
  text`). Committing it is one Undo step. Stale revisions are refused.
- **UI:** Sketch → Common → **Text** (`sketch-text`, alias `text`).
  1. Set these fields in the Inspector: the multi-line text, the font, the
     size, the anchor (3 × 3 picker; the default is center), the line
     alignment and the rotation. A live outline preview follows the cursor.
  2. Click to place the text. The click sets the anchor position.
  3. To edit placed text, click one of its letters or choose it in the
     *Placed texts* list. Edit any field, including the anchor position, and
     apply with **Apply text**, or remove it with **Remove text**.
- **MCP:** `sketch_add_text`, `sketch_edit_text` and `sketch_remove_text` use
  the same operation and validation.
  - Arguments: `text`, `height_mm`, `position_mm` (`origin_mm` is accepted as
    an alias), `anchor` (default `bottom_left`), `alignment` (default
    `left`), `rotation_deg` and `font` (default `cantarell_regular`).
  - The calls return the text ID and its contour curve IDs.
  - `inspect_sketch` lists the text elements with their style, contour IDs
    and `block_corners_mm`, in the order bottom-left, bottom-right, top-right,
    top-left.

## Refusals

Each of these is refused with an actionable message and leaves the source
unchanged:

- empty or whitespace-only text
- more than 64 characters or 16 lines
- a blank first or last line
- a character outside the supported set or outside the chosen font, named
  with its code point or with the font to use instead
- a size outside 0.1–10 000 mm, including zero, negative and non-finite
  values
- an anchor position or rotation outside its limits
- more than 64 contours or 16 texts per sketch
- an edit to a missing text, or an edit that changes nothing

## Anchor as a sketch point (not done)

The anchor position is a literal coordinate, edited in the Inspector or over
MCP. It is **not** a solver point, so it cannot be fixed, dimensioned or made
coincident in this iteration. Doing that would need three things:

- a text-owned point in the solver's point set
- a rule for evaluating contours from solved positions instead of source
  values
- deletion and reference semantics for that point

These cut across the solver and binding layers that parallel work is
changing. CAD-149 carries this as an open question. Glyph contours are not
source curves, so they cannot be constrained, trimmed or used as dimension
references either.

## Limitations

- There are two bundled fonts and no font management UI.
- There is no kerning. Text cannot follow curves or curved surfaces.
- In Cantarell, an "ABO" → "ABD" edit does not keep the extruded profile.
  This was observed over MCP; the likely cause is a different contour order
  in O and D, but that is not confirmed. Dependent extrusions must be
  reselected after such an edit. A string edit that changes a character's contour count, or
  adds or removes characters before it, changes those identities in either
  font. The model reports this as "Selected profile changed".
- The Text tool uses the generic sketch icon until reviewed ribbon artwork
  exists.
- The interactive behaviour (Inspector layout, anchor picker, highlight,
  click selection) is covered by headless egui tests. It has not had a
  macOS interactive review.
