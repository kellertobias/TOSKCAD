# Native Architecture navigation verification, 2026-11-05

Actual macOS interaction verified Architecture's root commands, nested categories,
all equipment-family entry points, draft preservation and saved project context.
The review found a stale equipment highlight when opening Library after Cancel.
The corrected build highlights the actual Library browser and clears the canceled
equipment highlight. Required Rust checks passed; human acceptance remains separate.

The [historical report](architecture-navigation-2026-11-02.md) retains foundation,
catalog, export and offscreen evidence. The
[current contract](../architecture/architecture-navigation.md) owns navigation
behavior. This review covers command navigation; linked architectural authoring,
compound cutouts and Wood assistants have separate verification scopes.

## Native categories and forms

The normal window captures are 2720 × 1720 pixels. Wall, Door, Window, Cutout,
Levels and Stairs appeared at the root, without a separate Shaft command.
Water, Electrical, Heating, Furniture and Landscaping each opened one child row.
The parent controls remained available in the narrow context cue with Back and
Close. Electrical separated connections from lighting; Water separated fixtures
from appliances. Furniture retained Add Cabinet and Panel.

The [family receipts](../../artifacts/architecture-navigation-review-20261105/family-entry-receipts.json)
record 24 actual tile entries covering all 22 canonical equipment families.
Washing machine and Dishwasher were reached from both Water and Furniture.
Each entry selected its named family in the existing equipment form; Cancel
left revision 0. These are entry and cancellation checks, without applying
equipment placements or independently validating family geometry.

An edited Junction box width of 96 mm survived Back, Water, Close and reentry
to Electrical. The same sequence passed at the compact 2200 × 1400 capture size;
the [compact draft tree](../../artifacts/architecture-navigation-review-20261105/compact-draft-preservation.txt)
retains the selected family and dimensions. Cancel returned to revision 0 with
Undo and Redo disabled. All six compact root/category captures were independently
inspected for readable labels, selected cues and reachable Back/Close controls.
Exact logical cue height comes from the painted regression, not raster inference.

The [command receipts](../../artifacts/architecture-navigation-review-20261105/native-command-route-receipts.json)
record native Socket, Washing machine, Stairs, Planting, Fence, Washer, Equipment,
Cabinet and Panel routes. A separate Junction box invocation selected Electrical
and its real family. Washer opened Mechanical hardware. Fence reported the
missing-material prerequisite without creating geometry. Cabinet and Panel used
Furniture while Architecture was active, consistent with contextual routing;
the canonical Wood fallback is covered by the existing command-discovery test.

## Library highlight correction

The normal category and family-entry captures used source evidence `979aa97`.
Opening the shared Installation Library after canceling Dishwasher reproduced a
blue Dishwasher tile while Library itself remained unselected. The retained
before-fix image and accessibility tree document that mismatch.

The correction's pinned source evidence is `14778d5`. Tool highlighting reads the
actual modeling-form or Library state, while broad interaction-conflict behavior
remains intact. The new regression failed against the prior implementation on
the canceled Dishwasher highlight, then passed with the correction. All eight
focused Architecture regressions passed, including draft preservation and recall.

The corrected native build repeated Dishwasher, Cancel and Library. Dishwasher
remained off and Library was highlighted. Water, Heating and Landscaping category
entries and Electrical's global Library action opened the same Installation
Library. The Junction filter exposed the bundled Junction box project and preview
in that browser. No definitions were loaded or imported during these checks.

## Project context and unchanged model

Two owned fixtures held independent categories: Electrical for the apartment and
Water for the box project. Switching through the native project selector restored
each category without opening a tool. Saving the apartment updated preview
metadata at revision 1. After quitting and reopening through the exact Recent
entry, Architecture/Electrical returned at revision 0 with Undo and Redo disabled.

The [independent source readback](../../artifacts/architecture-navigation-review-20261105/independent-native-categories-source-readback.json)
compares the saved fixture with its original empty-apartment source. All original
entities, identifiers and nonentity document fields remain identical. There are
no added equipment placements, levels or geometry changes. Saving added one
`core.file_info` entity containing preview metadata, so the complete document
is not byte-identical. Both checked native readers exited successfully without
modifying the input files.

## Checks and retained evidence

The corrected review application is
`artifacts/architecture-navigation-review-20261105/optimized-review/MultiCAD.app`.
Its executable SHA-256 is
`62ce63d7fbe3e5b44377030bdff02b3e9c8740ceb3d1882d056140cf77c8d120`.
The separate development package is
`artifacts/development/build-ZHx5K6/MultiCAD.app`.

The [check summary](../../artifacts/architecture-navigation-review-20261105/checks/summary.json)
links the retained counterfactual, focused and required-check logs.
Formatting, strict workspace/all-target Clippy, the release build and `./build`
passed with the pinned toolchain and committed lockfile. The full locked workspace
suite exited 0: 2,581 passed, 0 failed and 4 intentional ignores across 295 groups,
including documentation tests. The suite used test optimization level 3 with
debug assertions and overflow checks enabled on the pinned corrected source.

The [screenshot manifest](../../artifacts/architecture-navigation-review-20261105/screenshot-manifest.json)
records dimensions and hashes for 49 retained JPEG captures. The width-edit
capture returned 1886 × 1200 pixels; its subsequent navigation capture returned
2200 × 1400. The artifact directory also retains raw accessibility receipts
and independent visual reviews. Two unsettled Library transition
captures live under `superseded/`; corrected settled captures supply final
evidence. All ToskCAD windows were closed after the final reopen check, and the
running-application inventory was empty. Windows/Linux runtime, performance
percentiles, manufacturing readiness and human acceptance are not established.
No application was installed, pushed or published.
