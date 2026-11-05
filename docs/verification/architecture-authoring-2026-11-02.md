# Architecture authoring verification, 2026-11-02

> Historical evidence. The [native review of 2026-11-05](architecture-authoring-2026-11-05.md)
> records actual macOS authoring, linked drawing exports and saved context checks.

CAD-246 connects Architecture authoring in Model to selectable canonical levels
and linked 2D/3D representations. Implementation source is frozen at `7c492d0`
on `main`; required Rust checks and the native build passed on that source.
Actual macOS OS-window interaction and human acceptance remain unverified.
Painted egui pointer tests, native geometry tests and offscreen Metal images
below are separate evidence. No OS windows were opened or applications installed
for this item.

See the [current contract](../architecture/architecture-navigation.md) and
[level contract](../architecture/levels.md).

## Implementation

| Commit | Change |
| --- | --- |
| `6666218` | Canonical level facts, ownership, filtered native scenes and local camera context |
| `9284a2a` | Architecture level sidebar, 2D/3D controls and active-level seeding of supported forms |
| `045813f` | Version 3 local representation/level recall with version 1/2 compatibility |
| `7b6efc5` | Graphical stair first-riser and walking-direction placement |
| `fc4ab06` | Native wall cuts, opening details and stair plan graphics; dedicated 2D framing |
| `6a18d0b` | Prepared-preview and post-Apply level filtering; Common selected-wall preservation |
| `9948e2f` | Fence level ownership, World plants, sidebar reopening and dependency/selection checks |
| `dc9f655` | Preserve evaluated per-request native query receipts alongside plan graphics |
| `a892e39` | Graphical saved-opening moves within the original wall, with stable identities and removal guards |
| `eddaeb0` | Follow a Common-selected opening host into its Architecture level and plan camera, including stale prior floor recall |
| `1d2caaa` | World Planting context through Architecture, Common and unvisited-context entry paths |
| `7c492d0` | View another floor during a pending modeling form without reassigning its level |

The canonical document and typed transactional modeling path remain shared by
UI and MCP. View and level navigation do not create another building document or
enter Undo. New walls, openings, slabs, stairs, equipment and fences use supported
canonical level context. Reopened objects preserve their saved attachments.
Plants use their existing World placement; entering Planting selects World
explicitly. Generic modeling objects without supported floor ownership retain
their existing contextual visibility rather than receiving invented attachments.

The native model worker derives wall/stair plan graphics through the existing
drawing evaluator without creating a drawing book. Wall cuts use the evaluator's
default plane 1000 mm above each wall's base. Door swings, window panes, section
gaps and hatch strokes replace occluding wall-top meshes. Other objects keep
their native top-view geometry. Native source geometry and canonical Part IDs
remain the object-picking/reference authority; physical topology picking and
saved section planes remain available in 3D.

## Observable behavior and native checks

| Requirement | Evidence |
| --- | --- |
| Graphical wall/opening authoring | Production native backend test clicks painted Draw/Finish controls, draws an upper-level wall, places a door/window through native wall picks, then redraws the wall |
| Preview and source ownership | Preview leaves canonical source unchanged; Cancel discards the candidate; Apply requires a current successful native receipt |
| Identity and history | Wall feature/Part and opening IDs survive redraw; opening and wall edits pass Undo/Redo and real native container reopen |
| Native plan correctness | Opening gaps are not bridged by section edges; door swing and window panes are present; original wall picking/topology and shared geometry remain available |
| Saved opening moves | Painted Existing/Pick controls reposition doors/windows without coordinate entry; native Preview/Cancel/Apply and Undo/Redo preserve opening IDs/types; other-wall picks and removal mode are guarded |
| Native query receipts | Request-specific probe IDs/results and latest-option suppression survive native plan publication, including a failed probe alongside valid wall/plan geometry |
| Plan failure | A failed optional projection shows a 2D diagnostic and preserves valid 3D geometry |
| Level context | Sidebar elevation/ownership and active-level creation are checked; prepared candidates and post-Apply handoffs use exact prepared membership without replacing canonical sidebar facts |
| Pending floor navigation | Painted level controls remain available for modeling drafts; native Preview/Apply retains the draft's assigned floor while another floor is viewed; Undo restores source |
| Common handoff | Explicit selected wall and its level are followed into Architecture; stale local floor recall cannot hide a readable selected host; the active level remains authoritative within Architecture |
| Stairs | Two snapped plan clicks choose first riser/direction; native bounds/solid count, stable identities, edit history and reopen are checked |
| Landscape | Native fence members follow the selected level; saved runs retain attachment; World plants have explicit context and saved-pose reopening |
| Linked drawing | A real drawing book consumes the wall; a host-level elevation edit moves native geometry and elevation vectors, rejects the older drawing receipt, and Undo restores both |
| Representation selection | Painted level/member controls retain native Part selection across repeated 2D/3D switches at 1100 × 700 / 1× and 1360 × 860 / 1× and 2× |
| Local reopen | Version 3 local metadata restores representation and level; World, missing-level repair, delayed history and coalesced visits preserve explicit intent; canonical project bytes are unaffected |

