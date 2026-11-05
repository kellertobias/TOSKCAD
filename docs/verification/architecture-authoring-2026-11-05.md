# Native Architecture authoring verification, 2026-11-05

Actual macOS interaction verified graphical wall, door, window and stair work
in Model, with canonical level ownership, linked 2D/3D representations and saved
context. Native readbacks confirm retained identities and dependent geometry.
Two exported Front PDFs independently confirm drawing associativity after a
level elevation edit. Required Rust checks passed; human acceptance remains separate.

The review found a creation-tool highlight leaking into generic saved-feature
editing and a room-footprint action reassigning an Upper slab to Ground.
Source evidence `4111f8a` corrects generic edit highlighting; `a520123` preserves
the slab's selected level while copying a footprint. Explicit Wall creation
still highlights Wall.
The [historical report](architecture-authoring-2026-11-02.md) retains the original
implementation and automated failure-case evidence. The
[navigation contract](../architecture/architecture-navigation.md) owns behavior.

## Graphical authoring and level ownership

An owned copy of the real empty-apartment fixture supplied three existing walls
and Apartment finished floor at elevation 100 mm. New level Upper used elevation
2900 mm, floor-to-floor height 2800 mm and slab thickness 200 mm. Native Preview
followed by Cancel left revision 0 without a new level or Undo entry. Recreating
and applying committed revision 1; Undo removed Upper and Redo restored it.

In Model's 2D plan, two viewport clicks and Finish authored Upper graphical wall.
No path coordinates were typed. Its hosting field showed Upper with zero local
base elevation. Switching the visible floor to Ground during the draft retained
Upper as the wall's host. Native Preview and Apply created the upper-owned wall
while Ground was viewed; selecting Upper displayed it.

The existing Generic apartment wall type supplies 2700 mm height and 200 mm
thickness. The level's floor-to-floor height does not override that wall type.
The [wall and opening readback](../../artifacts/architecture-authoring-review-20261105/independent-upper-wall-openings-readback.json)
records native bounds, positive-volume solids and unchanged original objects.

Door and Window were placed with actual wall picks in plan. Door Preview/Cancel
kept the source unchanged; repeating the pick and applying created its opening.
The plan displayed an opening gap and door swing, with panes for the window.
Saved-opening selectors and Pick location in viewport moved both openings within
their original wall. A canceled door move retained its original placement;
repeating it applied, and one Undo/Redo restored each state.

| Saved input | Initial placement | Edited placement |
| --- | --- | --- |
| Wall path | [1580,2940] to [9840,2940] mm | [1580,2940] to [11600,2940] mm |
| Door, 900 × 2100 mm, sill 0 | Offset 1740 mm | Offset 2700 mm |
| Window, 1200 × 1200 mm, sill 900 | Offset 4850 mm | Offset 6230 mm |
| Straight stair, width 1000 mm, 16 risers | Going 270 mm, run 4320 mm | Going 280 mm, run 4480 mm |

At the compact window size, the wall's sidebar entry reopened its saved feature.
Draw in viewport, two clicks and Finish extended its path. Native Preview/Apply
retained the wall, body, level and opening identities. The
[redraw readback](../../artifacts/architecture-authoring-review-20261105/independent-redrawn-wall-readback.json)
confirms the longer native solid and exact wall-minus-apertures volume.

Stair placement used two plan clicks for the first riser and walking direction.
The saved stair connects Apartment finished floor to Upper and starts at
[1990,1870] mm with zero-degree direction. A compact saved-feature edit changed
its tread going after native Preview. The
[final stair readback](../../artifacts/architecture-authoring-review-20261105/independent-final-stair-and-save-readback.json)
confirms stable identities, connected native geometry and unchanged other Parts.
These geometric inputs do not establish building-code compliance.

A separate owned floor-slab fixture exposed an ownership failure. Although Upper
was selected, Use room boundary copied the Ground perimeter's level as well as
its footprint. Independent native readback found coincident slabs on Ground.
That failing fixture and its readback remain retained as failure evidence.

The correction preserves an assigned slab level; an unassigned slab can inherit
the room's level. The new regression failed on the wrong level before the fix.
Three room regressions pass after it, including native upper-elevation bounds,
unchanged draft source, existing feature/body identity and unassigned fallback.
The corrected native repeat retained Upper after copying the Ground perimeter's
inside footprint. Preview/Apply and cold reopen preserved that assignment.
The [fixed slab readback](../../artifacts/architecture-authoring-review-20261105/independent-native-floor-slabs-fixed-readback.json)
confirms separate Ground and Upper slabs, each 11800 × 7800 × 200 mm.
Their Z bounds are -100 to 100 mm and 3000 to 3200 mm respectively.
All preexisting model entities and native Parts remain unchanged; saving updates
File information metadata.

## Linked level and drawing changes

Upper elevation edits used native Preview/Apply and history. Moving the level
changed the wall's world Z bounds and the stair's rise while preserving their
attachments. The
[dependency readback](../../artifacts/architecture-authoring-review-20261105/independent-stair-level-linked-source-readback.json)
checks each saved checkpoint with native geometry and unchanged lower walls.

