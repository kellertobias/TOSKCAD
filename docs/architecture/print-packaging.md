# Print packaging

Export → Print writes a 3MF print package from finished Parts. It can also write one STL per Part. It packages exact geometry and nothing more. MultiCAD does not slice, arrange a plate, generate supports, control a printer or confirm that a package prints.

| Code | Contents |
| --- | --- |
| [`crates/multicad-core/src/exchange/print.rs`](../../crates/multicad-core/src/exchange/print.rs) | Request, Part resolution and the preview/receipt contract |
| [`crates/multicad-model/src/export/print.rs`](../../crates/multicad-model/src/export/print.rs) | Native tessellation, findings, readback, atomic writes and the print worker |
| [`crates/multicad-ui/src/print/`](../../crates/multicad-ui/src/print/mod.rs) | The Print workspace |
| [`crates/multicad-ui/src/application/print.rs`](../../crates/multicad-ui/src/application/print.rs) | Ribbon commands and the native save dialog |
| [`crates/multicad-app/src/print.rs`](../../crates/multicad-app/src/print.rs) | The installed native adapter |

## Choosing Parts

A print request names its Parts in one of two ways:

- **Selection.** Parts captured from the viewport selection when Export is entered, or when a Print command is run from another mode. Later viewport changes do not alter it until the next capture.
- **Tag.** Every Part carrying one exact, case-sensitive tag. `printed` is proposed.

Only world Parts are packaged. The preview lists each of these as **Not packaged**, with its reason:

- a chosen Instance or Assembly;
- an Assembly component definition;
- owned scale geometry;
- a Part consumed by a later operation or without a current solid.

An explicit Part that no longer exists fails the preview. The package is never broadened to other Parts. At most 128 Parts, including aids, go into one package.

## Removable FDM aids

[FDM aids](fdm-aids.md) are classified by their `fdm.aid` feature, never by name or tag.

- **Include removable FDM aids** (the default) adds every aid serving a chosen finished Part. A chosen aid Part is included too.
- Turning it off leaves every aid Part out and lists each one as **Not packaged**.

The finished Parts are the same either way.

## Preview

A preview resolves the request against one immutable source snapshot on the print worker. The worker compiles the native model there, and the UI thread only polls for replies. A changed revision or setting re-resolves after a short pause. An older preview stays visible, marked display-only, and cannot be written.

The preview shows:

- **Objects.** Each packaged Part with its role (finished, or the aid and the Part it serves), its Z range, exact volume and triangle count.
- **Files.** The 3MF and any STL files, named from the file chosen later (`<name>.3mf`, `<name>-<Part>.stl`).
- **Not packaged.** Skipped entities and left-out aids, with reasons.
- **FDM process records.** Every process record named by an aid serving the chosen Parts, even when the aids are left out: layer height, line width, minimum layers and lines, support gap and fit clearance.
- **Findings.** These are review notes. They never block writing and are not a printability check. They contain:
  - the FDM aid inspection issues for aids serving the chosen Parts (a left-out aid is marked "(left out)");
  - a Part that extends below the build plane z = 0;
  - a package whose lowest point is above the build plane.

An empty package cannot be written. The preview says why: nothing was selected, no Part carries the tag, or no chosen entity is a world Part with a current solid.

## The package

- **3MF.** One file with one mesh object per packaged Part and one identity build item per object. Coordinates stay in world millimeters, so +Z remains the build direction the aids assume. The model declares `unit="millimeter"`, and objects use Part names. The ZIP layout is the same deterministic layout as the [Export mesh writer](export-sets.md#mesh-output).
- **STL per Part** (optional). Binary STL files beside the 3MF, in millimeters. STL has no unit field; the header states the unit. A Part name is cleaned of path separators and reserved characters, and repeated names get `-2`, `-3` and so on.
- **Tessellation.** Meshes come from current exact solids at the chosen chord deviation (default 0.1 mm, angular 15°). Display meshes are never used.

## Writing

**Write 3MF package** first checks that the preview is current and writable. It then opens the native save dialog with `<document name>.3mf` suggested. The dialog adds `.3mf` when no extension is given.

The dialog keeps a source guard. If the document changes while the dialog is open, nothing is written and the user is told to start again.

The chosen path goes to the print worker, which:

1. checks the source stamp against the snapshot;
2. requires an absolute `.3mf` path in an existing folder;
3. rebuilds the package and requires the previewed plan hash;
4. reads every generated file back, comparing objects, build items, millimeter unit, triangles, tessellated volume and the exact volume within the chord allowance;
5. writes each file to a staging file in the same folder and flushes it;
6. renames the staged files over the targets, then rereads each target and compares bytes.

Writing stops with nothing written if:

- the source is stale;
- the plan changed;
- evaluation failed, for example a support with no room under its target;
- a readback failed;
- a target is a directory or symlink;
- a staging write failed;
- the job was cancelled before renaming.

A rename failure partway reports which files were already replaced. Existing files with the previewed names are replaced. The OS dialog confirms replacing the 3MF. The STL names are shown in the preview before writing.

A pending write keeps the window open until its result arrives. The receipt lists every path, size, content hash, triangle count and read-back volume. The project source is never changed.

## Verification

- [Core tests](../../crates/multicad-core/src/exchange/print.rs) cover:
  - aid pairing under both aid policies;
  - process records;
  - exact tag selection;
  - missing identities;
  - request validation.
- [Native tests](../../crates/multicad-model/tests/print_packages.rs) write packages to disk and read them back:
  - a lifted block with a support and a post, with and without the support: objects, items, unit, volumes and bounds;
  - per-Part STL files and atomic replacement of an existing 3MF;
  - tag selection and refused empty packages;
  - stale sources, changed plans and failed support evaluation, all writing nothing.
- [Workspace tests](../../crates/multicad-ui/src/print/tests.rs) cover:
  - request settings;
  - current, stale and late previews;
  - blocked writes;
  - receipt attribution;
  - rendered controls.
- [Application tests](../../crates/multicad-ui/src/application/print.rs) cover:
  - the Print tab;
  - the save dialog kind and suggested name;
  - a source edit while the dialog is open;
  - close deferral during a write.
- The [production adapter test](../../crates/multicad-app/src/print.rs) writes a real 3MF and STL through the workspace and reads them back.

These checks do not establish interactive macOS behavior, Windows or Linux runtime behavior, slicer import, or printability.

## Limits

- **No slicing or printer control.** There is no plate arrangement, orientation change, support generation, G-code or printer connection.
- **World Parts only.** Placed occurrences are not packaged per occurrence.
- **No print settings in the 3MF.** No materials, colors, slicer metadata or process records are written into the package.
- **Not in MCP.** MCP does not expose print packaging yet.
