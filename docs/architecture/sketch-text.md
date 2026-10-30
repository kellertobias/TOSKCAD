# Sketch text (CAD-142)

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
  "style": { "text": "ABO", "font": "hack_regular", "height_mm": 10.0,
             "origin_mm": [0.0, 0.0], "rotation_deg": 0.0, "alignment": "left" } }
```

- `height_mm` is the **cap height** (the height of capital letters such as H),
  from 0.1 to 10 000 mm. The H glyph spans exactly `0..height_mm`.
- `origin_mm` is the baseline anchor in sketch coordinates. `alignment`
  (`left`, `center`, `right`) chooses which end of the advance width sits on it.
- `rotation_deg` turns the baseline counterclockwise from sketch U, within
  ±360°. Multiples of 90° use exact sine and cosine values.
- Text forces constrained-sketch schema ≥ 2. Any project with text requires
  the capability `core.sketch_text.1`, so older readers refuse the file with a
  clear message instead of silently dropping the text.

Only the parameters are stored. Contours are derived during evaluation and are
never solver points or source curves. The 256-point and 512-curve solver
budgets therefore do not apply to glyph poles.

## Font and licences

- The font is **Hack Regular**, taken from the pinned `epaint_default_fonts`
  0.36.2 crate, which already ships in the application through egui. No system
  font is read, so evaluation is reproducible.
- Hack's licence is MIT (Source Foundry). Its glyph base carries the Bitstream
  Vera licence, and the DejaVu changes are public domain. The crate declares
  `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0` for its other fonts.
- The release notice collector now also copies a font's same-stem licence
  sidecar (`Hack-Regular.txt`) and `UFL.txt` into `third-party/`.
- The outline parser is `skrifa` 0.44.0 (MIT OR Apache-2.0, already in the
  lockfile), built with `default-features = false, features = ["std"]`.
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
- Cubic outlines are refused. The bundled font has none.
- There is no polyline tolerance: the only operations are scaling by
  `height_mm / capHeight`, rotation and translation.

Each contour has an identity derived by name: SHA-256 over the text ID, the
character index and the contour index, laid out as a UUID v8. Height,
position, rotation and alignment edits keep every contour identity. A string
edit keeps the identities of characters that keep their position and contour
count ("ABO" → "ABD"). In both cases dependent extrusions rebuild without
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
sketch. Text is therefore limited to 64 glyph contours per sketch, at most 16
text elements and 64 characters per element. Region extraction skips native
predicates when convex-hull bounding boxes are disjoint. This is an exact
exclusion, and it keeps text extraction fast.

### Kernel fix

OCCT's single-curve self-intersector reported false "overlap" zones on C0
splines with parallel straight pieces, such as L and H outlines. Non-rational,
non-periodic C0 B-splines are now split at their C0 knots. Each piece is
checked on its own and against the others, and hits at a shared joint or at
the closed seam are ignored. Region queries and profile builds share this
check. Tests show that it still finds a bow-tie crossing and a retraced edge.

### Supported characters

The supported set is printable ASCII (U+0020–U+007E) plus `ÄÖÜäöüßéèàç°±µ`.
A native test proves that every one of these characters extracts closed,
non-intersecting and correctly nested regions.

## Editing paths

- `TextOperation` (`add`, `edit`, `remove`) is the single typed operation.
  `multicad_solver::prepare_text` validates it against the current canonical
  source and prepares one labelled transaction (`Add/Edit/Remove sketch
  text`). Committing it is one Undo step. Stale revisions are refused.
- **UI:** Sketch → Common → **Text** (`sketch-text`, alias `text`).
  1. Set the string, cap height, rotation and alignment in the Inspector. A
     live outline preview follows the cursor.
  2. Click to place the text.
  3. The placed text, or any text chosen in the Inspector's *Placed texts*
     list, can be edited there (string, height, position, rotation,
     alignment) and applied with **Apply text**, or removed with **Remove
     text**.
- **MCP:** `sketch_add_text`, `sketch_edit_text` and `sketch_remove_text` use
  the same operation and validation. They return the text ID and its contour
  curve IDs. `inspect_sketch` lists the text elements.

## Refusals

Each of these is refused with an actionable message and leaves the source
unchanged:

- empty or whitespace-only text, or more than 64 characters
- a character outside the supported set, named with its code point
- height outside 0.1–10 000 mm, including zero, negative and non-finite values
- position or rotation outside its limits
- more than 64 contours or 16 texts per sketch
- an edit to a missing text, or an edit that changes nothing

## Constraints and placement (owner decision)

Text does **not** take part in constraints or dimensions in this iteration.
Its placement is a literal origin, rotation and alignment, edited in the
Inspector or over MCP. Glyph contours are not source curves, so they cannot
be constrained, trimmed or used as dimension references. Editing tools that
act on source curves are not supported on them. Making the origin a constrainable sketch
point is an open question on CAD-142.

## Limitations

- There is one bundled font and no font management UI.
- There is no kerning, and text is a single line.
- There is no text on curved surfaces.
- Placed text is selected for editing through the Inspector list. Clicking a
  glyph outline in the canvas does not select the text element.
- The Text tool uses the generic sketch icon until reviewed ribbon artwork
  exists.
- A string edit that changes a character's contour count, or adds or removes
  characters before it, changes those contour identities. Extrusions that used
  them must be reselected, and the model reports this as "Selected profile
  changed".
