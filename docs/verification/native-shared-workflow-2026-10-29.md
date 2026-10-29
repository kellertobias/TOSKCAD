# Native shared workflow review, 2026-10-29

This is the native macOS review for the CAD-132 shared workflow checkpoint.
It records agent verification, not human acceptance.

## Session

- **When:** 13:58–14:16, with the user's launch approval.
- **Bundle:** `artifacts/development/build-FarEB5/MultiCAD.app`, from source
  `406ac5c`. The SHA-256 of its executable is
  `ee35873420bc70d29904fdec80abec40d839fa0f5666049aa76b8bc647f0119c`.
- **How it was driven:**
  - CoreGraphics mouse events;
  - System Events keystrokes;
  - screenshots of the window.

  This needed macOS Accessibility for the session's helper app.
- **Screenshots:** in `artifacts/shared-workflow-review/gui-screenshots/`.

## Workflow

1. **Open.** The native Open dialog opened
   `artifacts/shared-workflow-review/shared-workflow.multicad` at r0.
   Parameters listed only the public `width` decoy and `plate_thickness`;
   the symbol-private parameters stayed hidden.
2. **Parameter edit.** Editing `plate_thickness` from 10 to 12 and choosing
   Apply and rebuild committed r1, which evaluated to 12 mm.
3. **Model.** Model showed the DXF-imported plate with its hole and slot,
   and the two symbol panels. Panel A's Inspector showed its pinned
   definition and three exposed inputs.
4. **Symbol edit.** Edit symbol parameters changed the width override from
   15 cm to 17.5 cm. The native Preview changed only Panel A, and Apply
   committed r2.
5. **W-101.** W-101 (A3 landscape) showed:
   - the 1:5 exterior projection;
   - the plate dimension at 100.0;
   - the updated panel dimension at 175.0;
   - the note, the table and a complete title block.
6. **W-102.** W-102 (A4 portrait) showed:
   - the section-only view: three separate hatched pieces cut through the
     hole and slot;
   - the 20 mm deep section: the same pieces plus the visible outline
     behind the cut.
7. **Export.** Export → Workshop outputs, with a new absolute directory.
   - Preview reported 3 matched occurrences and 2 pages, and PDF (1), DXF
     (2), OBJ (3) and 3MF (3) as Ready.
   - Run wrote `artifacts/shared-workflow-review/gui-exports/`.
8. **Undo/Redo.** Header Undo restored the panel dimension to 150.0 at r3,
   and Redo restored 175.0 at r4.
9. **MCP against the window.** Allow local MCP connections opened the
   same-user socket. A stdio client ran `multicad --mcp --app` as
   `cad132-review`:
   - `document_info` reported r4, application mode and the local socket.
   - Two `set_parameter` requests (14 and 16 mm) queued as reviews 1 and 2,
     prepared at r4, with actor, tool and affected entity.
   - Accept on #1 committed r5. #2 became stale: "The document moved from
     revision 4 to 5; prepare it again".
   - A replayed request at revision 4 failed with `stale_revision`.
   - `evaluate_model` reported the plate at 75200.44 mm³ (14 mm).
   - `history` attributed r5 to `mcp:cad132-review` on the `mcp` channel.
10. **Save and quit.** Save project as wrote
    `shared-workflow-native-gui.multicad`; the status read "Project saved
    atomically · r5". Quit closed the app with no unsaved-changes prompt.

## Independent checks of the window's files

**GUI export: PDF.** `pdfinfo` reports 2 pages, the first A3. `pdftotext`
contains 100.0 and 175.0.

**GUI export: OBJ.** Signed volumes:

| Part | Signed volume (mm³) | Expected |
| --- | --- | --- |
| Panel A | 126000.0 | 175 × 90 × 8 |
| Panel B | 240000.0 | 200 × 100 × 12 |
| Plate | 64458.7 | Profile area × 12; the holes are tessellated |

**Saved project.** A headless read-only server read it back
(`multicad --mcp --project … --read-only`, `evaluate_model` at revision 0):
the plate is 75200.44 mm³ with bounds 100 × 60 × 14 mm. The saved file is
`artifacts/shared-workflow-review/shared-workflow-native-gui.multicad`,
SHA-256
`2739703b0fd861257b81dde5315954222d6dfb4d446669827a54b395c42c17de`. It was
renamed after the session, because the agent's keystrokes had garbled the
name.

## Defects found and fixed

| Commit | Defect | Fix and evidence |
| --- | --- | --- |
| `9d81a61` | Table text was drawn as one block above its grid, in the window, PDF, DXF and SVG. | Each cell is now a label centered in its row and column. A core test checks every cell's bounds. A poppler render of the regenerated PDF shows every cell inside the grid. |
| `4889068` | The export recipe editor called PDF, drawing DXF, OBJ and 3MF "not available yet", though they ran. | The warning now appears only for STL and GLB. Drawing DXF notes that it uses fixed role layers. |
| `3ee3f00` | MCP `list_parameters` exposed symbol-private parameters, and `evaluate_model` omitted placed instances. | Both are fixed; the checkpoint MCP test asserts them. |

## Verification at `3ee3f00`

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| Strict all-target Clippy | Exit 0 |
| `cargo test --workspace --locked --offline` | 1,377 passed, 0 failed, 2 existing ignored |
| `npm test` | Passed |
| `./build` | Passed |

The build wrote `artifacts/development/build-VVughV/MultiCAD.app`.

## Limits

- **Fixes not seen in the window.** The rebuilt bundle with the three fixes
  was not launched, because launching a new GUI executable needs separate
  approval. Automated tests and a PDF render cover the fixes.
- **Not driven in this session.**
  - DXF/SVG import through the window: the review project was generated
    with an import.
  - Section-view creation controls and the staged repair panel.
- **No Windows or Linux interaction.** There are no hardware performance
  measurements and no human acceptance.
