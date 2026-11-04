# Project shell native verification, 2026-11-04

CAD-220's actual macOS workflow verifies the ribbon-free Project layout,
parameter/metadata transactions, file save and Library registration. This check
reused the final `5cbd525` release bundle from CAD-206; it introduces no Rust
changes. The [historical implementation report](project-shell-2026-11-02.md)
retains the original source commits, offscreen layouts and component tests.
This report records the later native interaction, without implying human
acceptance.

## Actual layout and image scope

The native cabinet copy showed five icon-and-label actions in the left column:
Apply and Rebuild, Discard Edit, Preview, Undo and Redo. The parameter table and
one file-metadata sidebar remained visible. Project had neither a ribbon nor a
reserved blank band. Switching to Model, finding Project through the command
finder and returning to Project preserved this layout.

Native screenshots were emitted in the CUA conversation at 2,720 × 1,720 pixels
and, after dragging the window edge, 2,200 × 1,400 pixels. A final read of the
returned JPEG frame headers confirmed both dimensions. These are consistent
with the configured 1,360 × 860 and minimum 1,100 × 700 point sizes at 2×. At the
minimum size, the table's Row column remained reachable using its local
horizontal scrollbar. The screenshots are conversation evidence; no standalone
after-PNG files are claimed or linked here.

The inspected earlier image is
[07-project.png](../../artifacts/shared-workflow-review/gui-screenshots/07-project.png).
Its retained PNG is 2,720 × 1,720 pixels. The full-width black tool band below
the purple Common row spans approximately Y=154–387 in that original image.
The inspected tool rendition was 1,974 × 1,248; its displayed coordinates are
not used as image metrology. The archive is not asserted to be the exact
product-owner feedback image. Earlier and current fixtures, capture scales and
UI revisions differ, so this is not a pixel-matched before/after comparison.

## Native transaction and validation workflow

A separate copy of the cabinet example began with `cabinet_width=800` and its
original document name. Native Project controls changed width to 840 mm and
File name to **Native Project layout review**.

| Action | Observed result |
| --- | --- |
| Preview | “Validated parameter preview”; committed document unchanged at r0. |
| Apply and Rebuild | Width and file name committed together in one transaction at r1. |
| Undo | r2 restored width 800 and the original name. |
| Redo | r3 restored width 840 and the review name. |
| Invalid `=$missing_driver` | Diagnostic appeared and Apply was disabled. |
| Discard Edit | Removed that intentional invalid draft; restored 840 and retained committed r3. |

The invalid draft was deliberately discarded, not silently repaired or committed.
The [independent saved-source readback](../../artifacts/project-review-20261104/independent-native-project-readback.json)
compares checksummed containers and read-only native parameter evaluation. It
confirms document identity, all original entity IDs and hierarchy. The only
changed existing entity is the width parameter, retaining its original ID,
unit and description. All material, stock-assignment, symbol and resource
entities remain unchanged. One new `core.parameter_table` entity validly stores
row order; it is metadata rather than additional model geometry. The missing
driver is absent from saved source.

These final files establish the saved result, not an independent replay of the
intermediate Preview/Undo/Redo/Discard events. Those are separately attributed
actual native observations. No fresh B-rep or panel-bounds metrology is claimed.

## Save, Library and independent reopen

Saving the existing copy at r3 wrote
[native-project-layout.tcd](../../artifacts/project-review-20261104/native-project-layout.tcd).
Add to Library displayed the green Listed state and its path. Opening the global
Library, filtering by the new name and inspecting the resulting file card showed
the preview and matching path. Cancelling the browser preserved r3.

After quitting and relaunching, Recent projects reopened the saved copy in
Project at revision 0. Width 840, the new name and Library registration remained;
Undo and Redo were disabled. Final cleanup closed all CAD review windows, and
the application inventory was empty.

The final saved file is 337,563 bytes, SHA-256
`20f5639b1d681847dbd2993585a0eea26a3237d38ac593f421502a994cd981d6`.
A read-only hash check after the reported workflow matches the independent audit.
This registration is local file-Library behavior, not remote publication.

## Source, required checks and limits

The reviewed release bundle is
`artifacts/architectural-symbol-review-20261104/optimized-review/MultiCAD.app`,
executable SHA-256
`e132edb5612845c63e4d8532395f12583ec8028f2e4d0c989ad5f3ebc2b9f679`.
[Retained checks](../../artifacts/architectural-symbol-review-20261104/checks/summary.json)
cover that final source: formatting and strict locked workspace/all-target Clippy
passed; the release build and `./build` passed; the complete locked workspace
suite exited 0 with **2,578 passed, zero failed and four ignored across 295
result groups**. These are reused exact-source checks, not a second run for this
verification-only step. The wrapper's development package and optimized review
bundle remain distinct outputs.

The historical painted production-egui tests separately cover action bounds,
1×/2× layouts, local scrolling and transactional handlers. Current native
observations establish the exercised Mac workflow, not Windows/Linux runtime,
a performance percentile or human acceptance. Materials and shared stock authoring
remain the canonical Project behavior documented under CAD-224. No application
was installed, pushed or published.
