# Architectural symbol graphical verification, 2026-11-04

CAD-206's recovered macOS workflow exercises pinned layered walls, independent
receiving context, openings, joined faces, equipment and drawing output. The
wall authoring/update workflow is recorded in the
[layered-wall report](layered-walls-2026-11-04.md); fence authoring, receiving
placement and revision updates are recorded in the
[fence report](fences-2026-11-04.md). The current fixes are `865391e` for missing
equipment-floor guidance and `5cbd525` for drawing-field focus. Pin-update history
and native rechecks passed. Formatting, strict Clippy, both builds and the full
workspace suite passed on `5cbd525`. Review windows are closed. CAD-206 is ready
for human review; human acceptance remains separate.

The [architectural symbol contract](../architecture/architectural-symbols.md)
owns blueprint capture, typed public inputs, embedded pins and explicit revision
updates. The [historical implementation report](architectural-symbols-2026-11-02.md)
retains native solids, atomic update/repair tests, MCP and provider restrictions.
Those historical checks are distinct from the actual graphical evidence here.

## Authored definitions and independent receiving walls

Actual native controls authored a layered-wall construction, exposed Height,
registered the saved author file in the installation Library and placed it in a
separate project. The layered-wall report records source revisions 1 and 2,
retained public identities, explicit comparison/update and a 2,800 mm receiving
height override against the revision-2 default of 2,900 mm. Opening a newer
Library file did not silently update that receiver.

The current receiving project contains two walls using the same embedded
revision-2 definition and pin:
`49e9282f47977a3bc79b2fb1533fd2c354c4bc16ba8f469d1a4e8f5a44f40e50`.
They have disjoint private source identities and independent receiving paths
and heights. The host remains 8,000 mm long and 2,800 mm high. The branch was
edited from 3,000 to 2,600 mm high and from 4,480 to 4,180 mm long. Both retain
the seven-layer construction and 412.5 mm total width, including the air cavity
and zero-thickness coating.

The [independent wall/join/window readback](../../artifacts/architectural-symbol-review-20261104/independent-pinned-wall-join-window-readback.json)
checks saved-container checksums and read-only native canonical wall/symbol
resolution. It confirms the same pin and definition, disjoint private IDs
between instances, retained branch Feature/Part and private IDs across edits,
and unchanged original receiver geometry. These inspections do not independently
regenerate or measure B-rep solids.

## Receiving window, T join and equipment

Actual graphical wall controls created a branch window, aligned the branch
start to the host and selected the supported flush-butt T relation. Saved source
retains the start at `[-350,-580]` mm and the 4,180 mm branch path. The host stays
continuous. The window is 1,200 × 1,200 mm, with 900 mm sill and 500 mm offset on
its branch segment. Wall context and typed pin ownership remain separate: these
receiving details do not change the authored definition or the other instance.

A generic Type F outlet was placed on that branch with an explicit Level and
wall host. Its 80 × 45 × 80 mm planning body uses 2,900 mm along-wall position,
zero standoff and a **400 mm floor-relative mounting height**. The selected
level has elevation 0, height 2,750 mm and slab thickness 200 mm. A later public
wall-height edit from 2,600 to 2,700 mm retained the outlet's exact resolved pose.

The [equipment-height comparison](../../artifacts/architectural-symbol-review-20261104/independent-pinned-equipment-height-readback.json)
confirms unchanged equipment attachment, settings, generic symbol pin and pose,
plus unchanged branch path, window, T join, layers and output identity. The host
wall source and its resolved properties remain unchanged. This outlet has no
service/door envelopes; no independent equipment-clearance claim follows here.
The actual graphical operations used native Preview before Apply; the independent
readbacks establish saved source and resolved properties, not fresh solid metrology
or electrical installation certification.

## Actual plan and section output

Native Drawings controls authored and exported
[one A4 landscape PDF](../../artifacts/architectural-symbol-review-20261104/Drawing_PDF_53baf7af-6c2c-4351-8b63-2e7aeea184ba_e1c70e558a1f_7c6cc8fb/Untitled_3_drawings.pdf).
The [source audit](../../artifacts/architectural-symbol-review-20261104/independent-plan-section-source-readback.json)
confirms the only new source entity is the drawing book; all previous geometry,
symbol, equipment and material entities remain unchanged.

| View | Saved definition and scope |
| --- | --- |
| View 1 | Top (+Z), 1:50, projection of current host and branch Parts. It is a top projection, not a horizontal cut plan. |
| View 2 | Right (+X), 1:20, section at X=1,000 mm, host Part only. It does not cross the branch window or T join at X=-350 mm. |

The [independent PDF review](../../artifacts/architectural-symbol-review-20261104/independent-plan-section-pdf-review.json)
checks saved frame cross products, container checksums, actual PDF page/path
bounds and a fresh raster render. Hatches, layer boundaries and the plan's
window/T arrangement are visible. The host-only section shows material bands
and the air gap. No clipped geometry, view collision or title-block overlap was
observed. The stroked frame remains within the 297 × 210 mm page with its
10 mm margin. The title block correctly reports mixed view scales.

The PDF is 157,302 bytes, SHA-256
`7b1ab15630fe94b4d137d83515f2de058a991138104ee5ac6cf6cf908f20a35c`.
Generic View 1/View 2 captions and blank project/title fields remain authoring
defaults. This is not a fully annotated construction drawing or a per-layer
independent dimensional measurement. The host section is not evidence of a
cut through the window, join or outlet.

## Explicit pin-update history

