# Shared workflow coverage, 2026-10-29

This matrix belongs to the CAD-132 engineering checkpoint. It maps each generic
parameter, sketch, symbol, drawing, exchange and external-control capability to
reproducible evidence:

- **Automated** evidence is a native test or a generator that uses the real
  kernel and independent readers.
- **Interactive** evidence is a recorded session in the production macOS
  application.

A blank interactive cell is a gap, not a pass. None of this is human
acceptance or Windows/Linux runtime evidence.

## Integrated checkpoint

`crates/multicad-app/examples/shared_workflow_review.rs` builds one native
editable project:

- two dynamic symbol panels with independent overrides, from one pinned
  definition;
- a DXF-imported plate, extruded by the exposed expression `plate_thickness`;
- a hatched steel material on the plate;
- a two-page drawing book:
  - **W-101:** an A3 exterior projection at 1:5, dimensions on the plate and on
    panel A, a note and a table;
  - **W-102:** A4 section-only and 20 mm deep sections at 1:2;
- an export set with PDF, drawing DXF, OBJ and 3MF recipes.

To write the project and its exports to a new directory:

```bash
cargo run -p multicad-app --example shared_workflow_review -- NEW_DIRECTORY
```

`crates/multicad-app/tests/shared_workflow.rs` checks the same project. The
checks below are independent; a value is read from an actual file.

### `shared_workflow_propagates_edits_into_solids_drawings_and_files`

- **Native volumes.**
  - Plate: profile area × 10 mm.
  - Panels: 150 × 90 × 8 and 200 × 100 × 12 mm.
  - Both panels share one pin.
- **Dimension values.** 100.0 for the plate and 150.0 for panel A.
- **Exported PDF.**
  - A two-page PDF.
  - `pdfinfo` reports 2 pages, when installed.
- **Exported DXF.**
  - One DXF per page.
  - Page 1 has the TEXT entities 100.0, 150.0, W-101 and the note, plus
    VIEW_VISIBLE lines.
  - Page 2 has VIEW_SECTION and VIEW_HATCH lines.
- **Exported meshes.**
  - Three OBJ files, with signed volumes matching each subject.
  - Three 3MF packages.
- **Propagation.**
  - `plate_thickness` 10 → 12 changes the plate volume.
  - Panel A's width override, 150 → 175 mm, changes only panel A. It also
    changes the drawing dimension and the exported DXF text to 175.0.
- **Refused edits.** A parameter cycle and a mass-unit width override are
  refused, and the source is unchanged.
- **Undo, Redo and reopen.**
  - Undo and Redo restore exact documents and dimensions.
  - Save/reopen gives an identical document whose drawings regenerate.

### `mcp_edits_the_shared_project_with_attribution_and_ui_equivalent_results`

This runs the MCP tool layer on a headless host.

- **Before the edit.** `document_info` and `list_parameters`.
- **Cancelled and applied.** A cancelled `set_parameter` commits nothing. An
  applied one is attributed to `mcp:checkpoint`.
- **Stale.** A stale replay is refused.
- **Result.** `evaluate_model` reports the plate. The parameter record and the
  native volume equal the same edit made through a window Session.
- **Undo.** It restores the original value.

### Defect fixed during the checkpoint

`65f3c90`: drawing views and dimensions of a placed Assembly instance, such as
a symbol, failed with "is not a Part". They now project the instance's leaf
Parts.

## Matrix

In the third column, "—" means there is no interactive record for that
capability.

