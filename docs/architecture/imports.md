# STEP and KiCad imports

Mechanical imports bring external solids and circuit boards into the
canonical document as **import features**. They are not editable modeling
history: their geometry comes from the source file, and later modeling
features build on their Parts like any other.

| Code | Contents |
| --- | --- |
| `crates/multicad-core/src/model/imports/` | Records, KiCad reading, transactions and reimport plans |
| `crates/multicad-model/src/imports.rs` | Reading files natively and building the solids |
| `crates/multicad-kernel` | `Solid::step_solids`, `to_brep` and `from_brep` |
| `crates/multicad-mcp/src/tools/imports.rs` | MCP tools |
| `crates/multicad-ui/src/mechanical_import/` | Window import Inspector |
| `crates/multicad-app/src/mechanical_imports.rs` | Native reader for the window |

## Global Import entry

CAD-221 exposes Import beside Add Sketch, Library, Measure and MCP history in
all modes. The format selector routes STEP and KiCad to the native mechanical
reader, SVG/DXF to the existing [artwork importer](sketch-import.md), and canonical
JSON snapshots to a separate unsaved project. Format limitations remain those of
each existing reader; selecting a format does not imply full source compatibility.
There is no Model Import workbench. STEP/KiCad commands remain unavailable when
the native reader is absent. Update-import commands remain available through the
finder and existing selected-import workflow.

Cancelling the format selector or OS picker changes neither workspace nor source.
Only an accepted file enters its existing preview/Apply workflow. Import does not
save or overwrite the current project as part of file selection.

## STEP

Reading a STEP file splits it into its solids. Each solid keeps:

- its placement from the file, in millimeters;
- its exact shape, stored as OCCT BRep text;
- the name of the STEP representation item that produced it, if any;
- a **signature**: its volume and bounds.

An `import.step_source` extension stores the file name, the path it was read
from, the SHA-256 of its bytes and every solid with a stable identity
(`solid-1`, `solid-2`…). One `import.step_body` feature per Part rebuilds
its solid from that text.

The compiler only sees the document, so the geometry is stored there, not as
a link to a file. One source may hold up to 8 MiB of B-rep text and 512
solids.

### Reimport

Updating an import with a new version of the file matches stored solids to
new ones in two passes:

1. **Same shape.** Volume within one part per million, and bounds within
   0.001 mm.
2. **Same name.** A matching non-empty name, with changed geometry.

| Result | Effect |
| --- | --- |
| Matched | Keeps its identity, Part and links, and takes the new geometry |
| New | Gets a new identity and Part |
| Not found again | Kept with its last geometry and marked `missing`, so its Part and anything built on it survive |

The plan reports each case. Solids without names, which is common in files
written by OCCT, cannot be matched once their geometry changes. They appear
as one missing and one new solid.

## KiCad boards

`kicad::read_board` reads a `.kicad_pcb` file and converts KiCad's Y-down
millimeters to Y up.

- **Board outline.** From Edge.Cuts: lines, rectangles, polygons,
  three-point arcs (KiCad 6 and later) and circles, chained into closed
  loops. The largest loop is the board; the others are cutouts.
- **Thickness.** From `(general (thickness …))`, defaulting to 1.6 mm.
- **Drilled holes.** Pads with a round drill at least `min_hole_mm` wide;
  the default of 2 mm keeps mounting holes and skips signal pads. Each hole
  records whether it is plated and its footprint reference.
- **Footprints.** Each records its reference, value, library name,
  placement and rotation, and side. Its **envelope** is its courtyard
  (F.CrtYd or B.CrtYd), or its pads plus 0.25 mm when it has no courtyard.
  Its 3D model references keep their offset, scale and rotation.

Vias, KiCad 5 angle arcs, oval drills and other unsupported items are
counted and reported, never silently dropped. An open outline or a file
without one is refused with the position of the gap.

### Board and component Parts

An `import.board` feature extrudes the outline from z = 0 by the board
thickness. Cutouts and holes are holes in the same profile.

Each selected footprint becomes an `import.component` Part:

- **Linked model.** When the footprint's STEP model is found, it is imported
  as a STEP source and linked. The component is then that model, offset and
  turned about Z as written, placed at the footprint.
