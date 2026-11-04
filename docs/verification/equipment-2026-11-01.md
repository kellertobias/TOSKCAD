<!-- Kind: verification checkpoint; Audience: CAD reviewers and maintainers; Purpose: record CAD-203 implementation evidence and remaining graphical checks; Non-goals: human acceptance, installation certification or cross-platform runtime claims. -->
# Apartment equipment checkpoint — 2026-11-01

> Historical checkpoint. See [the 2026-11-04 native verification](equipment-2026-11-04.md) for resumed graphical furnishing, fixes and final checks.

CAD-203 remains in progress. Commits `4dd3ea7` and `f640d39` implement hosted equipment and its shared offline Library. The [equipment contract](../architecture/equipment.md) describes canonical source, hosting, plan symbols and clearance envelopes. The later quit-dialog change is recorded separately under CAD-239.

## Verified source and native output

The catalog contains 21 generic editable families. Native tests check every family's declared physical bounds and authored plans at 1:50 and 1:100. Source tests cover 300 mm outlet and 1100 mm switch floor-relative mounting, wall edits, missing/topology-changed hosts, repair, private symbol inputs, copy/move and atomic history. MCP tests exercise shared create/edit validation, cancellation, stale requests, unit errors and invalid mounting/dimensions. Inspector tests cover stable Preview identities, cancellation, restored edit state and repeated-placement identity renewal.

The saved all-family fixture is `artifacts/equipment-review-20261101/readable/apartment-equipment.tcd`. Its three PDF/DXF sheets show furnished plans at 1:100 and 1:50 and an electrical-only plan at 1:50. Independent PDF path and DXF entity parsing checks true-size footprints, a 3.5 mm Type F mark outside the physical plan cut and category filtering. Visual PDF inspection led to larger appliance codes, wall marks beyond the wall hatch with attachment leaders, and separated labels. The final targeted native output run passed five tests, including the all-family export test and three planting regressions.

The shared installation Library contains 27 entries: four unchanged bearings, two plants with corrected rendered previews and 21 equipment definitions. Assembly preview generation now places a temporary occurrence before native rendering and rejects empty output. It does not modify retained definition pins.

The frozen equipment source passed formatting, strict workspace/all-target Clippy, 32 packaging tests and the full Rust workspace suite: 2,080 passed, zero failed and three ignored across 243 test groups. Logs and a machine-readable summary are retained with the local review artifacts. These results precede CAD-239; its full suite is recorded separately.

## Actual macOS evidence and remaining work

The uniquely identified Equipment Review development app opened the all-family fixture at revision 0. The explicit Architecture → Equipment action, native Library previews, wall host selector and floor datum were visible. Clicking a perimeter wall selected a snapped 5,300 mm along-wall position with a 300 mm outlet center height. Native Preview and Apply committed the outlet at revision 1. The user subsequently interacted with that window, so it remains open and is excluded from agent cleanup.

A separate Equipment QA development copy demonstrated wall selection, snapping, the 300 mm outlet and 1100 mm switch datums, Keep placing this type, and cancellation preserving revision 0. It did not establish a successful repeated-placement commit: several Preview/Apply attempts had no confirmed revision change, and automation also reported timeouts and transient missing-window errors. These observations do not identify a product root cause. That isolated QA process was closed after verifying its exact executable path; independent development builds were preserved.

Remaining verification includes a furnished apartment placed entirely through graphical controls across all categories; confirmed repeat/copy; graphical dimension, host and level edits; independent clearance visibility/checks; and save/reopen of those GUI edits. Source and native export tests do not replace this operator workflow. The architecture ADR remains Proposed, and CAD-203 must not be presented as accepted or complete from this checkpoint. No application was installed, published or pushed.
