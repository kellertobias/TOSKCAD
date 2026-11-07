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

## One-shot command line

`multicad --command REQUEST.json --project FILE.tcd [--read-only] [--save]`
runs one public tool through the same dispatcher, revision guards and native
candidate validation as MCP. It does not start a window or socket. The request
file is a JSON object with exactly two fields: `tool` (string, a name from
`tools/list`) and `arguments` (the tool's argument object). For example:

```json
{"tool":"inspect_screw_connection","arguments":{"connection":"CONNECTION_UUID"}}
```

The command prints one JSON receipt. Success has `ok: true`, the tool's
`result`, and `persistence`: `read_only`, `memory_only`, or `saved`. The optional
`save` receipt names the written project. Exit status is 0 on success, 1 for a
command or storage failure, and 2 for invalid command-line options.

A successful mutation remains in the private process unless `--save` is
present. File persistence is disabled without that flag, including an envelope
that invokes `save_project` directly. With `--save`, successful mutating tools
are followed by `save_project` at the resulting revision, using the desktop's
atomic, conflict-checked store. A failed dispatch never triggers this save.
`--read-only` refuses mutating tools and conflicts with `--save`. Export folders
and desktop connection flags are unavailable in this one-shot route.

Each invocation opens a fresh Session at revision 0 from the saved canonical
file. A mutation's returned revision belongs to that process; the next
invocation uses revision 0 again. Saving retains the document, not the process's
Undo/Redo history. Use the long-lived MCP server when several commands must
share one revision sequence and history.

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
| `import_symbol` | yes | Embed one pinned definition from a scoped `.tcd` Library |
| `place_symbol` | yes | Native-preflighted placement with exposed parameter overrides |
| `edit_symbol_instance` | yes | Replace exposed overrides while retaining the pin and identities |
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
| `thread_profiles` | no | Offline family coverage, metric/UNC/UNF/BSW/BSF/BSPP presets, fit limits, primary sources and manufacturing limits; see [threads](threads.md) |
| `inspect_thread` | no | Canonical thread source, separate basic/fitted pitch diameters, applicable fit limits, pitch and lead |
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
| `set_wood_box` | yes | Create/edit one canonical optional-face box and all panels after native preflight; see [wood boxes](cabinetry.md#wood-boxes) |
| `preview_wood_box` | no | Native-check a proposed box without committing; returns stable identities and outside/inner/actual bounds |
| `inspect_wood_box` | no | Read one recipe or list all with resolved dimensions and panel identities |
| `remove_wood_box` | yes | Remove the complete recipe/panels atomically; referenced faces require repair |
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
UNC and UNF are separate `unc` and `unf` families. The legacy ambiguous
`unified` family remains unavailable. The catalog exposes each supported
inch diameter/TPI combination, its millimeter dimensions, external `unified_2a`
and internal `unified_2b` limits, and the versioned primary source URLs.
Limit ranges have `min_mm` and `max_mm` endpoints; `null` means that endpoint
is unspecified. An external minor-diameter reference is informational and
must not become an invented acceptance limit.

Metric `nominal_basic` remains unchanged. `metric_6g_external` and
`metric_6h_internal` select the bounded historical M3/M6 fitted subset.
`thread_profiles.metric_fit_presets` provides source diameter limits and
exclusive-lower/inclusive-upper normal-engagement bounds; the
[fit reference](threads.md#metric-fit-classes) owns edition and equations.
`inspect_thread.resolved.metric_fit` retains construction targets, basic pitch
diameter and external circular-root radius; `resolved.fit` contains actual
diameters after fabrication allowances and unchanged source limits.
Unsupported classes, sizes, modes, multiple starts and out-of-class allowances
fail before commit. Nested mechanical-part inspection exposes the same
`metric_fit` receipt; existing catalog definitions are not upgraded silently.

The optional `metric_fit_engagement` recipe accepts
`{"kind":"intended_length","length":{"value":3,"unit":"mm"}}` or
`{"kind":"normal_group_assumed"}`. Intended length also accepts a bound
expression. Physical thread length stays independent; an explicit group
assumption supplies no numeric or verified engagement. Omission/null preserves
an edit's saved intent. `clear_metric_fit_engagement:true` restores the legacy
complete-profile rule and cannot accompany a replacement. Thread and nested
hardware inspection report `resolved.metric_fit_engagement` with its basis,
intended length (null for assumed), complete-profile interval and normal limits.
This does not prove assembled overlap, strength or blind clearance. See the
[fit reference](threads.md#metric-fit-classes) for source and reader capability.

Standalone metric `end_treatments` is an explicit replacement object with
`start` and `end` entries. Start is `"flat_cut"` or
`{"chamfer45":{"radial_depth":{"value":0.2,"unit":"mm"}}}`. End is
`"flat_cut"` or `{"cylindrical_relief":{"length":{"value":0.5,"unit":"mm"},
"radial_depth":{"value":0.1,"unit":"mm"}}}`. Quantities can retain parameter
expressions. Omission or null retains an edit's saved object;
`clear_end_treatments:true` removes it and cannot accompany a replacement.
`resolved.end_treatments` reports consumed/full-profile intervals and the
complete maximum cutter radius used for receiving-wall validation.
Authored total length is unchanged. These named custom operations are neither
DIN 76 shoulder relief nor natural helical runout; nested hardware endpoint
treatments are explicitly unavailable. The
[endpoint reference](threads.md#custom-metric-endpoint-geometry) owns geometry
and material constraints.

Rounded series are explicit `bsw`, `bsf` and `bspp`; the ambiguous legacy
`whitworth` remains unavailable. `rounded_presets` lists six supported rows,
their physical basic diameter, TPI/pitch, external/internal limits and source
URLs. BSW/BSF use `whitworth_medium_external` or
`whitworth_medium_internal`; BSPP uses `bspp_class_b_external` or
`bspp_internal`. These require one start and zero axial allowance.
BSW/BSF engagement is bounded to 15 pitches. BSPP's `engagement_limit_mm`
is null because its source table supplies no bound; native limits still apply.
G pipe labels are not physical inch diameters. BSPP sealing is outside the
thread, and the selected table does not establish a pressure-sealing or full
ISO-compliance result.

BSPT uses explicit saved `pipe` intent with form `r` or `rc`, supported nominal
size and the pinned gauge convention. Its `bspt_nominal` fit requires one
right-hand start and zero axial allowance. The `thread_profiles` response
exposes `bspt_presets` and `bspt_support`; the [pipe reference](threads.md#tapered-pipe-construction)
owns source coverage, base-plane dimensions and useful-length rules.
Creation requires a base-plane major diameter and pitch consistent with that
intent. Omitted or null pipe intent on an edit retains its saved value;
changing to another family clears it. A pipe object on another family is refused.
`inspect_thread.resolved.pipe` and nested hardware inspection expose the same
gauge receipt, source and allowance displacement in turns.

NPT uses `family: "npt"`, `fit: "npt_historical_midpoint"` and an explicit
`npt` object: `form` is `external` or `internal`, `size` is `1/8` or `1/4`,
`gauge` is `nbs1959_plain_face` and `truncation` is
`midpoint_cone_parallel`. Creation requires the corresponding base major
diameter, pitch and supported length from the canonical registry.
`thread_profiles.npt_presets` retains the printed source rows;
`npt_support` states construction limits. Direct and nested inspection return
the same `resolved.npt` receipt, including actual gauge travel and actual
stock intersection separately from source basic L5. Omitted or null intent
retains an existing edit; changing family clears it. An NPT object on another
family is refused. The [NPT reference](threads.md#npt-construction-and-source-limits)
defines the bounded historical conventions and unsupported combinations.

Buttress uses `family: "buttress"`, `fit: "nominal_basic"` and an explicit
`buttress` object with `series: "bis_4696_nominal_2004"` and
`load_flank_side: "negative_axis_side"` or `"positive_axis_side"`.
The schema is generated from the [typed request](../../crates/multicad-mcp/src/tools/threads.rs).
The side selects the external tooth's 3-degree face and remains independent
of hand. `thread_profiles.buttress_presets` and `buttress_support` expose
bounded source coverage. Direct and nested inspection return the same
mode-specific `resolved.buttress` receipt. Omitted or null intent preserves
it on same-family edits; changing family clears it. Supplying it for another
family, omitting it on new Buttress creation, unsupported sizes/starts/fit
or nonzero axial fabrication allowance fails before commit. See the
[buttress construction](threads.md#metric-buttress-profile) for phase and source limits.

Round uses `family: "round"`, `fit: "nominal_basic"`, a listed metric
nominal diameter and reciprocal-inch pitch. `thread_profiles.round_presets`
and `round_support` declare size, hand/start and allowance bounds. No extra
authored intent object is needed. Direct and nested inspection expose the
same mode-specific `resolved.round` receipt, distinguishing material from
positive-cutter circle radii. Omitted fields retain the saved canonical
values on edits. Unsupported rows, starts, classes and nonzero axial
fabrication allowance fail before commit. The
[Round reference](threads.md#round-profiles) owns source editions, exact
construction, nominal pitch-diameter semantics and limits.

Trapezoidal and Acme use their distinct `trapezoidal` and `acme` families
with existing `fit: "nominal_basic"`, a listed nominal diameter/pitch, hand,
starts and fabrication allowances. `thread_profiles.lead_screw_presets`
returns the four supported source rows; `lead_screw_support` declares the
engineering start/hand coverage and unavailable tolerance classes. There is
no extra authored intent object. `inspect_thread.resolved.lead_screw` and
nested hardware inspection expose the same row and mode-specific construction
receipt. The canonical nominal major diameter remains the external basic
size; an internal cutter uses the source internal major diameter. Both
nominal pitch diameter and post-allowance actual pitch diameter are reported.
[Lead-screw conventions](threads.md#trapezoidal-and-acme-lead-screw-profiles)
own units, sharp corners, source clearances and fit limitations.

`inspect_thread` reads canonical source and resolved dimensions without
evaluating a solid. Use `evaluate_model` for native geometry and diagnostics.
Its `nominal_pitch_diameter_mm` remains the basic-profile value.
`actual_pitch_diameter_mm` and `fit` describe the intended fitted construction
after allowances, with class, designation, provenance and applicable diameter
limits. They are not measurements or gage/physical-fit certification.
`resolved.profile` reports depth, flat crest width, flank angles and
`rounding_radius_mm`. A positive rounding radius identifies the circular
crest/root profile; a zero flat crest width is intentional for that form.
Nested hex nuts expose the same fit evidence through `inspect_mechanical_part`.
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

The shared recipe optionally accepts `representation_parameter`, an existing
Count parameter UUID. Exact count 0 selects Cosmetic envelope and 1 selects
Modeled helix. While bound, that input is the sole authority and a manual
`representation` is refused. Omitted or null binding fields preserve the saved
binding. `clear_representation_parameter: true` explicitly switches to fixed
representation; when no fixed choice is supplied it retains the current
effective choice. Setting and clearing a binding together is refused. Invalid
bindings never fall back to a dormant literal. The same source is used by
nested nut/screw recipes and existing per-instance symbol overrides.
[ADR-0037](0037-parameterized-thread-representation.md) records dependencies,
reader compatibility and semantic update conflicts.

For a listed UNC/UNF size, supply `fit: "unified_2a"` for external modes or
`fit: "unified_2b"` for an internal cut or nested hex nut. Omitting `fit` on
an edit retains its saved value. Creation defaults to `nominal_basic`, which
does not authorize UNC/UNF; those families require an explicit matching class.
The initial sourced coverage is uncoated, single-start UN with a flat root and
threaded length no greater than 1.5 nominal diameters. Unsupported size/series,
class/direction and allowance combinations fail before committing. These
historical table limits establish no current-ASME or calibrated-gage claim.
Custom flank angles require angle units, and radius taper requires ratio units.
A supplied Custom profile replaces the complete profile; omission retains it
on an edit, while creating a Custom thread requires the explicit profile.

Frame edits accept a principal plane or both explicit basis vectors.
Combining these choices, supplying only one vector, or supplying a
non-orthonormal basis fails. Omission retains the saved basis and origin.
Pitch, lead, handedness and starts retain their separate source meanings.

The tool prepares a canonical transaction, evaluates its candidate natively,
then commits or queues it under the owner's review policy.
After native evaluation, the final exclusive owner operation revalidates the
prepared session and revision before either action. A stale candidate cannot
be queued as a new review item following an owner edit or session replacement.
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

## Screw connection control

| Tool | Mutation | Meaning |
| --- | --- | --- |
| `set_screw_connection` | yes | Create or edit the complete canonical connection in one native-checked transaction |
| `inspect_screw_connection` | no | Return the editable canonical request and normalized owned references |
| `remove_screw_connection` | yes | Remove owned hardware, receiver features, axis and connection record |

Source: [connection tools](../../crates/multicad-mcp/src/tools/screw_connections.rs).
The generated tool catalog owns the outer argument schemas.
`set_screw_connection` requires `expected_revision` (nonnegative integer) and
`request` (the complete canonical `ScrewConnectionRequest` object). It is a
replacement request, not an omitted-field patch: use the request returned by
inspection, change its authored values, then submit the current revision.
The canonical request has these required fields; omission and `null` do not
mean “retain”. Source: `model/screw_connections.rs` in `multicad-core`.

| Field | JSON value | Meaning |
| --- | --- | --- |
| `connection` | UUID string | Stable connection identity; new for creation, retained for an edit |
| `name` | string | Connection display name |
| `mode` | string | `through_nut`, `tapped_receiver`, `direct_wood`, `wood_insert`, or `hanger_bolt`; choosing a mode does not establish source or native support |
| `axis` | object | `id`, exact `source` axis definition and retained `face` capture |
| `hardware` | array, 1-64 rows | Ordered rows with `role_id`, `role`, `name`, `source`, `instance` and `driver` |
| `receivers` | array, 1-16 rows | Receiving requests with `feature`, `name` and full typed `source` |
| `parameters` | object | Typed `grip`, `minimum_engagement` and `blind_tip_clearance` dimensions, plus optional `end_reference` |

Hardware roles are `screw`, `hanger_bolt`, `head_washer`, `nut_washer`, `nut` and
`insert`. A hardware source is `{"kind":"generated","part":...}` using the canonical
mechanical-part request, or `{"kind":"library","definition":...,"placement":...}` using
a pinned symbol definition and canonical placement request. These nested
objects use the shared core serializers, not the shorter recipe arguments of
`set_mechanical_part` or `place_symbol`. A Library connection role must resolve
to a single mechanical Part; arbitrary multi-part symbols are refused.

Explicit engineered wood screws, female wood inserts and hanger bolts use the
canonical mechanical source `{"kind":"custom_connection","parameters":...}`.
Its nested `parameters.connection_kind` selects `wood_screw`, `wood_insert` or
`hanger_bolt`. These sources retain their actual modeled thread starts and
complete intervals. They remain distinct from the manufacturer
`wood_screw` envelope, whose actual thread and point are unknown.
`inspect_mechanical_part` reports the custom starts, intervals, frame and
resolved profile scalars; this source inspection does not perform native
validation or establish purchased-part identity.

A literal dimension is `{"value":10,"unit":"mm"}`. An expression dimension contains
`expression`, whose bound source retains its unit and explicit parameter UUID
bindings. The receiving source retains its output body, frozen input body and
producer, occurrence path, shared axis, station, round or tapped cut, through or
blind extent, minimum wall, and optional counterbore or countersink seat.
Through extent is `{"kind":"through"}`; blind extent additionally requires
`depth` and `floor`. No field infers a missing usable insert-thread interval or
silently converts wood intent to a metric machine thread.

Captured faces make the connection follow receiving geometry changes. These
optional fields use the same canonical face captures as the graphical editor:

| Field | JSON value | Omitted or `null` | Constraints and meaning |
| --- | --- | --- | --- |
| `parameters.end_reference` | object with `receiver`, `occurrence`, `face` | Use the required manual `grip` dimension | `receiver` contains `body` and frozen `producer` UUIDs; `occurrence` contains `root` (UUID) and `members` (ordered UUID array); `face` contains `source` (UUID) and `intent` (geometry intent). The receiver and occurrence must match a selected receiving request. The current native planar face determines the end of the clamped clearance stack. `grip` must be the literal `{"value":0,"unit":"mm"}`. |
| `receivers[].source.entry_reference` | canonical face-capture object | Use the required manual `station` dimension | The face must belong to this receiving cut's frozen input producer. Its current native plane determines the cut entry along the shared axis. `station` must be the literal `{"value":0,"unit":"mm"}`. |

The clamped grip excludes the tapped or wood receiving material. A single
tapped or wood receiver can therefore use zero grip. A captured grip end must
be parallel to the entry face and lie on or beyond it in the fastening
direction. Native material checks still prove the actual contiguous stock;
the face distance alone does not establish a valid connection. Outputs omit
absent captures. On a replacement request, omission or `null` clears the
capture rather than retaining it. Source:
[connection parameters and grip resolution](../../crates/multicad-core/src/model/screw_connections.rs),
[receiving entry resolution](../../crates/multicad-core/src/model/connection_receivers.rs).

Saved hardware drivers derive stations, component mounts and metric helix
phase from the connection's current canonical providers. Changing a retained
parameter or Library override updates those poses without creating another
connection. The driver stores zero station, zero roll and an identity mount
when its connection binding is present; conflicting manual pose values are
refused. Missing or cyclic suppliers require repair. Source:
[hardware placement](../../crates/multicad-core/src/model/screw_hardware_placement.rs),
[connection kinematics](../../crates/multicad-core/src/model/screw_connection_kinematics.rs).

A retained connection identity edits the existing connection. Typed dimensions,
source choices, ordered hardware, receiver intent and stable identities retain
the canonical editor's meanings and validation; no transport-specific recipe
is substituted. Missing or invalid canonical fields refuse the request.

Inspection requires `connection` (UUID string) and returns `revision`,
`connection`, `request` and `definition`. Removal requires that UUID and
`expected_revision`. Unknown outer fields are refused. Removal refuses foreign
or downstream ownership conflicts rather than deleting unrelated geometry.
Both mutations compile the prepared native candidate before committing once;
invalid source, native failure, cancellation and stale revision leave the
canonical document unchanged. Set returns `revision`, `connection`, `native_geometry_checked: true` and the
current native `checks`; removal returns `revision` and
`native_geometry_checked: true`. `evaluate_model.screw_connections` returns
the same current-model receipts. This reports native evaluation, not strength,
installation suitability or standards certification. Native validation refuses
models beyond its 64-connection or 4096 candidate Boolean collision-query
budgets; disjoint native bounding boxes do not consume collision queries.

The CLI accepts these same argument objects. For an edit, save the inspected
`request` object in a `set_screw_connection` envelope and invoke:

```sh
multicad --command edit-connection.json --project owned.tcd --save
```

## Mechanical part control

`mechanical_part_profiles` reports generic geometry coverage and its limits.
`set_mechanical_part` creates or edits a washer, hex nut, headed machine screw or bearing mounting
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

Machine screws retain separate head and drive kinds. `socket_cap` heads accept
diameter and height; `external_hex` heads accept across-flats and height. Drives
are `hex_socket` with across-flats and depth, or `external_hex`. Omitted edit
fields retain source. Switching a head or drive kind requires every dimension
of the new kind. The generated schema owns these argument shapes.

`underhead_length` runs from the bearing plane to the tip. Nested `thread.length`
is the threaded extent ending at that tip, and `thread.frame` locates the
underhead bearing plane. The head extends opposite the frame normal. A shorter
thread leaves a plain shank before its start. Default custom screw dimensions
are M3 with 10 mm underhead and thread lengths, 3 mm shank, a 5.68 mm diameter
head of 3 mm height, and a 2.55 mm hex socket of 1.3 mm depth. The recess has a
flat floor; tooling bottoms, fillets and edge treatments are omitted. These
editable dimensions do not establish catalog membership or fitted metric
tolerance compliance. `inspect_mechanical_part` reports head/drive dimensions,
thread start and extent, representation, and separate nominal/fitted thread
evidence. Use `evaluate_model` to check native geometry.

Cosmetic nut and screw threads are refused for STEP, mesh and Print manufacturing output
until their representation is `modeled`. A bearing envelope contains only its
external annular mounting geometry; exported geometry does not include races,
rolling elements or seals. No fit, load or standards compliance is implied.

`inspect_mechanical_part` reads retained source, resolved dimensions and whether
they match an optional literal source reference. `attribution` separates the
assigned material from the catalog description and names the basis: `authored`,
`reference_dimensions`, or `custom_derived`. A mapped Library feature also
returns its `library_pin` with a hexadecimal `content_hash` and the revision.
Placement does not change the dimension comparison. A reference match does not
certify catalog membership or stock. Use `evaluate_model` to check native geometry.

## Pinned symbol control

`import_symbol`, `place_symbol` and `edit_symbol_instance` use the canonical
[symbol transactions](../../crates/multicad-mcp/src/tools/symbols.rs). The generated
`tools/list` schema owns their field types. Imported source remains pinned and
works after the Library file becomes unavailable. `get_entity` reads the raw
embedded definition and its exposed template parameters.

`import_symbol` reads one reusable definition from a bare `.tcd` file name in
the configured exchange folder. Paths escaping that folder, including symlink
targets, are refused. `source_definition` selects the source extension ID;
omission requires exactly one definition in the file. The result supplies the
embedded definition ID, pin and exposed template parameter IDs. Import validates
and embeds source without placing or evaluating the template solid.

`place_symbol` instantiates an embedded definition with a required `overrides`
map. Each key names an exposed parameter in the pinned template, and each value
is a finite literal with a compatible unit. An empty map uses pinned defaults.
Optional placement supplies typed translation lengths and an x/y/z/w unit
quaternion. Architectural construction paths use their dedicated draw tools.

`edit_symbol_instance` takes an instance or ownership-record ID and replaces its
complete override map. Omitted keys inherit pinned defaults; `{}` restores every
default. Preserve all desired overrides when changing one value. Materialized
private parameter and feature IDs cannot be used as public override keys or
edited directly through `set_parameter` or `set_mechanical_part`. Configuration
preserves the pin, placement and materialized identities. Placement and editing
evaluate the complete candidate natively before one attributable commit or
owner review. Invalid geometry, stale revisions and cancellation leave source
unchanged.

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

For rotary gears, the module system selects the convention for both module and
pressure angle. Worm recipes use axial module with a normal pressure angle, as
defined below.
External cylindrical gears also accept `pitch_input: "diametral_pitch"`.
`module` remains the one active authored scalar: a length for the default
`pitch_input: "module"`, or a `teeth_per_in` value for diametral pitch. For
example, `{"pitch_input":"diametral_pitch","module":{"value":12.7,"unit":"teeth_per_in"}}`
represents a 2 mm module in the selected plane. Changing the marker requires a
compatible authored scalar; it does not convert or reinterpret a saved length.
Omitted fields preserve the original convention, formula and bindings.
Helix angle is a positive magnitude with separate handedness. Design backlash
is tangential tooth-thickness reduction **per gear** at its reference circle.
Fabrication allowance adds further tangential thinning; neither changes the
reference, base or tip circles. A pair's total backlash is not represented by
a single gear's setting.

Current geometry covers external spur, helical, continuous-center
herringbone and gapped double-helical gears with a through bore and optional
intrinsic `hub { diameter, axial_start, axial_end }`. Hub dimensions use the
member's local Z frame, with its tooth band at `0..face_width`. Omitted/null
hub edits retain source; `clear_hub: true` removes it. Clear and replacement
together fail. The [retained gear contract](0035-retained-gear-pitch-hubs-and-shift.md)
owns the overlap, bore-wall and root-clearance limits. Root fillets
are custom circular arcs, not a generated cutter trochoid. The separate
[internal-ring subset](gears.md#internal-ring-subset) supports unshifted
20-degree 48/60-tooth spur rings with a 24-tooth pinion. `outer_diameter` is
required for a ring and omitted for external recipes. Omitted/null edits
retain it; `clear_outer_diameter: true` explicitly clears it when switching to
an external family. Clearing and replacement together fail. Ring
`bore_diameter` must be zero; the toothed opening is derived from module and
count. `inspect_gear` reports outside diameter and remaining radial rim.
The [finite straight rack subset](gears.md#finite-straight-rack-subset) uses
`family: "rack"`, whole pitch-cell `teeth` and rack-only `backing_thickness`.
Creating or switching to Rack defaults backing to 4 mm and clears inapplicable
rotary defaults before applying explicit arguments. Explicit unsupported bore,
helix or outside diameter still refuses. Omitted/null backing edits retain the
saved value. Inspection reports actual rack pitch, length and backing, with
no invented rotary diameters. Nonzero `profile_shift` is available for the four
external cylindrical families within the selected-plane engineering bound
`-0.5..1`, further limited by exact basic-rack undercut, crest/root separation,
actual-fillet contact and native geometry checks. Pressure retains the existing
selected-plane bounds. A standalone gear may retain
`shift_mate { teeth, profile_shift }` to declare sourced clearance-matched tip
shortening. `clear_shift_mate: true` removes that context; omission/null retains
it for a standalone edit, and clear plus replacement fails. Matched pairs retain
both opposite original typed sources and require atomic shared edits. Shifted
non-Spur pairs require matching selected module systems. Internal ring, Rack,
Bevel and Worm still refuse nonzero shift. The
[current retained-gear decision](0035-retained-gear-pitch-hubs-and-shift.md#2026-11-06-extend-retained-shift-to-external-helical-families)
owns the normal/transverse center, phase, contact and interference equations.
The worm recipe contract below does not establish product availability;
`gear_profiles.families[].supported` reports the current family gate.
Zero shift without matched context keeps the conservative normal-angle count
policy. Active shifted/matched intent uses the direct transverse basic-rack
projection, with no approximate virtual count. The custom fillet is not a hob-tip
radius. `model.gear.shifted_external.1` protects expanded saved intent, including
suppressed/template data; it is separate from pitch-unit and coverage gates. No
fit, accuracy grade, load rating or cutter-generation certification follows from
a successful preview.

The [spherical-involute straight bevel subset](gears.md#spherical-involute-straight-bevel-subset)
uses `family: "bevel"` and `bevel_system: "spherical_involute_straight"`.
Standalone creation requires explicit `mate_teeth` and `shaft_angle`.
The origin denotes the pitch apex, and `face_width` is measured along the pitch
cone. Inspection returns cone angles, cone distances, heel/toe radii and toe
bore wall, with nominal, design and physical tooth thickness reported separately.
It does not synthesize cylindrical spur base or tip diameters.
The architecture reference owns the bounded envelope and primary source pages;
Gleason/octoid, spiral and Zerol values are rejected.

The [generated ZA worm-wheel construction](0034-generated-za-worm-wheel.md)
uses `family: "worm"` for both members. `worm_role` selects `"worm"` or
`"wheel"`; `worm_system` selects `"za_axial_straight"`. Standalone creation
requires explicit `worm_role` and `mate_teeth`. ZI, ZN and ZK enum values are
recognized but refused. Creating or switching to the family establishes the
bounded defaults before applying explicit arguments. Explicit unsupported
arguments still fail. Same-family edits retain omitted or null fields, including
member role, mate source, quotient and axial dimensions.

`module_system: "axial"` means axial `module` with **normal** `pressure_angle`
of 20 degrees. Axial pressure and lead angle are derived; the authored
`helix_angle` remains zero. The recipe requires one right-hand worm start,
`diameter_quotient` 10 and axial module 0.5-2 mm. `teeth` is the member's own
count: one for the worm or 32-64 for the wheel. `mate_teeth` holds the opposite
count. `face_width` independently denotes worm threaded length, bounded to
16-32 times module, or wheel axial width, bounded to 0.5-2 times module. Each
member's `backlash` plus `fabrication_allowance` is nonnegative and no greater
than 0.1 times module. Nonzero center gap, profile shift or authored root-fillet
ratio is refused, as are unrelated rack, bevel or outside-diameter inputs. A bore must
leave the requested positive minimum wall at the minimum root. The member
origin is its axial center, with local Z along its shaft.

Worm inspection reports the declared member and system, axial pitch and lead,
normal and derived axial pressure angles, derived lead angle, reference/tip
radii and minimum root radius. It preserves nominal, design and physical
thickness evidence and reports generating-hob pitch-space thickness for the
wheel. It does not invent a cylindrical involute base radius. The construction
reference owns the surface equations and source limitations; no efficiency,
load, accuracy grade or physical contact certification follows from a preview.

Rotary involute profiles use a C1 cubic Hermite approximation to the analytic
involute, bounded by the active geometry policy's linear tolerance. Tip and
root arcs and circular fillets remain native circular curves. Helical flanks
follow a continuous twist, rather than a linear loft between two end profiles.

The tool prepares the canonical transaction, evaluates its candidate with the
native kernel, then commits or queues it under the owner's review policy.
Unsupported families, invalid units or dimensions, inadequate bore walls,
native failures, cancellation and stale revisions leave the document unchanged.

`set_gear_pair` creates two named gear Parts, a reusable Assembly, its internal
relationships and one placed occurrence in a single transaction. Creation
requires a name. A rotary first member defaults to 20 teeth, or 24 when its
requested family is bevel. For external pairs, the second inherits the first
recipe with 30 teeth and opposite helix hand, while defaulting to no hub, zero
profile shift and no explicit matched context. It then applies explicit
second-member overrides and derives any required matched context from the
opposite original typed sources. An internal-ring first member
instead defaults its second member to a 24-tooth spur pinion without an
outside-diameter field. A first rack also defaults its second member to a
24-tooth spur pinion, without inheriting rack-only backing. Either member may
be the rack when explicit recipes are supplied. Spur members use hand `none`.
A worm-family first member requires explicit `first.worm_role` and defaults its
own count to one worm start or 32 wheel teeth. Its second
member defaults to the opposite role, with 32 wheel teeth and 1 mm wheel width,
or one worm start and 24 mm threaded length. These are module-1 creation
defaults; a different module may require explicit axial dimensions. Both use
right hand. Member-specific `face_width` values remain independent.

An edit retains omitted fields independently for each member. Supplying both
recipes changes them atomically, so a shared module change can keep the pair
compatible. A drive-only edit preserves the gear definitions exactly. Explicit
`null` has the same effect as omission. Edits retain names and output identities.
`inspect_gear_pair` reads source and derived center distance, phases and frames;
for racks, `rack_travel` supplies member roles, allowed translation/drive ranges,
current translation and the pinion tooth period. Here center distance means
pitch-line-to-pinion-center distance.
`resolved.shifted_gear` reports working pressure, center, tip shortening and
transverse contact ratio for any matched shifted external family. The legacy
`resolved.shifted_spur` field remains populated only for Spur.
For bevels, `resolved.bevel` reports the shared shaft angle, cone distance,
cone face width and both pitch-cone angles. Their shared apex makes
`center_distance_mm` zero.
For worm pairs, `resolved.worm` identifies the worm and wheel members,
wheel count, center distance, crossed shaft angle and full worm-turn period.
`evaluate_model` reports actual native pair checks and physical leaf paths.
Each pair includes `cycle_reused`, `sampled_cycle_ms` and `current_pose_ms`.
Reuse skips only completed sampled-cycle work; the current native pose is
checked on every full evaluation. `sampled_cycle_ms` is zero on a hit.
The additional `native_timings` object reports successful compiler wall times
in milliseconds: `total_ms`, `gear_construction_ms`,
`assembly_and_occurrence_ms`, `shape_publication_ms` and `pair_validation_ms`.
Pair validation contains the per-pair sampled/current timings. The other
buckets are partial and need not sum to total; they exclude later manufacturing
serialization and OS paint. Failed/cancelled evaluations return their usual
error rather than a successful timing object. These are diagnostic measurements,
not latency guarantees or new geometry evidence.
Application and standalone host ownership, bounded reuse and publication are
defined in [ADR-0036](0036-shared-gear-cycle-evidence.md).
For shifted Spur/Helical, named-flank proximity may return
`shifted_spur_flank_faces` or `shifted_helical_flank_faces`; a candidate miss uses
`whole_solid_fallback`. Shifted chevrons require both bands independently and
report `shifted_chevron_both_bands` with the farther accepted band witness.
Whole-solid overlap checks run first. All face ordinals are evaluation-local;
these are existence witnesses, not global minima or continuous-contact claims.

For matched shifted external cylindrical pairs, omitted mate fields follow the
edited opposite member's original typed tooth-count and shift sources. Explicit mate fields must
match those sources before normalization; equal evaluated values with different
bindings are conflicting intent. To remove matched-profile intent, supply
literal-zero shifts and `clear_shift_mate: true` for both members. A drive-only
edit leaves both recipes intact.

Bevel pairs derive each member's `mate_teeth` from the opposite original typed
`teeth` source. Omitted or null mate counts follow that source, including changes
to the opposite count during a geometry edit. An explicit mate count must match
the opposite typed source exactly; equal evaluated values from different
expressions do not match. A literal cannot replace an expression implicitly.

An explicit `shaft_angle` on either member supplies the shared typed shaft
source. Two explicit shaft arguments must be identical typed sources; conflicting
bindings are refused even when both evaluate to the same angle. With both omitted
or null, edits retain the saved shared source and creation defaults to 90 degrees.
After reconciling intent, the tool clones the coupled dimensions into both
recipes and validates them before native work. A conflict leaves source,
transaction history and Undo state unchanged. Unsupported systems and mixed
bevel/non-bevel pairs also fail without a partial update.

The [pair tool](../../crates/multicad-mcp/src/tools/gear_pairs.rs) derives each
worm member's `mate_teeth` from the opposite original typed
`teeth`, and both diameter quotients from the declared worm's original typed
`diameter_quotient`. Explicit mate sources must match the opposite typed count
exactly. An explicit wheel quotient must match the worm's typed quotient,
including its expression binding; equal evaluated values alone do not match.
Omitting or passing null for these coupled dimensions derives them again after
a count or quotient edit. Other omitted member fields retain their independent
saved values. A conflicting source, identical member roles or mixed worm/non-worm
recipes fails before native work without changing source or history.

Pair origin and axes control layout within the Assembly definition. Member
frames describe their local Part definitions; derived anchors maintain mating
placement when those local frames change. The returned occurrence supports
normal component Move/Copy. Copies share the definition's internal drive angle
and keep independent outer placements.

External pairs require matching family, transverse module and pressure angle,
face width and center gap, with equal helix magnitude and opposite hands where
applicable. The bounded internal-ring/spur subset requires matching transverse
module, 20-degree pressure angle and face width, with zero helix/gap. Its centers
use the pitch-radius difference and its members rotate in the same direction.
A rack/spur pair requires matching module, pressure and width and zero helix,
with the finite travel and drive convention defined in the rack reference.
Rack pairs add both finite endpoints and native proximity checks. Bevel pairs
share their pitch apex and use the shaft/ratio convention in the bevel reference.
They retain no-overlap and engagement proximity checks at every sample.
Worm pairs use matching axial module and normal pressure, opposite member roles
and a shared generating recipe. `drive_angle` always rotates the worm, regardless
of member order; the wheel follows the one-start ratio. The drive coordinate is
bounded to plus or minus 1000 worm turns. Worm evaluation checks the current pose
and nine phases over one full worm turn, retaining whole-solid no-overlap and
engagement proximity checks.
Their current Part heads must remain the gear generators. For non-worm pairs,
native evaluation checks the current pose and nine phases over one pinion tooth cycle.
Unchanged geometry can reuse a completed cycle check; the current pose is always
checked. These samples do not establish continuous collision-free motion, load
capacity or manufacturing fit.

For bevel and worm checks, `current_engagement` records an actual native witness with
`distance_mm`, `kind`, `faces` and `points_mm`. The `bevel_flank_faces`
and `worm_flank_faces` kinds identify a close pair of named flank faces after
whole-solid no-overlap.
This is an existence witness within total authored thinning plus ten linear
tolerances; it does not report the global minimum distance. A candidate-face miss invokes `whole_solid_fallback`, whose `faces`
field is null. Face ordinals are evaluation-local; persistent references use
source ancestry instead. The two points use the evaluated pair layout frame
before independent outer occurrence placement. These witnesses retain the
same sampled cycle and current-pose checks; they do not establish
continuous contact, loaded behavior or manufacturing fit.

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