| Capability | Automated native evidence | Interactive macOS record |
| --- | --- | --- |
| Parameters: names, literal values, expressions, units, exposure, table order | `multicad-model/tests/parameter_workflow.rs`; checkpoint propagation | `native-parameter-workflow-2026-10-28.md`, `sketch-parameters-2026-10-27.md` |
| Parameter cycles, invalid units, impossible constraints keep repairable source | `parameter_workflow.rs`; `multicad-core` parameter and solver tests; checkpoint refusals | `native-parameter-workflow-2026-10-28.md` |
| Constrained sketch drawing, dimensions, solve states | `multicad-solver` tests; `multicad-ui` sketch tests | `sketch-entry-2026-10-27.md`, `sketch-tools-2026-10-27.md` |
| Spline editing, exact trim and bounded extension | `multicad-model/tests/spline_trim.rs` | `native-spline-editing-2026-10-28.md`, `hierarchy-spline-2026-10-27.md` |
| Sketch references, projection, intersection, single repair | `sketch_external.rs`, `source_repair.rs`, `occurrence_external.rs` | `native-sketch-references-2026-10-28.md`, `native-occurrence-references-2026-10-29.md` |
| Staged multi-reference repair sets | `repair_set.rs`; `multicad-mcp/tests/reference_repair.rs`; `multicad-ui` `reference_repair_set_tests` | — |
| DXF/SVG import as editable geometry | `sketch_import.rs`; checkpoint plate | — (`native-sketch-import-2026-10-29.md` records no launch) |
| Dynamic and static symbols, independent overrides, pinned definition, explicit update, save/reopen, Undo/Redo | `symbols.rs`, `symbol_definition_gate.rs`; checkpoint | `native-symbols-2026-10-29.md` |
| Modeling features: extrude, Boolean, treatments, advanced, move/copy, transfer | `extrude_group*.rs`, `booleans.rs`, `edge_treatment.rs`, `advanced.rs`, `transform.rs`, `transfer.rs` | `native-modeling*.md`, `native-extrusion-*.md`, `native-move-copy-2026-10-28.md`, `native-feature-*.md` |
| Assemblies, placement, hierarchy, measurements, materials | `assemblies.rs`, `placement_batch.rs`, `measurements.rs`; core material tests | `native-assemblies-*.md`, `native-placement-2026-10-28.md`, `native-measurements-2026-10-28.md`, `native-material-properties-2026-10-28.md` |
| Drawing pages and projected views | `drawings.rs`; checkpoint W-101 | `native-drawing-pages-2026-10-28.md`, `native-drawing-views-2026-10-28.md` |
| Exterior, section-only and finite-depth views | `drawings.rs`, `drawing_exports.rs`; checkpoint W-102 | — (`native-section-views-2026-10-29.md` records none) |
| Associative dimensions, notes, leaders, welds, tables | `drawing_annotations.rs`; checkpoint dimensions | — (`native-drawing-annotations-2026-10-29.md` records none) |
| Multi-page PDF and drawing DXF | `drawing_exports.rs`; checkpoint files | — |
| STEP export | `multicad-model` export tests | `native-step-export-2026-10-28.md` |
| OBJ and 3MF export | `mesh_exports.rs`; checkpoint files | — |
| Planar 1:1 DXF | `planar_dxf.rs` | `native-planar-dxf-2026-10-28.md` |
| MCP inspect, preview, commit, stale and cancel, attribution | `multicad-mcp/tests/*.rs`, `multicad-app/tests/mcp.rs`; checkpoint MCP test | — (stdio client processes ran; no window session recorded) |
| MCP review queue and audit | `multicad-mcp/tests/tools.rs` | — |
| Project files: save, reopen, recovery | `multicad-core` project file tests; checkpoint reopen | `project-files-2026-10-27.md` |

## Gaps

- **Interactive macOS review.** The bundle has not been launched yet, because
  launching it needs the user's approval. This covers:
  - the integrated checkpoint project;
  - the newer features with no interactive record: import, section ranges,
    annotations, PDF/DXF/OBJ/3MF export, MCP against the window, and repair
    sets.

  CAD-132 owns this review, and it is not complete.
- **Occurrence rebinding in the window.** The staged repair window can rebind
  only feature-source references. Occurrence references are rebound through
  the API. The limit is recorded in `reference-repair-sets.md`.
- **Windows and Linux.** Interaction and low-power performance remain separate
  evidence.