The initial production graphical/dependency test passed in 0.47 seconds. An intermediate full
UI checkpoint passed 887 tests with 0 failures and 2 intentional ignores in
24.33 seconds. The subsequent native selection regression passed in 0.50 seconds.
Final workspace results below supersede intermediate checkpoints.
After the final corrections, all 12 production native backend/lifecycle checks
passed in 0.80 seconds. The different-wall/removal guard passed in 0.01 seconds;
the expanded graphical saved-opening lifecycle separately passed in 0.79 seconds.
The final host-level correction passed its source-binding/camera regression in
0.01 seconds. Its full UI checkpoint passed 888 tests with 0 failures and
2 intentional ignores in 25.24 seconds.
World Planting entry passed five application/control cases in 0.74 seconds.
Pending-floor navigation with native Preview/Apply/Undo passed in 0.14 seconds.

## Visual review

The initial real-apartment review exposed wall tops hiding openings and an
undersized plan fit. Native plan graphics and an X/Y fit corrected those issues;
the fit also reserves space above the floating viewport toolbar.

The corrected matrix at `a892e39` completed in
`artifacts/architecture-views-review-20261102-corrected-final/`: 2D plan and 3D model
at 1100 × 700 / 1×, 1360 × 860 / 1× and 1360 × 860 / 2×. All six images were
inspected. Opening details are visible and the plan clears the floating toolbar.
PNG headers match all logical sizes and densities, including 2720 × 1720 pixels
for Retina. Every image retained revision 0 and native scene generation 1.
Later changes affect Common selected-host entry, World Planting entry and
pending-form level navigation, which these static captures do not exercise;
their application, source, camera and native lifecycle regressions are separate.
The renderer uses `examples/apartment.tcd`, production native modeling, viewport
renderer resources and a Depth32Float attachment. Project and Library writes are
rejected. `report.json` records source, GPU, size, density and native scene stamps;
the adapter asserts unchanged canonical source/revision and equal scene stamps
across representation navigation. This is not an OS interaction or latency test.

```sh
cargo run -p multicad-ui --locked --example project_shell_review -- \
  examples/apartment.tcd artifacts/architecture-views-review --architecture-views
```

## Required final checks

Pinned Rust 1.99.1 and the committed lockfile are used.

| Check | Status |
| --- | --- |
| `cargo fmt --all -- --check` | Passed on corrected source |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Frozen source passed in 3m17s |
| `cargo test --workspace --locked` | Frozen source passed: 2,290 / 0 failed / 4 intentional ignores across 255 groups |
| `./build` | Passed; native compile 36.76s |
| Built binary `--version` / `--diagnostics` | Both exited 0; macOS/aarch64, OCCT 7.9.3, compiled Rust/native runtime |
| Documentation links and `git diff --check` | Passed; no broken local links |

The initial full run stopped in the app unit group: 67 passed, 3 failed and
1 intentionally ignored. Two failures exposed lost request-specific cut-target
receipts when publishing from the retained model's source snapshot. Publication
now uses the evaluated per-job snapshot while borrowing the native model for
plan graphics. The saved-opening regression also required viewport picking that
the earlier form disabled. Both corrections passed the focused checks above;
the preceding integration run subsequently passed. The final navigation
corrections passed the fresh required test pipeline on their committed source.
The final UI suite passed 889 tests with 0 failures and 2 intentional ignores in
24.59 seconds. The four workspace ignores are the interrupted-writer subprocess
helper, a Blender-dependent import check and two opt-in hardware GPU tests.
The separate offscreen Metal matrix is the renderer evidence described above.

The verified development bundle is
`artifacts/development/build-lQjpGc/MultiCAD.app`. Diagnostics report
`Source commit: not stamped`; this development bundle is not release provenance.
It was not installed, published or opened as an OS window. The earlier
`build-WcYvKa` bundle is an intermediate checkpoint and is superseded by this one.

## Remaining acceptance

Actual macOS review still needs graphical 2D wall/opening creation and edits,
supported-object placement, level switching/editing, repeated 3D inspection,
selection, Preview/Cancel/Apply, Undo/Redo and save/reopen at normal and compact
window sizes. Windows/Linux runtime evidence is separate. Compound box cuts
(CAD-247), Wood grouping (CAD-248) and configurable Box faces (CAD-249) remain
sibling work. This item does not establish those workflows or human acceptance.
