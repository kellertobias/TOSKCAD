# Planar manufacturing DXF

The [native exporter](../../crates/multicad-model/src/export/planar.rs) writes
1:1 contours from an explicitly chosen finite planar face. Its input is the
current owned B-rep. Viewport meshes, camera settings and drawing scales do not
participate. The [writer](../../crates/multicad-model/src/export/dxf.rs) emits
DXF R2007 with exact line and circular-arc boundaries.

## Saved face intent and coordinates

Each recipe needs one selected face for every matched Part occurrence. A saved
query that matches another Part requires another explicit face selection before
Run. The current native catalog carries the document identity, revision, source
hash and exact requested scope. The UI discards foreign or stale replies.

The [export definition](../../crates/multicad-core/src/exchange/source.rs) has an
optional `planar_faces` array, omitted when empty and defaulting to an empty
array when absent. Explicit bindings use schema 2 and capability
`exchange.planar_faces.1`. Schema 1 has no explicit face bindings and cannot
authorize planar output. Persistent geometry intent lives only in the entity's
`inputs` links with role `planar_face`.

Source: [`exchange/planar.rs`](../../crates/multicad-core/src/exchange/planar.rs).

| Binding field | Wire type | Required | Meaning |
| --- | --- | --- | --- |
| `recipe` | UUID string | yes | Existing planar DXF recipe identity |
| `subject` | Part occurrence reference | yes | Part, ordered Instance path and optional assembly root from the resolved scope |
| `source` | UUID string | yes | The final Instance in that path, or the Part for a definition |
| `reference` | UUID string | yes | The matching face intent identity in `inputs` |
| `horizontal` | string enum | yes | `world_x`, `world_y` or `world_z` projected into the current face plane |

The five binding fields are non-null; the subject retains its optional assembly
root. Duplicate recipe/subject pairs or reference identities
fail validation. Missing links, different source identities and non-face intents
also fail; evaluation-local face ordinals never enter saved bindings.

The coordinate origin is the world origin projected onto the current face plane.
The chosen world direction defines positive X after normalization in that plane.
Positive Y completes a right-handed frame with the face's outward normal.
Choosing an axis normal to the face fails explicitly. This rule avoids depending
on a surface's native UV origin and preserves actual occurrence placement once.

## Exact loops and file semantics

The [manufacturing contract](../../crates/multicad-core/src/manufacturing/mod.rs)
contains finite line and circular-arc loops in model millimeters. Producers
classify outer boundaries and holes from native topology. They normalize outer
loops counterclockwise and inner loops clockwise. Validation checks closure,
finite coordinates, nonzero lengths, signed areas and bounded output size.

Every loop becomes a closed `LWPOLYLINE`. Circular boundaries remain analytic
bulges; arcs larger than a semicircle are split into exact arc segments. Full
circles use two semicircles. DXF group codes follow Autodesk's
[LWPOLYLINE reference](https://help.autodesk.com/cloudhelp/2017/ENU/AutoCAD-DXF/files/GUID-748FC305-F3F2-4F74-825A-61F04D757A50.htm).
Coordinates convert directly to the declared millimeter, centimeter, meter or
inch units. The header uses the matching
[`$INSUNITS` value](https://help.autodesk.com/cloudhelp/2020/ENU/AutoCAD-DXF/files/GUID-A85E8E67-27CD-4C59-BE61-4DC9FADBE74A.htm).

| Content | Recipe layer setting | File representation |
| --- | --- | --- |
| Outer material boundary | `cut` | Closed counterclockwise polyline |
| Internal opening boundary | `cut` plus `_INNER` | Closed clockwise polyline |
| Circular opening center | `construction` | Non-cut POINT |
| Part name | `annotation` | Non-cut TEXT below the contour bounds |
| Bends | Unused | No inferred bend entities |

The file's internal opening contours do not assert blind-versus-through depth,
tool diameter or a machining process. A face with a pocket can therefore require
separate operator interpretation. Ellipses, splines, curved faces, disconnected
wires and duplicate or degenerate native boundary edges fail explicitly.

The catalog is bounded to 128 source occurrences, 512 faces per source and 4096
aggregate planar face choices. A contour contains at most 256 loops and 16,384
edges; the serialized DXF limit is 16 MiB. Native extraction lives in
[`planar_contours.cpp`](../../crates/multicad-kernel/src/planar_contours.cpp).

## Publication and producer boundary

The [shared publisher](export-sets.md) rebuilds the exact source and accepted
output plan before Run. Planar receipts include the frame, model-space bounds,
area, loop counts and exact-contour hash. Publication verifies the actual
written file against the freshly generated coordinates, bulges, layers and
labels before publishing the owned bundle. Reuse checks those bytes again.

`DevelopedContourProvider` is an optional owner-thread module contract. It receives
the current complete native Model and returns `StampedPlanarContours` for the
requested feature. The wrapper checks source identity, full revision/hash,
cancellation and the exact-loop contract. The sheet-metal module supplies
`SheetFlatProvider`, which returns the cut loops of an unfolded sheet part. No
export-set recipe route uses it yet; the developed sheet DXF has its own
exporter, described in [sheet metal](sheet-metal.md#developed-dxf). A projected
folded solid cannot stand in for an unfolded sheet.

The [native tests](../../crates/multicad-model/tests/planar_dxf.rs) reopen actual
files with the pinned developer-only Rust `dxf` reader. They verify panel holes,
bulges, winding, units, layers, labels, world occurrence placement, source edits,
save/reopen and rejection of stale, missing or inappropriate face selections.
The provider contract fixture uses genuine native planar geometry; it does not
establish sheet unfolding, CNC toolpaths or manufacturing acceptance.
