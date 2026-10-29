# Drawing annotations verification, 2026-10-29

CAD-88 adds associative dimensions, notes, leaders, weld symbols, tables and
sheet drafting to drawing pages. The
[annotations note](../architecture/drawing-annotations.md) describes the
contract.

## Commits

| Commit | Step |
| --- | --- |
| `c724c46` | Annotation source and capability, native dimension evaluation, detachment, rigid sheet transforms, overlap checks, SVG page writer |
| `f39b361` | Annotate and Drafting tools, annotation inspector, canvas rendering and selection |

## Native document workflow

[`model/tests/drawing_annotations.rs`](../../crates/multicad-model/tests/drawing_annotations.rs)
uses real parametric primitives:

- a block whose width is the parameter `width` = 40 mm;
- a cylinder whose radius is the parameter `radius` = 6 mm.

An A3 page holds two front views, at 1:1 and 1:2.

- **Dimensions.** Each view has a horizontal dimension between the block's ±X
  faces. There is a diameter dimension of the cylinder's circular edge and an
  angle dimension between an X edge and a Z edge.
- **Sheet content.** The page also has a note, a fillet weld symbol, a parts
  table and a sheet hatch.
- **Scale independence.** Both views read 40.0. The paper span of the
  dimension line is 40 mm at 1:1 and 20 mm at 1:2. The diameter reads Ø12.0
  and the angle 90.0°.
- **Layout.** `page_conflicts` reports no overlap.
- **SVG.** The SVG page writer's output is parsed independently with
  quick-xml. It contains dimension texts 40.0, Ø12.0 and 90.0° at 2.5 mm, the
  note, the table row "Block | 1", the weld size "a3", the title-block labels,
  0.25 mm visible view lines, and the drafting hatch lines.
- **Parameter edits.** Setting `width` = 65 and `radius` = 7.5 updates both
  views to 65.0 and the diameter to Ø15.0.
- **Sheet edits.** Moving every sheet annotation by (5, −5) mm and editing
  the note commits a page edit. Parameter entities and solid volume are
  unchanged, and the note is at (35, 195) with the new text.
- **Save and reopen.** The project requires `drawings.annotations.1`, and
  reopening restores the annotations exactly.
- **Detachment.** Removing the cylinder detaches its diameter dimension.
  `checked_page` fails with "Detached", the SVG writer refuses the page, and
  the block's dimension still reads 65.0.

## Core contracts

[`core/tests/drawing_annotations.rs`](../../crates/multicad-core/tests/drawing_annotations.rs)
checks the following.

- **Scale independence.** At view scales 1:1, 1:2 and 1:5, a horizontal
  dimension reads 40.00 while its paper span scales.
  - Aligned dimensions read 50, and reference dimensions read "(50.0)".
  - Vertical dimensions read 40, diameters read "Ø…", and angles read "90°"
    with an arc.
- **Rigid transforms.** A 50 mm sheet line on a 1:2 page reads 100.0. After
  a rotation of 33° about a point plus a translation, the line is still
  50 mm and the dimension still reads 100. Model dimensions refuse sheet
  transforms.
- **Hatching and layout.** A 20 × 20 mm square hatched at 2 mm gives 9
  clipped lines, all inside it. Table bounds are exact. A weld symbol on both
  sides with all-around, field and tail produces its glyphs and the text
  "a4 50 · ISO 2553".
- **Validation and wire form.** An annotated page round-trips and requires
  the capability, and pages without annotations omit the field. Six invalid
  annotations are refused:
  - a note off the paper;
  - a note containing a control character;
  - a one-point leader;
  - a table larger than the page;
  - a zero-radius circle;
  - a duplicate identity.
- **Conflicts.** `page_conflicts` reports title-block and annotation
  overlaps.

## UI

[`ui/drawings/tests.rs`](../../crates/multicad-ui/src/drawings/tests.rs) checks
the following.

- Without a pinned measurement, the model-dimension tool explains that it
  needs one.
- All eight sheet tools add valid annotations.
- Move and Rotate keep a 30 mm drafting line at 30 mm. Copy adds one, and
  Remove deletes it.
- The headless canvas paints the leader text, the weld size, the table title
  and "60.0" for the 30 mm sheet dimension on a 1:2 page.
- Apply commits one revision, and Undo removes every annotation.
- The ribbon test confirms that every new tool is in the Drawings workspace
  and parses its command alias.

## Verification commands

All on the committed tree at `f39b361`, Rust 1.99.1, locked and offline:

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline --no-fail-fast
npm test
CARGO_INCREMENTAL=0 ./build
```

- Formatting and strict Clippy pass.
- Tests: 1,364 passed, 0 failed, 2 existing ignored.
- Release contracts: 25/25.
- The native build produced
  `artifacts/development/build-1IqFMS/MultiCAD.app`, executable SHA-256
  `94886a962a586413f06a9571583ba7182793dcd1263dd1f9dd70b77e5e8e6daa`.
- Logs are in `/private/tmp/multicad-cad88/`.

## Limitations

- **Creating dimensions.** Model dimensions are created from pinned
  measurements. Edges cannot be picked directly in a drawing view, because
  projected drawing vectors carry no topology identity.
- **Projected lengths.** Linear values are projected view-plane distances, so
  a feature oblique to the view reads its projected length. Radius, diameter
  and angle values are true model values.
- **No PDF or DXF yet.** The SVG page writer is the vector fixture; PDF and
  DXF files belong to CAD-93. Fonts are monospace at fixed heights, and text
  width is estimated for overlap checks, not measured from glyphs.
- **Producer schedules.** Tables are authored. Producer-filled schedules from
  CAD-89 and CAD-82 arrive later.
- **No interactive macOS check.** The drawing tools were checked by headless
  rendering only.
