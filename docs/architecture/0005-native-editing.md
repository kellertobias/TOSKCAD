# Native editing and revised workspace layout

This document owns the native editor contract and its implementation limits.
The canonical entity model remains described in [0004](0004-canonical-document.md).

## Product decision, 2026-10-27

The product owner requested five top-level modes: Parameters, Sketch, Model,
Drawings and Export. Parameters uses a full-width table without sidebars, with
an action ribbon for Apply and Rebuild, Discard and preview/history actions.
Export contains Common, Print and Render. Materials belong to Model.
These choices supersede the corresponding layout rules in
[0001](0001-project-boundaries.md).

Why: the parameter table needs explicit draft actions without losing its editing
space. Rendering belongs with output preparation, while materials affect model
properties and drawing conventions. Keeping the previous separate Render mode
would preserve the prototype layout, but conflicts with the requested workflow.

The requested coordinate control is a transform gizmo with world, object and
custom axes. It does not request NURBS curve or surface editing. The first
implementation translates datum sketches; instance rotation and reusable named
coordinate systems remain future work.

## Canonical editing

[`Session`](../../crates/multicad-core/src/commands.rs) owns revision checks,
attributable previews, atomic commits and Undo/Redo. UI and command-line actions
submit the same source mutations. The typed interface supports an external MCP
adapter; server transport, authentication and remote previews remain unimplemented.

[`parameters`](../../crates/multicad-core/src/parameters/mod.rs) evaluates typed
expressions, resolves dependency names to stable entity links and rejects invalid
units, duplicate names and cycles. [`ParameterEditor`](../../crates/multicad-ui/src/parameter_editor.rs)
keeps unapplied rows outside the committed document. Completing a row adds a
trailing blank row. Apply commits the draft as one transaction; validation failures
retain it and the last valid source. Exposed values are available in evaluated
parameter queries. Dynamic symbol authoring remains future work.

[`Registry`](../../crates/multicad-ui/src/registry.rs) describes modes, tabs, tool
labels, vector icons and command aliases. The ribbon, tool search and bottom
command entry consume that catalog. Workspace components own their interactions
and call the shared source API. Unimplemented extension tools are disabled;
the registry does not provide a binary plugin ABI.

## Datum sketch starter

[`sketch`](../../crates/multicad-core/src/sketch.rs) stores a versioned
`core.datum_sketch` definition with stable line/rectangle IDs and millimeter
coordinates. Known payloads validate at the document boundary. Missing plane links
remain loadable for repair; reading geometry from such a sketch returns an error.

[`SketchEditor`](../../crates/multicad-ui/src/sketch_editor.rs) retains the 3D viewer
before entry. New Sketch previews and selects XY, XZ or YZ in the scene. Cancel
creates no entity. Creating a sketch commits its source plane and definition,
then enables drawing tools. Existing sketches can be opened from the source tree.
Two clicks preview and commit a line or rectangle; Finish retains the sketch.

The camera toolbar includes the transform toggle and coordinate selector.
World uses XYZ. Object uses the sketch plane basis. Custom uses a chosen datum
basis and an editable angle around its normal. Numeric moves remain previews
until Apply; dragging an axis commits on release. Cancel drops the preview.
Moving a sketch on a shared plane creates an independent translated plane.

The viewport displays source wires and a configurable XY grid. Orbit, pan,
projection, fit and the orientation cross reuse the shared camera controls.
Solid display styles require geometry supplied by future modeling modules.
Face attachment, projected external geometry, constraint solving and region
extraction remain separate work. These source wires establish no manufacturing
geometry or large-model performance result.

## Persistence and evidence

[`SnapshotStore`](../../crates/multicad-app/src/files.rs) writes canonical JSON
through a temporary file and atomic replacement. Loading validates source before
adding another document, preserving existing sessions. The format contains source
only; native containers, migrations, recovery and derived payloads remain pending.

The source tests cover units, stable references, invalid drafts, transactional
rollback, save/load failures, sketch round trips and shared-plane translations.
Headless egui interaction tests feed pointer events to draw a rectangle and
move its gizmo. They also check inverse projection in both camera modes.
Tests live in the core integration tests and the UI component modules.

macOS window/render review is separate from these automated results. Windows and
Linux runtime behavior and low-power hardware performance require their own checks.
The proof laboratory in `spikes/native-foundation` remains a separate reference;
its private sessions and meshes never become production document authority.
