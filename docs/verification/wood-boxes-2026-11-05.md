# Native editable Wood Box verification, 2026-11-05

Actual macOS interaction verified box creation, independent face choices,
Preview/Cancel, history, saved recipe editing and a downstream pocket that
survives resizing and cold reopen. Independent native measurements and STEP
readbacks confirm separate solid panels with the displayed outside and usable
inner dimensions. Human acceptance remains separate.

This closes the window-interaction gap in the
[historical implementation report](wood-boxes-2026-11-02.md). The
[wood-box contract](../architecture/cabinetry.md#wood-boxes) owns behavior.
No Rust changes were needed for this verification.

## Shared stock and graphical creation

A new empty project was created through the native welcome screen. Project →
Materials authored Review plywood, with the default review density 700 kg/m³,
and Review panel 18 mm, a 2440 × 1220 × 18 mm panel stock using that material.
Each definition passed Preview and Apply through Project's left-side actions.
The empty project was saved under the owned review folder before creating a box.
These names and density are verification inputs, not measured material data.

Model → Wood → Box opened the five-panel, open-top default. The Inspector showed
600 × 400 × 300 mm outside dimensions, the shared stock/material choice and six
independent Left, Right, Front, Back, Bottom and Top checkboxes. Its usable inner
dimensions were 564 × 364 × 282 mm. Native Preview and Home view displayed the
five solid panels with labeled guides and the omitted Top.

All faces produced a six-panel native preview. Omitting Top and Left produced
four panels with usable width 582 mm. Omitting Front as well produced three.
Removing every face showed “A Wood Box must contain at least one face” and
disabled Preview and Apply. Open top restored the default. Preview/Cancel left
revision 2 with no new box, preserving only the existing material/stock edits.

Repeating Preview/Apply created one recipe transaction at revision 3. Undo
removed it at revision 4; Redo restored it at revision 5. Save produced revision
6 and five individually selectable panel Parts in the tree.

The [initial native audit](../../artifacts/wood-box-review-20261105/independent-native-wood-box-initial-readback.json)
checks the actual GUI-created file against the saved stock baseline. It confirms
the shared stock/material binding, one recipe and five single-solid panels.
Top's Part/feature identities are reserved without materialized entities.

| Face | Physical dimensions, mm | Native volume, mm³ |
| --- | --- | --- |
| Left / Right, each | 18 × 400 × 300 | 2,160,000 |
| Front / Back, each | 564 × 18 × 300 | 3,045,600 |
| Bottom | 564 × 364 × 18 | 3,695,328 |

Independent axis-aligned panel-prism bounds confirm zero positive-volume
intersection for every panel pair. This verifies the saved unrotated butt
layout without claiming a separate Boolean intersection query. Sides span the
full depth/height; front/back fit between sides; bottom fits between vertical
faces. All five panel STEP exports pass native write/readback volume checks.
Their hashes and byte counts are recorded in the audit.

## Downstream machining and retained panel identities

Selecting Bottom → Drill / pocket → Pocket authored a 40 × 30 × 5 mm cut at
panel coordinates u=100, v=100. Top face was selected; canonical source records
`top`. Only Bottom was checked under Target panels. Viewport placement
was turned off for these typed panel coordinates. Preview/Apply and Save retained
the operation at revision 8.

A saved Left member's Edit wood box action reopened the shared recipe. Omitting
Left passed native Preview, committed revision 9 and saved revision 10. The
four-panel result retained Bottom's pocket and showed usable inner width 582 mm.
Reopening the group and re-enabling Left restored its original physical Part and
feature identities; Save retained that five-panel state at revision 12.

At 1100 × 700 logical window size, the group reopened the same assistant. Width
650 mm gave usable inner dimensions 614 × 364 × 282 mm. Native Preview retained
the pocket. Apply, Undo, Redo and Save produced revisions 13 through 16.

The [checkpoint audit](../../artifacts/wood-box-review-20261105/independent-native-pocket-omission-resize-readback.json)
compares actual saved pocket, omitted-side, re-enabled-side and resized files.
The recipe, group, stock, six reserved physical identity pairs and pocket history
remain attributable. Re-enabling Left recreates its manufacturing-properties
metadata under a new entity ID with identical contents; physical panel identity
retention does not imply every metadata entity ID is retained.

The pocket removes 6000 mm³ throughout. Bottom volume is 3,689,328 mm³ before
omission, 3,807,264 mm³ with Left absent, 3,689,328 mm³ after restoration and
4,016,928 mm³ after the width grows to 650 mm. Each value matches independently
calculated panel dimensions minus the pocket. The final Bottom STEP readback
matches 4,016,928 mm³. No disconnected stale copy remains among the native Parts.

## Failure handling, compact controls and reopen

The following actual native checks left revision 16 and Apply disabled:

| Candidate edit | Observed refusal |
| --- | --- |
| Width 100 mm with the existing pocket | The 40 × 30 mm pocket leaves the face of Bottom |
| Omit Bottom after restoring width 650 mm | Cannot omit a face with downstream history; repair or remove its operations first |

The first Bottom-omission capture precedes Preview and only shows Apply waiting
for validation. Capture 25 records the subsequent native repair diagnostic.
Cancel discarded the invalid drafts.
Settled repeat captures 30 through 32 after another cold launch show both
diagnostics readably, including at compact size after scrolling, with Apply
disabled and revision 0. Cancel preserved the saved project.

The compact Placement controls accepted X=50, Y=25, Z=10 mm and rotation 30°.
Native Preview showed five panels and assembly bounds 762.9 × 671.4 × 300 mm,
while the design envelope stayed 650 × 400 × 300 mm. Cancel preserved the saved
unrotated recipe. This checks native placement preview, not a new rotated export.

Quit closed the review window. Fresh launch and the exact Recent entry reopened
the saved project at revision 0 with Undo/Redo disabled. Edit wood box retained
width 650 mm, the five-face selection, shared stock and inner dimensions.
Bottom → Edit latest feature reopened the retained 40 × 30 × 5 mm pocket with
u/v=100/100 and its original target. Both editors were canceled; Quit closed the
window. The final running-ToskCAD inventory is empty.

## Checks and evidence limits

The local runtime is the existing optimized final bundle from `a520123`,
executable SHA-256
`b30145efba6d7355bdaba54a818165368a1967c8d0db30e250e664391aa10080`, at
`artifacts/architecture-authoring-review-20261105/optimized-final/MultiCAD.app`.
Subsequent main commits changed documentation only. Diagnostics reports the
source commit as not stamped; this is local verification, not release provenance.

The unchanged Rust tree reuses the completed exact-source
[required-check receipts](../../artifacts/architecture-authoring-review-20261105/checks/summary.json):
formatting, strict locked all-target workspace Clippy, release build, `./build`,
version and diagnostics passed. Full locked workspace tests finished with 2583
passed, 0 failed and four intentional ignores across 295 groups. Tests used
optimization level 3 with debug assertions and overflow checks enabled. Existing
core/native/UI/MCP tests cover all nonempty face sets, stock changes, dependency
repair and persistence; this report adds actual-window evidence to those checks.

The owned root `artifacts/wood-box-review-20261105/` contains saved checkpoints,
native JSON audits, five initial STEP exports, the resized Bottom STEP and
`capture-manifest.json` indexing 32 JPEG/AX receipts at normal and compact sizes.
Several initial frames precede settled rendering; numerical and persistence
claims use saved source/native measurements and explicit current preview states.

The [independent visual review](../../artifacts/wood-box-review-20261105/independent-wood-box-visual-review.json)
records remaining presentation defects for the final UI consistency pass:
early compact resize frames lose leading summary text at the Inspector boundary;
machining's
placement/face controls extend beyond the right edge, and saved Box editing can
retain a Drill / pocket ribbon highlight. A Right callout can overlap the viewer
gizmo; the viewport's bottom-left dimension summary can also clip while the
Inspector summary remains readable. Primary footer actions remain visible.
Some refusal messages appear in
AX before the captured raster displays them. The later settled repeat shows
readable summary rows and refusal messages after scrolling. These limits remain
assigned to the existing final UI review item.

No Windows/Linux runtime, human acceptance, installation, push or publication is
claimed. Work remains local on main, and unrelated `.claude/` content is preserved.
