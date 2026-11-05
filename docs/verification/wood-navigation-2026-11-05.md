# Native Wood navigation verification, 2026-11-05

Actual macOS interaction verified the grouped Wood commands, their configuration
forms and representative native previews. Batten creation and a new Pocket passed
Apply, Undo, Redo and Save. Cold reopen retained the Pocket's editable inputs.
Human acceptance remains separate.

The [cabinetry contract](../architecture/cabinetry.md) owns the workflow behavior.
The command inventory is in
[`wood_navigation.rs`](../../crates/multicad-ui/src/wood_navigation.rs), with
dispatch in [`application.rs`](../../crates/multicad-ui/src/application.rs).
The owned evidence root is `artifacts/wood-navigation-review-20261105/`.

## Grouped commands and shared definitions

Model → Wood displayed the following groups at normal and compact window sizes.
Every entry opened an existing configuration workflow or the shared Hardware
menu. Joint contained the supported types without a separate peer Domino tile.

| Group | Visible entries | Native check |
| --- | --- | --- |
| Create | Panel, Batten, Screw / Hardware | Shared panel stock, solid rectangular profile stock and canonical metric hardware controls |
| Modify | Drill, Pocket, Joint | Distinct machining defaults, selected panel targets and the shared Joint type selector |
| Assistants | System 32 shelf pins, Cabinet, Box | Existing forms and successful native previews |

Batten without eligible stock showed an actionable Project → Materials
prerequisite and disabled Preview/Apply. Project authored a solid rectangular
profile named Review solid profile 20 x 40, with a 3000 mm stock length.
It used the existing Review plywood material. These names are review inputs;
the material name does not establish a measured timber specification.

The Hardware menu exposed Configure screw / hardware and Browse Library.
Configure opened the shared Machine screw form with ISO metric defaults:
3 mm nominal diameter, 0.5 mm pitch and 10 mm underhead/thread length.
The form retained modeled and cosmetic representations, fit choices and
fabrication allowance controls. Browse Library opened the installation's shared
Library browser. A disabled Attach screw connection entry explained that native
attachment to receiving Parts remains unavailable. No second catalog or completed
connection workflow is claimed.

Search found Batten, Drill, Pocket, Joint, Domino and Hardware commands.
Selecting the retained Domino search result opened the shared Joint form with
Loose tenon selected. Captures 33 through 35 record those searches and dispatch.

## Native creation and history

Batten used two points, from the origin to X=1000 mm, with centered section
anchors and zero rotation. Preview/Cancel preserved revision 1.
Apply created the member at revision 2; Save produced revision 3.
The visible toolbar Undo removed it at revision 4, and Redo restored revision 5.
Save retained the result at revision 6. Initial keyboard attempts produced no
observed history change; the history claim uses the visible toolbar checks.

The [independent Batten audit](../../artifacts/wood-navigation-review-20261105/independent-native-batten-final248-readback.json)
checks the GUI-saved source, shared stock/material references and native shape.
It confirms one connected solid of 800,000 mm³ with bounds
[0, -10, -20] to [1000, 10, 20] mm. Its STEP write/readback matches that volume.
The saved feature uses the canonical member definition; reopening a saved member
uses the shared generic member editor.

The existing five-panel Box fixture supplied the machining targets.
Selecting Bottom's feature before Drill seeded only that panel.
Drill opened with a 5 mm diameter, 8 mm blind depth and one hole at u=100, v=50.
Its native preview succeeded; Cancel preserved revision 0.
Pocket opened with the distinct rectangular cut controls and the same target.

A new Pocket at u=200, v=100 used 40 × 30 × 8 mm on the Top face.
Preview succeeded before Apply at revision 1.
Undo and Redo produced revisions 2 and 3; Save retained revision 4.
The [independent Pocket audit](../../artifacts/wood-navigation-review-20261105/independent-native-wood-pocket-final248-readback.json)
confirms a 9,600 mm³ removal and Bottom volume 4,007,328 mm³.
The original pocket and the other four panel shapes remain unchanged.
The final Bottom STEP readback matches its native volume.

