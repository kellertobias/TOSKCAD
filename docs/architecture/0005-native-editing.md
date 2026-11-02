# Native editing and revised workspace layout

This document owns the native editor contract and its implementation limits.
The canonical entity model remains described in [0004](0004-canonical-document.md).

## Product decision, revised 2026-11-01

The native modes are Project, Sketch, Model, Drawings and Export. CAD-220
renames Parameters to Project and replaces its action ribbon with left-side
icon-and-label actions: Apply and Rebuild, Discard Edit, Preview, Undo and Redo.
Project has no ribbon or reserved ribbon band. The canonical parameter table
remains the primary content, with one 300 px file-metadata sidebar on the right.
The left actions also retain Library, Measure and MCP history access.
Export contains Common, Print and Render. Materials belong to Model.
These choices supersede the corresponding layout rules in
[0001](0001-project-boundaries.md).

The 2026-11-01 product-owner revision supersedes the 2026-10-27 Parameters
action-ribbon requirement. Draft actions stay visible beside the table; a
locally scrollable table preserves editable columns at the 1100 × 700 native
minimum. The same existing transactional handlers and shortcut scopes remain.
The finder presents Project first for its exact name; `params` and `parameters`
remain aliases, and `project-mode` is explicit because `project` already names
the retained project-reference command. A parameter Preview validates the draft;
it does not replace the file preview image.

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

Parameter drafts are bound to their original Session and revision. Filtering uses
the visible row list before virtualization, while new rows and a matched row being
renamed remain editable until Apply. Unit-only input is a draft. Clearing an
existing row requires completion or explicit deletion. Evaluated cells show only
current valid values; invalid and stale drafts show no evaluated number. Apply
requires a current checked parameter preview. Source-valid edits may still leave
dependent geometry in a repairable failed state; native consumers reject stale
results. The [workflow verification](../verification/native-parameter-workflow-2026-10-28.md)
records actual native interaction and saved-source checks.

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

Project files store no camera. When a document's evaluated model first has
world bounds and the viewport is laid out, the viewer frames the whole model
once ([`initial_fit`](../../crates/multicad-ui/src/initial_fit.rs)). Until the
user moves the camera it reframes when the viewport is resized, so a window
restored at startup still shows the whole model; after any orbit, pan, zoom,
Home or Fit it never overrides the view. An empty or failed model keeps the
default view. **Home** returns to the standard orientation and frames the
whole model, ignoring the selection; **Fit** keeps the current orientation and
frames the selection, or everything when nothing is selected.
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

## Shell refinement, 2026-10-27

The product owner retains black ribbon tools and a full-width black command input,
with status overlaid at the right. Information is gray; errors are red. Long focused
commands use the whole input without status covering their text. The prompt opens
an upward drawer containing manual commands, their results and document transactions.
History is bounded to the current application session; source snapshots do not store it.

The shared blue theme applies to panels and dialogs while mode bands keep their
individual colors. Parameter content has wider side gutters and a top inset.
Outlined fields identify editable inputs. Row controls use stacked arrows and a
trash icon. The centered tool input opens a bounded, scrollable results popup.
A compact transform icon opens coordinate choices on right-click.
[`theme`](../../crates/multicad-ui/src/theme.rs) owns the surfaces and tooltip delay.

New parameter input distinguishes literal values from formulas beginning with `=`.
References use `$name`, for example `=600 - $height`. Unsuffixed additive constants
in explicit formulas use the parameter's declared unit. Comparison limits require
compatible units, and comparison results use `ratio`: `1` for true, `0` for false.
These comparisons evaluate values; they do not solve geometric inequalities.
Legacy stored formulas remain readable and display with the explicit notation.
The typed `override_exposed` operation replaces an exposed default formula with
a supplied literal, preserving its ID and exposure while clearing old dependencies.

The audited [native window adapter](../../crates/multicad-ui/src/native_window.rs)
borrows the live frame's AppKit handle and positions the standard macOS buttons.
Its Objective-C++ implementation stays on the main thread and retains no window
pointer. Other platforms do not compile or invoke that adapter.

## Scoped keyboard interaction

[`shortcuts`](../../crates/multicad-ui/src/shortcuts.rs) owns declarative bindings,
collision validation and input dispatch. Workspace bindings override sketch-editing,
mode, viewport and global bindings, in that order. Equal-scope collisions are rejected.
Command shortcuts use the same enabled-state checks and transactions as ribbon tools;
unavailable modeling modules remain disabled. Tooltips reflect active bindings.

| Keys | Context | Action |
| --- | --- | --- |
| W, A, S, D | Viewport, outside text entry | Pan up, left, down, right |
| Tab / Shift+Tab | Outside text entry and dialogs | Next / previous ribbon workspace, wrapping |
| E / Q / R | Project, outside text entry | Apply and rebuild / discard / preview |
| Cmd/Ctrl+Enter | Project, including text entry | Apply and rebuild |
| Q | Sketch entry / editing | New sketch / finish sketch |
| E / R | Sketch editing | Line / rectangle |
| Delete | Sketch editing | Delete selected geometry |
| E / R | Model | Extrude / revolve when their modules are available |
| F / Shift+F | Viewport | Fit (current orientation) / Home (standard orientation, whole model) |
| V / X / Escape | Viewport | Projection / transform gizmo / cancel interaction |
| Cmd/Ctrl+F | Application | Focus tool finder |
| Backtick | Outside text entry | Focus command input |
| Cmd/Ctrl+S / N / O | Application | Save / new document / open |
| Cmd/Ctrl+Z / Cmd/Ctrl+Shift+Z | Outside text entry | Document undo / redo |
| F1 | Outside text entry | Shortcut list |

Text fields retain plain typing, local Undo and Tab navigation. Dialogs and menus
block application tool dispatch. WASD movement starts from an eligible unmodified
press, stops on text entry or loss of focus, and never starts from a Save shortcut.
Held navigation uses elapsed frame time, normalizes diagonals and caps long-frame
movement. Discrete tool actions ignore key repeats. Middle-mouse pan remains active.

The shortcut list is available from File and the command input. Modules can register
scoped bindings through the map API; a preferences editor and persistent custom
bindings remain future work. Ribbon tiles retain their fixed width, provide room
for two-line labels and have a separate bottom inset. The finder centers its text
and includes an original vector hammer icon.
