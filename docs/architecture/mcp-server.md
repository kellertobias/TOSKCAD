# Local MCP server

MultiCAD exposes local application control through the
[Model Context Protocol](https://modelcontextprotocol.io/). The
[`multicad-mcp`](../../crates/multicad-mcp/src/lib.rs) crate uses rmcp 3.8.0
and calls the same Session, solver and native model functions as the desktop
UI. It does not click the UI, keep a second modeling engine, or embed an AI
model or chat.

## Connections

| Mode | Start | Document | Transport |
| --- | --- | --- | --- |
| Headless | `multicad --mcp [--project FILE.multicad] [--export-dir DIR] [--read-only]` | The `--project` file, opened or created on first save; otherwise an unsaved document | The client launches the process and talks over stdin/stdout |
| Window | `multicad --mcp --app [--socket PATH]` | The project open in the desktop window | The bridge connects to the window's same-user local socket |

A typical client configuration:

```json
{ "mcpServers": { "multicad": {
    "command": "/Applications/MultiCAD.app/Contents/MacOS/multicad",
    "args": ["--mcp", "--project", "/Users/me/parts/bracket.multicad",
             "--export-dir", "/Users/me/parts/out"] } } }
```

Use `"args": ["--mcp", "--app"]` to control the open window instead.

Neither mode opens a network port, and remote control is not offered.

- **Window listener.** It starts only after **Allow local MCP connections** is
  switched on in **Command and MCP history**. That window shows the endpoint,
  the connected clients and the project each client controls.
- **Socket location.** The socket lives in `multicad-mcp-<uid>` under
  `$XDG_RUNTIME_DIR` or the per-user temporary directory. The directory must be
  owned by the user with mode 0700, and the socket has mode 0600.
- **Turning it off.** Switching it off closes connections and refuses calls that
  are still queued.
- **Windows.** Window control is reported as unavailable on Windows. Headless
  mode works on every platform.

## Scope

`document_info` reports the scope, and no tool can widen it.

- **Project.** `save_project` writes only the headless `--project` file. It uses
  the desktop's atomic, conflict-checked project store. A file changed by
  another writer is not replaced. In window mode, saving stays with the window.
- **Exports.** `export_step` writes only into `--export-dir`, using a bare
  `.step` or `.stp` file name. It writes to a hidden partial file, reads the file
  back, then renames it. It fails without an export folder. In window mode,
  exports are unavailable.
- **Read-only.** `--read-only` refuses every mutating tool.
- **Window binding.** A window connection controls the project that was active
  at its first call. It does not follow later tab switches. When that project
  closes, its calls fail with `unavailable`.
- **Window drafts.** While the window has an unapplied draft, such as a
  parameter row, sketch edit, page draft or export draft, commits, Undo, Redo
  and saves fail with `busy`. Inspection stays available.

## Review and activity

Each document keeps a
[`ControlLedger`](../../crates/multicad-core/src/control.rs) with an audit log
and a review queue.

**Review policy**

- Headless clients are trusted: their changes commit directly.
- Window clients are reviewed unless they are trusted for the window session.
  Trust is set with **Apply without review** next to each connected client.

**What a reviewed change does**

1. The tool prepares and checks the full candidate, then queues it. It returns
   `status: "pending_review"`, a `review` ID and the unchanged base
   `revision`; the document does not change. Undo and Redo are queued the same
   way.
2. The window's **Pending review** section shows each change: client, tool,
   arguments, base revision, affected entities, and whether it is stale.
3. **Preview in viewport** evaluates the candidate with the native kernel and
   shows it in Model without committing.
4. **Accept** applies it as one undoable transaction attributed to the client.
   **Reject** discards it.
5. A change whose base revision moved becomes `stale`: it is closed and never
   rebased, and the client must prepare it again.

Applied calls carry `status: "applied"`. `review_status` reports a review as
`pending` (with `stale`), `applied` with its revision, `rejected`, `stale` or
`failed`.

**Activity**

`activity` returns the audit, newest first. Each entry records:

- the outcome: `inspected`, `previewed`, `applied`, `rejected`, `cancelled`,
  `failed`, `undone`, `redone` or `stale`;
- actor and channel, tool and arguments;
- base and resulting revision, and affected IDs;
- the review ID, if any.

Window edits, Undo and Redo on any channel are imported in revision order.
Moving history backwards adds entries and never removes them. The log keeps
10,000 entries and reports how many older ones were dropped. The queue holds
64 changes.

Command and MCP history shows Connections (permissions), Pending review and
Activity as separate sections. There is no chat or prompt composer.

## Transactions

Every mutating tool requires `expected_revision`. A request based on another
revision fails with `stale_revision` and the live `current_revision`.

Each success commits exactly one transaction. Its actor is `mcp:<client name>`
on the `mcp` channel, and Undo and Redo apply to it like any UI edit.

Sketch edits are solved before they commit. A failed solve returns
`solve_failed` with the conflicting constraints. Extrusions are also evaluated
with the native kernel before they commit, and any evaluation error is returned.

Native work runs outside the document owner and can be interrupted. A client's
`notifications/cancelled` stops it at the next check. A cancelled call never
commits; internally it reports `cancelled`, and rmcp suppresses the response.

## Tools

The tool names and input schemas below are the compatibility contract. Each
schema is generated from the Rust argument types with schemars and returned by
`tools/list`. Unknown fields are rejected.

- **Annotations.** Read-only tools carry `readOnlyHint: true`. Every tool that
  needs `expected_revision` is mutating.
- **Results.** Each result carries `structuredContent`, with the same JSON as
  text.
- **Errors.** A failure sets `isError` and carries
  `{code, message, involved?, current_revision?}`.

| Tool | Mutates | Purpose |
| --- | --- | --- |
| `document_info` | no | Name, identity, revision, dirty state, undo/redo, entity counts, units and scope |
| `list_entities` | no | IDs, names, kinds and references, optionally filtered by kind |
| `get_entity` | no | The complete canonical source record |
| `list_parameters` | no | Expression, unit, evaluated value, role and exposure of project parameters; symbol-private parameters are omitted |
| `set_parameter` | yes | Create a parameter, or edit the one with that name |
| `create_sketch` | yes | Empty sketch on the `xy`, `xz` or `yz` origin plane |
| `sketch_add_rectangle` | yes | Fully constrained rectangle with a fixed corner and driving width and height |
| `sketch_add_circle` | yes | Circle with a fixed center and a driving diameter |
| `sketch_add_text`, `sketch_edit_text`, `sketch_remove_text` | yes | Editable parametric text (string, cap height, origin, rotation, alignment) whose exact glyph contours become closed regions; see [sketch-text.md](sketch-text.md) |
| `set_sketch_dimension` | yes | New literal or formula for an existing driving dimension |
| `inspect_sketch` | no | Solve state, degrees of freedom, conflicts, geometry, constraints, text elements with their contour curve IDs, measurements, and regions with index and area |
| `extrude` | yes | Extrude sketch regions, or planar model faces (`faces`: source feature and face ordinal from `reference_candidates`; exact boundary with holes, persistent face reference), by a signed distance as `new_body`, or `join`, `cut` or `intersect` a `target_part`; curved, missing, ambiguous or mixed inputs are refused unchanged |
| `evaluate_model` | no | Native evaluation of every final part and placed instance, plus reference diagnostics |
| `undo`, `redo` | yes | The most recent transaction on any channel |
| `history` | no | Recent activity with actor, channel, label, kind and changed IDs |
| `review_status` | no | State of a queued change |
| `activity` | no | Append-only control audit and the review queue |
| `save_project` | yes | Save to the scoped project file |
| `export_step` | no | STEP of one final part into the export folder, with readback volume and SHA-256 |
| `import_sketch_artwork` | yes | DXF or SVG text into an existing or new sketch, with provenance (see [sketch import](sketch-import.md)) |
| `set_level` | yes | Create or edit a building level; hosted slabs, stairs and walls rebuild (see [levels and stairs](levels.md)) |
| `create_slab` | yes | Slab or pedestal from a closed outline on a level |
| `set_stair` | yes | Straight, quarter-turn or half-turn stair between levels, with geometric riser/going notes |
| `create_envelope` | yes | Service envelope around a placed Part or Instance |
| `check_clearances` | no | Overlap of every service envelope with final model solids |
| `set_drawing_guide` | yes | Floor plan, section or elevation guide, optionally generating or updating its linked drawing views (see [drawing guides](drawing-guides.md)) |
| `create_wall_type`, `edit_wall_type` | yes | Reusable wall types; a type edit rebuilds every wall that does not override the value (see [walls](walls.md)) |
| `create_opening_type` | yes | Reusable door (hinge, swing) or window type with width, height and sill |
| `create_wall`, `edit_wall` | yes | Wall along a path or closed loop with overrides and openings, evaluated natively before commit; an edit keeps vertical cutouts |
| `inspect_wall` | no | A wall's resolved segments, doors, windows and vertical cutouts with IDs, plus any placement diagnostic |
| `set_wall_opening` | yes | Add a door or window to one segment at an offset, or edit one by ID, with an existing or new opening type |
| `set_vertical_cutout` | yes | Add or edit a vertical cutout and choose what it cuts: a full-height wall cutout (through, or a chase from one face), holes in floor slabs (`slabs`, or `from_level`/`to_level` for a shaft) with a `plan` rectangle or the wall footprint, or both, in one transaction |
| `remove_wall_opening` | yes | Remove one door, window or vertical cutout by ID, including the cutout's slab holes |
| `remove_vertical_cutout` | yes | Remove a vertical cutout from its wall and every slab |
| `inspect_vertical_cutout` | no | What a cutout cuts (wall, slabs with level, span and hole corners), or every slab-cutting cutout |
| `create_panel_stock` | yes | Panel stock (thickness, sheet size, grain) on an existing or new Material (see [cabinetry](cabinetry.md)) |
| `set_carcass` | yes | Create or edit a parametric carcass; every panel is evaluated natively before commit |
| `create_panel` | yes | Rectangular panel from panel stock with grain, per-edge banding and oversize |
| `create_panel_stack` | yes | Face-to-face panel layers kept as their own Parts, optionally grouped |
| `set_panel_joint`, `delete_panel_joint` | yes | Butt (A or B through), miter or finger joints on panel pairs overlapping at a corner; each panel gets its own cut |
| `set_hardware_pattern` | yes | Reusable hardware hole pattern with per-material diameter and depth allowances |
| `set_machining`, `delete_machining` | yes | Groove, rabbet, pocket, drilling or placed hardware pattern on one or more panels |
| `import_step` | yes | Import or update a STEP file's solids from the exchange folder (see [imports](imports.md)) |
| `import_kicad` | yes | Import or update a KiCad board, its holes and footprint components from the exchange folder |
| `schedule` | no | BOM, panel cut list, machining, member cut list or weld schedule as rows, optionally written as CSV into the export folder (see [schedules](schedules.md)) |
| `cut_list` | no | Finished and core panel sizes measured from the solids, stock fit and stack contact |
| `set_fdm_process` | yes | FDM process allowances (layer height, line width, minimum layers and lines, support gap, fit clearance) used to inspect aids (see [FDM aids](fdm-aids.md)) |
| `set_fdm_aid` | yes | Create or edit a breakaway tab, thin bridge or derived-height support as its own Part serving a finished Part |
| `remove_fdm_aid` | yes | Remove an aid feature and its Part; the target is untouched |
| `fdm_aid_report` | no | Aid bounds, contact with target and bridge partner, fit-clearance neighbours and allowance findings measured from the solids |
| `set_render_scene` | yes | Render scene stored in the document: camera, background, sun and point lights, display style, Part scope and aid exclusion (see [render handoff](render-handoff.md)) |
| `create_appearance` | yes | Render appearance definition (base color, roughness, metallic), separate from Materials, stock and hatch |
| `assign_appearance` | yes | Give one Part an appearance, or clear it, keeping its Material, stock and hatch |
| `blender_handoff` | no | Write NAME.glb with the scene camera and lights and NAME.render.json into the export folder |
| `create_profile_stock` | yes | Rectangular, round or angle profile stock on an existing or new Material (see [weldments](weldments.md)) |
| `create_members` | yes | One member Part per sketch line or point pair, with section anchor, rotation and end extensions |
| `edit_member` | yes | Member stock, point path, anchor, rotation or extensions, keeping its Part |
| `connect_members` | yes | Miter, butt or cope connection between two members, with gap; cuts derived from both members |
| `weldment_cut_list` | no | Member lengths, sections, orientation and both end treatments measured from the solids |
| `set_weld` | yes | Weld on a member connection: mark, symbol, size, process and notes |
| `set_weld_sequence` | yes | Authored weld order, created or reordered without losing references |
| `weld_schedule` | no | Welds in sequence order with joint positions, and grouped member cuts with quantities |
| `place_weld_plan` | yes | Weld symbols at projected joints and a weld sequence table on a drawing page |
| `create_sheet_stock` | yes | Sheet-metal stock (thickness, sheet size, default inside radius and K-factor) on an existing or new Material (see [sheet metal](sheet-metal.md)) |
| `set_sheet_part` | yes | Create or edit a sheet Part with a rectangle or sketch-region base and optional radius and K-factor overrides, evaluated natively before commit |
| `set_sheet_forming` | yes | Replace a sheet part's flanges, hems, jogs, reliefs and cutouts; touching or overlapping material is refused |
| `convert_to_sheet` | yes | Analyze a Part's solid as a constant-thickness plate and create a sheet Part from its stationary face, or report why it cannot |
| `sheet_metal` | no | Sheet Parts measured from their solids, bend inputs with their sources and the developed flat pattern |
| `unfold_sheet` | no | Developed pattern of one sheet Part: cut loops, bend lines, drill points, millimeter units and scoped diagnostics |
| `set_sheet_fasteners` | yes | Replace a sheet part's mounting patterns (screws, standoffs, catalogue inserts); holes are cut and hardware Parts kept in step |
| `sheet_hardware` | no | Attached hardware Parts with designation, source, measured quantity and compatibility findings |
| `export_sheet_dxf` | no | Developed blank of one sheet Part as a 1:1 DXF in the export folder, with CUT, CUT_INNER, BEND_UP, BEND_DOWN, DRILL and LABEL layers |
| `reference_candidates` | no | Current exact faces, edges and vertices of a source feature, with element keys and analytic geometry |
| `preview_reference_repairs` | no | Stage several reference repair rows on the current source and report rows, blockers and `can_apply` (see [reference repair sets](reference-repair-sets.md)) |
| `apply_reference_repairs` | yes | The same rows, committed as one transaction only when the complete candidate passes |

`evaluate_model` reports each part's producing feature, solid count, volume
and bounds, and each placed instance's component, solid count, volume and
bounds.

### Values and units

- **Lengths.** A length is `{"value": 40, "unit": "mm"}` or
  `{"expression": "=$width/2", "unit": "mm"}`, with exactly one of `value` or
  `expression`. The unit is always required: `mm`, `cm`, `m`, `in` or `ft`.
- **Parameters.** A parameter takes `"40"` or `"=$width*2"` and any declared
  parameter unit.
- **Coordinates.** Sketch coordinates (`origin_mm`, `center_mm`) are
  millimeters in the sketch plane.
- **Results.** Results report millimeters, square millimeters and cubic
  millimeters, plus parameter values in their declared unit.
- **IDs.** Persistent entity IDs are UUID strings.
- **Regions.** Region indices come from `inspect_sketch` and apply only at the
  revision it reported.

### Error codes

| Code | Meaning |
| --- | --- |
| `invalid_request` | Malformed arguments, wrong unit kind, missing target, or a region index that does not exist |
| `stale_revision` | `expected_revision` is not current; `current_revision` is set |
| `stale_preview`, `invalid_document`, `invalid_parameter`, `no_history`, `revision_limit` | Session errors |
| `invalid_sketch`, `solve_failed` | The sketch cannot be read, or the edit does not solve |
| `model_failed` | Native evaluation failed; `involved` names the entity |
| `not_found` | The ID does not exist or has the wrong kind |
| `read_only`, `out_of_scope`, `busy`, `unavailable` | Scope or owner refusals; `busy` also when 64 changes await review |
| `save_failed`, `export_failed`, `import_failed` | File operations |
| `cancelled` | Cancelled before commit; nothing changed |
| `unknown_tool` | The tool name is not in the catalog |

## Architecture

1. **Owner boundary.** [`control.rs`](../../crates/multicad-core/src/control.rs)
   in core defines `SessionOwner` and `ControlJob`, an owner-thread job.
2. **Tool layer.** The [tools](../../crates/multicad-mcp/src/tools.rs) submit
   short jobs to read or commit. Each job runs to completion on the owner
   before the next one starts. Solving, region extraction and native
   compilation work on snapshots.
3. **Headless owner.**
   [`Headless`](../../crates/multicad-mcp/src/headless.rs) owns a Session
   behind a mutex.
4. **Window owner.** The window's
   [`external_control`](../../crates/multicad-ui/src/application/external_control.rs)
   runs jobs on the UI thread against the bound project, up to 32 jobs per
   frame.
5. **Transports.** The [local socket](../../crates/multicad-mcp/src/local_socket.rs)
   and [app control](../../crates/multicad-app/src/app_control.rs) connect the
   window. The UI crate has no protocol dependency.
6. **Clean stdout.** Before serving stdio, the server calls
   `multicad_kernel::route_native_output_to_stderr()`. That sends OCCT messages
   and C++ `std::cout` to stderr, at warning level, so native diagnostics never
   corrupt the JSON-RPC stream.

## Not yet covered

- **More tools.** There are no tools yet for revolve, fillet and chamfer,
  patterns, assemblies and joints, drawings, symbols, materials, or exports
  other than STEP. They will use the same tool layer as their shared commands
  become available.
- **Review limits.**
  - Each queued change is its own transaction; several calls cannot be merged
    into one group.
  - Undo and Redo have no geometry preview.
  - Trust lasts for the window session and is not saved.
- **No push updates.** There are no MCP resources, prompts or progress
  notifications. Clients poll `document_info` for the revision.
