# Native navigation verification, 2026-11-02

> Historical checkpoint. Actual native mode/sketch/import workflows and the later
> units-control fix are recorded in
> [Navigation verification, 2026-11-04](navigation-2026-11-04.md).

CAD-221 replaces the top-level Sketch tab with contextual sketch editing in
Model, exposes Motion, and gives every mode a shared right-side Add Sketch,
Import, Library, Measure and MCP history row. Project retains its left draft
controls and one file-metadata sidebar, without a ribbon. Implementation: `39c2485`; existing-sketch reopen correction: `dc3dcb6`;
current contracts: `bae9af6`, on `main`.

Actual macOS window interaction remains unverified. The automated and offscreen
evidence below does not establish that acceptance criterion or human acceptance.
No application windows were opened for this item.

## Behavior checked

| Contract | Evidence |
| --- | --- |
| Revised navigation | Production catalog and painted UI use Project, Model, Motion, Drawings and Export; contextual Sketch selects the Model top tab |
| Global controls | Pointer tests locate the five right-side buttons within the same row at 1100 × 700 in all five modes; Add Sketch enters plane selection without a source edit |
| One sketch entry | Creation is removed from local ribbons and Create Engraving from the UI/finder; active sketches disable Add Sketch while keeping pinned Finish/Abort controls |
| Face attachment | The existing real planar-face fixture verifies attachment and persistent occurrence; a real stale selected face falls back to plane choice with a reason and no document/revision change |
| Editing state | Returning to a live editor preserves contextual editing; old saved Sketch visits enter Model without stale tools; Finish and Abort retain pending-edit/history semantics |
| Import routing | Painted format buttons select existing STEP, KiCad, SVG/DXF or canonical JSON picker kinds; no Import workbench remains |
| Cancellation | Cancelling the selector or OS picker preserves mode, document, revision and history; a format choice alone does not change mode |
| Motion | Existing assembly commands route to Motion and cancel without edits; placement keeps its Model workbench, with one canonical project and scene |
| Stored data | App preference and recent-project tests retain old Parameters records and contextual Sketch preferences, migrate old Sketch visits, and round-trip Motion |

Existing native UI tests cover accepted STEP/KiCad imports, preview/Apply, explicit
updates, source-neutral cancellation and history. The global selector reuses those
handlers. SVG/DXF keeps its existing units/target/unsupported-entity checks. JSON
source import opens a separate unsaved project; it does not overwrite the current
project during format selection.

The legacy strict engraving API remains available to existing external clients;
this item removes the duplicate UI creation concept. A failed read-only face
inspection can request another plane; creation still revalidates before commit.

A pointer regression test creates a rectangle sketch, finishes it, uses Open
selected sketch from Model, and verifies the same selected sketch, unchanged
source/revision, contextual editing under Model, and visible Finish/Abort controls.
The tree and selected-sketch actions no longer require the removed top-level
Sketch mode.

## Visual evidence

`artifacts/navigation-review-20261102/` contains production `Application::show`
renderings of the actual `examples/cabinet.tcd` and its embedded file preview:

- `project-1100x700-1x.png`
- `project-1360x860-1x.png`
- `project-1360x860-2x.png`
- `report.json`, with source, GPU and scale provenance

The images were inspected. The global row fits on the right at both sizes,
Project has no ribbon band, its parameter table has local horizontal scrolling
at the minimum size, and its real preview/Library controls remain visible. These
are offscreen Metal/egui images of Project, not native-window screenshots of all
modes. The review adapter refuses project and Library writes.

Reproduce with:

```sh
cargo run -p multicad-ui --locked --example project_shell_review -- \
  examples/cabinet.tcd artifacts/navigation-review-20261102
```

A restricted execution could not find a Metal adapter. Running the same compiled
review tool with local GPU access succeeded, without a product source workaround.

## Checks

The pinned Rust 1.99.1 toolchain and committed lockfile are used.

| Check | Result |
| --- | --- |
| UI library tests on final source | 868 passed, 0 failed, 2 intentionally ignored |
| App preferences / recent projects | 3 / 8 passed, including migration checks |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed, 2m55s on corrected source |
| `cargo test --workspace --locked` | Passed: 2252 passed, 0 failed, 4 ignored across 254 groups, including doc tests |
| `./build` | Passed; native development bundle assembled |
| Local documentation links and `git diff --check` | Passed |

Development artifact: `artifacts/development/build-MHzGmk/MultiCAD.app/Contents/MacOS/multicad`.
Its `--version` and `--diagnostics` commands both exited successfully. The
development build does not stamp a source commit; no GUI window was opened.
The complete workspace run used local Unix socket access for app-control tests.

## Remaining acceptance

The actual macOS review must exercise navigation, selected planar-face and plane
fallback entry, existing-sketch reopening, shared-sketch operations, each import
route and cancellation, shortcuts and saved workspace recall. A native window
controller was unavailable; offscreen rendering and painted pointer events are
separate evidence. Windows/Linux runtime and CI results were not observed here.
No installation, push or publication was performed.

CAD-240 retains detailed Motion clearance/witness/result review; CAD-222 and
CAD-223 own the subsequent Model and Architecture grouping changes. The current
contracts are [native editing](../architecture/0005-native-editing.md),
[sketch entry](../architecture/sketch-entry.md) and
[imports](../architecture/imports.md).
