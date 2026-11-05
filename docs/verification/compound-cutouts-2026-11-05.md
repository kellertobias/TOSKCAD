# Native compound Cutout verification, 2026-11-05

Actual macOS interaction verified multi-box creation, explicit wall/slab and
floor-range controls, invalid-candidate refusal, saved editing, history and cold
reopen. Independent native measurements and STEP readbacks confirm the selected
receivers change together while other Parts stay unchanged. The exported linked
plan matches the saved L-shaped opening. Human acceptance remains separate.

This closes the native-window gap in the
[historical implementation report](compound-cutouts-2026-11-02.md). No Rust source
changes were needed for this verification. The
[compound source contract](../architecture/compound-cutouts.md) owns behavior.

## Graphical creation and validity

An owned copy of the corrected two-level apartment supplied seven real Parts,
including Ground and Upper slabs and an Upper wall with saved openings. Cutout →
Use multiple boxes opened one form. Named checkboxes selected Upper graphical
wall, Ground floor slab and Upper floor slab explicitly. Through receivers
resolved the shared height to −100 through 5900 mm.

Two opposite viewport clicks placed each box. Typed dimensions then made their
footprints exact. The first box spans [2200,3000] to [3200,4000] mm; the second
spans [2200,3500] to [4200,4500] mm. Their overlap forms one connected 2.5 m² L.
Native Preview enabled Apply. Cancel left revision 0 and no Undo entry.

Repeating creation exercised graphical Move, Resize, Remove box and re-add.
Removing a constituent left a valid single-box preview; re-adding restored two
editable boxes. Receiver checks produced successful wall-only, slabs-only and
Upper-range previews. For wall-only, the second box was temporarily extended to
touch the wall as required; it was restored before the combined transaction.

The following actual-window failures retained revision 0 and disabled Apply:

| Candidate | Observed refusal |
| --- | --- |
| Move Box 2 away from Box 1 | Boxes must overlap into one connected cutting volume |
| Add the untouched apartment perimeter as receiver | Chosen receiver is not cut; repair target or extent |
| Upper-only floor range with Ground slab checked | Chosen slab is outside the explicit floor range |
| Full-height, full-thickness interior wall cut | Cutout would split a receiving solid |
| Fixed bottom 0 mm through top 5900 mm | Slab cutout must pass completely through its thickness |

Unchecking Ground made the Upper range valid. The range constrains explicit
receivers; it does not silently select or discard slabs. Returning to the exact
combined L and Preview/Apply committed one revision. Undo removed that operation;
Redo restored it, followed by Save at revision 4.

## Native geometry, retained editing and persistence

The [initial native audit](../../artifacts/compound-cutout-review-20261105/independent-native-compound-initial-readback.json)
checks the actual saved project through the public native MCP path. It confirms
one source, two boxes, three generated receiving cuts and the original seven
Part identities. Each slab loses 500,000,000 mm³. The wall loses
108,000,000 mm³ from a 40 mm-deep rear-face chase, retaining one solid.
All four unselected native Parts remain unchanged.

Three STEP files exported from those actual receiving Part identities pass
native write/readback volume checks. They live under
`artifacts/compound-cutout-review-20261105/initial-receivers-step/`; hashes, bytes
and readback volumes are recorded in the audit. These are volume/solid readbacks,
not a separate independent STEP-bounds measurement.

Selecting Ground floor slab → Edit latest feature reopened the retained source.
At 1100 × 700 logical window size, Box 2 was edited to center X 3300 mm and width
2200 mm, keeping its left edge at 2200 mm. Preview/Cancel preserved revision 4.
Repeating the edit and applying committed revision 5; Save produced revision 6.

The [edited-source audit](../../artifacts/compound-cutout-review-20261105/independent-native-compound-edited-readback.json)
confirms the same source, box, receiving-cut and Part identities. The union grows
to 2.7 m²; each slab now loses 540,000,000 mm³. Wall removal remains unchanged.
The four unselected Parts retain their source and native geometry. A new Ground
STEP readback matches the edited volume.

Remove complete cutout from all receiving parts was previewed and applied at
revision 7. Undo restored it at revision 8; Save retained it at revision 9.
After later drawing edits, Quit closed the window. A fresh launch and the exact
Recent project reopened at revision 0 with Undo/Redo disabled. Reopening through
Ground's latest feature retained two boxes, three receivers and Box 2's saved
3300/4000 mm center and 2200 × 1000 mm size. Cancel and Quit closed that window;
the final running-ToskCAD inventory is empty.

## Linked plan and horizontal section

The existing drawing's View 1 was changed through the native controls to Top,
Ground slab only, scale 1:100. Apply page saved its linked source at revision 10.
The real Export as PDF flow wrote the review PDF. Its
[independent vector audit](../../artifacts/compound-cutout-review-20261105/independent-linked-ground-plan-pdf-review.json)
checks every exported geometry line against the saved source projection.
The slab outline measures 11800 × 7800 mm and the six-corner L has area 2.7 m².
All expected edges are covered without gaps or overlapping duplicates; maximum
coordinate residual is 0.000465 PDF points, within export rounding precision.
The complete rendered page has no clipping or title-block overlap. This is
projection evidence; blank metadata and absent dimensions leave it short of a
complete fabrication drawing.

A second native Section view selected the same Ground slab with Section only
and offset 10 mm. The
[saved section audit](../../artifacts/compound-cutout-review-20261105/independent-native-linked-section-readback.json)
confirms its X/Y frame and horizontal plane at world Z=10 mm, inside the slab's
−100 to 100 mm thickness. The actual window rendered its L-shaped section and
reported 12 native vectors. This is a horizontal section, not vertical-section
evidence. Both views share a paper position in this verification fixture; the
plan PDF was exported before adding the overlapping second view.

The final saved source retains the edited compound identities and all seven
Parts' native bounds and volumes. Existing automated linked plan/section tests
separately cover associative editing and reopen; this window check does not
claim a new before/after section-export comparison.

## Checks, evidence and limits

The tested native runtime is the existing optimized final bundle built from
`a520123`, executable SHA-256
`b30145efba6d7355bdaba54a818165368a1967c8d0db30e250e664391aa10080`.
Its path is
`artifacts/architecture-authoring-review-20261105/optimized-final/MultiCAD.app`.
Subsequent commits only changed documentation. The executable reports its
source commit as not stamped; this is local build evidence, not release stamping.

The unchanged Rust tree reuses the completed
[required-check receipts](../../artifacts/architecture-authoring-review-20261105/checks/summary.json)
on `a520123`: formatting, strict locked all-target workspace Clippy, release build,
`./build`, version and diagnostics all passed. The locked workspace test run
finished with 2583 passed, 0 failed and four intentional ignores in 295 groups.
Tests used optimization level 3 with debug assertions and overflow checks enabled.
The run includes the compound native, persistence, UI and MCP workflows described
in the historical report; no redundant full run was started for documentation.

Actual AX text and 33 JPEG captures are indexed in
`artifacts/compound-cutout-review-20261105/capture-manifest.json`. The compact
inspector keeps box actions and Preview/Apply/Cancel usable. Its long optional
per-box vertical-extent checkbox label clips at the right edge; the control is
available and this cosmetic limitation remains recorded. The complete-cutout
label wraps readably. Early captures may show a frame before layout settles;
saved source and independent geometry receipts establish the results above.

No Windows/Linux runtime, human acceptance, installation, push or publication is
claimed. Changes remain local on main; unrelated `.claude/` content is preserved.
