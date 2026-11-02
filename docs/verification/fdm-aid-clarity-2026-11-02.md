# FDM aid choice and category verification, 2026-11-02

CAD-225 exposes separate Tab, Bridge and Support choices in Model → Other,
with distinct reviewed artwork, type-specific search aliases and guidance.
Native representative projects and nine production UI captures passed their
checks. Required workspace verification and the application build passed.
Actual macOS OS-window interaction is unverified; this is not human acceptance.

## Source and retained behavior

Work is local on `main` in `/Users/keller/repos/_apps/multicad`.

| Commit | Scope |
| --- | --- |
| `615d02d` | Typed choices, correct aliases/icons, Other category, saved navigation/preferences, guidance, precise picks, native guides and unit/UI tests |
| `bec2855` | Native three-aid fixture and production capture mode |
| `21541dd` | Preserve selected/missing/ambiguous target feedback for all three presets; final verification source freeze |

The canonical `AidShape::Tab`, `Bridge` and `Support` geometry definitions remain
separate. The native kernel, MCP aid contracts and Export inclusion/exclusion
policy are unchanged. The user-selected category answer on CAD-225 is Other.
The user also explicitly answered CAD-224: move shared material/stock authoring
to Project. That separate move follows this item; Materials was not merged into
Other here.

The new toolbar entries and command aliases open the intended shape. The legacy
`fdm-aid` identifier still opens Tab. A new form can switch type; an applied aid
keeps its saved type. Switching invalidates the old contact review and requires
a new viewer contact or explicit **Use coordinates** before Preview. Picking and
aiming remain live while Preview is blocked. Cancel leaves CAD source unchanged.

Native contact/direction guides now paint after the solid callback. The viewer
shows authored dimensions and measured native world bounds. Bridge exposes its
second Part beside the target selector. Reopened bridges no longer retain a
false unselected-target warning. A valid support gap is shown as a successful
separation check. Picked and snapped literals retain their exact canonical value
while display units are converted, including inch display and degree angles.

## Representative native geometry

`crates/multicad-app/examples/fdm_aid_review.rs` generates the projects under
`artifacts/fdm-aid-review-20261102/`. The finished block remains 8000 mm³ in
all three cases. Each aid passes the native contact checks, exports to STEP,
reads back its volume within 0.0001 mm³, and reopens from its canonical TCD file.

| Aid | Authored geometry and contact | Native volume | Removal intent |
| --- | --- | --- | --- |
| Tab | Base-edge point (0,10,0), outward 180°, reach 8 mm, overlap 2 mm, width 6 mm, thickness 0.4 mm | 24 mm³ | Break away the overlapping pad |
| Bridge | Anchors (39,10,0.2) and (61,5,0.2) on two Parts; width 2 mm, thickness 0.4 mm, span 22.56 mm | 18.0488226763 mm³ | Cut away the connecting bar |
| Support | Footprint 30×10 mm at (5,5), base 0 mm, underside 15 mm, gap 0.2 mm; derived height 14.8 mm | 4440 mm³ | Remove the separate support below the gap |

These are model geometry and operator intent, not physical print/removal tests.
`native-report.json` records source definitions, identities, bounds, volumes,
contact results and STEP/TCD paths.

## Targeted UI and navigation checks

The final `fdm` filter passed 11 tests, including native curved-face picking,
exact snapping, crowding refusal, neutral Cancel, type switching, explicit
coordinate entry, inch display and alias/recall behavior. The workbench filter
passed six tests, including all three presets and legacy recent-view migration.
Earlier focused preference and registry runs passed eight and nine tests.
The final target-selection filter passed three tests, with the aid case checking
all three presets against selected, absent and ambiguous Part selections.

Legacy `FDM` and `FDM aids` preference/recall titles map to Other, preserving
visibility and navigation behavior. Type aliases resolve separately: `tab`,
`bridge`, and `support`. Model source and Undo history are unchanged by routing.

Tests caught two issues during implementation: the pick-completion guard
initially blocked the draft geometry used for aiming, and a three-decimal input
formatter rounded exact picked coordinates. The corrected draft/commit boundary
and exact-literal path passed the final focused run.

## Production offscreen review

All nine final PNGs in `captures-final-tab/`, `captures-final-bridge/` and
`captures-final-support/` were inspected after final artwork mapping. Each shape
was rendered at 1100×700 logical points (1×), 1360×860 (1×) and 1360×860 (2×,
2720×1720 physical pixels) using the production Application/egui and native GPU
renderer. The fixture uses orange aid geometry beside the finished Parts.

The capture opens the saved editable aid, clicks the real Preview control,
waits for matching native Apply enablement, asserts source neutrality and checks
that type/dimension labels are visible after the native solid callback. Tab,
Bridge, Support and footer actions remain visible. Smaller inspectors scroll
numeric details; authored sizes and native bounds remain visible in the viewer.
Earlier `captures-tab/bridge/support/` are superseded by the final directories.

No OS window is created by this helper. These captures are not macOS window
interaction evidence.

## Required checks and build

The first run passed formatting and strict workspace/all-target Clippy on
`bec2855` (3m37s), then stopped in the UI group: 895 passed, one failed and two
ignored. The selected-Part status test caught a real regression: preset routing
overwrote target feedback. `21541dd` preserves that feedback and extends the
existing target test across all three presets. Its focused run passed.

Final formatting and strict locked workspace/all-target Clippy passed on
`21541dd` (Clippy 3m41s). `cargo test --workspace --locked` passed: 2318 tests,
zero failures and four ignored across 261 result groups. The UI group passed
896 tests with zero failures and two ignored in 24.76s. Test compilation took
4m14s. `./build` passed in 23.14s and produced
`artifacts/development/build-NUmWlu/MultiCAD.app`. Its executable returned zero
for `--version` and `--diagnostics`: development version 0.1.0-dev,
macOS/aarch64, static OCCT 7.9.3, eframe 0.36.2/wgpu, compiled native runtime.
The development bundle reports its source commit as not stamped; this is not
release provenance. The diagnostics workspace summary is legacy scaffold text,
so it is used only for basic binary/platform/runtime identification here.

Native fixture geometry
and the nine reviewed captures precede this status-only correction; their
geometry and layout are unchanged.

## Remaining gate

Actual macOS OS-window review of creating/switching each aid, contact selection,
editing, Cancel, Undo/Redo and Export behavior remains unverified because the
native CUA controller is unavailable. No substitute private input mechanism was
used. Windows/Linux runtime/CI and human acceptance are separate and unproven.

No installation, push or publication is authorized or performed here. No OS
windows were opened; completed offscreen QA processes exited. Unrelated
`.claude/` is preserved.
