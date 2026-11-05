# Print packaging

Export → Print writes a 3MF print package from finished Parts and their selected placed copies. It can also write one STL per packaged object. It packages exact geometry and nothing more. MultiCAD does not slice, arrange a plate, generate supports, control a printer or confirm that a package prints.

| Code | Contents |
| --- | --- |
| [`crates/multicad-core/src/exchange/print.rs`](../../crates/multicad-core/src/exchange/print.rs) | Request, Part resolution and the preview/receipt contract |
| [`crates/multicad-model/src/export/print.rs`](../../crates/multicad-model/src/export/print.rs) | Native tessellation, findings, readback, atomic writes and the print worker |
| [`crates/multicad-ui/src/print/`](../../crates/multicad-ui/src/print/mod.rs) | The Print workspace |
| [`crates/multicad-ui/src/application/print.rs`](../../crates/multicad-ui/src/application/print.rs) | Ribbon commands and the native save dialog |
| [`crates/multicad-app/src/print.rs`](../../crates/multicad-app/src/print.rs) | The installed native adapter |

## Choosing Parts and placed copies

A print request names what it packages in one of two ways:

- **Chosen Parts.** An explicit list of Parts, placed copies and placed Assemblies. It starts from the viewport selection captured when Export is entered, or when a Print command is run from another mode; later viewport changes do not alter it until the next capture. The **Printable Parts** list in Export → Print edits it directly, so no visit to Model is needed:
  - it offers every world Part, placed copy (Instance, labelled `placed copy of <component>`) and placed Assembly that an explicit request packages without a skip (`print_candidates`). Component definitions, members of reusable Assemblies, owned scale geometry and removable aid Parts are not offered; aids travel with their targets;
  - **All**, **None** and **Model selection** (back to the captured selection) adjust the list, and a count shows how many offered entries are chosen;
  - a chosen identity the list does not offer (for example a captured definition Part) stays chosen, is shown as `Also chosen`, and Preview lists it as Not packaged with its reason;
  - choosing changes only the Print workspace. The document, its history and the model selection are untouched. A changed list makes the current preview display-only and re-resolves it.
- **Tag.** Every world Part carrying one exact, case-sensitive tag, and every placed occurrence of a Part carrying it. `printed` is proposed.

Each packaged object is either a world Part or one placed occurrence:

- A chosen **Instance** is packaged at its world placement. This covers copies placed by Move/Copy, placed components, symbol placements and placed assemblies.
- A chosen Instance of an assembly, or a placed **Assembly**, is walked to every leaf Part occurrence. The walk is the same one an explicit [export scope](export-sets.md) uses. Each leaf is packaged at its nested world placement.
- Only selected copies are packaged. Choosing a Part does not add its copies, and choosing a copy does not add the Part. A tag matches both, because both carry the Part's tags.
- An occurrence is labelled with the Part name and its occurrence names, for example `Post (Left rack / Slot)`. Repeated labels are numbered (`#1`, `#2`).

The preview lists each of these as **Not packaged**, with its reason:

- an Assembly component definition Part (choose its placed Instances; when choosing by tag, its tagged placed occurrences are packaged instead);
- a member Instance of a reusable Assembly definition, or the reusable Assembly itself;
- owned scale geometry chosen as a Part (choose its Instance);
- a Part consumed by a later operation or without a current solid;
- a placed occurrence without current exact geometry.

An explicit entity that no longer exists fails the preview. The package is never broadened to other Parts or occurrences. At most 128 objects, including placed occurrences and aids, go into one package.

## Removable FDM aids

[FDM aids](fdm-aids.md) are classified by their `fdm.aid` feature, never by name or tag.

- **Include removable FDM aids** (the default) adds every aid serving a chosen finished Part. A chosen aid Part is included too.
- Aids apply per occurrence. When a placed copy of a Part is chosen, each aid serving that Part is packaged again at the copy's placement, labelled like the copy (for example `Support (Lifted copy)`). The aid moves with its target.
- Turning it off leaves every aid Part out and lists each one as **Not packaged**.

The finished Parts are the same either way.

## Preview

A preview resolves the request against one immutable source snapshot on the print worker. The worker compiles the native model there, and the UI thread only polls for replies. A changed revision or setting re-resolves after a short pause. An older preview stays visible, marked display-only, and cannot be written.

The preview shows:

- **Objects.** Each packaged world Part or placed occurrence with its label, its role (finished, or the aid and the Part it serves), its world Z range, exact volume and triangle count. The summary counts the placed occurrences.
- **Files.** The 3MF and any STL files, named from the file chosen later (`<name>.3mf`, `<name>-<label>.stl`), with their mesh objects and build items.
- **Not packaged.** Skipped entities and left-out aids, with reasons.
- **FDM process records.** Every process record named by an aid serving the chosen Parts, even when the aids are left out: layer height, line width, minimum layers and lines, support gap and fit clearance.
- **Findings.** These are review notes. They never block writing and are not a printability check. They contain:
  - the FDM aid inspection issues for aids serving the chosen Parts (a left-out aid is marked "(left out)");
  - a Part that extends below the build plane z = 0;
  - a package whose lowest point is above the build plane.

