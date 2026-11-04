# Native navigation and import verification, 2026-11-04

CAD-221's actual macOS workflow verified the five top-level modes, global
commands, contextual sketch entry, existing-sketch editing and four accepted
import formats. A units-row layout correction is committed on `main` as
`2a77b3f`. Final formatting, strict workspace Clippy and both builds passed;
the full workspace suite passed. Human acceptance is separate.

The [historical navigation report](navigation-2026-11-02.md) retains the original
implementation, preference migration, offscreen layouts and painted route tests.
The main native workflow below used `5cbd525`; the final units-control recheck
and reopening after relaunch used the `2a77b3f` release bundle. The UI-only fix
does not change imported canonical geometry.

## Modes and contextual sketch editing

Actual native controls exposed Project, Model, Motion, Drawings and Export.
Each mode had the same global Add Sketch, Import, Library, Measure and MCP
history actions; no top-level Sketch tab appeared. Native screenshots were
emitted in the CUA conversation, not retained as standalone image files.

From Project with no face selected, Add Sketch entered Model's plane chooser
without exposing sketch editing tools or changing r0. Cancel preserved the
source. Choosing XY and Apply created Sketch 1 at r1. Two actual viewport picks
with Rectangle produced four lines and one region at r2. Escape ended the active
tool while retaining the geometry; Finish returned to Model at r2, and Save
updated preview metadata at r3.

Opening saved Sketch 1 returned to contextual editing with Finish/Abort. Abort
retained its four lines and revision 3. Selecting the actual box's planar top
face and pressing the global Add Sketch entered Sketch 2 directly, without a
plane chooser. That empty face-attached sketch was finished at r4 and saved at
r5. Its emptiness is intentional; no second drawn profile is claimed.

The [independent sketch/navigation readback](../../artifacts/navigation-review-20261104/independent-native-sketch-navigation-readback.json)
checks saved containers, native sketch solving, closed-region extraction and
actual box topology. Sketch 1 has a real 100 mm² rectangle on XY. Sketch 2's
persistent `cap.end` reference resolves to the actual top face at Z=10 mm with
+Z normal. Original box source and IDs remain unchanged; native evaluation
retains one 20 × 20 × 10 mm solid with 4,000 mm³ volume. Saved snapshots do not
independently prove the intermediate GUI mode, Escape or Cancel events; those
remain separately attributed native observations.

## Global imports and source-neutral cancellation

Cancelling the global format selector from Export preserved Export and the
source. Choosing STEP and cancelling the native file picker also left revision
5 and Export active, with explicit unchanged-document feedback. A format choice
alone did not move the application into a different mode.

Accepted files used the existing review/Apply handlers:

| Global entry mode and format | Actual review and saved result |
| --- | --- |
| Export → STEP | Model review showed one 100 × 60 × 18 mm solid, 108,000 mm³. Apply r6; Save r7. |
| Drawings → SVG | New XY target, From file units and 80 × 40 source viewport; 13 curves and five solved regions. Apply r8; Save r9. |
| Motion → DXF | From file, INSUNITS 4/mm; 13 curves, three closed regions and three open curves. Apply r10; Save r11. |
| Project → KiCad | Minimum hole 2 mm, default envelope height 5 mm; 1.6 mm board, eight outline edges and four 3.2 mm holes. First review Cancel preserved r11; second Apply r12 and Save r13. |

SVG review explicitly listed hidden/image/script/style/text/use omissions. Its
actual imported point-coordinate bounds are X=5–75, Y=5–35 mm, separate from the
80 × 40 source viewport. DXF review retained 140 × 60 mm coordinate bounds and
reported INSERT, paper-space and TEXT omissions; open curves are not claimed to
be closed manufacturing profiles.

KiCad review reported ten component envelopes, five missing external models and
the two unsupported categories, vias and oval drills. Accepted components are
generic envelopes, not loaded manufacturer models. The review did not silently
claim the omitted geometry. The board itself has native geometry; the final
project has 13 physical Parts including the original box and STEP import.

The [independent import readback](../../artifacts/navigation-review-20261104/independent-native-import-readback.json)
checks actual input hashes, saved canonical data, native sketch regions and
native model geometry. All original box/sketch/plane IDs and every earlier
import entity remain unchanged through later stages. STEP embeds a real native
B-rep. SVG/DXF persist editable imported curves, input hashes, byte lengths and
omission diagnostics, not the complete original raw file bytes. KiCad persists
parsed board/component geometry and a content hash. These are independent
geometry/source checks; review Cancel and intermediate revision observations
remain separately attributed to the native workflow.

The current native acceptance observations cover STEP, SVG, DXF and KiCad.
Canonical JSON snapshot routing remains covered by the historical painted
handler tests; a new accepted native JSON import is not claimed here.

## Shortcuts, saved mode and inspector correction

From Drawings, Q entered Model's plane chooser without changing source r13;
Cancel preserved it. Opening existing Sketch 1 and pressing Q finished its
contextual editor at r13 without a duplicate sketch. Finder search for
**Create Engraving** returned no matching tool, consistent with the single
sketch-entry model.

The project was closed in Motion at r13. The final release bundle relaunched
and Recent projects reopened that same file in Motion at revision 0, with
Undo/Redo disabled. Its SHA-256 remained
`c53bca751c75b35461bde24661f1f19a189f16e7c9b4977e60ab2bc2006119a1`,
identical to the retained `kicad-imported.tcd` snapshot.

Commit `2a77b3f` repairs the SVG/DXF Source units row in a narrow inspector.
A separate label, bounded shared combo and full-value tooltip prevent the long
selection from pushing the control outside its panel. The painted
counterfactual failed on the old source with “Missing painted Source units” at
244 points. The four-test focused run passed, including 244/250 point bounds,
source/history preservation, bounded labels and report visibility.
The [focused regression receipt](../../artifacts/navigation-review-20261104/checks/artwork-import-layout-focused-transcript.json)
retains those tool-transcript results; it is not a standalone captured test log.

The actual final native SVG recheck showed Source units above the bounded,
truncated selection, its full value available and the unit choices in the application
dropdown. Counts and bounds stayed unchanged. Cancel without Apply
left r0 and the saved hash identical. This proves the observed native row at
that size; the painted tests independently cover their defined widths.

## Required checks and limits

The final review bundle is
`artifacts/navigation-review-20261104/optimized-review/MultiCAD.app`, executable
SHA-256 `8dc6bd810aace2cbbb73430cf65cf589cea3b01096907de07c769cce7bf5b530`.
[Copied logs and check summary](../../artifacts/navigation-review-20261104/checks/summary.json)
record formatting, strict locked workspace/all-target Clippy, the release build
and the `./build` package at `artifacts/development/build-fSJt05/MultiCAD.app`.
The optimized review bundle and wrapper development package are distinct outputs.
The full locked workspace suite passed with 2,579 passed, 0 failed and
4 ignored across 295 groups, including documentation tests (exit 0).
It used test optimization level 3 with debug assertions and overflow checks enabled.
The complete output is retained as `workspace.log`. The earlier
run stopped during a session interruption after 1,139 passed / 0 failed /
1 ignored in 174 completed groups. Its retained `workspace-interrupted.log`
is partial evidence and is not counted as a complete run.

All CAD review windows were closed and the final application inventory was empty.
No Windows/Linux runtime, performance percentile, fabrication readiness or human
acceptance follows from this Mac workflow. Detailed Motion clearance/witness
acceptance remains CAD-240 scope. No application was installed, pushed or published.