## Existing assistants and failure handling

Joint exposed Butt A, Butt B, Miter, Fingers, Loose tenon (Domino) and Paired
dowels. An incomplete pair disabled Preview/Apply with a missing-panel message.
Choosing Bottom and Right allowed the default Domino native preview.
Cancel discarded the preview at revision 4.

System 32 retained its two rows, 37 mm offsets, 32 mm pitch, diameter/depth
controls and panel targets. A native preview of the selected Bottom succeeded.
Cabinet retained main stock, construction, back and shelf controls, including
individual shelf material, depth and adjustable support choices.
Its default native preview succeeded. Panel opened the shared panel-stock form.
Box retained independent faces, outside/inner dimensions and its native preview.
These exploratory forms were canceled without changing revision 4.

Captures 22, 24 and 27 show the corresponding preview overlay and enabled Apply.
Their accessibility trees truncate before the footer; the report does not rely
on an absent accessible “Native preview ready” row for those captures.

Quit closed the review process. A fresh launch and the exact Recent entry
reopened the saved machining fixture at revision 0.
Bottom → Edit latest feature retained the new Pocket's dimensions and target.
Changing its depth to 100 mm produced a native cut-through diagnostic for the
18 mm panel and disabled Apply. Cancel preserved the saved source at revision 0.
The final [running-window inventory](../../artifacts/wood-navigation-review-20261105/closed-review-windows.json)
is empty.

## Compact layout and verification limits

All nine tiles, their group labels and the scrolled Pocket footer were readable
at the current minimum 1100 × 700 logical window size.
An attempt to resize to 1024 × 600 was clamped to that minimum.
Captures 31 and 32 are 2200 × 1400 pixels; they do not verify the smaller target.
The minimum is declared in
[`lib.rs`](../../crates/multicad-ui/src/lib.rs).

The [independent visual review](../../artifacts/wood-navigation-review-20261105/independent-wood-navigation-visual-review.json)
records remaining presentation work for the final UI consistency pass:
System 32 Remove buttons wrap in narrow columns, the placement label clips at
the Inspector's right edge, and Panel's Grain Any option wraps or clips.
The review also records the smaller-size limitation. Primary compact Pocket
actions remain reachable after scrolling.

The GUI runtime was assembled from source commit `3a9da7a`, executable SHA-256
`e2bc46957fd6365871d9137e9791bfa1899171e23f50de3ea057f795d1f073f9`, at
`artifacts/wood-navigation-review-20261105/optimized-final/MultiCAD.app`.
Version and diagnostics exited successfully. Diagnostics reports the source
commit as not stamped; this is local verification, not release provenance.
Both final native audit receipts identify this exact executable and its hash.

Formatting, strict locked all-target workspace Clippy, the release build and
`./build` passed on this Rust source. The focused Wood run passed 11 tests.
The required commands were:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked -p multicad-app
./build
```

The test run used optimization level 3 with debug assertions and overflow checks
enabled, under `CARGO_TARGET_DIR=/private/tmp/multicad-cargo-optimized-tests-20261104`.
The focused command was `cargo test -p multicad-ui --lib --locked wood` with the
same test profile. The owned `checks/` folder retains the command logs.

The final locked workspace run exited zero with 2591 passed, zero failed and
four intentional ignores across 295 result groups. The UI group passed 961
tests, with zero failures and two intentional GPU ignores, in 63.63 seconds.
The [check receipts](../../artifacts/wood-navigation-review-20261105/checks/summary.json)
record the terminal result and final source/runtime identity.
The earlier deliberately interrupted run is retained as partial evidence only.
Hidden-workbench routing and generic shortcut conflict/typing guards have
automated coverage. A Wood-specific custom shortcut binding and persistence
round trip was not checked in the native window.

No screw-generation performance improvement, Windows/Linux runtime, human
acceptance, installation, push or publication is claimed. Work remains local on
main, with unrelated `.claude/` content preserved.
