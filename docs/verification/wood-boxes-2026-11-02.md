# Editable optional-face wood boxes: verification, 2026-11-02

CAD-249 adds Model → Wood → Box and shared typed MCP tools for a canonical,
editable six-face recipe. Implementation, targeted native/offscreen checks, required workspace checks
and the application build passed. Actual macOS OS-window interaction remains
unverified. This report
is not human acceptance.

## Source and behavior

All work is local on `main` in `/Users/keller/repos/_apps/multicad`.

| Commit | Change |
| --- | --- |
| `446b5a1` | Canonical recipe, schema-2 recipe-backed panels, stable face identities, shared stock/material and native geometry |
| `9af083d` | Clean hidden-face visibility, preserve unrelated group members, rotated bounds and invalid downstream resize tests |
| `02aa0d7` | Typed MCP preview/set/inspect/remove, identity reuse, native preflight and preview audit |
| `74b3834` | Wood assistant, independent faces, labeled live viewport, preview gating, reopening and review fixtures |
| `71e7fc6` | Equivalent derived identity defaults and iterator-based native bounds assertions; final runtime source freeze |

The assistant starts with five panels and no top. Width/depth/height describe
an outside design envelope. Shared panel stock supplies thickness and material.
Usable inner dimensions subtract present opposing faces; actual world bounds
come from the generated panels. A bottom-only recipe therefore has stock
thickness in Z, even when its design envelope is taller. The butt corner policy
is explicit: sides span full depth/height; front/back fit between sides;
top/bottom fit between vertical faces. Corner material does not overlap.

Retained faces keep feature and Part IDs, panel modifiers and downstream history.
Disabled face IDs remain reserved for re-enabling. A downstream pocket follows
box dimensions and file reopen. Invalid downstream resize or omission of a
referenced face requires repair and commits nothing. Complete removal refuses
unrelated group children. Existing rectangle/profile/carcass panels retain their
schema and source behavior. See [ADR-0032](../architecture/0032-editable-wood-box-panels.md)
and the [wood-box contract](../architecture/cabinetry.md#wood-boxes).

## Targeted checks

| Check | Result |
| --- | --- |
| Core `wood_boxes` | 5 passed: every nonempty face mask, empty/invalid space failure, nominal/clear/actual bounds, stable IDs, stock changes, neutral preparation, Undo/Redo, TCD reopen, dependency refusal, hidden-face cleanup and unrelated-group preservation |
| Native model `wood_boxes` | 2 passed: closed/open-top/side/multiple omissions, rotated placement, exact solids/volumes/bounds, pair intersections, STEP readback, downstream pocket association and invalid resize |
| MCP `wood_boxes` | 1 passed: neutral preview, identity reuse, Set/Inspect/Remove, stock/face edits, native final volumes, stale revision, cancellation, audit and Undo/Redo |
| Production UI `wood_box` | 2 passed: painted checkbox changes the local recipe and inner dimensions; native controller tests Preview/Cancel, atomic Apply, Undo/Redo, recipe/member reopening and omitted-body scene proof |
| App MCP integration/discovery | 5 passed, including complete tool discovery and the read-only preview revision contract |

## Native review project and UI captures

`artifacts/wood-box-review-20261102/wood-box.tcd` is a real canonical open-top
600 × 400 × 300 mm box using 18 mm stock. Its usable inner dimensions are
564 × 364 × 282 mm. The bottom has a 40 × 30 × 5 mm pocket. The fixture exports
five panel STEP files, independently reads their native volumes back, and
reopens the TCD file with five native Part bodies. Results are recorded in
`native-report.json` beside the project.

`captures-final/` contains six production Application/egui/native-renderer PNGs:
open-top and omitted-left previews at 1100 × 700 logical points (1×),
1360 × 860 (1×), and 1360 × 860 (2×, 2720 × 1720 physical pixels). All six were
visually inspected. The omitted-left case clicks the real production checkbox,
then Preview, waits for matching native Apply enablement and leaves the saved
source unchanged. It produces four panels and 582 mm usable inner width while
retaining the bottom pocket.

The capture helper checks that all face controls and footer actions are visible,
and all six viewport labels are unclipped, mutually nonoverlapping and painted
after the native solid callback. Outside and usable-inner dimensions also appear
in the viewer. Initial captures exposed mid-word checkbox wrapping and a hidden
dimension summary; the final layout corrects both. Earlier `captures/` and
`captures-corrected/` are superseded by `captures-final/`.

These are offscreen production/native checks, not OS-window interaction.

## Required checks and build

`cargo fmt --all -- --check` passed on `71e7fc6`. Strict
`cargo clippy --workspace --all-targets --locked -- -D warnings` passed in
3m26s after correcting a derivable Default implementation and four iterator
lints in the new native tests. Full `cargo test --workspace --locked` and
`./build` passed on the same runtime source. The complete workspace run reports
2,315 passed, zero failed and four intentional skips across 261 result groups.
The UI unit group reports 893 passed, zero failed and two intentional GPU skips
in 24.42s. Test compilation took 20m17s; that is development verification time,
not application drawing-open performance.

The four skips are the interrupted-writer subprocess helper (its parent executes
and verifies it), an optional local Blender consumer, and two opt-in GPU tests.
The six separate production GPU captures described above ran successfully.

`./build` passed in 1m14s and produced:

`artifacts/development/build-C8Waqf/MultiCAD.app`

The bundle's `Contents/MacOS/multicad --version` and `--diagnostics` both exited
zero: ToskCAD 0.1.0-dev, macOS/aarch64, statically linked OCCT 7.9.3 and eframe
0.36.2/wgpu, compiled Rust/native runtime without Python or Node.js. Diagnostics
says the source commit is not stamped; this local build is not release-provenance
proof. Runtime sources remained frozen at `71e7fc6`; the final report commit
changes documentation only.

## Remaining gate

Actual macOS OS-window creation, face omission, downstream editing, save and
reopen have not been verified for CAD-249 because the native CUA controller is
unavailable. No replacement private input mechanism was used. Windows/Linux
runtime and CI evidence are separate and were not established here. Keep the
item eligible for that explicit native interaction gate; agent evidence does
not constitute human acceptance.

No OS windows were opened for this item. No installation, push or publication
was performed. Unrelated `.claude/` remains untouched.
