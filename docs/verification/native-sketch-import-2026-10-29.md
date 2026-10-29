# DXF/SVG sketch import verification, 2026-10-29

CAD-129 imports DXF and SVG artwork as editable constrained-sketch curves. The
[architecture note](../architecture/sketch-import.md) describes the format
interpretation, supported-entity matrix, limits and provenance.

## Commits

| Commit | Step |
| --- | --- |
| `18895f1` | Core DXF/SVG parsers, placement, welding, provenance record |
| `1cc1930` | Solver `prepare_import` as one guarded transaction |
| `00e8aad` | Native model tests with measured solids |
| `fbdbb61` | Sketch ribbon command, picker, Inspector, preview, Apply/Cancel |
| `f9e62d8` | Architecture note |

## Fixtures

- [`plate.dxf`](../../crates/multicad-core/tests/fixtures/sketch_import/plate.dxf):
  hand-authored ASCII DXF in millimeters. It has a closed `LWPOLYLINE` with a
  bulged R20 corner, a `CIRCLE`, and a slot built from separate `LINE` and
  `ARC` entities. It also has a rational two-span `SPLINE`, an `ARC` with a
  −Z extrusion and a bulged 2D `POLYLINE`. `TEXT`, `INSERT` and paper-space
  content test the diagnostics.
- [`badge.svg`](../../crates/multicad-core/tests/fixtures/sketch_import/badge.svg):
  hand-authored SVG, 80 × 40 mm with a 160 × 80 `viewBox` (0.5 mm per user
  unit). It has a rounded `rect` and a circle hole containing a circle island.
  It also has an ellipse under nested `rotate(90 …)` groups, and a cubic plus
  quadratic path hole. `script`, `style`, `text`, an external `image`, `use`
  and a hidden group test the diagnostics.
- An inch DXF written at test time by the independent, developer-only `dxf`
  crate. It has a bulged `LWPOLYLINE`, a `CIRCLE` and an open `SPLINE`.

## Measured results

Native region areas and OCCT solid volumes agree with independent expected
values within a relative tolerance of 1e-7. The expected values are analytic,
or come from five-point Gauss-Legendre integration of the authored Bézier
control points.

| Case | Expected | Evidence |
| --- | --- | --- |
| DXF plate region | 6000 − (400 − 100π) − 100π − (150 + 25π) mm² | Exactly 3 regions: plate with 2 holes, Ø20 disc, slot |
| DXF plate, 10 mm extrusion | 10 × area | Native volume; bounds 100 × 60 mm |
| Radius constraint R10 → R12 on the imported circle | −10π(144 − 100) mm³ | Regenerated volume |
| SVG island Ø3 cut through the plate | −22.5π mm³ | Cut volume, STEP readback volume |
| Save/reopen | Identical document | Recompiled volume; provenance links the sketch, format DXF, layer `CUT` |
| SVG badge on XZ | Outline 2078.54 mm² minus circle, ellipse (18π) and Bézier holes | 5 regions; hole areas sum; X 5…75 mm, Z 5…35 mm |
| Independent inch DXF on YZ | (2 + π/8 − π/16) in² | Area, volume, world Y max 2.5 in |

Core tests check these interpretation details:

- the bulge geometry, the −Z OCS mirror (center (130, 10), sweep −90°) and
  the semicircular `POLYLINE` bulge;
- spline knots and weights, `$INSUNITS` and an explicit-inch override;
- that unitless DXF is refused until a unit is chosen;
- SVG Y-up orientation, the 1/96 in px policy, explicit user units, skew
  turning circles into exact ellipses, and DTD-entity rejection;
- that rational ellipse spans lie on the ellipse within 1e-12.

## Transactions and UI

Solver tests cover these transaction rules:

- New-sketch and existing-sketch imports each commit as one revision, and
  Undo restores the exact prior document.
- Redo restores the curves and the provenance record.
- A stale revision, layers without curves, or a superseded receipt cannot
  commit, and the document stays unchanged.
- Imported points can be referenced by later typed drawing.

UI tests render the Inspector headlessly. The painted preview text shows:

- groups with curve counts;
- the bounds `X 5.000 … 75.000 mm, Y 5.000 … 35.000 mm`;
- `4 lines · 4 arcs · 2 circles · 3 splines` and `5 closed loops · 0 open chains`;
- the unsupported-content lines.

A pointer click on the footer Apply creates one transaction, which Undo
reverts. Clicking Cancel leaves revision 0. The unitless DXF keeps Apply
disabled until a unit is chosen. An application test follows the full route:
Import DXF/SVG, a picker request of kind `ImportArtwork`, the worker read, the
Inspector opening, and a document that stays unchanged.

## Verification commands

All on the committed tree at `f9e62d8`, Rust 1.99.1, locked and offline:

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline --no-fail-fast
npm test
CARGO_INCREMENTAL=0 ./build
```

- Formatting and strict Clippy pass.
- Tests: 1,328 passed, 0 failed, 2 existing ignored.
- Release contracts: 25/25.
- The native build produced
  `artifacts/development/build-PU7FPA/MultiCAD.app`, executable SHA-256
  `b61e9a83b8319fc5f0d822ead0ea0f2c15836f02f8c39e21dede6998ad2b1673`.
- Logs are in `/private/tmp/multicad-cad129/`.

## Limitations

- The new bundle has not been launched. There is no interactive macOS pass
  through the native file sheet. Windows and Linux runtime are unverified.
- The import panel has no graphical overlay in the viewport. The preview is
  textual: bounds, counts and diagnostics. Placed geometry is visible after
  Apply.
- Sketch limits of 256 points and 512 curves apply. Larger artwork must be
  split across sketches by layer or group.
- DXF blocks (`INSERT`), fit-point-only and periodic splines, text and hatches
  are reported, not imported. SVG `use`, text, clipping and CSS are reported,
  not interpreted.
- Provenance is historical. There is no reimport or linked source update.
  Symbol templates reject the provenance record as an unsupported provider.
