# Drawing PDF and DXF export verification, 2026-10-29

CAD-93 exports drawing pages to multi-page PDF and per-page drawing DXF. The
[drawing exports note](../architecture/drawing-exports.md) describes the
render, writers and current-result policy.

## Commits

| Commit | Step |
| --- | --- |
| `5ba04ed` | Shared page render; SVG, PDF and drawing-DXF writers; export provider and publication |
| `8978c3a` | Test fix: the window/headless equivalence test no longer depends on random ID order |

## Independent consumers

[`model/tests/drawing_exports.rs`](../../crates/multicad-model/tests/drawing_exports.rs)
builds a book with two pages:

- **A-101, A3 landscape.** All five title fields are filled. It has a 1:1
  front view of a parametric block (width = 40) and a pin, a horizontal model
  dimension, a note and a table.
- **A-102, A4 portrait.** It has a section-only view and a 5 mm deep section,
  both cutting the hatched block at y = 10 at 1:2, plus a weld symbol. The pin
  lies beyond the 5 mm depth.

One export set has a PDF recipe and a drawing-DXF recipe. Checks:

- **Plan.** The preview plans one PDF and two DXFs.
- **PDF page order and size.** The PDF evidence lists A-101 then A-102 at
  420 × 297 mm and 210 × 297 mm. The MediaBoxes read 1190.551 × 841.89 pt and
  595.276 × 841.89 pt.
- **poppler `pdfinfo`.** It reads 2 pages: page 1 is 1190.55 × 841.89 pt
  (A3) and page 2 is 595.276 × 841.89 pt (A4). The title is "Drawing export".
- **poppler `pdftotext`.** Page 1 contains "Bench block", "T. Keller",
  "Workshop", "QA", "A-101", the dimension "40.0", the note and "Block | 1".
  Page 2 contains "A-102" and the weld size "a4".
- **Ghostscript.** The `bbox` device renders both pages. Every bounding box
  lies within its page, so nothing is clipped off-page.
- **`dxf` crate, page 1.**
  - DIMENSIONS has the text "40.0" at 2.5 mm and a 40.00 mm horizontal
    dimension line, the true length at 1:1.
  - SHEET_TITLE has "Bench block", and TABLES has "Block | 1".
  - VIEW_HIDDEN uses the DASHED linetype.
  - The sheet border spans exactly 10…410 × 10…287 mm.
- **`dxf` crate, page 2.**
  - Both views show the 40 × 30 mm cut as 20 × 15 mm on paper, at 1:2.
  - Every VIEW_HATCH line lies inside a cut.
  - VIEW_VISIBLE appears only around the deep section and spans the block's
    15 mm height. The section-only view has no visible lines, and the pin is
    absent.
- **Stale preview.** After a parameter edit, running the old preview fails as
  Stale and writes nothing. A fresh preview can run.
- **Detached dimension.** A dimension with a broken reference blocks both
  recipes, reporting "Page A-101 cannot export: …".

Existing tests cover the rest:

- The annotation SVG test runs on the shared render.
- The ordinary 3D, planar DXF and mesh export tests are unchanged.

## Verification commands

All on the committed tree at `5ba04ed`, Rust 1.99.1, locked and offline:

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline --no-fail-fast
npm test
CARGO_INCREMENTAL=0 ./build
```

- Formatting and strict Clippy pass.
- Tests: 1,370 passed, 0 failed, 2 existing ignored.
- Release contracts: 25/25.
- The native build produced
  `artifacts/development/build-ovChJf/MultiCAD.app`, executable SHA-256
  `9031b83ad5684b0e0b45dcb3a59b00deba2d21b987ed6d61e06dc7b7f1cd2d85`.
- Logs are in `/private/tmp/multicad-cad93/`.

## Limitations

- **Optional consumers.** poppler and Ghostscript checks run only when the
  tools are installed; they ran here. CI without them relies on the internal
  structure checks and the `dxf` crate.
- **PDF text.** It uses Courier with WinAnsi encoding. Characters outside
  WinAnsi become "?", and no font is embedded.
- **Drawing DXF.** It is R12 with per-layer weights. Custom recipe layer names
  are not applied to drawings, there are no paper-space layouts, and the
  output is one file per page.
- **Arcs.** Arcs in PDF are fine polylines. DXF arcs and circles stay
  analytic.
- **No interactive macOS check.** Export from the Export workspace was not
  exercised by hand.