- **Envelope.** Otherwise the component is its envelope extruded by a
  default height (5 mm). The missing model is reported by reference and
  path; `prepare_component_model` links a replacement STEP import later.

Model paths resolve `${KIPRJMOD}` to the board's folder and other variables
from the environment. A `.wrl` reference is looked for as `.step` or `.stp`
beside it. Models turned about X or Y, or scaled, are reported and fall back
to the envelope.

Front components stand on the board top. Back components are turned over
about X and hang below z = 0.

### Board reimport

`prepare_kicad_update` keeps the board Part and replaces its outline and
holes. It matches components by reference:

- moved or unchanged components keep their Parts;
- new selected footprints are added;
- vanished components are kept, marked `missing`.

The plan reports whether the outline or holes changed, and which components
were moved, unchanged, added or missing.

## Driving panel cutouts

Board holes and component envelopes drive cutouts in other Parts through
the shared transfer engine
([associative feature transfer](0021-associative-feature-transfer.md)), not
an import-specific copy:

- `FaceHoles` on the board top transfers its mounting holes and cutouts;
- `FaceBoundary` on an envelope's top face cuts an opening for that
  component.

## Access

- **Window.** Model → **Import** has **Import STEP**, **Import KiCad board**
  and **Update import**. See [Window import](#window-import).
- **MCP.** `import_step` and `import_kicad`, for both a first import and an
  update. Both read bare file names from the exchange folder in scope
  (`--export-dir`) only. KiCad model lookups outside that folder count as
  missing.
- **API.** `read_step_file` and `read_kicad_file` in `multicad-model`, then
  the `prepare_*` transactions in `multicad-core`.

## Window import

The commands sit on their own **Import** tab in Model, next to Materials:
imports create Parts, and other Model features build on them. Sketch has
its own DXF/SVG import for curves.

1. **Choose a file.** The native file dialog filters `.step`/`.stp` or
   `.kicad_pcb`. Cancelling it leaves the document unchanged.
2. **Read.** A worker thread reads the file with `read_step_file` or
   `read_kicad_file`. The window keeps responding. An unreadable, empty or
   invalid file closes the import with the reader's message.
3. **Check.** A second worker prepares the same `prepare_*` transaction as
   MCP against a snapshot of the document and builds the resulting model.
   A model that does not build blocks Apply and shows why. Changing an
   option, or any edit to the document, starts a new check.
4. **Review in the Inspector.**
   - **STEP:** each solid with its name, volume and size.
   - **KiCad:** thickness, outline, cutouts and holes. You can set the
     minimum hole size, which reads the board again on the worker, and the
     default envelope height. A checkbox selects each footprint and shows
     whether its STEP model was found. Unsupported items are listed.
   - **Apply will:** the Parts to create, models that were not found, or
     the reimport plan.
5. **Apply or Cancel.** Apply commits the checked transaction as one
   undoable step. Cancel discards it.

**Updating an import.** **Update import** uses the import that owns the
selected Parts: a STEP solid, the board or a component. With nothing
selected, it uses the only import in the document. It then opens the
dialog for that kind of file. An import of either kind can also choose
**Update …** as its target in the Inspector. Before Apply, the Inspector
shows:

- **STEP:** matched, new and missing solids;
- **KiCad:** whether the outline and holes changed, and the moved,
  unchanged, added and missing components.

Missing solids and components are kept with their last geometry, as
described above.

The user picks the board in the dialog, so its footprint models are found
wherever KiCad would find them: `${KIPRJMOD}` and environment variables.
MCP looks only in the exchange folder.

## Limits

- **Window preview.** The Inspector lists what Apply creates, but the 3D view
  shows the candidate only after Apply. The check builds the whole document,
  so a feature that already fails elsewhere also blocks the import.
- **STEP structure.** The STEP assembly tree, product names and colors are
  not read; solids are flat Parts with their file placement.
- **KiCad coverage.**
  - Only round drills are read.
  - Back-side footprints use their stored coordinates, verified only for
    envelopes.
  - Model rotation about X or Y and model scaling are not applied.
  - Copper, silkscreen, zones and tracks are not imported.
- **Not verified.** Files from other CAD systems or real KiCad projects were
  not run; the tests use OCCT-written STEP files and hand-written boards.
  The window import is covered by application and Inspector tests with
  those files. It has not been checked interactively in the macOS window.