A separate receiving copy with no Height override began at revision-1 default
2,700 mm. Actual Compare, native Preview and Apply selected revision 2 and its
2,900 mm default. One top-button Undo from r2 restored the original revision-1
pin at r3, with Undo disabled and Redo enabled. Redo restored revision 2 at r4;
Save retained that revision.

The [independent pin-history comparison](../../artifacts/architectural-symbol-review-20261104/independent-pin-update-history-readback.json)
checks the saved before/applied/redone states. Applied and redone files are
byte-identical; all canonical entity IDs and the 13-entry private map, output,
receiving path, layer thicknesses and materials remain stable. Public overrides
are empty in this fixture, so resolved heights are 2,700/2,900/2,900 mm. The
unsaved Undo state is separately attributed to actual GUI observation: there is
no saved Undo snapshot, and the audit does not reconstruct one.

## Reopen with the registered author file unavailable

The graphical workflow closed prior CAD windows, temporarily renamed the
registered author file and relaunched the receiving project from Recent projects.
The Library displayed Missing for the original registered path. Embedded receiving
geometry still rendered the T join and window. The branch Inspector retained
revision 2, its pin and 2,700 mm override. Native Preview showed the full geometry
and enabled Apply. Applying the unchanged source closed the no-op editor at
revision 0; this is not claimed as a newly committed transaction. The author file
was restored afterward.

The [independent reopen audit](../../artifacts/architectural-symbol-review-20261104/independent-offline-reopen-readback.json)
confirms the offline snapshot is byte-identical to the saved plan/section source:
90,824 bytes, SHA-256
`a5fc00580066b9bd722dcfc25e65a034e525a2bea7cc923961d9abda22458654`.
The two architectural instances, pin, drawing, equipment, opening and join source
remain identical, and read-only native wall resolution succeeds. Byte identity
cannot itself prove the earlier GUI event; the report attributes those observations
to the native operator workflow separately. Only the registered local author file
was unavailable. No network isolation or general Library-service outage was tested.

The final `5cbd525` bundle repeated this check for the receiving fence while its
registered author file was temporarily unavailable. Recent projects reopened the
receiver at r0. Its pin, independently drawn 845.3 mm path, two posts and five
infill members remained visible. Preview reported **Native preview ready** and
enabled Apply. Unchanged Apply closed the no-op editor at r0. The author file was
restored and the review window closed.

The [independent fence reopen audit](../../artifacts/architectural-symbol-review-20261104/independent-offline-fence-readback.json)
confirms byte identity with the saved CAD-205 revision-2 receiver, retained private
and member IDs, a 2,300 mm height override against its 2,400 mm authored default,
and native resolution of two posts, one rail and four pickets. As with the wall,
this tests registered-author-file unavailability, not network isolation.

## Recovered UI failures and final gates

Commit `865391e` checks the explicit equipment Floor datum before requesting a
viewport position. A missing Level now explains how to create/select it; a missing
or invalid selected Level gives a repair instruction. Source and placement remain
unchanged on failure. A valid explicit floor continues to drive the prepared
attachment. This fixes misleading pick feedback; it does not invent a floor datum.

Commit `5cbd525` gives drawing origin/paper-position fields stable view-specific
IDs and reserves a fixed status region. Preview progress or a long diagnostic no longer
moves those fields or transfers focus while typing. The counterfactual regression
failed against the previous behavior; the focused drawing run passed 37 tests.
These are painted production-component checks; the separate native recheck is
recorded below.

On the final `5cbd525` release bundle, an actual native recheck reopened the
receiving project and selected drawing View 2. OS text input and Return retained
the complete Y position `160`, with X `65`, 451 vectors, Apply disabled and
revision 0 unchanged. This no-op check proves the full value survives the
observed interaction. It does not itself exercise every asynchronous error or
progress transition; the painted counterfactual regression covers those states.

A separate empty project then opened the Socket equipment tool. The native UI
visibly explained: “Create a Level in Model / Architecture, then select it as
the equipment Floor datum before placing equipment.” Preview and Apply remained
disabled. Cancel preserved revision 0. These final checks used
`artifacts/architectural-symbol-review-20261104/optimized-review/MultiCAD.app`,
whose executable SHA-256 is
`e132edb5612845c63e4d8532395f12583ec8028f2e4d0c989ad5f3ebc2b9f679`.

[Retained logs and current check summary](../../artifacts/architectural-symbol-review-20261104/checks/summary.json)
record the final formatting and strict locked workspace/all-target Clippy pass,
the optimized release build and the `./build` package at
`artifacts/development/build-yO0PqJ/MultiCAD.app`. The review release and wrapper
package are distinct outputs. The complete locked workspace run exited 0:
2,578 passed, zero failed and four ignored across 295 result groups, including
doctests. Its optimized test profile retained debug assertions and overflow
checks. The complete log and machine-readable totals are retained in `checks/`.

An earlier sandboxed workspace attempt failed at three local Unix-socket
listeners with EPERM. The first retry was interrupted before the final drawing
fix. Those logs are retained under `checks/`; the current final-source run uses
authorized local test sockets and passed. Neither earlier run is counted as a
passing workspace check.

The empty-project fixture was saved as
`receiving-walls.tcempty-floor-diagnostic.tcdd.tcd`; this filename retains a
native file-picker focus error. The existing receiver was not overwritten.
All ToskCAD review windows were then closed; the final app inventory showed
no running CAD application.

Existing provider restrictions remain: layered wall schema 2 and fence
construction schema 1; supported wall joins and bounded flat fence paths only.
No arbitrary provider/role conversion, physical fabrication, current-standard,
Windows/Linux runtime or human-acceptance claim follows. No installation, push
or publication was performed.
