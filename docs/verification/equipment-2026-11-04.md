# Apartment equipment native verification, 2026-11-04

The native graphical workflow furnished an initially empty apartment with all
22 equipment families, including junction boxes. Repeated outlets and a copied
refrigerator bring the saved fixture to 25 instances. Actual placement exposed
two repeat-placement defects, now fixed: retained forms keep their chosen host
and floor, and Apply preserves the view for the next pick.

[CAD-203](https://plans.tokenet.de/CAD/203) is submitted for human review.
The [earlier checkpoint](equipment-2026-11-01.md) remains historical evidence.

## Source and execution

Commits on main:

- `e65da59`: preserve equipment host controls when reopening a repeat form.
- `ec8eefe`: suppress automatic Fit while a repeat form remains active.

The ordinary new-tool path still seeds the active level. Retained repeat forms
renew occurrence identities and require another viewport pick. Existing native
preview, source consistency and Apply validation remain active. A native
controller regression reproduces the floor reset before the fix and passes
afterward, including two placements and a separate Undo step. The wall-host and
view behavior were checked in the actual macOS window.

The optimized QA executable, temporary bundle and initial empty fixture hashes
are recorded in `artifacts/equipment-review-20261104/native-review-fixed-metadata.json`.
The machine is Mac17,6, Apple M5 Max, 64 GB, macOS 26.6.2, with its built-in
120 Hz display. Workspace verification ran concurrently during later graphical
checks; no input-latency distribution is claimed for this workflow.

## Graphical authoring and persistence

The starting fixture contained three walls and a floor datum, prepared through
the typed API and compiled natively. Every equipment occurrence in the final
fixture originated through visible placement or Duplicate in the native app.
Viewport picks supplied positions; model coordinates were not typed.

Model / Architecture exposed Electrical, Heating, Water and Furniture routes.
The graphical session placed all ten electrical families, the radiator, all five
sanitary families and all six appliance families. Cooker/oven and cooktop,
outlets and lamp holders, and junction boxes remained distinct. The shared
offline Library listed 57 entries; its equipment definitions use the same pins
as the graphical presets.

The initial repeat attempt applied an outlet to the perimeter wall but reopened
on a different wall. After the fix, two successive placements committed at
separate revisions, retained the perimeter host and kept the same framing.
Each needed its own fresh viewport pick and native Preview before Apply.

Further graphical edits and independent saved-file readbacks verified:

| Operation | Observed result |
| --- | --- |
| Washing-machine properties | Width 650 mm, depth 600 mm, height 850 mm and rotation 90 degrees; native preview and Apply succeeded. |
| Refrigerator Duplicate and edit | Separate identities; the original stayed 600 mm wide, while the moved copy became 700 mm wide. Both retained depth 650 mm and height 1800 mm. |
| Floor datum 100 to 500 mm | All 25 instances moved exactly 400 mm upward, with sizes unchanged. The 300 mm outlets resolved at world Z 800 mm; the 1100 mm switch at Z 1600 mm. |
| Perimeter thickness 200 to 240 mm | Its three outlets followed the inner face by exactly 20 mm, retaining their identities. Other equipment positions stayed unchanged. |
| Envelope display | Both overlays, service-only, and both hidden were observed independently of the physical footprint. |
| Toolbar history | The refrigerator disappeared through Undo and returned through Redo. Shortcut attempts without confirmed revision changes are not counted as history evidence. |
| Native save/reopen | The reopened Inspector retained the washer's 650 mm width, 90-degree rotation and hidden envelope settings; the floor remained 500 mm. Cancelling inspection left revision 0 unchanged. |

Read-only stdio MCP reopened saved copies and resolved every equipment record.
It found 25 instances covering all 22 families, with no unresolved hosts.
`graphical-copy-readback.json`, the later readbacks and the floor/wall comparison
JSON files retain identities, poses, settings and dimensions.

The native clearance checker returned 13 envelope results. Showing versus hiding
the washer overlays produced exactly identical native results, including three
reported conflicts. The review layout therefore demonstrates conflict reporting;
it is not claimed clearance-free. These are conservative generic access boxes.

The saved GUI fixture is
`artifacts/equipment-review-20261104/graphical-final.tcd`, SHA-256
`a02e0a2a60c874ce2c937c6c2d5cc22ae262e333cbc18137b39d3a255b9ee463`.
The live file retained that hash after native reopen and cancellation.

## Drawing output

`graphical-sheets/` contains furnished plans at 1:50 and 1:100 and an electrical
plan at 1:50, each as PDF and drawing DXF, plus a project containing those sheets.
Sheet setup used the shared typed Rust API over the saved graphical source.
The exporter asserted that every original entity remained unchanged, compiled
the native model and preserved authored resources in the derived project.

Independent review rendered all three PDFs and found no clipping or colliding
equipment marks. PDF paths and ASCII DXF geometry confirm A3 landscape and
millimeter units. The sampled Type F mark remains 3.5 mm on paper. The rotated
washer footprint measures 6 by 6.5 mm at 1:100 and 12 by 13 mm at 1:50. The
700 mm refrigerator copy measures 7 by 6.5 mm and 14 by 13 mm respectively.
The electrical sheet excludes those furnishings while retaining electrical
marks. These checks sample the stated geometry; they do not measure every glyph.
`sheet-independent-check.json` records the checks and file hashes.

## Checks and cleanup

The pinned Rust 1.99.1 toolchain and committed lockfile were used. Formatting,
strict all-target workspace Clippy, the repository build wrapper and the
optimized workspace build pass. The complete `cargo test --workspace --locked`
run exits 0: **2,571 passed, zero failed, four ignored across 295 groups**.
Its UI group passes 941 tests, with two ignored.

This full run uses test opt-level 3 and no debug symbols, with debug assertions
and overflow checks explicitly enabled. The initial sandbox attempt could not
open the native cache lock; the authorized rerun completed with cache and
localhost access. That environment failure is retained separately from test
results. Logs and `checks-summary.json` are in the review artifact directory.

An ordinary uninstrumented optimized app is retained in
`artifacts/equipment-review-20261104/optimized-review/MultiCAD.app`.
Its identity is recorded alongside it. All ToskCAD QA windows were closed through
native controls, and the final inventory showed no running CAD app.
No installation, push or publication occurred. Human acceptance remains pending.
