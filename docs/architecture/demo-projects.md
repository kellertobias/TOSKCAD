# Shipped demo projects

Demo projects are ordinary native `.tcd` files that a person can open, edit and save. They are not test fixtures written by hand.

## Location and generation

- **Files:** `examples/` at the repository root, one `.tcd` per demo, listed in `examples/examples.json`. The directory is outside every crate and ships as application data (see [Bundling and the Examples gallery](#bundling-and-the-examples-gallery-cad-186)).
- **Generators:** each demo has a generator in `crates/multicad-app/examples/`. It builds the project through the typed transaction API that the window and the MCP server share. The native kernel compiles every geometry transaction before it commits.
- **File information:** the generator sets the document name, a description and a preview image through `project_file::info::edits` (see [file metadata](file-metadata-library.md)). It saves the file through the production `ProjectStore`. Capture on save stays on, so a user's own save replaces the preview with their 3D view.
- **Regeneration:** a generator refuses to overwrite a file. To refresh a demo, delete the file and run the generator again. Entity identities are new on every run, so the bytes change even when the design does not.
- **Drift check:** each demo's app test opens the shipped file and also builds the project fresh, then compares names, geometry records, schedules and file information. A generator change without a regenerated file fails that test.

## Bundling and the Examples gallery (CAD-186)

Source: [`examples.rs`](../../crates/multicad-ui/src/examples.rs) (catalog, location, copies), [`examples_gallery.rs`](../../crates/multicad-ui/src/application/examples_gallery.rs) (gallery, welcome cards, open-as-copy), [`bundled_examples.rs`](../../crates/multicad-app/src/bundled_examples.rs) (production wiring, self-test) and [`native-package.mjs`](../../tools/native-package.mjs) (packaging).

### Manifest

`examples/examples.json` lists the gallery in display order. Each entry names a top-level `.tcd` and, optionally, the other files in `examples/` it needs (relative paths, `/` separators, no `..`):

```json
{
  "format": "toskcad.examples",
  "version": 1,
  "examples": [
    { "file": "welded-workbench.tcd" },
    { "file": "apartment.tcd", "requires": ["cabinet.tcd", "welded-workbench.tcd"] }
  ]
}
```

The intended order is cabinet (CAD-182), workbench, front panel, apartment (CAD-185), and Werkzeugkoffer 2 (CAD-291). A new demo must be added here. Packaging refuses an unlisted `.tcd`. At run time an unlisted top-level `.tcd` is still shown after the listed ones, sorted by name.

### Package layout and validation

| Platform | Location |
|---|---|
| macOS | `MultiCAD.app/Contents/Resources/examples/` (sealed by the bundle signature) |
| Windows, Linux archives | `resources/examples/` beside the executable |
| Debug builds from the repository | `examples/` in the checkout; release builds never embed this path |

`assembleRuntime` (used by `./build` and release packaging) copies exactly the manifest, the listed demos and their `requires`. Before copying it refuses, with the file name and a fix: a missing manifest or listed file, an unlisted `.tcd`, a missing or unsafe `requires` path, a container whose signature, version, manifest checksum, source checksum or length is wrong, trailing data, a recovery snapshot, and an absolute developer path (`/Users/…`, `/home/…`, `C:\Users\…`, `/private/var/folders/…`) in the manifest or source. The packaged smoke run (`multicad --self-test`) then finds the examples beside the packaged executable and opens every one with the production `ProjectStore`; it fails when none are found.

### Gallery and welcome screen

- **Examples** is in the File menu, in the tool finder (`examples`, `example`, `demo`, `sample`, `gallery`) and on the welcome screen. It opens a window of example cards.
- The welcome screen shows the same cards under "Start from an example" when the recent-project history has loaded and is empty. With recent files it shows them as before; Examples stays one click away.
- Each card shows the file's own name, description (three rows; the full text on hover) and saved preview, read with `project_file::card` like recent-project cards. A missing or unreadable example keeps its card with the reason and "Reinstall ToskCAD to restore its bundled examples"; it cannot be opened. A missing examples folder lists the folders searched.

### Editable copies

Selecting a card copies the example, then opens the copy through the ordinary open path:

- **Folder:** `<Documents>/ToskCAD Examples/` — `~/Documents` on macOS, `%USERPROFILE%\Documents` on Windows (a OneDrive-redirected Documents folder is not detected), `$XDG_DOCUMENTS_DIR` or `~/Documents` on Linux, falling back to the home folder, and to the application-data folder when no home folder is known. The folder is user data: app upgrades and reinstalls never touch it.
- **Dependencies:** every bundled file the example needs (its `requires`, followed transitively, plus whatever the dependency hook returns) is copied to the same relative path, so sibling references resolve. A dependency that already exists is kept as the user left it.
- **The example itself:** written as `<file>.tcd`. If that file exists unchanged, it is reused. If the user changed it, it is kept and a new copy `<stem> 2.tcd` (3, 4, …) is written. Files are written to a temporary name and renamed, and copies are made writable.
- **Origin:** the status line reads "Opened your editable copy of the example “…” · path · the bundled original stays unchanged". The tab shows the copy's file name.
- **Protection:** Save, Save As and the replace confirmation refuse any destination inside the bundled examples folder, including new names. Opening a bundled original with Open, a recent entry or the operating system opens a copy instead.
- **Hook for cross-file references:** `ExampleLibrary::resolver` (`examples::DependencyResolver`) receives a bundled `.tcd` path and returns the bundled files it references. `bundled_examples::library()` and the self-test install `project_references::referenced_files`, which returns the stored paths of the file's [project references](project-references.md). The apartment's copy therefore gets `cabinet.tcd` (with the cabinet's own `libraries/cabinet-hardware-library.tcd`) and `welded-workbench.tcd` beside it, where its references find their sources again. The manifest's `requires` lists the same files so packaging copies them.

### Limitations

- Native interactive macOS evidence (first launch with no recents, Examples with recents, edit/save/reopen of each demo) is pending human review. Automated evidence: headless egui tests of the welcome screen and gallery, open-as-copy and original protection; catalog, copy and location unit tests; package layout and validation tests; the packaged self-test.
- Windows and Linux layouts are covered by the packaging code and its tests on macOS only until CI runs them.
- `multicad --mcp --project` does not refuse a bundled original; an MCP client could write one if the operating system allows it (a signed macOS bundle then fails its signature check).
- The cabinet (CAD-182) is listed first and requires `libraries/cabinet-hardware-library.tcd`, the pinned hardware symbols' source library. The library sits in a subfolder so it is copied with the cabinet but not shown as a demo of its own.

## Werkzeugkoffer 2 (CAD-291)

Generate the editable case and its fabrication outputs from the repository root.

```sh
cargo run -p multicad-app --example werkzeugkoffer_2 --locked -- examples/werkzeugkoffer-2.tcd artifacts/tool-case-exports
```

Open the example and use **Project → Parameters** to change `drawer_disk_cell`, `small_runner_play` or `dovetail_fit`. Use **Motion** for the closed and extended poses and three saved clearance studies. **Drawings** shows the mirrored side countersinks. **Project → Export** contains the six through-cut profiles, fabrication paperwork and print layouts.

The 185 mm large-drawer travel is a design assumption pending measurement of the installed slides. The slide models include their envelopes and hole patterns. The aluminium side frames represent the finished shape, with the notch locations recorded in the drawing and the separate aluminium cut list. Check stock and fastener fit before fabrication.

## Welded steel workbench (CAD-183)

`examples/welded-workbench.tcd`, generated by

```sh
cargo run -p multicad-app --example workbench_demo --locked --offline -- examples/welded-workbench.tcd
```

### Design choices

| Item | Choice |
|---|---|
| Worktop | 1500 × 1000 mm, 40 mm solid beech (`Beech block 40`, grain along the length), resting on the frame top |
| Height | Frame top 860 mm, working surface 900 mm |
| Tube | RHS 60×40×3 S235, the 60 mm side vertical on rails and along the rails on legs; 6000 mm bars |
| Top frame | Four rails with mitered corners, 50 mm inside the worktop edge on all sides |
| Legs | Four legs, 40 mm in from the frame corners, butted under the front and back rails, standing on 80×80×8 flat bar foot plates |
| Stretchers | Two side stretchers 150 mm above the floor, butted between the legs, and a long stretcher at mid depth butted between them |
| Top attachment | Four 40×5 flat bar tabs, 50 mm long, butted to the inside of the front and back rails, flush with the frame top; four Ø4 × 25 mm pilot holes in the worktop underside over the tab centers, for wood screws from below |
| Welds | 22 connections, each with one 3 mm fillet weld (135 MAG) in the sequence "Workbench frame sequence": top corners, leg tops, foot plates, side stretchers, long stretcher, tabs. Tabs get a 40 mm weld across the tab end; all others are all around |
| Hole grid | 10 rows × 10 columns, Ø20 through, 80 mm pitch, centered on the top (edge offsets 390 mm and 140 mm) |

### Parameters

| Parameter | Default | Drives |
|---|---|---|
| `top_length`, `top_depth` | 1500, 1000 | Worktop, frame, stretchers, tabs, the centered grid |
| `top_overhang` | 50 | Worktop edge to the frame outside |
| `frame_height` | 860 | Frame top and worktop underside |
| `stretcher_height` | 150 | Stretcher centerlines |
| `hole_diameter` | 20 | Grid hole diameter |
| `hole_pitch` | 80 | Grid pitch in both directions |
| `hole_edge_x`, `hole_edge_y` | `=($top_length - 9 * $hole_pitch) / 2`, `=($top_depth - 9 * $hole_pitch) / 2` | Worktop edges to the first column and row. Replace the expression with a number to move the grid off center |

The grid is one drill operation with 100 parameter-bound hole centers. A grid that would leave the worktop face is refused before commit. The hole count is fixed at 10 × 10.

### Drawings, schedules and exports

The drawing book has four A3 pages at 1:10:

- **WB-101** assembly: front, side and top views with associative overall dimensions;
- **WB-102** weld plan: an isometric frame view with weld symbols and the weld sequence table;
- **WB-103** worktop drilling: the worktop plan, the machining schedule and the panel cut list;
- **WB-104** the member cut list and the BOM.

Tables come from `multicad_model::schedules`. After a parameter edit they are stale page content until they are placed again; the generator's `refresh_tables` does that. The export sets are "Workbench frame and drawings" (PDF, STEP) and "Worktop drilling" (1:1 DXF of the upper face, STEP).

### Fabrication assumptions not modeled

- No load rating, weld strength, welding procedure, distortion allowance or fabrication approval is implied.
- The clearance holes in the tabs and the screws are not modeled: weldment members cannot yet be machined, and screws would be purchased Parts. The tab holes are drilled on assembly to match the pilot holes.
- Tube ends are open; no end caps, leveling feet or floor anchors.
- Holes closer than their diameter are not refused; choose `hole_pitch` larger than `hole_diameter`.
- Grid holes near the frame may run over a rail; the default grid clears the rails by 40 mm.

### Reuse in other demos

The whole file is the reusable unit. Another project places it as a pinned [project reference](project-references.md): its 20 Parts become one static symbol definition, with the relative path, document identity and source hash of `welded-workbench.tcd`. The apartment demo (CAD-185) uses this in its workshop.

### Preview

The preview is rendered without a GPU by the generator: an orthographic isometric view of the native solids' tessellation, flat-shaded in the material colors, then downscaled and encoded by `preview_capture::preview_from`.

## Apartment with workshop (CAD-185)

`examples/apartment.tcd`, generated next to the workbench it references by

```sh
cargo run -p multicad-app --example apartment_demo --locked --offline -- examples/apartment.tcd
```

### Design choices

| Item | Choice |
|---|---|
| Envelope | Exterior wall loop on a 10 000 × 8 000 mm centerline, 300 mm masonry (with a 45° section hatch), 2600 mm high; outside 10 300 × 8 300 mm |
| Partitions | 120 mm plaster board, the default wall hatch: one long partition at y = 4400 between the front and back rooms, one at x = 6000 between living room and kitchen, one at x = 4500 between bedroom and workshop. Partitions end at the faces of the walls they meet |
| Rooms (clear) | Living room 5790 × 4190 (24.3 m²), kitchen 3790 × 4190 (15.9 m²), bedroom 4290 × 3390 (14.5 m²), workshop 5290 × 3390 (17.9 m²) |
| Access | Entrance door 1000 × 2100 into the living room; 900 × 2100 doors from the living room into the kitchen, the bedroom and the workshop, each swinging into the room it serves |
| Windows | Six 1200 mm high windows with a 900 mm sill: two in the front wall, one in the kitchen side wall, one each in the bedroom and workshop back wall, one in the living room side wall |
| Floor | One 200 mm screed slab on the "Apartment floor" level, 150 mm beyond the wall faces |
| Furniture | Placeholder boxes, not manufacturing data: sofa and coffee table, bed and wardrobe, kitchen counter and dining table |
| Workshop | Pinned [project references](project-references.md) to `welded-workbench.tcd`, its 1500 × 1000 top at x 5150–6650, y 6800–7800 under the back window, and to `cabinet.tcd`, turned −90° with its back on the right wall (x 9332–9850 including the doors, y 5400–6200, 2000 high, all 64 leaves including its pinned hardware) |

Room names are drawing notes; the canonical model has no room entity. No building-code, egress, accessibility or structural compliance is claimed.

### Drawings

Three A3 pages with the standard title block:

- **AP-101** floor plan at 1:50: walls cut 1000 mm above the floor with hatching, door gaps with leaves and swing arcs, window panes, furniture and the referenced cabinet and workbench; room names with clear areas; overall outside dimensions and clear room widths and depths (associative to the wall faces);
- **AP-102** a fitted isometric view of the furnished apartment;
- **AP-103** the workshop references at 1:25 from above and from the front, with notes naming each reference's source file and pinned revision.

### References

Each reference records its file name (`cabinet.tcd`, `welded-workbench.tcd`, both next to the apartment), the source document identity and hash, and pins the source's physical content as a static symbol definition with revision `source <hash>`. The workshop sheet AP-103 prints both. The cabinet's own pinned hardware symbols arrive as ordinary materialized geometry inside the cabinet's pin; the hardware library is needed only to edit the cabinet itself, not to open the apartment.

### Regeneration and drift

The references pin the cabinet's and the workbench's content, so regenerating either changes the apartment's pin. The app test `tests/apartment_demo.rs` compares the shipped file with a fresh build, including the pinned revision, and fails until the apartment is regenerated too. Until then, opening the shipped apartment reports that reference as changed rather than replacing it.

