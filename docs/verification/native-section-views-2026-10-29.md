# Section ranges and exterior views verification, 2026-10-29

CAD-130 extends the CAD-87 drawing views. It adds finite-depth and
section-only sections, lateral section windows, sections picked from a parent
view, guides in parent views, and custom exterior directions. The
[drawing views note](../architecture/drawing-views.md) describes the contract.

## Commits

| Commit | Step |
| --- | --- |
| `233e9ac` | Section range and window schema, `drawings.sections.1`, native box clipping and section-only, model mapping, guide geometry |
| `1308835` | Inspector section controls, parent-view guides on the canvas, custom look direction |
| `fe11e9a` | Pick a finite section line in a parent projection |

## Native fixtures

[`kernel/tests/drawing.rs`](../../crates/multicad-kernel/tests/drawing.rs)
sets up a front section at y = 0. The view looks along +Y, keeps y ≥ 0 and
hatches every input. It uses four real OCCT solids:

- **A at the cut:** a 10 mm cube spanning y −5…5.
- **B within range:** y 10…20.
- **C crossing the far limit:** a ramp spanning y 20…40 and limited to
  z ≤ y − 20, built as a Boolean intersection with a rotated box. Its volume
  is checked against the analytic 2000 mm³.
- **D beyond range:** y 50…60.

The results:

- **Section only.**
  - Output comes from A only, with only Section and Hatch roles: four
    outline edges forming exactly the 10 × 10 mm cut, and hatch lines inside
    it.
  - Moving the plane into B cuts only B.
- **Unlimited.** All four solids appear, and the ramp's silhouette reaches
  z = 20.
- **Depth 30 mm.**
  - A, B and C appear; D does not. Only A has section and hatch vectors; B
    and C are visible lines only.
  - The ramp's silhouette ends at z = 10, its height where the far plane
    y = 30 clips it, so the clip position is shown on paper.
- **Shallower depths.** A 12 mm depth, ending inside B, keeps A and B (B
  clipped). An 8 mm depth, ending before B, keeps only A.
- **Windows.**
  - A window of u 15…35 shows only B.
  - A window of u 25…100 and v −1…5 clips B to u ≥ 25 and height 5, and keeps
    C and D.
  - A section-only window that no solid crosses yields no vectors.
- **Refused inputs.** A section-only request without a section, section-only
  combined with a depth, a zero depth, a reversed window and a NaN window are
  all refused.

## Document workflow

[`model/tests/drawings.rs`](../../crates/multicad-model/tests/drawings.rs)
builds parametric primitive Parts.

- **Fixture.** D's depth position is the parameter `rear`. Every Part has a
  hatched material.
- **Hatching.** In a 30 mm-deep section, only A carries Section and Hatch
  vectors. B and C have visible and hidden lines only.
- **Parameter edit.** Setting `rear = 20` regenerates the view with D. Undo
  removes it again.
- **Depth and section only.** A depth of 8 mm shows only A. Section only
  shows only A's cut and hatch: 0.18 mm hatches inside the 10 × 10 mm cut in
  view coordinates.
- **Window.** The window u 15…55 shows only B and C. Undo restores section
  only, and Redo restores the window.
- **Capability.** The project requires `drawings.sections.1`. A reader
  without it refuses the file.
- **Save and reopen.** Reopening keeps the range and window and produces the
  same vector count.
- **Legacy wire form.** Returning to an unlimited range restores the original
  wire form, `offset_mm` with no `range` or `window`, and drops the
  capability.
- **Invalid limits.** A negative depth or an empty window cannot enter the
  document. A window beyond every solid is an unresolved view saying it has
  "no projected edges".

## Core and UI

- **Core.**
  [`core/tests/drawing_views.rs`](../../crates/multicad-core/tests/drawing_views.rs)
  checks the following.
  - A legacy section deserializes unchanged and serializes byte-identically.
    Limited sections round-trip, and unknown fields are refused.
  - The guide of a front section, cut at y = 5 with a 20 mm depth and an
    x 10…30 window, appears in a top view from x = 10 to 30 at y = 5, with
    its far limit at y = 25. Without a window it spans the parent's bounds;
    a parallel view sees no guide.
  - A picked plan segment from (0, 5) to (40, 5) gives a section frame with
    origin (10, 25, 0), u = X and v = Z, and a 0…40 window. That section's
    guide lies back on the picked segment. `model_point` inverts
    `paper_point`, including details.
- **UI.**
  [`ui/drawings/tests.rs`](../../crates/multicad-ui/src/drawings/tests.rs)
  renders the editor headlessly and checks the following.
  - A section's range and window change needs a fresh preview before Apply.
    The inspector paints the range, depth and description text.
  - The page canvas paints the section's guide label in an edge-on plan view,
    and not in a view facing the plane.
  - Apply commits one revision, and Undo restores the unlimited section.
  - Picking two points in a plan adds "Section A-A" with the expected frame
    and window. One point alone adds nothing, and a section cannot be a pick
    parent.
- **Custom directions.** The custom look direction gives right-handed frames
  that match the Front and Top presets and look along an oblique input.

## Verification commands

All on the committed tree at `fe11e9a`, Rust 1.99.1, locked and offline:

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline --no-fail-fast
npm test
CARGO_INCREMENTAL=0 ./build
```

- Formatting and strict Clippy pass.
- Tests: 1,352 passed, 0 failed, 2 existing ignored.
- Release contracts: 25/25.
- The native build produced
  `artifacts/development/build-vaAxYw/MultiCAD.app`, executable SHA-256
  `966fceeb627ce305ec4687196be6d42c562734858bea4aa72dc7ee4d98a01a08`.
- Logs are in `/private/tmp/multicad-cad130/`.

## Limitations

- **Picking in the 3D viewer.** A section plane cannot be picked in the 3D
  viewer. Sections are created by picking a line in a page projection or
  numerically (offset, direction presets, custom direction).
- **Attached dimensions.** There are no attached drawing dimensions yet;
  CAD-88 owns them. View geometry regenerates and stale results are refused,
  but marking attached dimensions as unresolved cannot be demonstrated.
- **Export writers.** There are no PDF or DXF drawing writers yet; CAD-93 owns
  them. Evidence uses the exact vector records and their paper coordinates,
  not exported files.
- **Window boundary lines.** Where a lateral window cuts a solid, it leaves
  straight boundary lines. They are not styled as break lines.
- **No interactive macOS check.** The guides, controls and picking were
  checked by headless rendering only. Windows and Linux are unverified.