An empty package cannot be written. The preview says why: nothing was chosen in the Printable Parts list, nothing carries the tag, or no chosen entity resolves to a world Part or placed occurrence with a current solid.

## The package

- **3MF.** One file with one build item per packaged object. A world Part has an identity build item. A placed occurrence has a build item carrying its world pose. A world Part and its placed copies share one mesh object, as do several copies of one Part. Each copy is still its own build item. Coordinates stay in world millimeters, so +Z remains the build direction the aids assume. The model declares `unit="millimeter"`, and mesh objects use Part names. The ZIP layout is the same deterministic layout as the [Export mesh writer](export-sets.md#mesh-output).
- **STL per object** (optional). One binary STL per packaged world Part or placed occurrence, beside the 3MF. Each is in world millimeters at its placement. STL has no unit field; the header states the unit. A label is cleaned of path separators and reserved characters, and repeated names get `-2`, `-3` and so on.
- **Tessellation.** Meshes come from current exact solids at the chosen chord deviation (default 0.1 mm, angular 15°), with native topological connectivity and bounded seam transport as described in [mesh output](export-sets.md#mesh-output). Both 3MF and the accompanying STLs use these manufacturing meshes. Display meshes are never used.

3MF files use the shared writer's
[indexed mesh validation](export-sets.md#mesh-output). Equal seam coordinates
share indices where needed for closed adjacency; valid coincident components
keep their own connectivity. No distance-based search chooses which faces join.
Invalid indexed topology stops writing before existing targets are replaced.

## Writing

**Write 3MF package** first checks that the preview is current and writable. It then opens the native save dialog with `<document name>.3mf` suggested. The dialog adds `.3mf` when no extension is given.

The dialog keeps a source guard. If the document changes while the dialog is open, nothing is written and the user is told to start again.

The chosen path goes to the print worker, which:

1. checks the source stamp against the snapshot;
2. requires an absolute `.3mf` path in an existing folder;
3. rebuilds the package and requires the previewed plan hash;
4. reads every generated file back, comparing mesh objects, build items, millimeter unit and triangles. It also compares each placed object's tessellated volume and its exact volume within the chord allowance;
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
  - placed occurrences chosen per path, with aids riding on them;
  - process records;
  - the Printable Parts list offering Parts and placed copies, never aids, with every offered entry resolving without a skip;
  - exact tag selection;
  - missing identities;
  - request validation.
- [Native tests](../../crates/multicad-model/tests/print_packages.rs) write packages to disk and read them back:
  - a lifted block with a support and a post, with and without the support: objects, items, unit, volumes and bounds;
  - per-Part STL files and atomic replacement of an existing 3MF;
  - tag selection and refused empty packages;
  - stale sources, changed plans and failed support evaluation, all writing nothing;
  - selected Move/Copy copies as build items sharing one mesh, with per-item volumes, bounds and per-copy STL files;
  - a copy of a supported block bringing its support at the copy placement, and leaving it out;
  - a tag matching a world Part and its copies;
  - two placed Instances of a reusable Assembly packaged at their nested placements, with the member Instance and the definition Part listed as not packaged, and the Printable Parts list offering the placed racks but not the definition, its member or its Part.
- [Native unit tests](../../crates/multicad-model/src/export/print.rs) refuse a readback whose per-object volumes or item count differ from the plan.
- [Workspace tests](../../crates/multicad-ui/src/print/tests.rs) cover:
  - request settings;
  - current, stale and late previews;
  - blocked writes;
  - receipt attribution;
  - rendered controls;
  - choosing one or both of two untagged Parts with no model selection, stale previews after a change, and restoring the captured selection without a document edit.
- [Application tests](../../crates/multicad-ui/src/application/print.rs) cover:
  - the Print tab;
  - the save dialog kind and suggested name;
  - a source edit while the dialog is open;
  - close deferral during a write.
- The [production adapter test](../../crates/multicad-app/src/print.rs) writes a real 3MF and STL files for a Part and a placed copy through the workspace and reads them back.

These checks do not establish interactive macOS behavior, Windows or Linux runtime behavior, slicer import, or printability.

## Limits

- **No slicing or printer control.** There is no plate arrangement, orientation change, support generation, G-code or printer connection.
- **No arrangement of copies.** Copies keep their model placement; overlapping copies overlap in the package.
- **Symbol placements by walk only.** Symbol placements are packaged through the generic Instance walk. No test covers a symbol placement directly.
- **No print settings in the 3MF.** No materials, colors, slicer metadata or process records are written into the package.
- **Not in MCP.** MCP does not expose print packaging yet.
