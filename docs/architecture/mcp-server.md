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
| Headless | `multicad --mcp [--project FILE.tcd] [--export-dir DIR] [--read-only]` | The `--project` file, opened or created on first save; otherwise an unsaved document | The client launches the process and talks over stdin/stdout |
| Window | `multicad --mcp --app [--socket PATH]` | The project open in the desktop window | The bridge connects to the window's same-user local socket |

A typical client configuration:

```json
{ "mcpServers": { "multicad": {
    "command": "/Applications/MultiCAD.app/Contents/MacOS/multicad",
    "args": ["--mcp", "--project", "/Users/me/parts/bracket.tcd",
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
  `--project` must end in `.tcd` (any case) or, for an existing project, the
  legacy `.multicad`, which is saved in place and never renamed.
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
| `sketch_add_text`, `sketch_edit_text`, `sketch_remove_text` | yes | Editable parametric text (multi-line string, font, size, anchor position, nine-point anchor, line alignment, rotation) whose exact glyph contours become closed regions; see [sketch-text.md](sketch-text.md) |
| `set_sketch_dimension` | yes | New literal or formula for an existing driving dimension |
| `inspect_sketch` | no | Solve state, degrees of freedom, conflicts, geometry, constraints, text elements with their contour curve IDs and block corners, measurements, and regions with index and area |
| `extrude` | yes | Extrude sketch regions, or planar model faces (`faces`: source feature and face ordinal from `reference_candidates`; exact boundary with holes, persistent face reference), by a signed distance as `new_body`, or `join`, `cut` or `intersect` a `target_part`; curved, missing, ambiguous or mixed inputs are refused unchanged |
| `evaluate_model` | no | Native evaluation of every final part and placed instance, plus reference diagnostics |
| `thread_profiles` | no | Offline family coverage, metric presets, source and manufacturing limits; see [threads](threads.md) |
| `inspect_thread` | no | Canonical thread source and resolved dimensions, including separate pitch and lead |
| `set_thread` | yes | Create or edit a native thread through the shared canonical transaction API; see [thread control](#thread-control) |
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
| `set_fence_type` | yes | Create a canonical construction or explicitly edit all its consuming runs; native preflight applies to used types |
| `inspect_fence_type` | no | Unit-bearing construction/formulas, stable recipe IDs and affected run IDs |
| `preview_fence` | no | Exact native create/edit preview with temporary member IDs, post positions and actual bay widths; no commit |
| `set_fence` | yes | Atomic run create/edit and optional explicit shared construction update, checked natively before commit or review |
| `inspect_fence` | no | Canonical run/context, source-resolved layout and named physical member IDs; see [fences](fences.md) |
| `inspect_architectural_symbol` | no | Embedded wall/fence definitions, public defaults, instance overrides and receiving context |
| `author_architectural_symbol` | yes | Capture a live canonical construction with selected typed public inputs; sample paths are excluded |
| `import_architectural_symbol` | yes | Embed a construction pin from a `.tcd` Library in the configured exchange folder |
| `draw_architectural_symbol`, `edit_architectural_symbol` | yes | Native-preflighted real wall/fence placement or public-input/context editing |
| `compare_architectural_symbol_update` | no | Explicit Library revision comparison and per-record repairs without changing the pin |
| `update_architectural_symbol` | yes | Atomic pin and instance replacement with repairs; see [architectural symbols](architectural-symbols.md) |
| `create_wall_type`, `edit_wall_type` | yes | Reusable wall types; a type edit rebuilds every wall that does not override the value (see [walls](walls.md)) |
| `create_opening_type` | yes | Reusable door (hinge, swing) or window type with width, height and sill |
| `create_wall`, `edit_wall` | yes | Wall along a path or closed loop with overrides and openings, evaluated natively before commit; an edit keeps vertical cutouts |
| `inspect_wall` | no | A wall's resolved segments, doors, windows and vertical cutouts with IDs, plus any placement diagnostic |
| `set_wall_opening` | yes | Add a door or window to one segment at an offset, or edit one by ID, with an existing or new opening type |
| `set_vertical_cutout` | yes | Add or edit a vertical cutout and choose what it cuts: a full-height wall cutout (through, or a chase from one face), holes in floor slabs (`slabs`, or `from_level`/`to_level` for a shaft) with a `plan` rectangle or the wall footprint, or both, in one transaction |
| `remove_wall_opening` | yes | Remove one door, window or vertical cutout by ID, including the cutout's slab holes |
| `remove_vertical_cutout` | yes | Remove a vertical cutout from its wall and every slab |
| `inspect_vertical_cutout` | no | What a cutout cuts (wall, slabs with level, span and hole corners), or every slab-cutting cutout |
| `set_compound_cutout` | yes | Create/edit one box union and every explicit wall/slab receiver after complete native preflight; see [compound cutouts](compound-cutouts.md) |
| `preview_compound_cutout` | no | Native-check the proposed complete source and all receivers without committing; returns proposed identities |
| `inspect_compound_cutout` | no | Complete canonical box and receiver definitions, or all compound cutouts |
| `remove_compound_cutout` | yes | Remove the source and generated receiver cuts atomically, refusing downstream dependencies that need repair |
| `create_panel_stock` | yes | Panel stock (thickness, sheet size, grain) on an existing or new Material (see [cabinetry](cabinetry.md)) |
| `set_carcass` | yes | Create or edit a parametric carcass; every panel is evaluated natively before commit |
| `create_panel` | yes | Rectangular panel from panel stock with grain, per-edge banding and oversize |
| `create_panel_stack` | yes | Face-to-face panel layers kept as their own Parts, optionally grouped |
| `set_panel_joint`, `delete_panel_joint` | yes | Butt (A or B through), miter or finger joints on panel pairs overlapping at a corner; each panel gets its own cut (also face-to-edge Domino-style loose tenons with a tenon Part) |
| `set_hardware_pattern` | yes | Reusable hardware hole pattern with per-material diameter and depth allowances |
| `set_machining`, `delete_machining` | yes | Groove, rabbet, pocket, drilling, System 32 shelf-pin rows or placed hardware pattern on one or more panels, on the top, bottom, inside or an edge face; suppression |
| `import_step` | yes | Import or update a STEP file's solids from the exchange folder (see [imports](imports.md)) |
| `import_kicad` | yes | Import or update a KiCad board, its holes and footprint components from the exchange folder |
| `schedule` | no | BOM, panel cut list, machining, member cut list or weld schedule as rows, optionally written as CSV into the export folder (see [schedules](schedules.md)) |
| `cut_list` | no | Finished and core panel sizes measured from the solids, stock fit and stack contact |
| `set_fdm_process` | yes | FDM process allowances (layer height, line width, minimum layers and lines, support gap, fit clearance) used to inspect aids (see [FDM aids](fdm-aids.md)) |
| `set_fdm_aid` | yes | Create or edit a breakaway tab, thin bridge or derived-height support as its own Part serving a finished Part |
| `remove_fdm_aid` | yes | Remove an aid feature and its Part; the target is untouched |
| `fdm_aid_report` | no | Aid bounds, contact with target and bridge partner, fit-clearance neighbours, allowance findings, placement findings (`contact_issues`) and a tab's exact contact, measured from the solids |
| `fdm_tab_contact` | no | Exact tab contact on a finished Part near a point: the outline point at its base and the outward angle |
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
| `set_sheet_forming` | yes | Replace a sheet part's flanges, hems, jogs, reliefs and cutouts; invalid bends and touching corners are refused with every finding, other touching or overlapping material by native evaluation |
| `sheet_edges` | no | Numbered base edges of one sheet feature, a guide per flange (bend angles, radius, leg, world leg ends, cross-section) and flange pre-check findings, for the applied or candidate flanges |
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

## Thread control

`thread_profiles` reports offline support and sourced presets. The
[thread reference](threads.md) owns family coverage, geometry and fit limits.
`inspect_thread` reads canonical source and resolved dimensions without
evaluating a solid. Use `evaluate_model` for native geometry and diagnostics.
The generated `tools/list` schema owns argument names, types and enum values;
the [tool implementation](../../crates/multicad-mcp/src/tools/threads.rs) defines
the operations below.

`set_thread` creates a new shaft or a receiving cut, or edits an existing
thread by feature ID. Creation requires a name; receiving cuts also require
the target Part. A new shaft refuses a target Part.
Edits retain the name, mode, Part and original predecessor.
They refuse renaming, retargeting or changing mode.

Omitted optional edit fields retain their canonical values. Creation uses
`ThreadParameters::default()` for omitted settings. Explicit `null` on an
optional field has the same effect as omission; it does not clear saved data.
All linear dimensions accept unit-bearing literals or formulas.
Custom flank angles require angle units, and radius taper requires ratio units.
A supplied Custom profile replaces the complete profile; omission retains it
on an edit, while creating a Custom thread requires the explicit profile.

Frame edits accept a principal plane or both explicit basis vectors.
Combining these choices, supplying only one vector, or supplying a
non-orthonormal basis fails. Omission retains the saved basis and origin.
Pitch, lead, handedness and starts retain their separate source meanings.

The tool prepares a canonical transaction, evaluates its candidate natively,
then commits or queues it under the owner's review policy.
Unsupported families, invalid dimensions, inadequate receiving walls,
native failures, cancellation and stale revisions leave the document unchanged.
Direct `export_step` also refuses a selected producer whose dependency history
contains a cosmetic thread. An unrelated cosmetic Part does not block export
of a physical Part.

## Apartment equipment control

`equipment_profiles`, `inspect_equipment` and `set_equipment` share the native equipment source API. Creation requires an explicit named floor identity and floor/wall/ceiling placement intent. Lengths carry units. Edits retain omitted fields and the original pinned family; a supplied replacement host repairs unresolved host intent. Inspection reports source and host errors separately. Service and door access boxes remain separate from physical geometry, and display visibility does not disable explicit clearance checks. Native preflight, cancellation and stale-revision guards apply before a single attributable commit. [Equipment source and drawing policy](equipment.md) defines mounting datums and output behavior. The generated tools list owns the exact schemas.

## Planting control

`set_plant` and `inspect_plant` share the Architecture planting transactions.
Generated `tools/list` owns their argument schemas; [plant tools](../../crates/multicad-mcp/src/tools/plants.rs)
own wire conversion. A create call chooses a generic tree or shrub, optional
unit-bearing height/spread and position, rotation in degrees, name, size basis,
plan visibility and optional label. It returns the canonical symbol ownership
`record` and outer `instance`. An edit supplies that record and the current
`expected_revision`; omitted controls retain their current values. An empty
label removes the label, while an omitted or null label retains it.

Every mutation native-preflights the prepared candidate before one commit.
Inspection returns the retained pin, linked plan sketch, resolved dimensions,
size meaning and current occurrence pose. Generic dimensions carry no botanical
or growth claim. Plan visibility is separate from physical section/elevation
geometry. [Planting source and drawing policy](plants.md) defines that behavior.

## Mechanical part control

`mechanical_part_profiles` reports generic geometry coverage and its limits.
`set_mechanical_part` creates or edits a washer, hex nut or bearing mounting
envelope through the canonical native-checked transaction. These tools do not
yet provide a verified standard-parts catalog. Generated `tools/list` owns the
argument schema; [mechanical part tools](../../crates/multicad-mcp/src/tools/mechanical_parts.rs)
implement the contract.

Creation requires a name and an explicit `recipe.kind`: `washer`, `hex_nut`
or `bearing_envelope`. Edits identify the generator `feature`, retain the
name/output Part and require the same recipe kind. Omitted optional recipe
values preserve the saved source; explicit `null` has the same effect.
Washer defaults are 3.2 mm bore, 7 mm outside and 0.5 mm thickness. Bearing
envelope defaults are 10 mm bore, 30 mm outside and 9 mm width. These numerical
defaults carry no catalog provenance or stock claim. Their frame accepts an
origin and either a principal plane or both explicit basis vectors.

Nut defaults are 10 mm across flats with a cosmetic M6/1 mm-pitch thread of
5.2 mm length. The nested `thread` accepts the shared thread recipe, including
Custom profiles, allowances and frame. `thread.length` is nut height and its
frame locates the whole body; there is no independent second height or axis.
It accepts no thread transaction controls such as `mode` or `target_part`.
The existing `set_thread` tool keeps its original flat arguments.

Cosmetic nut threads are refused for STEP, mesh and Print manufacturing output
until their representation is `modeled`. A bearing envelope contains only its
external annular mounting geometry; exported geometry does not include races,
rolling elements or seals. No fit, load or standards compliance is implied.

`inspect_mechanical_part` reads retained source, resolved dimensions and whether
they match an optional literal source reference. Placement and display-detail
choice do not alter dimensional attribution. A match establishes neither
trusted catalog membership nor stock availability; changed dimensions retain
the reference while becoming a derived/custom variant. Source inspection does
not evaluate native geometry; use `evaluate_model` for that.

## Gear control

`gear_profiles` reports supported families and primary geometry sources.
`inspect_gear` reads canonical source and resolved dimensions; it does not
evaluate the solid. Use `evaluate_model` for native geometry. The generated
`tools/list` schema owns argument names, types and enum values. The
[gear tools](../../crates/multicad-mcp/src/tools/gears.rs) implement these operations.

`set_gear` creates a gear with a required name or edits an existing feature
while retaining its Part identity. Edits refuse a name argument. Omitted
optional fields retain the saved recipe; creation uses
`GearParameters::default()`. Explicit `null` has the same effect as omission.
Linear dimensions, tooth count, angles and ratios accept unit-bearing literals
or formulas. A frame accepts a principal plane or both explicit basis vectors;
mixed or incomplete frame choices fail.

The module system selects the convention for both module and pressure angle.
Helix angle is a positive magnitude with separate handedness. Design backlash
is tangential tooth-thickness reduction **per gear** at its reference circle.
Fabrication allowance adds further tangential thinning; neither changes the
reference, base or tip circles. A pair's total backlash is not represented by
a single gear's setting.

Current geometry covers unshifted external spur, helical, continuous-center
herringbone and gapped double-helical gears with a through bore. Root fillets
are custom circular arcs, not a generated cutter trochoid. Internal rings,
racks, bevel gears, worms and nonzero profile shift are
unavailable. The conservative tooth-count policy uses the normal pressure angle
without virtual-tooth credit for a helix. No fit, accuracy grade, load rating
or cutter-generation certification follows from a successful preview.

The native profile uses a C1 cubic Hermite approximation to the analytic
involute, bounded by the active geometry policy's linear tolerance. Tip and
root arcs and circular fillets remain native circular curves. Helical flanks
follow a continuous twist, rather than a linear loft between two end profiles.

The tool prepares the canonical transaction, evaluates its candidate with the
native kernel, then commits or queues it under the owner's review policy.
Unsupported families, invalid units or dimensions, inadequate bore walls,
native failures, cancellation and stale revisions leave the document unchanged.

`set_gear_pair` creates two named gear Parts, a reusable Assembly, its internal
relationships and one placed occurrence in a single transaction. Creation
requires a name. The first member defaults to 20 teeth. The second inherits
the first recipe with 30 teeth and opposite helix hand, then applies explicit
second-member overrides. Spur members use hand `none`.

An edit retains omitted fields independently for each member. Supplying both
recipes changes them atomically, so a shared module change can keep the pair
compatible. A drive-only edit preserves the gear definitions exactly. Explicit
`null` has the same effect as omission. Edits retain names and output identities.
`inspect_gear_pair` reads source and derived center distance, phases and frames;
`evaluate_model` reports actual native pair checks and physical leaf paths.

Pair origin and axes control layout within the Assembly definition. Member
frames describe their local Part definitions; derived anchors maintain mating
placement when those local frames change. The returned occurrence supports
normal component Move/Copy. Copies share the definition's internal drive angle
and keep independent outer placements.

Pairs require matching external family, transverse module and pressure angle,
face width and center gap, with equal helix magnitude and opposite hands where
applicable. Their current Part heads must remain the gear generators. Native
evaluation checks the current pose and nine phases over one pinion tooth cycle.
Unchanged geometry can reuse a completed cycle check; the current pose is always
checked. These samples do not establish continuous collision-free motion, load
capacity or manufacturing fit.

Direct `export_step` accepts an Assembly's definition-only gear Part as well as
a standalone Part. It exports that Part's definition coordinates. Export sets
with occurrence paths select posed assembly members. Definition-only sources
do not become duplicate physical outputs, and consumed Parts remain unavailable
through direct Part export.

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
