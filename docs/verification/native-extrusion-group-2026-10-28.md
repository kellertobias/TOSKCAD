# Native extrusion group checkpoint, 2026-10-28

The production macOS app reopened a saved multi-target cut, edited its captured
targets and dimensions, and restored the complete operation with one Undo/Redo.
Fresh native regeneration confirms the restored stock and the two edited cuts.
This records implementation evidence; human acceptance remains separate.

## Executable and checks

The reviewed bundle was `artifacts/development/build-LO1pIU/MultiCAD.app`, built
from the captured-group editor checkpoint `8a0d850`. Its executable SHA-256:

```
241f4c430a9bc58393f14f5b0ec13936f272a11b20837c08c99060b7c519d845
```

At that checkpoint, formatting, all-target Clippy with warnings denied and the
locked offline workspace tests passed: **702 passed, two existing ignored**.
Commands were `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and
`cargo test --workspace --locked --offline`.
Logs are `/private/tmp/multicad-group-integrated-{fmt,clippy,tests}.log`.

The fixture is the six-panel source described in the
[extrusion operations checkpoint](native-extrusion-operations-2026-10-28.md).
This bundle includes the saved group editor and source-edit recovery controls.
New-body operation conversion is outside this executable checkpoint.

## Save, reopen and captured-target edit

1. Open `cut-scope-native.multicad`, select its 400 mm² sketch region, and start
   Extrude with Cut, start offset -5 mm and distance 120 mm.
2. Preview lists A grouped panel and A visible panel as the exact affected set.
   Apply creates the operation at revision 1. Save As writes
   `group-cut-native.multicad`.
3. Quit the app. Inventory confirms every MultiCAD review instance is closed.
   Relaunch the same bundle and open the saved group through File.
4. In Model, select A visible panel and Edit latest feature. The Inspector opens
   Edit extrusion operation with both saved targets checked and their predecessor
   feature versions shown.
5. Uncheck A grouped panel and check B foreign panel. Set start offset to 2 mm
   and distance to 85 mm. Preview validates the complete candidate.
6. Apply edits the group at revision 1 in this fresh session. The tree restores
   A grouped panel's primitive head, retains A visible panel's cut and adds B's
   new cut result.
7. One Undo restores both original cut targets at revision 2. Redo restores the
   edited operation at revision 3. Save As writes `group-edited-native.multicad`.

Fresh reports give the following rounded native volumes, in mm³:

| Part | Reopened original cut | Edited group |
|---|---:|---:|
| A visible panel | 44,000 | 44,800 |
| A grouped panel | 44,000 | 48,000 |
| B foreign panel | 48,000 | 45,200 |
| A hidden, Nested and Document root panels, each | 48,000 | 48,000 |

The edited tool spans Z = 2 to 87 mm. It removes 400 x 8 mm³ from A visible
panel and 400 x 7 mm³ from B. Removing the grouped target restores its stock.
The retained A visible cut producer remains
`27c55635-aae5-4ed5-a254-e80973e04a78` across both saved reports.

## Face-attached sketch and downstream cut

In a fresh document, create the default 100 x 60 x 20 mm Box and Apply at
revision 1. Switch to Sketch, choose New sketch, and click its top face in the
3D viewer. The selected frame reports origin (0, 0, 20) mm and normal +Z.
Use plane creates the linked sketch at revision 2.

The Rectangle command `rect 10 10 30 30` creates four edges at revision 3.
Finish sketch returns to Model. Select the 400 mm² profile and Extrude with Cut,
Reverse direction and distance 10 mm. Exact discovery lists only Box 1; native
Preview succeeds and Apply reaches revision 4. Save As writes
`face-sketch-cut-native.multicad`.

Edit the upstream Box feature's height to 30 mm. Native Preview rebuilds the
attached sketch and downstream cut; Apply reaches revision 5. Save As writes
`face-sketch-rebuilt-native.multicad`. Fresh reports retain the same Part and cut
producer while the resolved attachment frame moves from Z20 to Z30.
Rounded volumes are 116,000 and 176,000 mm³, respectively, matching the stock
volume minus the 400 x 10 mm³ pocket. Both contain one native solid.

## Evidence boundaries and closure

The files and corresponding `*-report.json` outputs are ignored local artifacts
under `artifacts/model-review/`. Each report runs
[`model_file_review`](../../crates/multicad-app/examples/model_file_review.rs)
in a fresh process using the production native compiler. The reader executable
SHA-256 was `2e5df92a11aaf7ee6f97272621760f3c711f8990106f045a3630911d9a6701b7`.
Geometry comes from canonical source; display meshes are not manufacturing input.

The same bundle also physically reopened `split-native.multicad` from the earlier
checkpoint and regenerated both semantic Split outputs in Model. Interactive
reopen of the final edited-group file remains a later check; its fresh native
report passed. The saved review documents were closed, and inventory confirmed
every MultiCAD instance stopped. The disposable empty startup tab was discarded.
The separate face-sketch review session saved its sole document before Quit.

This checkpoint does not establish general fragment splitting, new-body group
conversion, physical Windows/Linux behavior, or low-power performance.