A real drawing book and A4 landscape sheet were created through Drawings.
View 1 selects only Upper graphical wall. The initial view remained Top because
custom direction fields require their own Apply action. That view is retained
as intermediate evidence; a Z change cannot demonstrate paper displacement in
its X/Y frame.

The subsequent standard Front selection was explicitly applied at scale 1:50.
Its saved X/Z frame, origin and source remained fixed while Upper moved from
3100 to 3200 mm. The native drawing regenerated after Apply, Undo and Redo.
The [Front source readback](../../artifacts/architecture-authoring-review-20261105/independent-front-level-link-readback.json)
confirms unchanged book/view/source identities and the dependent native model.

Both PDFs were exported through the actual native Export as PDF flow. The
[PDF vector audit](../../artifacts/architecture-authoring-review-20261105/independent-front-pdf-vector-readback.json)
measures all 12 wall/opening lines moving upward by 2 mm, within the exporter’s
rounding precision. All page and title-block rectangles remain unchanged.
Complete rendered pages show no clipping or title-block overlap. The
[before PDF](../../artifacts/architecture-authoring-review-20261105/front-before.pdf)
is the level-Undo state; the
[after PDF](../../artifacts/architecture-authoring-review-20261105/front-after.pdf)
is the raised state. Vector-count equality alone is not the association evidence.

## Highlight correction, selection and reopen

The stale highlight reproduced after Window work followed by reopening a saved
wall from its level group. The inspector showed Edit applied feature, but the
Window creation tile stayed selected. A painted regression using the checked
real apartment project reproduced Window=True where False was required.

The correction excludes generic edit execution modes from creation-form
highlighting. Broad pending-operation guards and specialized FDM behavior remain
unchanged. All nine focused Architecture regressions passed. The corrected native
build repeated Window, Cancel and saved-wall editing: Window and Wall creation
tiles were both off. Explicit Wall creation still selected Wall.

At compact size, repeated 2D/3D switching retained the selected Stair 1 member
and one selected Part without source changes. The
[selection receipts](../../artifacts/architecture-authoring-review-20261105/representation-selection-receipts.json)
retain actual accessibility states. After Save, Quit and the exact Recent entry,
Model/Architecture reopened in 2D plan on Apartment finished floor at revision 0,
with Undo/Redo disabled. Upper remained at its final elevation, and its wall
could be inspected in 3D. Local view recall did not create another building model.

## Checks and evidence boundaries

Formatting, strict locked workspace/all-target Clippy, release and development
packaging passed on the final slab correction. The complete frozen-source workspace
suite exited 0: 2,583 passed, 0 failed and 4 intentional ignores across 295 groups,
including documentation tests. Test optimization level 3 retained debug assertions
and overflow checks.
An initial sandboxed run could not
bind temporary application-control sockets. The retained failure log distinguishes
that environment restriction from product behavior; the authorized rerun has
local socket access. Strict Clippy likewise passed after access to the existing
symlinked OCCT cache lock was allowed.
The later highlight-only run was interrupted after discovery of the slab bug;
its partial log is intermediate evidence, without a complete-suite pass claim.

The [check summary](../../artifacts/architecture-authoring-review-20261105/checks/summary.json)
links retained logs. The [screenshot manifest](../../artifacts/architecture-authoring-review-20261105/screenshot-manifest.json)
records actual JPEG dimensions, source provenance and hashes for all captures.

Native captures 01-17 use the preceding review source `14778d5`. Captures 18-35
use the highlight-corrected source `4111f8a`; slab captures 34-35 precede the
ownership correction and are failure evidence. Captures 36 onward use final source
`a520123`. The final release review bundle is
`artifacts/architecture-authoring-review-20261105/optimized-final/MultiCAD.app`,
executable SHA-256
`b30145efba6d7355bdaba54a818165368a1967c8d0db30e250e664391aa10080`.
The distinct development package is
`artifacts/development/build-G6CPOn/MultiCAD.app`.
The final binary's version and diagnostics commands exited 0, confirming
macOS/aarch64, statically linked OCCT 7.9.3 and the compiled Rust/native runtime.
Diagnostics report Source commit: not stamped; the development package and
manually retained review bundle are not release-provenance evidence.

The visual review inspected representative normal and compact captures. Capture
28 shows the 280 mm stair input during editing; it is not standalone evidence of
a completed native preview. The actual ready-state observation and saved native
readback establish that edit's validation and geometry separately.

Source and geometry audits are read-only and do not independently replay GUI
focus, Cancel or history. Existing native regressions cover invalid openings,
different-wall picking, removal guards and stale drawing receipts. The actual
GUI checks here cover supported placements and explicit lifecycle observations.
Windows/Linux runtime, response-time percentiles, manufacturing readiness and
human acceptance remain separate. All ToskCAD windows were closed after the
final reopen check. No application was installed, pushed or published.
