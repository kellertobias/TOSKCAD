# Native Common and Split verification, 2026-11-05

CAD-222's actual macOS review verified the Common menus and explicit Split
roles at normal and compact window sizes. It found a keyboard focus leak:
Return in the plane-offset editor executed Split. Commit `979aa97` captures
text focus before drawing the editor, so finishing that text edit no longer
also requests execution. A fresh Enter after selecting the splitting-tool role
still validates and applies the operation.

Formatting, strict workspace Clippy, release and development builds passed.
The full workspace suite passed: 2,580 passed, 0 failed and 4 ignored
across 295 groups, including documentation tests (exit 0). Human
acceptance is separate. The [historical report](explicit-split-2026-11-02.md)
retains the original implementation and geometry/failure contracts.

## Native controls and keyboard correction

The native Common/Create group contained Add Primitive, Extrude, Revolve, Sweep
and Loft. The primitive popup showed readable Box and Cylinder icons and labels.
Boolean contained Join, Subtract, Intersect and Split; Move / Copy contained
Move / Copy and Align. Fillet / Chamfer and Shell / Thickness remained present.

With no selection, Split immediately showed Targets · 0 selected · active and
Splitting tool · 0 selected. Model was the initial tool kind, with no chosen
model or implicit plane. Selecting the actual box through the model tree filled
Targets; Enter moved focus to Splitting tool. Compatible preselection also
filled Targets on entry. Plane mode offered explicit XY/+Z, XZ/+Y and YZ/+X
choices. Role-button clicks changed selection focus without exchanging inputs.

On the 20 × 20 × 10 mm box, XY at offset 20 mm failed native validation because
both sides must contain positive-volume material. Cancel returned to r0 with
Undo disabled. This was an owned test draft, not a discarded user document.

The before-fix valid 5 mm offset reproduced the focus bug: the offset editor was
focused, yet Return committed the split at r1. Its saved geometry is retained
separately in `plane-before-keyboard-fix.tcd`. The new painted regression failed
on that implementation with “Return in the offset editor must not request
native execution.” After the fix all 11 focused Split tests passed, including
the native two-target execution and history regression. The tests also verify
that explicitly refocusing the tool role restores fresh Enter execution.

The actual `979aa97` release bundle repeated the valid offset edit. Return left
the form open at r0 without a native preview or Apply. Explicit Preview then
reported ready with positive/negative roles. Clicking Splitting tool and pressing
a fresh Enter committed r1. One Undo button click restored the original box at
r2; Redo restored both outputs at r3; Save updated preview metadata at r4.

A Cmd+Z injection did not change history. The native Undo/Redo buttons worked;
keyboard Undo is not asserted from that attempt. A source audit found no
Split-specific stale text focus after Apply. Automation key translation remains
an unproven explanation, not an application diagnosis.

## Compact multi-target model boundary

A separate developer fixture used public canonical MCP panel-stock and panel
creation APIs to make three closed box solids. It was not created through the
GUI. The [creation receipt](../../artifacts/split-review-20261105/native-model-split-fixture-creation.json)
records the actual API inputs, native bounds and provenance:

| Body | Bounds, mm | Volume, mm³ |
| --- | --- | --- |
| Target A | [0,0,0]–[20,20,10] | 4,000 |
| Target B | [30,0,0]–[50,20,10] | 4,000 |
| Closed cutter | [-1,10,-1]–[51,22,11] | 7,488 |

At the actual 2200 × 1400 capture size, tool-first entry allowed both target
checkboxes to remain selected. Enter advanced to the tool field; selecting
Closed cutter in the model tree left Targets at two and set Splitting tool to
one. Clicking either role permitted correction without losing the other role.
Keep splitting model remained off.

Native Preview reported inside/outside results at the cutter's actual boundary.
The compact inspector showed both required input fields; scrolling reached all
four output names grouped under Target A and Target B, the limitations and
native-ready status. Footer Preview, Apply and Cancel stayed reachable. Cancel
left r0, Undo disabled and the exact original file hash
`da6f4806a535375ef8d182750bee9c79b41b5219d3b3df6881cbb1de46bb645f`.

The repeated operation corrected the retained cutter preselection to Target A
and Target B, acquired the separate cutter and executed with fresh Enter. It
committed four outputs at r1, named Split inside/outside 2 and 3. One Undo button
restored all three original inputs at r2. Redo restored the outputs at r3; Save
updated preview metadata at r4. Relaunch and the exact Recent entry reopened the
saved file in Model at r0 with all four output entries and Undo/Redo disabled.
The saved hash stayed
`db78441d1dd26f10dc8794fb80150f8e2a54fd833f6747386f2a7d8e53b40d32`.

## Independent geometry and retained evidence

The [after-fix plane readback](../../artifacts/split-review-20261105/independent-native-plane-after-readback.json)
confirms two connected 2,000 mm³ solids with Z=0–5 and Z=5–10 mm bounds. The
original box source and IDs remain unchanged and consumed. The separate
[before-fix readback](../../artifacts/split-review-20261105/independent-native-plane-split-readback.json)
is geometry evidence for that reproduction, not proof of the corrected keyboard.

The [model split readback](../../artifacts/split-review-20261105/independent-native-model-split-readback.json)
confirms four connected 2,000 mm³ outputs and 8,000 mm³ total. Each target has
inside Y=10–20 and outside Y=0–10 mm results, with its original X bounds and
Z=0–10 mm. Both targets and the cutter are consumed with Keep tools off; their
source entities and producer IDs remain unchanged. Native readers exited
successfully without modifying inputs. These saved-source checks do not
independently prove intermediate GUI focus, Cancel or Undo/Redo events.

The [screenshot manifest](../../artifacts/split-review-20261105/screenshot-manifest.json)
records 19 retained, unmodified JPEG captures and hashes at 2720 × 1720 and
2200 × 1400 pixels. The controller returned JPEG bytes despite its documentation's
PNG example; filenames use the actual format. The Cmd+Z no-change capture is
explicitly named and is not presented as successful Undo evidence.

## Checks and limits

The [check summary](../../artifacts/split-review-20261105/checks/summary.json)
links retained counterfactual, focused, formatting, strict Clippy and build logs.
The complete `workspace.log` records the passing locked suite on frozen source
`979aa97` with test optimization level 3, debug assertions and overflow checks
enabled. The focused tests and full suite include the corrected Enter guard.

The release review bundle is
`artifacts/split-review-20261105/optimized-review/MultiCAD.app`, executable SHA-256
`02601f3d8292f691709c58a4832863923f7f20d181f40965db70fc595f0b3689`.
The distinct `./build` package is
`artifacts/development/build-miuMch/MultiCAD.app`.

All CAD review windows were closed; the final running-application inventory was
empty. General open-shell splitting and disconnected output sides remain
unsupported with diagnostics under [ADR-0030](../architecture/0030-explicit-split-inputs.md).
Actual GUI model splitting here used a closed solid. Windows/Linux runtime,
performance percentiles, manufacturing readiness and human acceptance are not
established by this macOS review. No application was installed, pushed or published.
