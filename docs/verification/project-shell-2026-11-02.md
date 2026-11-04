# Project shell verification, 2026-11-02

> Historical checkpoint. Later actual native Project layout, save/Library and
> fresh-process reopen evidence is recorded in
> [Project shell verification, 2026-11-04](project-shell-2026-11-04.md).

CAD-220 renames the visible Parameters mode to Project, removes its ribbon and
places its five draft/history actions in a left column. The parameter table and
single file-metadata sidebar retain their existing transactional handlers.
Implementation: `aa1c750`; reproducible offscreen review tool: `f007eee`, on `main`.

Actual macOS interaction and an after screenshot from a native application
window remain unverified. The production egui rendering and pointer tests below
are separate evidence. No application windows were opened for this review.

## Layout evidence

The inspected native archive is
`artifacts/shared-workflow-review/gui-screenshots/07-project.png`. Its full-width
black action band occupies approximately x0–2720, y154–387, below the purple
Common row and above the table. The implementation created that region with an
unconditional fixed-height ribbon panel and a black tool-band fill. Project now
omits that entire panel, including when returning from Model.

The archive predates the current metadata sidebar. CAD-220, CAD-209 and CAD-48
had no attached images when checked; the archive is not asserted to be the exact
image from the product owner's November feedback.

The after images use the actual `examples/cabinet.tcd`, its canonical parameters
and embedded preview image. The production `Application::show` paints through
egui/WGPU on the local Metal GPU, without creating a native window:

```sh
cargo run -p multicad-ui --locked --example project_shell_review -- \
  examples/cabinet.tcd artifacts/project-review-20261102
```

| Local artifact | Logical size | Scale |
| --- | --- | --- |
| `artifacts/project-review-20261102/project-1100x700-1x.png` | 1100 × 700 | 1× |
| `artifacts/project-review-20261102/project-1360x860-1x.png` | 1360 × 860 | 1× |
| `artifacts/project-review-20261102/project-1360x860-2x.png` | 1360 × 860 | 2× |

The images were inspected. They show no reserved ribbon band, a 184 px action
column, the table and a 300 px metadata sidebar. At 1100 × 700, the table has a
visible local horizontal scrollbar; its final column is reachable in pointer
scroll tests. The description editor scrolls within 104 px so the real file
preview and Library controls remain visible with the cabinet's long description.
The review adapter refuses file and Library writes; these images do not establish
successful save or registration interaction. `report.json` records the source,
GPU, scale and evidence limits. Artifacts are local ignored outputs.

## Observable behavior

Production egui pointer tests exercise Preview, Apply and Rebuild, Undo, Redo and
Discard Edit on a real native parameter-panel fixture. Parameter width and file
name apply together as one transaction; the kernel independently measures the
resulting solid at 160 × 80 × 8 mm. Undo restores the complete previous document,
and Redo restores the change. Preview changes neither the committed document nor
the file preview. An invalid parameter expression disables Apply; Discard clears
the invalid draft without changing the committed document.

Layout tests cover both sizes and 1×/2× density, action hit areas and order,
no Project ribbon panel, no full-width black action band, metadata bounds,
actual horizontal scrolling to the final table column and returning from Model.
Registry tests check the visible Project name and finder priority while retaining
legacy command identifiers, `params` and `parameters`. `project-mode` selects the
mode; the existing `project` reference command keeps its meaning. Project is
removed from ribbon-customization controls because it no longer has a ribbon.

## Checks

The pinned Rust 1.99.1 toolchain and committed lockfile are used.

| Check | Result |
| --- | --- |
| Full UI library tests | Passed: 864 passed, 0 failed, 2 intentionally ignored |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed, 2m52s on the final source |
| `cargo test --workspace --locked` | Passed: 2246 passed, 0 failed, 4 ignored across 254 groups, including doc tests |
| `./build` | Passed; native development bundle assembled |
| Local documentation links and `git diff --check` | Passed |

Development artifact: `artifacts/development/build-xavO6q/MultiCAD.app/Contents/MacOS/multicad`.
Its `--version` and `--diagnostics` commands both exited successfully. The
development build does not stamp a source commit; no GUI window was opened.
The full test run used local Unix socket access for app-control tests.

## Remaining acceptance

A native macOS window review still needs to show Project navigation and finder,
edit parameters and metadata, exercise all five actions, save/register through
the existing file/Library controls, resize to the supported minimum and capture
the after layout. A native window controller was unavailable. This work does not
claim human acceptance, Windows/Linux runtime results or performance timings.
No installation, push or publication was performed.

Materials and sheet/profile stock definitions remain CAD-224's scope. Sketch
mode and Model material placement are unchanged by this shell step; subsequent
queue items own those revisions. The current layout contract is
[native editing](../architecture/0005-native-editing.md), with file behavior in
[file metadata and Library](../architecture/file-metadata-library.md).
